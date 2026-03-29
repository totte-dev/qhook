// bench-k6-cloudflare.js — k6 scenarios for qhook on Cloudflare Containers + D1.
//
// This script is invoked by bench-cloudflare.sh with the SCENARIO env var
// selecting which test to run: ingest, delivery, or d1.
//
// Usage (standalone):
//   k6 run -e QHOOK_URL=https://qhook.example.com -e SCENARIO=ingest scripts/bench-k6-cloudflare.js

import http from "k6/http";
import { check, sleep } from "k6";
import { Rate, Trend, Counter } from "k6/metrics";

// ---------------------------------------------------------------------------
// Config from env
// ---------------------------------------------------------------------------
const BASE_URL = __ENV.QHOOK_URL || "http://localhost:8888";
const SCENARIO = __ENV.SCENARIO || "ingest";
const VUS = parseInt(__ENV.VUS || "50");
const DURATION = __ENV.DURATION || "60s";
const RAMP_UP = __ENV.RAMP_UP || "10s";
const RAMP_DOWN = __ENV.RAMP_DOWN || "5s";
const WEBHOOK_SECRET = __ENV.WEBHOOK_SECRET || "";

// ---------------------------------------------------------------------------
// Custom metrics
// ---------------------------------------------------------------------------
const errorRate = new Rate("error_rate");
const eventLatency = new Trend("event_ingest_latency", true);
const deliveryLatency = new Trend("delivery_e2e_latency", true);
const d1WriteLatency = new Trend("d1_write_latency", true);
const d1ReadLatency = new Trend("d1_read_latency", true);
const eventsIngested = new Counter("events_ingested");
const eventsQueried = new Counter("events_queried");

// ---------------------------------------------------------------------------
// Scenario selection
// ---------------------------------------------------------------------------
const scenarioConfig = {
  ingest: {
    ingest_ramp: {
      executor: "ramping-vus",
      startVUs: 0,
      stages: [
        { duration: RAMP_UP, target: VUS },
        { duration: DURATION, target: VUS },
        { duration: RAMP_DOWN, target: 0 },
      ],
      exec: "ingestScenario",
    },
  },
  delivery: {
    delivery_ramp: {
      executor: "ramping-vus",
      startVUs: 0,
      stages: [
        { duration: RAMP_UP, target: VUS },
        { duration: DURATION, target: VUS },
        { duration: RAMP_DOWN, target: 0 },
      ],
      exec: "deliveryScenario",
    },
  },
  d1: {
    d1_write: {
      executor: "ramping-vus",
      startVUs: 0,
      stages: [
        { duration: RAMP_UP, target: VUS },
        { duration: DURATION, target: VUS },
        { duration: RAMP_DOWN, target: 0 },
      ],
      exec: "d1WriteScenario",
    },
    d1_read: {
      executor: "ramping-vus",
      startVUs: 0,
      stages: [
        { duration: RAMP_UP, target: Math.ceil(VUS / 2) },
        { duration: DURATION, target: Math.ceil(VUS / 2) },
        { duration: RAMP_DOWN, target: 0 },
      ],
      exec: "d1ReadScenario",
    },
  },
};

