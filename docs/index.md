---
layout: default
title: Home
---

# qhook

**SQS for webhooks and events** -- a lightweight event gateway with built-in queue and retry.

- **No infrastructure tax.** Single binary, no Redis, no RabbitMQ, no SQS.
- **Webhook verification built in.** GitHub, Stripe, Shopify, and generic HMAC.
- **Reliable delivery.** Exponential backoff retry with DLQ.
- **CloudEvents native.** Binary and structured mode detection.
- **AWS SNS ready.** Subscription confirmation, envelope unwrapping, X.509 verification.

## Quick Start

```bash
# Install
cargo install qhook

# Or run with Docker
docker run -p 8888:8888 -v $(pwd)/qhook.yaml:/data/qhook.yaml ghcr.io/totte-dev/qhook
```

Create `qhook.yaml`:

```yaml
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

[Full Getting Started guide](getting-started.md)

## Documentation

### Guides

| Guide | Description |
|-------|-------------|
| [Getting Started](getting-started.md) | Installation, first config, first event |
| [Configuration](configuration.md) | Full YAML config reference |
| [CLI Reference](cli.md) | All CLI commands and options |
| [Webhook Verification](guides/webhook-verification.md) | GitHub, Stripe, Shopify, HMAC signature checks |
| [CloudEvents](guides/cloudevents.md) | Binary and structured mode support |
| [AWS SNS](guides/sns.md) | Receive events from SNS topics |
| [Filtering & Transformation](guides/filtering.md) | Event filtering and payload reshaping |
| [gRPC Output](guides/grpc.md) | Deliver events via gRPC |
| [Monitoring](guides/monitoring.md) | Prometheus metrics, health checks, alerts |
| [Security](guides/security.md) | Security features and best practices |

### Deployment

[Deployment overview & platform comparison](deploy/)

| Platform | Guide |
|----------|-------|
| AWS (ECS / EC2) | [deploy/aws.md](deploy/aws.md) |
| Fly.io | [deploy/flyio.md](deploy/flyio.md) |
| Railway | [deploy/railway.md](deploy/railway.md) |
| Render | [deploy/render.md](deploy/render.md) |

### Examples

| Example | Description |
|---------|-------------|
| [quickstart](https://github.com/totte-dev/qhook/tree/main/examples/quickstart) | Minimal setup, no Docker needed |
| [github-webhook](https://github.com/totte-dev/qhook/tree/main/examples/github-webhook) | GitHub push/PR with signature verification |
| [filter-transform](https://github.com/totte-dev/qhook/tree/main/examples/filter-transform) | Event filtering and payload transformation |
| [stripe-checkout](https://github.com/totte-dev/qhook/tree/main/examples/stripe-checkout) | Stripe checkout with dual handlers |

### Other

| Page | Description |
|------|-------------|
| [Why qhook?](why-qhook.md) | Before/after comparison, DIY vs qhook |
| [Examples](examples.md) | All example projects with descriptions |
