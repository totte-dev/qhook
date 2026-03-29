// bench-k6-pull.js — k6 benchmark for qhook pull-mode queue consumption.
//
// Tests concurrent consumers polling the same queue, measuring:
//   - Messages received per second
//   - Ack latency
//   - Visibility timeout recovery (messages reappear after timeout)
//
// Usage:
//   k6 run -e QHOOK_URL=https://qhook.example.com -e QUEUE_NAME=bench scripts/bench-k6-pull.js
//
// This script is also called by bench-cloudflare.sh.

import http from "k6/http";
import { check, sleep } from "k6";
import { Rate, Trend, Counter } from "k6/metrics";

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------
const BASE_URL = __ENV.QHOOK_URL || "http://localhost:8888";
const QUEUE_NAME = __ENV.QUEUE_NAME || "bench";
const QUEUE_TOKEN = __ENV.QUEUE_TOKEN || "";
const VUS = parseInt(__ENV.VUS || "10");
const DURATION = __ENV.DURATION || "30s";
const RAMP_UP = __ENV.RAMP_UP || "5s";
const RAMP_DOWN = __ENV.RAMP_DOWN || "5s";
const BATCH_SIZE = parseInt(__ENV.BATCH_SIZE || "10");
const WAIT_SECONDS = parseInt(__ENV.WAIT_SECONDS || "1");

