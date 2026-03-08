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

## In Progress

| Task | Priority |
|------|----------|
| (none) | |

## Todo (v0.4) — Security hardening

| Task | Priority | Notes |
|------|----------|-------|
| Stripe signature timestamp validation | High | Reject signatures older than 5 minutes to prevent replay attacks (Stripe recommended) |
| Migrate serde_yaml → serde_yml | Medium | serde_yaml is deprecated (archived by dtolnay) |
| Add LICENSE file to repo root | Low | Full Apache-2.0 text file |
| Request body size limit | High | Prevent OOM from oversized payloads |
| TLS support (or document reverse proxy) | Medium | Ensure webhook secrets are not sent over plaintext |
| Auth token hashing | Medium | Store api.auth_token as hash, not plaintext in memory |
| Rate limiting on inbound endpoints | High | Prevent abuse / DDoS on /webhooks, /events, /sns endpoints |
| Security headers on responses | Low | X-Content-Type-Options, X-Frame-Options, etc. |
| Audit logging | Medium | Log signature verification failures, auth failures, DLQ events |

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
