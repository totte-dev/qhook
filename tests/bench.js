// k6 load test for qhook
// Usage: k6 run tests/bench.js
// Env vars: QHOOK_URL (default: http://localhost:8888)

import http from "k6/http";
import { check } from "k6";
import { Rate, Trend } from "k6/metrics";

const BASE_URL = __ENV.QHOOK_URL || "http://localhost:8888";

const errorRate = new Rate("errors");
const eventLatency = new Trend("event_latency", true);

export const options = {
  scenarios: {
    // Ramp up to target RPS, hold, ramp down
    event_ingest: {
      executor: "ramping-arrival-rate",
      startRate: 10,
      timeUnit: "1s",
      preAllocatedVUs: 50,
      maxVUs: 200,
      stages: [
        { duration: "10s", target: 100 },
        { duration: "30s", target: 100 },
        { duration: "5s", target: 0 },
      ],
    },
  },
  thresholds: {
    http_req_duration: ["p(95)<200", "p(99)<500"],
    errors: ["rate<0.01"],
  },
};

export default function () {
  const payload = JSON.stringify({
    id: `bench-${__VU}-${__ITER}`,
    timestamp: new Date().toISOString(),
    data: { value: Math.random() },
  });

  const res = http.post(`${BASE_URL}/events/bench.test`, payload, {
    headers: { "Content-Type": "application/json" },
  });

  const success = check(res, {
    "status is 202": (r) => r.status === 202,
  });

  errorRate.add(!success);
  eventLatency.add(res.timings.duration);
}
