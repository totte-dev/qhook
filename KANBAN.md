# qhook Kanban

## Done (v0.1)

| Task | Priority |
|------|----------|
| Webhook receive + queue + retry | Very High |
| Signature verification (GitHub/Stripe/Shopify/HMAC) | High |
| Idempotency (JSONPath dedup key) | High |
| Dead Letter Queue | High |
| CLI commands (jobs list/retry, events list) | Low |
| Docker image + Compose | High |
| Deploy guides (AWS/Railway/Fly.io/Render) | Low |
| lib.rs separation for Cloud version | Low |
| Stripe checkout example app | Low |

## Done (v0.2)

| Task | Priority |
|------|----------|
| CloudEvents support (binary + structured) | High |
| AWS SNS input + X.509 signature verification | High |
| skip_verify option for testing | Low |
| Unit tests (34 tests) | High |
| E2E tests (14 + 8 SNS tests) | High |
| devcontainer + LocalStack setup | Low |
| Update README/docs for v0.2 features | High |

## In Progress

| Task | Priority |
|------|----------|
| (empty) | |

## Todo (v0.3)

| Task | Priority |
|------|----------|
| gRPC output support | High |
| Prometheus metrics endpoint | High |
| Rate limiting per handler | Low |
| Batch delivery (group multiple events) | Low |

## Backlog

| Task | Priority |
|------|----------|
| GitHub Actions CI/CD | High |
| Cloud: Web UI dashboard | High |
| Cloud: Multi-tenant support | High |
| Cloud: Event analytics | Low |
| AWS EventBridge input | Low |
| GCP Pub/Sub input | Low |
| JSONPath-based event filtering | Low |
| Payload transformation | Low |
