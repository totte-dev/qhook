---
layout: default
title: Home
---

# qhook

**Lightweight event-to-action engine.** Turn webhooks and API events into reliable HTTP actions — single binary, no Redis, no Kubernetes.

- **Zero infrastructure.** Single binary, SQLite for dev, Postgres for production.
- **Webhook verification built in.** GitHub, Stripe, Shopify, PagerDuty, Grafana, Terraform Cloud, GitLab, HMAC, AWS SNS X.509.
- **From one action to a pipeline.** Single HTTP call or multi-step workflow with branching, parallelism, and rollback.
- **Production ready.** Prometheus metrics, health checks, alerts, CloudEvents.

## Quick Start

```bash
# Install
cargo install qhook

# Or run with Docker
docker run -p 8888:8888 -v $(pwd)/qhook.yaml:/data/qhook.yaml ghcr.io/totte-dev/qhook
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
      - name: deploy
        url: http://deployer:3000/deploy
        catch:
          - errors: [all]
            goto: rollback
      - name: notify
        url: http://slack:3000/notify
        end: true
      - name: rollback
        url: http://deployer:3000/rollback
        end: true
```

```bash
qhook start
```

[Full Getting Started guide](getting-started.md)

## Documentation

### Guides

| Guide | Description |
|-------|-------------|
| [Getting Started](getting-started.md) | Installation, first config, first event |
| [Configuration](configuration.md) | Full YAML config reference |
| [CLI Reference](cli.md) | All CLI commands and options |
| [Webhook Verification](guides/webhook-verification.md) | GitHub, Stripe, Shopify, PagerDuty, Grafana, Terraform Cloud, GitLab, HMAC |
| [CloudEvents](guides/cloudevents.md) | Binary and structured mode support |
| [AWS SNS](guides/sns.md) | Receive events from SNS topics |
| [Workflows](guides/workflows.md) | Multi-step pipelines with error routing |
| [Filtering & Transformation](guides/filtering.md) | Event filtering and payload reshaping |
| [Monitoring](guides/monitoring.md) | Prometheus metrics, health checks, alerts |
| [Security](guides/security.md) | Security features and best practices |
| [Local Development](guides/local-development.md) | Dev mode, echo endpoint, test events, tunnels |
| [Database Schema](guides/database-schema.md) | Tables, columns, indexes, and conventions |
| [Error Reference](guides/error-reference.md) | HTTP status codes and error messages |
| [API Spec](openapi.yaml) | OpenAPI 3.1 specification |

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
| [workflow](https://github.com/totte-dev/qhook/tree/main/examples/workflow) | Multi-step pipeline with catch routing |
| [tenant-provision](https://github.com/totte-dev/qhook/tree/main/examples/tenant-provision) | Tenant provisioning with rollback and auth headers |
| [alert-remediation](https://github.com/totte-dev/qhook/tree/main/examples/alert-remediation) | PagerDuty alert → triage → remediate → escalate |

### Other

| Page | Description |
|------|-------------|
| [Why qhook?](why-qhook.md) | Use cases, comparisons, and positioning |
| [Examples](examples.md) | All example projects with descriptions |