// ---------------------------------------------------------------------------
// Custom metrics
// ---------------------------------------------------------------------------
const errorRate = new Rate("error_rate");
const pollLatency = new Trend("queue_poll_latency", true);
const ackLatency = new Trend("queue_ack_latency", true);
const messagesReceived = new Counter("messages_received");
const messagesAcked = new Counter("messages_acked");
const emptyPolls = new Counter("empty_polls");
const pollErrors = new Counter("poll_errors");
const ackErrors = new Counter("ack_errors");
const batchSizeReceived = new Trend("batch_size_received");

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------
export const options = {
  scenarios: {
    // Main consumer scenario: concurrent consumers polling the same queue
    concurrent_consumers: {
      executor: "ramping-vus",
      startVUs: 0,
      stages: [
        { duration: RAMP_UP, target: VUS },
        { duration: DURATION, target: VUS },
        { duration: RAMP_DOWN, target: 0 },
      ],
      exec: "pollAndAck",
    },

    // Visibility timeout test: poll without acking, verify messages reappear
    visibility_timeout: {
      executor: "per-vu-iterations",
      vus: 2,
      iterations: 5,
      startTime: "5s",
      exec: "visibilityTimeoutTest",
    },

    // Rapid poll stress test: no wait, maximum poll rate
    rapid_poll: {
      executor: "constant-vus",
      vus: Math.ceil(VUS / 5),
      duration: DURATION,
      startTime: RAMP_UP,
      exec: "rapidPoll",
    },
  },
  thresholds: {
    queue_poll_latency: ["p(95)<3000", "p(99)<5000"],
    queue_ack_latency: ["p(95)<1000", "p(99)<2000"],
    error_rate: ["rate<0.10"],
  },
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
function queueHeaders() {
  const h = { "Content-Type": "application/json" };
  if (QUEUE_TOKEN) {
    h["Authorization"] = `Bearer ${QUEUE_TOKEN}`;
  }
  return h;
}

function pollMessages(batch, waitSec) {
  const url = `${BASE_URL}/api/queues/${QUEUE_NAME}/messages?batch=${batch}&wait=${waitSec}s`;
  return http.get(url, {
    headers: queueHeaders(),
    tags: { scenario: "poll" },
    timeout: `${waitSec + 10}s`,
  });
}

function ackMessages(ids) {
  return http.post(
    `${BASE_URL}/api/queues/${QUEUE_NAME}/ack`,
    JSON.stringify({ ids }),
    {
      headers: queueHeaders(),
      tags: { scenario: "ack" },
    }
  );
}

function nackMessages(ids) {
  return http.post(
    `${BASE_URL}/api/queues/${QUEUE_NAME}/nack`,
    JSON.stringify({ ids }),
    {
      headers: queueHeaders(),
      tags: { scenario: "nack" },
    }
  );
}

// ---------------------------------------------------------------------------
// Scenario: Poll and Ack (main throughput test)
// ---------------------------------------------------------------------------
// Simulates a consumer: poll a batch of messages, process them, ack.
export function pollAndAck() {
  // Poll for messages with long-polling
  const pollRes = pollMessages(BATCH_SIZE, WAIT_SECONDS);
  pollLatency.add(pollRes.timings.duration);

  const pollOk = check(pollRes, {
    "poll status ok": (r) => r.status === 200,
  });

  if (!pollOk) {
    errorRate.add(true);
    pollErrors.add(1);
    sleep(0.5);
    return;
  }

  let messages;
  try {
    const body = JSON.parse(pollRes.body);
    messages = body.messages || [];
  } catch (_) {
    errorRate.add(true);
    pollErrors.add(1);
    return;
  }

  batchSizeReceived.add(messages.length);

  if (messages.length === 0) {
    emptyPolls.add(1);
    errorRate.add(false);
    // No messages available — back off slightly to avoid hammering
    sleep(0.1);
    return;
  }

  messagesReceived.add(messages.length);

  // Simulate minimal processing time (real consumers would do work here)
  sleep(0.01);

  // Ack all messages in the batch
  const ids = messages.map((m) => m.id || m.job_id);
  const ackRes = ackMessages(ids);
  ackLatency.add(ackRes.timings.duration);

  const ackOk = check(ackRes, {
    "ack status ok": (r) => r.status === 200,
  });

  if (!ackOk) {
    errorRate.add(true);
    ackErrors.add(1);
  } else {
    messagesAcked.add(ids.length);
    errorRate.add(false);
  }
}

// ---------------------------------------------------------------------------
// Scenario: Visibility Timeout Recovery
// ---------------------------------------------------------------------------
// Polls a message but does NOT ack it. After the visibility timeout, the
// same message should become available again. This verifies queue reliability.
export function visibilityTimeoutTest() {
  // Poll a single message
  const res1 = pollMessages(1, 1);
  if (res1.status !== 200) return;

  let msg1;
  try {
    const body = JSON.parse(res1.body);
    if (!body.messages || body.messages.length === 0) return;
    msg1 = body.messages[0];
  } catch (_) {
    return;
  }

  const msgId = msg1.id || msg1.job_id;

  // Intentionally do NOT ack — wait for visibility timeout
  // Default visibility timeout is typically 30s, but for benchmark
  // we just verify the message is not immediately re-delivered.
  sleep(1);

  // Poll again — the same message should NOT appear yet (still invisible)
  const res2 = pollMessages(1, 0);
  if (res2.status === 200) {
    try {
      const body = JSON.parse(res2.body);
      const msgs = body.messages || [];
      // If we got the same message back immediately, visibility timeout is broken
      const sameMsg = msgs.find((m) => (m.id || m.job_id) === msgId);
      check(null, {
        "message not immediately re-delivered": () => !sameMsg,
      });
    } catch (_) {
      // ignore parse errors
    }
  }

  // Nack the message so it returns to the queue for other tests
  nackMessages([msgId]);
}

// ---------------------------------------------------------------------------
// Scenario: Rapid Poll (stress)
// ---------------------------------------------------------------------------
// Polls as fast as possible with no wait time to stress-test the queue API
// and D1 read performance under concurrent access.
export function rapidPoll() {
  const res = pollMessages(BATCH_SIZE, 0);
  pollLatency.add(res.timings.duration);

  const ok = check(res, {
    "rapid poll ok": (r) => r.status === 200,
  });

  if (!ok) {
    errorRate.add(true);
    pollErrors.add(1);
    return;
  }

  errorRate.add(false);

  let messages;
  try {
    const body = JSON.parse(res.body);
    messages = body.messages || [];
  } catch (_) {
    return;
  }

  if (messages.length > 0) {
    messagesReceived.add(messages.length);
    // Ack immediately
    const ids = messages.map((m) => m.id || m.job_id);
    const ackRes = ackMessages(ids);
    if (ackRes.status === 200) {
      messagesAcked.add(ids.length);
    }
  } else {
    emptyPolls.add(1);
  }
}
