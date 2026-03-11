---
layout: default
title: Why qhook?
---

# Why qhook?

qhook is a **lightweight workflow engine with built-in queue and retry**. It receives events (webhooks, SNS, API calls) and executes HTTP actions reliably — from a single call to a multi-step pipeline with error routing and rollback. Single binary, no Redis, no Kubernetes.

---

## The Problem

When an event arrives — a GitHub push, a Stripe payment, a monitoring alert, a tenant signup — you need to reliably call one or more HTTP endpoints in response.

| Current approach | Limitation |
|-----------------|-----------|
| Custom scripts | No retry, no error routing, no monitoring |
| CI/CD tools (GitHub Actions, etc.) | Weak at orchestrating external HTTP services with retry and rollback |
| Kubernetes-native tools (Argo Events, etc.) | Requires Kubernetes |
| Managed workflow services (Step Functions, etc.) | Vendor lock-in, per-transition costs |
| Heavyweight orchestrators (Conductor, etc.) | JVM + DB + Elasticsearch, complex setup |

qhook fills the gap: **lightweight, reliable event-to-action execution without infrastructure overhead**.

---

## How It Works

```
Event source               qhook                           Your services

  Webhook (verified) ---->|                              |
  SNS message ----------->| Route + execute              |
  API call (POST /events/{source}/{type}) ->|              |
                           |  Simple (handler):           |
                           |    event → POST /billing --->|  one HTTP call
                           |    (retry, DLQ, fan-out)     |
                           |                              |
                           |  Multi-step (workflow):      |
                           |    event → build → deploy -->|  chained HTTP calls
                           |              ↓ fail          |  with error routing
                           |          rollback → alert -->|
```

**Handlers** execute a single HTTP action per event — with retry, DLQ, filtering, transformation, and fan-out to multiple services.

**Workflows** chain multiple HTTP actions — with branching, parallelism, wait, callback, and error routing (catch → rollback). Each step can send custom headers for authentication.

Both are defined in the same YAML config. Both share the same retry, DLQ, and monitoring infrastructure.

---

## Use Cases

### Simple: Webhook Fan-out

Receive Stripe webhooks, verify the signature, and deliver to multiple services with independent retry.

```yaml
sources:
  stripe:
    type: webhook
    verify: stripe
    secret: ${STRIPE_WEBHOOK_SECRET}

handlers:
  billing:
    source: stripe
    events: [invoice.paid]
    url: http://billing:3000/webhook
    idempotency_key: "$.id"
    retry: { max: 8 }
  analytics:
    source: stripe
    events: ["*"]
    url: http://analytics:3000/ingest
```

### Multi-step: Tenant Provisioning

API event triggers infra creation → DB setup → service configuration, with rollback on failure. Input parameters are validated, and each step can authenticate to external APIs.

```yaml
sources:
  platform:
    type: event

workflows:
  tenant-provision:
    source: platform
    events: [tenant.create]
    timeout: 300
    params:
      - name: tenant_id
        type: string
      - name: region
        type: string
      - name: plan
        type: string
        required: false
    steps:
      - name: create-infra
        url: https://api.terraform.io/v2/runs
        headers:
          Authorization: "Bearer ${TF_API_TOKEN}"
        result_path: "$.infra"
        retry: { max: 3, errors: [5xx, timeout] }
        catch:
          - errors: [all]
            goto: notify-failure

      - name: wait-infra
        type: callback
        url: https://api.terraform.io/v2/notification-configs
        headers:
          Authorization: "Bearer ${TF_API_TOKEN}"
        callback_timeout: 600

      - name: create-db
        url: http://db-service:3000/tenant
        result_path: "$.db"
        catch:
          - errors: [all]
            goto: rollback-infra

      - name: configure-services
        url: http://integration:3000/setup
        catch:
          - errors: [all]
            goto: rollback-db

      - name: activate
        url: http://platform:3000/tenant/activate
        end: true

      # --- rollback (reverse order) ---
      - name: rollback-db
        url: http://db-service:3000/tenant/delete
        on_failure: continue
        goto: rollback-infra

      - name: rollback-infra
        url: https://api.terraform.io/v2/runs
        headers:
          Authorization: "Bearer ${TF_API_TOKEN}"
        on_failure: continue
        goto: notify-failure

      - name: notify-failure
        url: http://slack:3000/notify
        end: true
```

### Multi-step: Deploy with Post-deploy Checks

GitHub push triggers build → deploy → smoke test → notify. Failure routes to rollback. Works as a post-deploy pipeline triggered by CI/CD or ArgoCD.

```yaml
sources:
  github:
    type: webhook
    verify: github
    secret: ${GITHUB_WEBHOOK_SECRET}

workflows:
  deploy:
    source: github
    events: [push]
    timeout: 600
    steps:
      - name: build
        url: http://ci:3000/build
        retry: { max: 2, errors: [5xx, timeout] }
        result_path: "$.build"

      - name: deploy-staging
        url: http://deployer:3000/deploy
        input: |
          {"env": "staging", "image": "{{$.build.image}}"}
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
        input: |
          {"env": "production", "image": "{{$.build.image}}"}
        end: true

      - name: rollback
        url: http://deployer:3000/rollback
        end: true
```

