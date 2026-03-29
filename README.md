# qhook

**The missing layer between webhooks and your app.** One binary. Verify, queue, deliver — nothing is lost.

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](#license)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org/)
[![CI](https://github.com/totte-dev/qhook/actions/workflows/ci.yml/badge.svg)](https://github.com/totte-dev/qhook/actions/workflows/ci.yml)

---

Stripe sends a webhook. Your server is deploying. The event is gone.

qhook sits between webhook providers and your app. It verifies signatures, persists every event before responding, and delivers to your handlers with retry and dead-letter queues. Your webhook source gets a 200 in under 500ms. Your app processes events when it's ready.

```
Provider → qhook (verify → queue → ACK) → your handler
              ↓                               ↑ retry
           persist                         circuit breaker
```

## Quick Start

```bash
cargo install qhook
```

```yaml
# qhook.yaml
database:
  driver: sqlite

sources:
  stripe:
    type: webhook
    verify: stripe
    secret: ${STRIPE_WEBHOOK_SECRET}

handlers:
  billing:
    source: stripe
    events: [invoice.paid, customer.subscription.updated]
    url: http://localhost:3000/webhook
    retry: { max: 5 }
```

```bash
qhook start
# → Listening on :8888
# Point Stripe webhook URL to http://yourhost:8888/webhooks/stripe
```

That's it. Stripe webhooks are verified, queued, and delivered with retry. If your handler is down, events wait in the queue and are retried with exponential backoff.

## Why qhook?

**Lightweight.** A single 16 MB binary. No Redis, no RabbitMQ, no external message broker. SQLite for development, Postgres or MySQL for production.

**Dev to prod, same tool.** Start with SQLite locally. Deploy to Cloudflare with D1, or scale with Postgres. One line change. Same config, same binary, same behavior.

```yaml
# Local dev            # Cloudflare           # Scale
database:              database:              database:
  driver: sqlite         driver: d1             driver: postgres
                         database_id: ...       url: ${DATABASE_URL}
```

Estimated throughput (based on DB write characteristics — run your own benchmarks with `scripts/bench-cloudflare.sh` or `tests/bench.sh` for actual numbers):

| Driver | Est. throughput | Best for |
|---|---|---|
| SQLite | ~1K events/sec | Local dev, single instance |
| D1 | ~500 events/sec | Cloudflare deploy, low-medium traffic |
| Postgres | ~10K+ events/sec | Production, multi-instance |

**13 webhook providers verified.** Stripe, GitHub, Shopify, Twilio, Paddle, PagerDuty, Grafana, Terraform Cloud, GitLab, Linear, Standard Webhooks, HMAC, AWS SNS. IP allowlisting per source.

**At-least-once delivery.** Events and jobs are persisted in a single transaction before ACK. If qhook crashes, the webhook source retries and nothing is lost.

**Production ready.** Prometheus metrics, health checks (`/health`), Slack/Discord alerts, rate limiting (GCRA), circuit breaker, graceful shutdown, OpenTelemetry tracing.

## Going Further

qhook does more than receive and forward. Explore when you need it:

| Need | Feature | Guide |
|------|---------|-------|
| Fan out one event to multiple handlers | Multi-handler routing | [Configuration](https://totte-dev.github.io/qhook/configuration) |
| Consumer pulls events instead of push | Pull-mode queues | [Pull-Mode Queues](https://totte-dev.github.io/qhook/guides/pull-mode-queues) |
| Webhook → build → deploy → rollback | Workflow engine | [Workflows](https://totte-dev.github.io/qhook/guides/workflows) |
| Send webhooks to your customers | Outbound webhooks | [Getting Started](https://totte-dev.github.io/qhook/getting-started) |
| Filter events by payload content | JSONPath filtering | [Filtering](https://totte-dev.github.io/qhook/guides/filtering) |
| Scale to millions of events/month | Postgres + multi-instance | [Scaling Guide](https://totte-dev.github.io/qhook/guides/scaling) |

## CLI

```bash
qhook start                # Start server
qhook status               # Overview at a glance
qhook tail                 # Stream events in real time
qhook inspect <EVENT_ID>   # Full lifecycle of an event
qhook validate             # Check config before deploy
```

> [All CLI commands](https://totte-dev.github.io/qhook/cli) | [Remote mode](https://totte-dev.github.io/qhook/cli#remote-mode) for managing deployed instances

## Installation

```bash
cargo install qhook                    # From crates.io
docker pull ghcr.io/totte-dev/qhook    # Docker image (~16 MB)
```

## Documentation

**[totte-dev.github.io/qhook](https://totte-dev.github.io/qhook/)** — Getting started, configuration, deployment guides, and more.

## Development

This project is developed by a solo engineer with AI pair programming (Claude Code). All code is reviewed, tested (400+ tests), and the architecture decisions are human-driven. AI accelerates implementation; design and quality ownership remain with the maintainer.

Skeptical? The entire codebase is open — read it yourself or let an AI review it for you.

## License

Apache-2.0. See [LICENSE](LICENSE).
