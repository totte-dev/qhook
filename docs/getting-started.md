---
layout: default
title: Getting Started
---

# Getting Started

Get qhook running in under 5 minutes.

## Installation

### Docker (recommended)

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

## Your First Config

Generate a starter config:

```bash
qhook init
```

Or create `qhook.yaml` manually:

```yaml
database:
  driver: sqlite

server:
  port: 8888
  allow_private_urls: true    # required for localhost handler URLs

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

This config:
- Uses SQLite (no external database needed; Postgres and MySQL also supported)
- Listens on port 8888
- `allow_private_urls` allows handler URLs pointing to localhost (disabled by default for SSRF protection)
- Defines one source called `app` (no signature verification)
- Routes `order.created` events to `http://localhost:3000/jobs/order`
- Retries up to 5 times on failure

## Start the Server

```bash
qhook start                        # uses ./qhook.yaml
qhook start -c /path/to/qhook.yaml # custom path
```

Or with Docker:

```bash
docker run -p 8888:8888 -v $(pwd)/qhook.yaml:/data/qhook.yaml ghcr.io/totte-dev/qhook
```

## Send Your First Event

```bash
curl -X POST http://localhost:8888/events/app/order.created \
  -H "Content-Type: application/json" \
  -d '{"id": "ord_123", "customer": "alice", "amount": 4999}'
```

qhook returns `202 Accepted` immediately. The event is persisted and a job is created for each matching handler. The queue worker delivers the payload to the handler URL with retry on failure.

## Check Status

```bash
# Health check
curl http://localhost:8888/health
# {"status":"ok","queue_depth":0}

# List recent events
qhook events list

# List jobs
qhook jobs list
```

## Architecture

```
Your App / Webhook Provider    qhook                          Your Backend
                               +--------------------------+
POST /events/app/order.created -> | Store event (dedup)      |
POST /webhooks/stripe -------> | Verify signature         |
POST /sns/my-topic ----------> | Verify X.509 + unwrap    |
                               |   |                      |
                               |   v                      |
                               | Create job(s)            |
                               |   |                      |
                               |   v                      |
                               | Queue worker ------------>  POST handler URL
                               |   success ----------------->  mark completed
                               |   failure (< max) -------->  exponential backoff
                               |   failure (= max) -------->  Dead Letter Queue
                               +--------------------------+
```

**Endpoints:**

| Route | Purpose |
|-------|---------|
| `POST /webhooks/{source}` | Receive external webhooks (signature verified) |
| `POST /sns/{source}` | Receive AWS SNS messages (X.509 verified) |
| `POST /events/{source}/{event_type}` | Receive internal events (bearer token auth) |
| `GET /health` | Health check with queue depth |
| `GET /metrics` | Prometheus metrics |

## Next Steps

- [Configuration Reference](configuration.md) -- all config options
- [Webhook Verification](guides/webhook-verification.md) -- add signature checks
- [Filtering & Transformation](guides/filtering.md) -- filter events, reshape payloads
- [Monitoring](guides/monitoring.md) -- Prometheus metrics, alerts
- [Deployment](deploy/aws.md) -- deploy to AWS, Fly.io, Railway, Render