### Multi-step: Alert Remediation

PagerDuty alert triggers severity-based action with verification. Signature-verified webhook, authenticated API calls for escalation.

```yaml
sources:
  pagerduty:
    type: webhook
    verify: pagerduty
    secret: ${PAGERDUTY_WEBHOOK_SECRET}

workflows:
  remediate:
    source: pagerduty
    events: [incident.triggered]
    timeout: 300
    steps:
      - name: triage
        type: choice
        choices:
          - condition: "$.severity == critical"
            goto: restart
          - condition: "$.severity == warning"
            goto: scale-up
        default: notify

      - name: restart
        url: http://orchestrator:3000/restart
        goto: verify

      - name: scale-up
        url: http://orchestrator:3000/scale
        goto: verify

      - name: verify
        type: wait
        seconds: 30
        goto: health-check

      - name: health-check
        url: http://orchestrator:3000/health
        catch:
          - errors: [all]
            goto: escalate
        end: true

      - name: escalate
        url: https://api.pagerduty.com/incidents
        headers:
          Authorization: "Token token=${PAGERDUTY_API_KEY}"
          Content-Type: "application/json"
        end: true

      - name: notify
        url: http://slack:3000/notify
        end: true
```

### Multi-step: Terraform Post-apply

Terraform Cloud run completion triggers application-layer configuration and verification.

```yaml
sources:
  terraform:
    type: webhook
    verify: terraform
    secret: ${TF_NOTIFICATION_SECRET}

workflows:
  post-apply:
    source: terraform
    events: [run:completed]
    timeout: 300
    steps:
      - name: update-config
        url: http://config-service:3000/apply
        retry: { max: 3, errors: [5xx, timeout] }
        catch:
          - errors: [all]
            goto: rollback-config

      - name: health-check
        url: http://health-service:3000/check
        catch:
          - errors: [all]
            goto: rollback-config

      - name: notify-success
        url: http://slack:3000/notify
        input: |
          {"text": "Terraform apply succeeded, config updated."}
        end: true

      - name: rollback-config
        url: http://config-service:3000/rollback
        on_failure: continue
        goto: notify-failure

      - name: notify-failure
        url: http://slack:3000/notify
        input: |
          {"text": "Terraform post-apply failed. Manual intervention required."}
        end: true
```

---

## Supported Webhook Providers

| Provider | Verification | Header |
|----------|-------------|--------|
| GitHub | HMAC-SHA256 | `X-Hub-Signature-256` |
| Stripe | HMAC-SHA256 + timestamp | `Stripe-Signature` |
| Shopify | HMAC-SHA256 (base64) | `X-Shopify-Hmac-SHA256` |
| PagerDuty | HMAC-SHA256 | `X-PagerDuty-Signature` |
| Grafana | HMAC-SHA256 | `X-Grafana-Alerting-Signature` |
| Terraform Cloud | HMAC-SHA512 | `X-TFE-Notification-Signature` |
| GitLab | Token comparison | `X-Gitlab-Token` |
| Standard Webhooks | HMAC-SHA256 (base64) + timestamp | `webhook-signature` |
| Linear | HMAC-SHA256 | `Linear-Signature` |
| AWS SNS | X.509 certificate | (in body) |
| Custom HMAC | HMAC-SHA256 | `X-Webhook-Signature` |

---

## How qhook Compares

### vs Custom Scripts / Cron

| | Scripts | qhook |
|---|---|---|
| **Retry on failure** | DIY (if at all) | Built-in (exponential backoff) |
| **Error routing** | DIY | `catch` → rollback steps |
| **Dead Letter Queue** | None | Built-in |
| **Monitoring** | DIY | Prometheus metrics, alerts |
| **Webhook verification** | DIY | Built-in (9 providers) |

### vs CI/CD Tools (GitHub Actions, etc.)

| | CI/CD tools | qhook |
|---|---|---|
| **Trigger** | Git events, schedules | Webhooks (verified), SNS, API |
| **HTTP orchestration** | Weak (shell scripts with curl) | Native (YAML-defined HTTP chains) |
| **Retry with error routing** | Limited | Built-in (catch → rollback) |
| **Auth to external APIs** | Per-step secrets | `headers` field with env var expansion |
| **DLQ** | None | Built-in |

### vs Kubernetes-native Tools

| | K8s tools (Argo Events, etc.) | qhook |
|---|---|---|
| **Requires Kubernetes** | Yes | No |
| **Webhook verification** | Limited | Built-in (9 providers) |
| **Setup** | K8s cluster + CRDs | Single binary |
| **Post-deploy hooks** | K8s Jobs only | Any HTTP service |

