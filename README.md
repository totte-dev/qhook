# qhook

**SQS for webhooks and events** -- a lightweight event gateway with built-in queue and retry.

<!-- badges -->
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](#license)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org/)
[![CI](https://github.com/totte-dev/qhook/actions/workflows/ci.yml/badge.svg)](https://github.com/totte-dev/qhook/actions/workflows/ci.yml)

**[Documentation](https://totte-dev.github.io/qhook/)** | **[Examples](./examples/)** | **[Why qhook?](./docs/why-qhook.md)**

---

## Why qhook?

- **No infrastructure tax.** Single binary, no Redis, no RabbitMQ, no SQS. SQLite for local dev, Postgres for production.
- **Webhook verification built in.** GitHub, Stripe, Shopify, and generic HMAC -- signature checks happen before your app ever sees the payload.
- **Reliable delivery.** Exponential backoff retry with configurable limits. Dead Letter Queue for jobs that exhaust all attempts. Graceful shutdown drains in-flight deliveries.
- **Production ready.** Prometheus metrics, health checks with queue depth, auto-cleanup of old records, stale job recovery.
- **Idempotency.** Configurable dedup key (JSONPath) prevents double-processing of the same event.
- **CloudEvents native.** Automatically detects CloudEvents (binary and structured mode) and forwards `ce-*` headers to your handlers.
- **AWS SNS ready.** Receive events from SNS topics with automatic subscription confirmation and X.509 signature verification.

> See [docs/why-qhook.md](./docs/why-qhook.md) for a detailed before/after comparison.

## Quick Start

Run with Docker:

```bash
docker run -p 8888:8888 -v $(pwd)/qhook.yaml:/data/qhook.yaml ghcr.io/totte-dev/qhook
```

Create a minimal config:

```yaml
# qhook.yaml
database:
  driver: sqlite

server:
  port: 8888

sources:
  app:
    type: event

handlers:
  process-order:
    source: app
    events: [order.created]
    url: http://localhost:3000/jobs/order
    retry: { max: 5 }
```

Send a test event:

```bash
curl -X POST http://localhost:8888/events/order.created \
  -H "Content-Type: application/json" \
  -d '{"id": "ord_123", "amount": 4999}'
```

## Installation

### Docker

```bash
docker pull ghcr.io/totte-dev/qhook
```

### Cargo install

```bash
cargo install qhook
```

### Build from source

```bash
git clone https://github.com/totte-dev/qhook.git
cd qhook
cargo build --release
# Binary at ./target/release/qhook
```

## Configuration

qhook is configured via a single YAML file. Environment variables are expanded using `${VAR_NAME}` syntax.

```yaml
# qhook.yaml

database:
  driver: sqlite                    # sqlite (default) or postgres
  # url: ${DATABASE_URL}            # connection string (required for postgres)
  max_connections: 10               # DB pool size (default: 10)

server:
  port: 8888                        # listen port (default: 8888)
  max_body_size: 1048576            # max request body in bytes (default: 1MB)
  max_inbound: 100                  # max concurrent inbound requests (default: 100)
  ip_rate_limit: 100                # per-IP requests/sec limit (default: 0 = disabled)

api:
  auth_token: ${QHOOK_API_TOKEN}    # bearer token for /events endpoint (optional)

delivery:
  signing_secret: ${QHOOK_SIGNING_SECRET}  # sign outgoing deliveries (optional)
  timeout: 30s                             # delivery HTTP timeout (default: 30s)
  default_retry:
    max: 5                          # max delivery attempts (default: 5)
    backoff: exponential            # exponential (default) or fixed
    interval: 30s                   # base retry interval (default: 30s)

worker:
  stale_threshold_secs: 300         # recover stuck jobs after N seconds (default: 300)
  retention_hours: 72               # purge completed/dead records after N hours (default: 72)
  drain_timeout_secs: 30            # max wait for in-flight deliveries on shutdown (default: 30)

sources:
  stripe:
    type: webhook                   # webhook = external provider, event = internal API
    verify: stripe                  # signature verification: github | stripe | shopify | hmac
    secret: ${STRIPE_WEBHOOK_SECRET}

  github:
    type: webhook
    verify: github
    secret: ${GITHUB_WEBHOOK_SECRET}

  app:
    type: event                     # no verification, uses auth_token if set

  my-sns:
    type: sns                       # AWS SNS input (auto-confirms subscriptions)
    # skip_verify: true             # skip X.509 verification (for LocalStack / testing)

handlers:
  payment-success:
    source: stripe                  # must match a source name
    events:                         # event types to match (empty = all)
      - checkout.session.completed
      - invoice.paid
    url: http://backend:3000/jobs/payment  # delivery target URL
    type: http                     # http (default) or grpc
    retry: { max: 8 }              # override default_retry per handler
    timeout: 60s                   # override delivery timeout per handler
    idempotency_key: "$.id"        # JSONPath to dedup key in payload
    rate_limit: 10                 # max 10 deliveries/sec to this handler (optional)

  deploy-on-push:
    source: github
    events: [push]
    url: http://deployer:4000/deploy
    filter: "$.ref == refs/heads/main"   # only deploy on push to main

  on-sns-notification:
    source: my-sns
    events: [order.created]
    url: http://backend:3000/jobs/sns-order
    transform: |                         # reshape payload before delivery
      {"order_id": "{{$.data.id}}", "total": {{$.data.amount}}}

# Optional: alert on DLQ and verification failures
alerts:
  url: ${SLACK_WEBHOOK_URL}
  type: slack  # slack / discord / generic
  on: [dlq, verification_failure]
```

Generate a starter config:

```bash
qhook init
```

Validate without starting:

```bash
qhook validate
```

## Usage

### Start the server

```bash
qhook start                        # uses ./qhook.yaml
qhook start -c /etc/qhook.yaml     # custom path
```

### CLI commands

```bash
# Generate a default qhook.yaml
qhook init

# Validate config
qhook validate
qhook validate -c /path/to/qhook.yaml

# List jobs (filterable by status)
qhook jobs list
qhook jobs list --status dead
qhook jobs list --status completed --limit 50

# Retry failed jobs
qhook jobs retry                    # retry all dead jobs
qhook jobs retry <JOB_ID>          # retry a specific job

# List received events
qhook events list
qhook events list --limit 50
```

Job statuses: `available`, `running`, `completed`, `retryable`, `dead`.

## Webhook Verification

qhook verifies inbound webhook signatures before processing. Configure a source with `verify` and `secret`:

### GitHub

```yaml
sources:
  github:
    type: webhook
    verify: github
    secret: ${GITHUB_WEBHOOK_SECRET}
```

Checks the `X-Hub-Signature-256` header using HMAC-SHA256.

### Stripe

```yaml
sources:
  stripe:
    type: webhook
    verify: stripe
    secret: ${STRIPE_WEBHOOK_SECRET}
```

Checks the `Stripe-Signature` header (t=...,v1=... format) using HMAC-SHA256 with timestamp.

### Shopify

```yaml
sources:
  shopify:
    type: webhook
    verify: shopify
    secret: ${SHOPIFY_WEBHOOK_SECRET}
```

Checks the `X-Shopify-Hmac-SHA256` header using HMAC-SHA256 (base64-encoded).

### Custom HMAC

```yaml
sources:
  my-service:
    type: webhook
    verify: hmac
    secret: ${MY_WEBHOOK_SECRET}
```

Checks the `X-Webhook-Signature` header using HMAC-SHA256 (hex-encoded).

All signature comparisons use constant-time equality to prevent timing attacks.

## AWS SNS

qhook can receive events from AWS SNS topics. It handles the full lifecycle automatically:

1. **Subscription confirmation** -- when SNS sends a `SubscriptionConfirmation` message, qhook automatically confirms by fetching the `SubscribeURL`.
2. **Message unwrapping** -- SNS wraps your payload in an envelope. qhook extracts the `Message` field and delivers only the actual payload to your handlers.
3. **Signature verification** -- each SNS message is verified using the X.509 certificate from AWS (SHA1/SHA256).

### Setup

```yaml
sources:
  my-sns:
    type: sns
```

Point your SNS subscription to `https://your-qhook-host/sns/my-sns`. The event type is extracted from the message payload (`type` field, `detail-type` for EventBridge, or the SNS `Subject`).

### Testing with LocalStack

For local development, use `skip_verify: true` to bypass X.509 signature verification:

```yaml
sources:
  my-sns:
    type: sns
    skip_verify: true     # LocalStack does not sign messages
```

## CloudEvents

qhook automatically detects and handles [CloudEvents](https://cloudevents.io/) in both content modes.

### Binary mode

CloudEvents metadata is sent as HTTP headers (`ce-type`, `ce-source`, `ce-id`, etc.). The body contains the event data directly.

```bash
curl -X POST http://localhost:8888/events/ignored \
  -H "Content-Type: application/json" \
  -H "ce-type: com.example.order.created" \
  -H "ce-source: /shop" \
  -H "ce-id: evt-001" \
  -H "ce-specversion: 1.0" \
  -d '{"orderId": "ord_123"}'
```

The `ce-type` header overrides the event type from the URL path.

### Structured mode

The entire CloudEvents envelope is sent as JSON with `Content-Type: application/cloudevents+json`.

```bash
curl -X POST http://localhost:8888/webhooks/my-source \
  -H "Content-Type: application/cloudevents+json" \
  -d '{
    "specversion": "1.0",
    "type": "com.example.order.created",
    "source": "/shop",
    "id": "evt-001",
    "data": {"orderId": "ord_123"}
  }'
```

The `type` field from the envelope is used as the event type.

### Header forwarding

All `ce-*` headers from the original event are automatically forwarded to your handlers on delivery, so your app can access CloudEvents metadata without parsing the payload.

## Architecture

```
Webhook / SNS / Event     qhook                          Your App
                          +--------------------------+
                          |                          |
  POST /webhooks/stripe ---> Verify signature        |
  POST /sns/my-topic ------> Verify X.509 + unwrap   |
  POST /events/order ------> Auth token check        |
                          |   |                      |
                          |   v                      |
                          | Detect CloudEvents       |
                          |   |                      |
                          |   v                      |
                          | Store event (dedup)      |
                          |   |                      |
                          |   v                      |
                          | Create job(s)            |
                          |   |                      |
                          |   v                      |
                          | Queue worker ------------>  POST http://backend/jobs/payment
                          |   |                      |   (+ ce-* headers forwarded)
                          |   |-- success ----------->  mark completed
                          |   |-- failure (< max) -->  exponential backoff, retry
                          |   |-- failure (= max) -->  move to Dead Letter Queue
                          +--------------------------+
```

**Endpoints:**

| Route | Purpose |
|-------|---------|
| `POST /webhooks/{source}` | Receive external webhooks (signature verified) |
| `POST /sns/{source}` | Receive AWS SNS messages (X.509 verified, auto-confirms subscriptions) |
| `POST /events/{event_type}` | Receive internal events (bearer token auth, CloudEvents-aware) |
| `GET /health` | Health check (JSON: status + queue depth, 503 if DB unreachable) |
| `GET /metrics` | Prometheus metrics |

## Filtering & Transformation

### Event filtering

Use `filter` on a handler to only create jobs when the payload matches a condition:

```yaml
handlers:
  paid-only:
    source: stripe
    events: [invoice.*]
    url: http://backend:3000/paid
    filter: "$.data.object.status == paid"
```

Supported filter expressions:

| Syntax | Example | Description |
|--------|---------|-------------|
| `$.path == value` | `$.status == active` | Equality (strings, numbers, booleans) |
| `$.path != value` | `$.env != test` | Inequality |
| `$.path in [a, b]` | `$.type in [created, updated]` | Set membership |
| `$.path` | `$.data.verified` | Truthy (exists, not null/false/0/"") |

Nested paths work: `$.data.object.customer.email == user@example.com`

### Payload transformation

Use `transform` to reshape the payload before delivery:

```yaml
handlers:
  notify:
    source: stripe
    events: [checkout.session.completed]
    url: http://slack-bot:3000/notify
    transform: |
      {"text": "New order from {{$.data.object.customer_email}} for {{$.data.object.amount_total}} cents"}
```

- Placeholders use `{{$.path}}` syntax (same JSONPath as filter/idempotency_key)
- Original payload is preserved in the database; transformation is applied at delivery time
- Supports strings, numbers, booleans, arrays, and nested objects
- Missing fields resolve to `null`

## gRPC Output

Handlers can deliver events via gRPC instead of HTTP by setting `type: grpc`:

```yaml
handlers:
  grpc-handler:
    source: app
    events: [order.created]
    url: http://grpc-service:50051
    type: grpc
```

qhook calls the `qhook.v1.EventReceiver/Deliver` unary RPC. The proto definition is at [`proto/qhook.proto`](proto/qhook.proto) — use it to generate server stubs in your language.

The `DeliverRequest` includes:

| Field | Description |
|-------|-------------|
| `event_id` | ULID of the original event |
| `event_type` | Event type (from CloudEvents header or payload) |
| `handler` | Handler name from config |
| `payload` | JSON payload (original or transformed) |
| `metadata` | CloudEvents headers and qhook metadata |
| `attempt` | Current attempt number (1-based) |

Return `success: true` in `DeliverResponse` to mark delivery complete. `success: false` triggers retry with exponential backoff (same as HTTP non-2xx).

## Monitoring

### Health check

```bash
curl http://localhost:8888/health
# {"status":"ok","queue_depth":3}
```

Returns `200` with `queue_depth` when healthy, `503` if the database is unreachable. Use as a readiness probe in Kubernetes / ECS.

### Prometheus metrics

```bash
curl http://localhost:8888/metrics
```

Exposed metrics:

| Metric | Type | Description |
|--------|------|-------------|
| `qhook_events_received_total` | counter | Total events received |
| `qhook_events_by_source_total{source}` | counter | Events received per source |
| `qhook_events_duplicated_total` | counter | Duplicate events ignored |
| `qhook_jobs_created_total` | counter | Total jobs created |
| `qhook_deliveries_total{result}` | counter | Delivery attempts (success/failure) |
| `qhook_deliveries_by_handler_total{handler,result}` | counter | Delivery attempts per handler |
| `qhook_delivery_duration_seconds_sum` | counter | Total delivery duration |
| `qhook_delivery_duration_seconds_count` | counter | Total delivery attempts |
| `qhook_verification_failures_total{source}` | counter | Signature verification failures per source |
| `qhook_dlq_total{handler}` | counter | Jobs moved to DLQ per handler |
| `qhook_delivery_errors_by_type_total{type}` | counter | Delivery errors by type (4xx/5xx/timeout/network) |
| `qhook_delivery_duration_seconds_max` | gauge | Max single delivery duration |
| `qhook_db_errors_total` | counter | Database errors |
| `qhook_alerts_sent_total` | counter | Alert webhooks sent successfully |
| `qhook_alerts_failed_total` | counter | Alert webhooks that failed |
| `qhook_queue_depth` | gauge | Jobs waiting to be delivered |
| `qhook_dead_jobs` | gauge | Jobs in dead letter queue |
| `qhook_metric_label_count` | gauge | Unique label values (monitors for label explosion) |

No external dependencies -- metrics are formatted using atomic counters.

### Alerts

qhook can send webhook alerts when jobs are moved to the DLQ or signature verification fails. Configure in `qhook.yaml`:

```yaml
alerts:
  url: https://hooks.slack.com/services/T.../B.../xxx
  type: slack  # slack / discord / generic
  on: [dlq, verification_failure]
```

Supported alert types:
- **`generic`** (default): JSON payload with `alert`, `message`, and event-specific fields.
- **`slack`**: Slack incoming webhook format (`{ "text": "..." }`).
- **`discord`**: Discord webhook format with embeds (`{ "embeds": [...] }`).

### Structured logging

By default, qhook outputs human-readable logs. For production log aggregation (CloudWatch, Datadog, etc.), enable JSON logging:

```bash
QHOOK_LOG_FORMAT=json qhook start
```

## Security

- **Signature verification**: All supported providers use constant-time comparison to prevent timing attacks.
- **Stripe replay protection**: Stripe signatures older than 5 minutes are automatically rejected.
- **Request body limit**: Configurable max body size (default: 1MB) prevents memory exhaustion from oversized payloads.
- **Inbound concurrency limit**: Configurable max concurrent requests (default: 100) protects against overload.
- **Security headers**: All responses include `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, and `Cache-Control: no-store`.
- **Handler URL validation**: URLs are validated at config load (http/https only). Private IP ranges (10.x, 172.x, 192.168.x) trigger warnings.
- **Config validation**: `qhook validate` checks source references, URL formats, and alert configuration.
- **TLS**: qhook does not terminate TLS itself. Use a reverse proxy (nginx, Caddy, ALB) in front of qhook for HTTPS.

```yaml
server:
  port: 8888
  max_body_size: 1048576   # 1MB (default)
  max_inbound: 100         # max concurrent requests (default)
  ip_rate_limit: 100       # per-IP requests/sec (0 = disabled)
```

## Reliability

qhook is designed to not lose events, even during crashes or restarts.

- **Graceful shutdown**: On SIGTERM/SIGINT, qhook stops accepting new requests and waits for in-flight deliveries to complete before exiting.
- **Stale job recovery**: Jobs stuck in `running` for over 5 minutes are automatically recovered and retried (on startup and hourly).
- **Auto cleanup**: Completed and dead jobs older than 72 hours are automatically purged to prevent database growth.
- **Concurrent delivery**: Up to 10 jobs are delivered in parallel with adaptive polling (50ms when busy, 1s when idle).
- **Rate limiting**: Set `rate_limit` on a handler to cap deliveries per second and protect downstream services.
- **Multi-instance safe (Postgres)**: Uses `SELECT ... FOR UPDATE SKIP LOCKED` to prevent duplicate delivery across multiple qhook instances.

## Examples

| Example | Description |
|---------|-------------|
| [quickstart](./examples/quickstart/) | Minimal setup, no Docker needed. Run in 2 minutes. |
| [github-webhook](./examples/github-webhook/) | GitHub push/PR events with signature verification and fan-out. |
| [filter-transform](./examples/filter-transform/) | Event filtering (`$.status == paid`) and payload transformation. |
| [stripe-checkout](./examples/stripe-checkout/) | Stripe checkout with dual handlers (payment + fulfillment). |

## Deployment

Deployment examples are provided for common setups:

- [`docker-compose.yaml`](./docker-compose.yaml) -- Local development with SQLite
- [`docker-compose.prod.yaml`](./docker-compose.prod.yaml) -- Production with Postgres
- [`docs/deploy/aws.md`](./docs/deploy/aws.md) -- AWS (ECS Fargate / EC2)
- [`docs/deploy/railway.md`](./docs/deploy/railway.md) -- Railway
- [`docs/deploy/flyio.md`](./docs/deploy/flyio.md) -- Fly.io
- [`docs/deploy/render.md`](./docs/deploy/render.md) -- Render

### Docker quick reference

```bash
# Development
docker compose up

# Production with Postgres
DATABASE_URL=postgres://user:pass@db:5432/qhook docker compose -f docker-compose.prod.yaml up
```

The Docker image exposes port `8888` and expects a config file at `/data/qhook.yaml`. Data directory is `/data`.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.
