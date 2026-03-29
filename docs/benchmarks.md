---
layout: default
title: Benchmarks
---

# qhook Benchmarks

Performance benchmarks for qhook across deployment targets.

## Latest Results

> **Note:** Run `scripts/bench-cloudflare.sh` to generate your own numbers. Results vary by region, time of day, and D1 load.

### Cloudflare Containers + D1

| Metric | Smoke (10 VU) | Load (50 VU) | Stress (200 VU) |
|--------|---------------|--------------|-----------------|
| **Ingestion RPS** | — | — | — |
| **Ingestion p50 (ms)** | — | — | — |
| **Ingestion p95 (ms)** | — | — | — |
| **Ingestion p99 (ms)** | — | — | — |
| **Delivery e2e p50 (ms)** | — | — | — |
| **Delivery e2e p95 (ms)** | — | — | — |
| **Pull msgs/sec** | — | — | — |
| **Pull poll p95 (ms)** | — | — | — |
| **Ack p95 (ms)** | — | — | — |
| **D1 write p95 (ms)** | — | — | — |
| **D1 read p95 (ms)** | — | — | — |
| **Error rate** | — | — | — |

### Self-Hosted (SQLite, single instance)

| Metric | Smoke (10 VU) | Load (50 VU) | Stress (200 VU) |
|--------|---------------|--------------|-----------------|
| **Ingestion RPS** | — | — | — |
| **Ingestion p95 (ms)** | — | — | — |
| **Delivery throughput/sec** | — | — | — |
| **Error rate** | — | — | — |

### Self-Hosted (Postgres, single instance)

| Metric | Smoke (10 VU) | Load (50 VU) | Stress (200 VU) |
|--------|---------------|--------------|-----------------|
| **Ingestion RPS** | — | — | — |
| **Ingestion p95 (ms)** | — | — | — |
| **Delivery throughput/sec** | — | — | — |
| **Error rate** | — | — | — |

---

## Test Environment

### Cloudflare Containers + D1

| Parameter | Value |
|-----------|-------|
| **Platform** | Cloudflare Containers (beta) |
| **Database** | Cloudflare D1 (SQLite-compatible) |
| **D1 connection** | Outbound Workers proxy (`d1_endpoint`) |
| **Container instances** | 1 |
| **qhook version** | v0.5.0 |
| **Region** | — |
| **k6 runner location** | — |
| **Date** | — |

### Self-Hosted

| Parameter | Value |
|-----------|-------|
| **Machine** | — (e.g., c5.xlarge, M1 Mac, etc.) |
| **CPU / RAM** | — |
| **Database** | SQLite / Postgres |
| **qhook version** | v0.5.0 |
| **Date** | — |

---

## Test Scenarios

### 1. Webhook Ingestion Throughput

Measures how fast qhook can accept incoming webhooks and persist them to the database.

- **Endpoint:** `POST /events/{source}/{event_type}`
- **Payload:** ~300B JSON (Stripe checkout event shape)
- **What is measured:** HTTP response time (event accepted + persisted)
- **D1 operations per request:** 1 INSERT (events) + N INSERTs (jobs, one per matching handler)

### 2. Push Delivery Latency

Measures end-to-end time from event ingestion to handler delivery.

- **Flow:** POST event → qhook persists → queue worker polls → HTTP delivery
- **What is measured:** Time from event POST to job creation (visible via API)
- **Note:** Actual delivery to external handler adds network latency on top

### 3. Pull-Mode Queue Throughput

Measures consumer performance using the pull-mode queue API.

- **Endpoints:** `GET /api/queues/{name}/messages` + `POST /api/queues/{name}/ack`
- **Concurrent consumers:** Matches VU count (10 / 50 / 200)
- **Batch size:** 10 messages per poll
- **Long-poll wait:** 1 second
- **Sub-tests:**
  - Concurrent consumer throughput (msgs/sec, ack latency)
  - Visibility timeout recovery (messages reappear after unacked timeout)
  - Rapid poll stress (no wait, max poll rate)

### 4. D1 Performance Under Load

Isolates D1 read/write performance.

