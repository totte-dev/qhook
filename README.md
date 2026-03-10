# qhook

**Lightweight event-to-action engine.** Turn webhooks and API events into reliable HTTP actions — single binary, no Redis, no Kubernetes.

<!-- badges -->
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](#license)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org/)
[![CI](https://github.com/totte-dev/qhook/actions/workflows/ci.yml/badge.svg)](https://github.com/totte-dev/qhook/actions/workflows/ci.yml)

**[Documentation](https://totte-dev.github.io/qhook/)** | **[Examples](./examples/)** | **[Why qhook?](https://totte-dev.github.io/qhook/why-qhook)**

---

## What qhook Does

1. **Receive** events — webhooks (with signature verification), AWS SNS, or internal API
2. **Execute** HTTP actions reliably — one call or a multi-step pipeline
3. **Handle** failures — retry, error routing, rollback, Dead Letter Queue

```
Simple:                           Multi-step:
  event → POST /billing             event → build → deploy → notify
  (with retry + DLQ)                             ↓ fail
                                             rollback → alert
```

## Why qhook?

- **Zero infrastructure.** Single binary, SQLite for dev, Postgres for production. No Redis, no message broker.
- **Webhook verification built in.** GitHub, Stripe, Shopify, PagerDuty, Grafana, Terraform Cloud, GitLab, HMAC, AWS SNS X.509.
- **From one action to a pipeline.** Start with a single HTTP call; grow into multi-step workflows with branching, parallelism, and rollback — same YAML, same engine.
- **Production ready.** Prometheus metrics, health checks, Slack/Discord alerts, rate limiting, circuit breaker, OpenTelemetry tracing, structured logging.

> See [Why qhook?](https://totte-dev.github.io/qhook/why-qhook) for detailed comparisons and use cases.

## Quick Start

```bash
cargo install qhook
# Or: docker run -p 8888:8888 -v $(pwd)/qhook.yaml:/data/qhook.yaml ghcr.io/totte-dev/qhook
```

### Simple: one event, one action

```yaml
# qhook.yaml
database:
  driver: sqlite

sources:
  github:
    type: webhook
    verify: github
    secret: ${GITHUB_WEBHOOK_SECRET}

handlers:
  deploy:
    source: github
    events: [push]
    url: http://deployer:3000/deploy
    filter: "$.ref == refs/heads/main"
    retry: { max: 5 }
```

### Multi-step: event triggers a pipeline

```yaml
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

```bash
qhook start
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

For Kubernetes, a Helm chart is available at `charts/qhook/`:

```bash
helm install qhook ./charts/qhook
```

## CLI

```bash
qhook start                        # Start server
qhook init                         # Generate default config
qhook validate                     # Validate config
qhook jobs list --status dead      # List dead-letter jobs
qhook jobs retry                   # Retry all dead jobs
qhook events list                  # List received events
qhook events replay --source stripe # Replay events for matching handlers
qhook workflow-runs list           # List workflow runs
qhook workflow-runs redrive <ID>   # Redrive a failed workflow
```

> See the [CLI Reference](https://totte-dev.github.io/qhook/cli) for all commands.

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
| gRPC Output | [guides/grpc](https://totte-dev.github.io/qhook/guides/grpc) |
| Monitoring & Alerts | [guides/monitoring](https://totte-dev.github.io/qhook/guides/monitoring) |
| Security | [guides/security](https://totte-dev.github.io/qhook/guides/security) |
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
| [alert-remediation](./examples/alert-remediation/) | PagerDuty alert → triage → remediate → escalate |

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.
