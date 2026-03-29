# qhook

**Lightweight webhook gateway in a single binary.** Replace your webhook infrastructure — signature verification, queue, retry, DLQ — with one 16 MB binary. Zero external dependencies, at-least-once delivery.

<!-- badges -->
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](#license)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org/)
[![CI](https://github.com/totte-dev/qhook/actions/workflows/ci.yml/badge.svg)](https://github.com/totte-dev/qhook/actions/workflows/ci.yml)

**[Documentation](https://totte-dev.github.io/qhook/)** | **[Examples](./examples/)** | **[Why qhook?](https://totte-dev.github.io/qhook/why-qhook)**

---

## Built For

- **You got your 3rd Stripe webhook timeout this week.** qhook ACKs in < 500ms, queues the event, and delivers with retry. Your webhook source never times out again.
- **Your checkout handler crashed and the event is gone.** qhook persists every event before responding. At-least-once delivery with DLQ means nothing is silently lost.
- **You need webhook → build → deploy → rollback, not just webhook → handler.** qhook has a built-in workflow engine. Same YAML, same binary.
- **You don't want to run Redis + RabbitMQ + a custom worker just to receive webhooks.** qhook is a single 16 MB binary. `cargo install qhook && qhook start`.

## What qhook Does

Every incoming webhook or API event follows the same reliable path:

```
  verify → enqueue → ACK (< 500ms) → deliver → retry/DLQ
```

1. **Verify** — signature validation for 13 providers (Stripe, GitHub, Shopify, Twilio, Paddle, Standard Webhooks, etc.)
2. **Enqueue** — persist to SQLite/Postgres/MySQL before responding. No event is lost.
3. **ACK** — return 200/202 immediately. Your webhook source never times out.
4. **Deliver** — POST to your handlers with retry, backoff, circuit breaker, and DLQ.

This is the [queue-first architecture](https://totte-dev.github.io/qhook/why-qhook) recommended by every production webhook guide — built into qhook by default.

```
Simple:                           Multi-step:
  event → POST /billing             event → build → deploy → notify
  (with retry + DLQ)                             ↓ fail
                                             rollback → alert
```

## Why qhook?

- **Push and Pull delivery.** Push mode delivers events to your HTTP endpoints instantly. Pull mode lets your app poll a queue and ACK when done — no public endpoint needed, no timeout pressure. Use both in the same config.
- **Zero infrastructure.** Single binary, SQLite for dev, Postgres or MySQL for production. No Redis, no message broker.
- **Webhook verification built in.** GitHub, Stripe, Shopify, Twilio, Paddle, PagerDuty, Grafana, Terraform Cloud, GitLab, Linear, Standard Webhooks (Clerk/Resend/etc.), HMAC, AWS SNS X.509. IP allowlisting per source.
- **Outbound webhooks.** Send webhooks to your customers with [Standard Webhooks](https://www.standardwebhooks.com/) compliant signatures. Dynamic endpoint management, subscription-based routing, per-endpoint signing secrets.
- **From one action to a pipeline.** Start with a single HTTP call; grow into multi-step workflows with branching, parallelism, and rollback — same YAML, same engine.
- **Production ready.** Prometheus metrics, health checks, Slack/Discord alerts, rate limiting, circuit breaker, OpenTelemetry tracing, structured logging.

> See [Why qhook?](https://totte-dev.github.io/qhook/why-qhook) for detailed comparisons and use cases.

## Quick Start

```bash
cargo install qhook
# Or: docker run -p 8888:8888 -v $(pwd)/qhook.yaml:/data/qhook.yaml ghcr.io/totte-dev/qhook
```

### Simple: Stripe webhook → billing + analytics

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
    url: http://billing:3000/webhook
    idempotency_key: "$.id"
    retry: { max: 8 }
  analytics:
    source: stripe
    events: ["*"]
    url: http://analytics:3000/ingest
```

### Multi-step: GitHub push → build → deploy → rollback

```yaml
sources:
  github:
    type: webhook
    verify: github
    secret: ${GITHUB_WEBHOOK_SECRET}

workflows:
  deploy-pipeline:
    source: github
    events: [push]
    timeout: 600
    steps:
      - name: build
        url: http://ci:3000/build
        retry: { max: 2, errors: [5xx, timeout] }
      - name: deploy-staging
        url: http://deployer:3000/deploy
        catch:
          - errors: [all]
            goto: rollback
      - name: smoke-test
        url: http://tester:3000/smoke
        catch:
          - errors: [all]
            goto: rollback
      - name: deploy-prod
        url: http://deployer:3000/deploy
        end: true
      - name: rollback
        url: http://deployer:3000/rollback
        end: true
```

### Pull mode: Stripe events → consumer polls when ready

```yaml
sources:
  stripe:
    type: webhook
    verify: stripe
    secret: ${STRIPE_WEBHOOK_SECRET}

queues:
  billing:
    source: stripe
    events: [invoice.paid, invoice.payment_failed]
    visibility_timeout: 60s
    max_attempts: 5
```

```bash
qhook start

# Consumer pulls messages
curl http://localhost:8888/api/queues/billing/messages?wait=10s&batch=5

# Acknowledge after processing
curl -X POST http://localhost:8888/api/queues/billing/ack \
  -H "Content-Type: application/json" \
  -d '{"ids": ["01J..."]}'
```

> See the [Getting Started guide](https://totte-dev.github.io/qhook/getting-started) for a full walkthrough.

## Step Types

| Type | Description |
|------|-------------|
| **HTTP** (default) | Call a URL with custom headers, chain response to next step |
| **Choice** | Conditional branching based on payload values |
| **Parallel** | Execute multiple branches concurrently |
| **Map** | Iterate over an array, processing each element |
| **Wait** | Pause for a duration or until a timestamp |
| **Callback** | Pause, notify external service, wait for `POST /callback/:token` |

> See the [Workflows guide](https://totte-dev.github.io/qhook/guides/workflows) for details.

## Installation

```bash
cargo install qhook                    # From crates.io
docker pull ghcr.io/totte-dev/qhook    # Docker image
```

Or build from source:

```bash
git clone https://github.com/totte-dev/qhook.git && cd qhook
cargo build --release
```

## CLI

```bash
qhook start                        # Start server
qhook status                       # Overview: events, jobs, queues, handlers at a glance
qhook init                         # Generate default config
qhook validate                     # Validate config (warns about missing auth)
qhook tail                         # Stream events and jobs in real time
qhook inspect <EVENT_ID>           # Full lifecycle of an event

qhook jobs list --status dead      # List dead-letter jobs
qhook jobs retry                   # Retry all dead jobs
qhook events list                  # List received events
qhook events replay --source stripe  # Replay events for matching handlers
qhook queues list                  # Queue depths and status
qhook queues dlq billing           # Dead-letter messages in a queue
qhook queues retry billing         # Retry all dead messages

qhook start --env production       # Config overlay (merges qhook.production.yaml)
qhook export events > events.jsonl # Export as JSONL
qhook replay-local events.jsonl    # Replay exported events

# Remote mode: manage a deployed instance from your laptop
qhook status --remote https://qhook.example.com --token $TOKEN
```

> See the [CLI Reference](https://totte-dev.github.io/qhook/cli) for all commands.

## Management API

Track events and jobs programmatically:

```bash
# Send an event with explicit source name
curl -X POST http://localhost:8888/events/platform/deploy.start \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"service": "api", "version": "1.2.3"}'
# → {"event_id": "01J...", "jobs_created": 2}

# The old route still works (defaults to source "app")
# curl -X POST http://localhost:8888/events/deploy.start ...

# Check event status
curl http://localhost:8888/api/events/01J... -H "Authorization: Bearer $TOKEN"
# → {jobs: [{status: "completed"}, {status: "running"}], ...}

# Check job details
curl http://localhost:8888/api/jobs/01J...?include_attempts=true -H "Authorization: Bearer $TOKEN"
```

The management API powers the CLI and can be consumed by any frontend (Backstage, Retool, or custom dashboards).

## Documentation

Full documentation at **[totte-dev.github.io/qhook](https://totte-dev.github.io/qhook/)**.

| Topic | Link |
|-------|------|
| Getting Started | [getting-started](https://totte-dev.github.io/qhook/getting-started) |
| Configuration Reference | [configuration](https://totte-dev.github.io/qhook/configuration) |
| CLI Reference | [cli](https://totte-dev.github.io/qhook/cli) |
| Webhook Verification | [guides/webhook-verification](https://totte-dev.github.io/qhook/guides/webhook-verification) |
| CloudEvents | [guides/cloudevents](https://totte-dev.github.io/qhook/guides/cloudevents) |
| AWS SNS | [guides/sns](https://totte-dev.github.io/qhook/guides/sns) |
| Workflows | [guides/workflows](https://totte-dev.github.io/qhook/guides/workflows) |
| Filtering & Transformation | [guides/filtering](https://totte-dev.github.io/qhook/guides/filtering) |
| Monitoring & Alerts | [guides/monitoring](https://totte-dev.github.io/qhook/guides/monitoring) |
| Security | [guides/security](https://totte-dev.github.io/qhook/guides/security) |
| Compliance & Audit Trail | [guides/compliance](https://totte-dev.github.io/qhook/guides/compliance) |
| Deployment | [deploy](https://totte-dev.github.io/qhook/deploy) |
| Why qhook? | [why-qhook](https://totte-dev.github.io/qhook/why-qhook) |

## Examples

| Example | Description |
|---------|-------------|
| [quickstart](./examples/quickstart/) | Minimal setup, no Docker needed |
| [github-webhook](./examples/github-webhook/) | GitHub push/PR with signature verification |
| [filter-transform](./examples/filter-transform/) | Event filtering + payload transformation |
| [stripe-checkout](./examples/stripe-checkout/) | Stripe checkout with dual handlers |
| [workflow](./examples/workflow/) | Multi-step pipeline with catch routing |
| [tenant-provision](./examples/tenant-provision/) | Tenant provisioning with rollback and auth headers |
| [outbound-webhook](./examples/outbound-webhook/) | Send webhooks to customers with Standard Webhooks signatures |
| [stripe-pull](./examples/stripe-pull/) | Stripe events consumed via pull-mode queue (Python + TypeScript) |
| [alert-remediation](./examples/alert-remediation/) | PagerDuty alert → triage → remediate → escalate |

## MCP Server

qhook includes an [MCP (Model Context Protocol)](https://modelcontextprotocol.io) server for AI agent integration. Send events, list jobs, and check health from Claude Code, Claude Desktop, or any MCP-compatible client.

See [mcp-server/README.md](./mcp-server/README.md) for setup instructions.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.