- **Write:** Rapid event ingestion measuring INSERT latency
- **Read:** Concurrent `GET /api/events?limit=20` queries
- **Purpose:** Identify D1 as a bottleneck vs qhook application overhead

---

## Comparison Framework

Use this framework to compare qhook against alternatives or across deployment targets.

### vs Self-Hosted (same qhook, different infra)

| Dimension | Cloudflare + D1 | VPS + SQLite | VPS + Postgres |
|-----------|-----------------|--------------|----------------|
| Ingestion RPS | — | — | — |
| p95 latency | — | — | — |
| Delivery throughput | — | — | — |
| Monthly cost (est.) | — | — | — |
| Ops effort | Managed | Self-managed | Self-managed |

### vs Other Webhook Platforms

| Dimension | qhook | Svix | Hookdeck | AWS EventBridge |
|-----------|-------|------|----------|-----------------|
| Ingestion RPS | — | — | — | — |
| p95 latency | — | — | — | — |
| Pull-mode support | Yes | Yes | No | No |
| Self-hostable | Yes | Yes (OSS) | No | No |
| Min monthly cost | $0 (self-hosted) / $5 (CF) | $0 (self-hosted) | $25 | Pay-per-use |

> **Note:** External platform numbers should come from their published benchmarks or your own testing. Ensure comparable conditions (same region, similar payload sizes).

---

## Running Benchmarks

### Cloudflare Containers + D1

```bash
# Prerequisites: wrangler configured, qhook deployed, k6 installed

# Quick smoke test
QHOOK_URL=https://qhook.your-domain.com ./scripts/bench-cloudflare.sh smoke

# Full load test
QHOOK_URL=https://qhook.your-domain.com ./scripts/bench-cloudflare.sh load

# Stress test
QHOOK_URL=https://qhook.your-domain.com ./scripts/bench-cloudflare.sh stress

# Pull-mode only
SKIP_INGEST=1 SKIP_DELIVERY=1 SKIP_D1=1 \
  QHOOK_URL=https://qhook.your-domain.com ./scripts/bench-cloudflare.sh load
```

### Self-Hosted (local)

```bash
# Uses the existing bench scripts
bash tests/bench.sh 1000 50        # 1000 requests, 50 concurrency
k6 run tests/bench.js              # k6 ramping test
cargo bench                        # Criterion micro-benchmarks
```

### Pull-Mode Only

```bash
# Standalone pull-mode benchmark against any qhook instance
k6 run \
  -e QHOOK_URL=http://localhost:8888 \
  -e QUEUE_NAME=my-queue \
  -e VUS=20 \
  -e DURATION=30s \
  -e BATCH_SIZE=10 \
  scripts/bench-k6-pull.js
```

---

## Interpreting Results

### Key Metrics

| Metric | What it means | Good | Acceptable | Investigate |
|--------|---------------|------|------------|-------------|
| **Ingestion p95** | Time to accept + persist a webhook | < 50ms | < 200ms | > 500ms |
| **Delivery e2e p50** | Median time from ingest to handler delivery | < 200ms | < 1s | > 5s |
| **Pull poll p95** | Time to fetch a batch of messages | < 100ms | < 500ms | > 2s |
| **Ack p95** | Time to acknowledge a batch | < 50ms | < 200ms | > 1s |
| **Error rate** | Percentage of failed requests | < 0.1% | < 1% | > 5% |

### D1-Specific Considerations

- D1 is SQLite-based with a single writer. Write throughput plateaus under high concurrency.
- Read replicas (D1 Smart Placement) can improve read throughput in multi-region setups.
- The Outbound Workers proxy mode has lower latency than the REST API mode for D1 access.
- Expect higher p99 latencies compared to local SQLite due to network hops.

### Known Limitations

- **D1 write concurrency:** Single-writer means write RPS has a ceiling. Monitor for `SQLITE_BUSY` errors.
- **Cloudflare Containers beta:** Performance characteristics may change as the platform matures.
- **Long-poll overhead:** Each long-poll connection occupies a thread. High consumer counts may hit connection limits.
- **k6 runner location:** Running k6 from a different region than the Cloudflare deployment adds network latency to all measurements.