### vs Managed Workflow Services (Step Functions, etc.)

| | Managed services | qhook |
|---|---|---|
| **Infrastructure** | Vendor-specific | Any cloud or on-prem |
| **Pricing** | Per-transition / per-run | Free (self-hosted) |
| **Webhook verification** | Not included | Built-in |
| **Local dev** | Emulators (partial) | SQLite |
| **Vendor lock-in** | Yes | None |

### vs Heavyweight Orchestrators (Conductor, etc.)

| | Heavy orchestrators | qhook |
|---|---|---|
| **Runtime** | JVM + DB + Elasticsearch | Single Rust binary |
| **Setup time** | 30+ minutes | 2 minutes |
| **Definition** | JSON DSL or Code SDK | YAML |
| **Web UI** | Yes | No (CLI only) |
| **Operational burden** | High | Low |

---

## Cost Comparison

Self-hosting qhook replaces paid webhook and workflow services. Here's what the numbers look like at typical scale.

### Outbound Webhooks (vs Svix)

| | Svix Starter | Svix Business | qhook (self-hosted) |
|---|---|---|---|
| **Monthly cost** | $490/mo | $1,290/mo | ~$7–15/mo |
| **Messages/mo** | 500K | 2M | Unlimited |
| **Endpoints** | 1,000 | 10,000 | Unlimited |
| **Retention** | 90 days | 90 days | Configurable |
| **Infrastructure** | Managed | Managed | 1 VM (t3.small or equivalent) |

### Inbound Webhooks (vs Hookdeck)

| | Hookdeck Starter | Hookdeck Growth | qhook (self-hosted) |
|---|---|---|---|
| **Monthly cost** | $0 (5K/mo) | $100+ | ~$7–15/mo |
| **At 100K events/mo** | $100/mo | $100/mo | ~$7–15/mo |
| **At 1M events/mo** | N/A | ~$500/mo | ~$15–30/mo |
| **Verification** | Limited | Full | 9 providers built-in |
| **Infrastructure** | Managed | Managed | 1 VM + Postgres |

### Workflow Orchestration (vs AWS Step Functions)

| | Step Functions Standard | Step Functions Express | qhook (self-hosted) |
|---|---|---|---|
| **Pricing model** | $0.025/1K transitions | Duration-based | Fixed infra cost |
| **100K workflows/mo (5 steps)** | $12.50/mo | ~$5/mo | ~$7–15/mo |
| **1M workflows/mo (5 steps)** | $125/mo | ~$50/mo | ~$15–30/mo |
| **10M workflows/mo** | $1,250/mo | ~$500/mo | ~$30–60/mo |
| **Vendor lock-in** | Yes (AWS) | Yes (AWS) | None |

### Infrastructure Estimates

| Setup | Monthly cost | Handles |
|---|---|---|
| **Minimal** (t3.small, SQLite) | ~$7/mo | Up to 100K events/mo |
| **Standard** (t3.medium, RDS Postgres) | ~$30/mo | Up to 1M events/mo |
| **Scale** (2x t3.medium, RDS) | ~$60/mo | Up to 10M events/mo |

**Break-even point**: qhook pays for itself compared to Svix at ~10K messages/month, compared to Hookdeck at ~50K events/month, and compared to Step Functions at ~5M transitions/month.

> These estimates assume on-demand AWS pricing. Reserved instances or spot reduce costs further. qhook's single-binary design means no Redis, no message broker, no Elasticsearch — just compute + database.

---

## Where qhook Fits

qhook is purpose-built for one pattern: **event arrives → execute HTTP actions reliably**.
Whether the event is an external webhook (Stripe, GitHub) or an internal API call (your app, IDP, CI/CD), qhook handles both through the same engine.

It is **not** a replacement for:
- Kubernetes-native tools if you already run K8s
- Managed workflow services at massive scale
- Heavyweight orchestrators with Web UI and plugin ecosystems

It **is** the right choice when:
- You need to turn events into reliable HTTP actions (one or many)
- You want retry, error routing, and DLQ without writing infrastructure code
- You need to call authenticated external APIs as part of a pipeline
- You don't run Kubernetes and don't want vendor lock-in
- You want a single binary with zero dependencies

---

## Summary

| Capability | qhook |
|---|---|
| **Deployment** | Single binary, zero external deps |
| **Database** | SQLite (dev) / Postgres (prod) |
| **Input** | Webhooks (9 providers verified), SNS, internal API (`POST /events/{source}/{type}` or `POST /events/{type}`) |
| **Actions** | HTTP with custom headers for authentication |
| **Management API** | Track events, jobs, and workflow runs programmatically |
| **Reliability** | Retry (exponential backoff), error routing, DLQ |
| **Workflows** | Sequential, choice, parallel, map, wait, callback |
| **Input validation** | Workflow params with type checking |
| **Monitoring** | Prometheus metrics, health checks, Slack/Discord alerts |
| **Config** | Single YAML file, env var expansion, validation CLI |
