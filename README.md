# qhook

**SQS for webhooks** -- a lightweight webhook receiver with built-in queue and retry.

<!-- badges -->
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](#license)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org/)

---

## Why qhook?

- **No infrastructure tax.** Single binary, no Redis, no RabbitMQ, no SQS. SQLite for local dev, Postgres for production.
- **Webhook verification built in.** GitHub, Stripe, Shopify, and generic HMAC -- signature checks happen before your app ever sees the payload.
- **Reliable delivery.** Exponential backoff retry with configurable limits. Dead Letter Queue for jobs that exhaust all attempts.
- **Idempotency.** Configurable dedup key (JSONPath) prevents double-processing of the same event.

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

server:
  port: 8888                        # listen port (default: 8888)

api:
  auth_token: ${QHOOK_API_TOKEN}    # bearer token for /events endpoint (optional)

delivery:
  signing_secret: ${QHOOK_SIGNING_SECRET}  # sign outgoing deliveries (optional)
  timeout: 30s                             # delivery HTTP timeout (default: 30s)
  default_retry:
    max: 5                          # max delivery attempts (default: 5)
    backoff: exponential            # exponential (default) or fixed
    interval: 30s                   # base retry interval (default: 30s)

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

handlers:
  payment-success:
    source: stripe                  # must match a source name
    events:                         # event types to match (empty = all)
      - checkout.session.completed
      - invoice.paid
    url: http://backend:3000/jobs/payment  # delivery target URL
    retry: { max: 8 }              # override default_retry per handler
    timeout: 60s                   # override delivery timeout per handler
    idempotency_key: "$.id"        # JSONPath to dedup key in payload

  deploy-on-push:
    source: github
    events: [push]
    url: http://deployer:4000/deploy
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

## Architecture

```
Webhook Provider          qhook                          Your App
(GitHub, Stripe, ...)     +--------------------------+
                          |                          |
  POST /webhooks/stripe ---> Verify signature        |
                          |   |                      |
                          |   v                      |
                          | Store event (dedup)      |
                          |   |                      |
                          |   v                      |
                          | Create job(s)            |
                          |   |                      |
                          |   v                      |
                          | Queue worker ------------>  POST http://backend/jobs/payment
                          |   |                      |
                          |   |-- success ----------->  mark completed
                          |   |-- failure (< max) -->  exponential backoff, retry
                          |   |-- failure (= max) -->  move to Dead Letter Queue
                          +--------------------------+
```

**Endpoints:**

| Route | Purpose |
|-------|---------|
| `POST /webhooks/{source}` | Receive external webhooks (signature verified) |
| `POST /events/{event_type}` | Receive internal events (bearer token auth) |
| `GET /health` | Health check |

## Deployment

Deployment examples are provided for common setups:

- [`docker-compose.yaml`](./docker-compose.yaml) -- Local development with SQLite
- [`docker-compose.prod.yaml`](./docker-compose.prod.yaml) -- Production with Postgres
- [`docs/deploy-aws.md`](./docs/deploy-aws.md) -- AWS (ECS Fargate / EC2)
- [`docs/deploy-railway.md`](./docs/deploy-railway.md) -- Railway
- [`docs/deploy-flyio.md`](./docs/deploy-flyio.md) -- Fly.io
- [`docs/deploy-render.md`](./docs/deploy-render.md) -- Render

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
