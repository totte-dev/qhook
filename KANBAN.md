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

## Done (v0.3)

| Task | Priority |
|------|----------|
| Stale running job recovery | Very High |
| SELECT FOR UPDATE SKIP LOCKED (Postgres) | Very High |
| Concurrent delivery (max 10) | High |
| Auto cleanup of completed/dead jobs (72h retention) | High |
| Graceful shutdown (SIGTERM/SIGINT + drain) | Medium |
| Benchmark / load test (tests/bench.sh) | High |
| Adaptive polling (50ms busy / 1s idle) | High |
| Prometheus metrics endpoint (/metrics) | High |
| Health check with queue_depth (/health) | Medium |
| GitHub Actions CI/CD (fmt + clippy + test + E2E) | Medium |
| Rate limiting per handler | Low |

## Done (v0.4)

| Task | Priority |
|------|----------|
| Stripe signature timestamp validation (5min replay protection) | High |
| Request body size limit (configurable, default 1MB) | High |
| Inbound concurrency limit (configurable, default 100) | High |
| Constant-time auth token comparison | Medium |
| TLS documentation (reverse proxy recommended) | Medium |
| Audit logging (auth failures) | Medium |
| Migrate serde_yaml → serde_yaml_ng | Medium |
| Security headers (nosniff, DENY, no-store) | Low |
| Add LICENSE file (Apache-2.0) | Low |

## In Progress

| Task | Priority |
|------|----------|
| (none) | |

## Backlog

| Task | Priority | Notes |
|------|----------|-------|
| gRPC output support | High | |
| Batch delivery (group multiple events) | Medium | |
| Cloud: Web UI dashboard | High | |
| Cloud: Multi-tenant support | High | |
| Cloud: Event analytics | Low | |
| AWS EventBridge input | Low | |
| GCP Pub/Sub input | Low | |
| JSONPath-based event filtering | Low | |
| Payload transformation | Low | |
| DynamoDB backend | Medium | ~$1-2/mo, very cheap. Requires dedicated storage layer (not sqlx compatible) |