export const options = {
  scenarios: scenarioConfig[SCENARIO] || scenarioConfig.ingest,
  thresholds: {
    http_req_duration: ["p(95)<2000", "p(99)<5000"],
    error_rate: ["rate<0.05"],
  },
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
function makeStripePayload(vu, iter) {
  return JSON.stringify({
    id: `evt_bench_${vu}_${iter}`,
    object: "event",
    type: "checkout.session.completed",
    created: Math.floor(Date.now() / 1000),
    data: {
      object: {
        id: `cs_bench_${vu}_${iter}`,
        amount_total: 2000,
        currency: "usd",
        customer_email: `bench-${vu}@example.com`,
      },
    },
  });
}

function makeEventPayload(vu, iter) {
  return JSON.stringify({
    id: `bench-${vu}-${iter}`,
    timestamp: new Date().toISOString(),
    data: {
      value: Math.random(),
      vu: vu,
      iter: iter,
    },
  });
}

// ---------------------------------------------------------------------------
// Scenario: Webhook Ingestion
// ---------------------------------------------------------------------------
// Measures raw ingestion throughput — how fast qhook can accept webhooks
// and persist events to D1.
export function ingestScenario() {
  const payload = makeStripePayload(__VU, __ITER);

  const headers = { "Content-Type": "application/json" };
  if (WEBHOOK_SECRET) {
    // For benchmarking with signature verification enabled, the Stripe
    // signature header would need HMAC computation. k6 does not have native
    // HMAC support, so we use the event endpoint which skips verification.
  }

  // Use the event endpoint for benchmarking (no signature verification overhead)
  const res = http.post(
    `${BASE_URL}/events/bench-source/checkout.session.completed`,
    payload,
    { headers, tags: { scenario: "ingest" } }
  );

  const ok = check(res, {
    "status is 200 or 202": (r) => r.status === 200 || r.status === 202,
  });

  errorRate.add(!ok);
  eventLatency.add(res.timings.duration);
  if (ok) eventsIngested.add(1);
}

// ---------------------------------------------------------------------------
// Scenario: Push Delivery (end-to-end)
// ---------------------------------------------------------------------------
// Measures how quickly events flow through qhook:
//   POST event → qhook persists → queue worker picks up → delivers to handler.
//
// This is an approximation: we send an event, then immediately query
// the jobs API to see if the job was created. Actual delivery latency
// depends on the handler endpoint and queue worker poll interval.
export function deliveryScenario() {
  const payload = makeEventPayload(__VU, __ITER);

  const sendStart = Date.now();

  // Send event
  const res = http.post(
    `${BASE_URL}/events/bench-source/bench.delivery`,
    payload,
    {
      headers: { "Content-Type": "application/json" },
      tags: { scenario: "delivery" },
    }
  );

  const sendOk = check(res, {
    "event accepted": (r) => r.status === 200 || r.status === 202,
  });
  errorRate.add(!sendOk);
  eventLatency.add(res.timings.duration);

  if (!sendOk) return;

  // Parse event_id from response
  let eventId;
  try {
    const body = JSON.parse(res.body);
    eventId = body.event_id || body.id;
  } catch (_) {
    return;
  }

  if (!eventId) return;

  // Poll for job creation (measures persistence + job creation latency)
  for (let attempt = 0; attempt < 10; attempt++) {
    sleep(0.2);
    const jobRes = http.get(`${BASE_URL}/api/events/${eventId}/jobs`, {
      tags: { scenario: "delivery_poll" },
    });
    if (jobRes.status === 200) {
      try {
        const jobBody = JSON.parse(jobRes.body);
        if (jobBody.jobs && jobBody.jobs.length > 0) {
          deliveryLatency.add(Date.now() - sendStart);
          return;
        }
      } catch (_) {
        // continue polling
      }
    }
  }
  // Timed out waiting for job
  deliveryLatency.add(Date.now() - sendStart);
}

// ---------------------------------------------------------------------------
// Scenario: D1 Write Performance
// ---------------------------------------------------------------------------
// Measures D1 write throughput by sending events rapidly.
// Each event triggers a D1 INSERT (event) + INSERT (job).
export function d1WriteScenario() {
  const payload = makeEventPayload(__VU, __ITER);

  const res = http.post(
    `${BASE_URL}/events/bench-source/bench.d1write`,
    payload,
    {
      headers: { "Content-Type": "application/json" },
      tags: { scenario: "d1_write" },
    }
  );

  const ok = check(res, {
    "write accepted": (r) => r.status === 200 || r.status === 202,
  });

  errorRate.add(!ok);
  d1WriteLatency.add(res.timings.duration);
  if (ok) eventsIngested.add(1);
}

// ---------------------------------------------------------------------------
// Scenario: D1 Read Performance
// ---------------------------------------------------------------------------
// Measures D1 read throughput via the events list API.
export function d1ReadScenario() {
  const res = http.get(`${BASE_URL}/api/events?limit=20`, {
    tags: { scenario: "d1_read" },
  });

  const ok = check(res, {
    "read ok": (r) => r.status === 200,
    "has events": (r) => {
      try {
        const body = JSON.parse(r.body);
        return body.events && body.events.length > 0;
      } catch (_) {
        return false;
      }
    },
  });

  errorRate.add(!ok);
  d1ReadLatency.add(res.timings.duration);
  if (ok) eventsQueried.add(1);
}
