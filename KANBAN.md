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

## Done (v0.5)

| Task | Priority |
|------|----------|
| Labeled metrics (source, handler) | High |
| Alert system (generic webhook) | High |
| Slack alert template | High |
| Discord alert template | High |
| DB error metrics | Medium |

## Done (v0.6)

| Task | Priority |
|------|----------|
| Handler URL validation (scheme + private IP warning) | Critical |
| Fix SNS cert URL domain validation | Critical |
| Configurable DB pool size (`database.max_connections`) | Critical |
| DB error backoff in worker poll loop | Critical |
| Config semantic validation (`Config::validate()`) | High |
| Structured JSON logging (`QHOOK_LOG_FORMAT=json`) | High |
| Merge event payload + headers into single query | High |
| Configurable stale_threshold / retention_hours / drain_timeout | High |
| Drain timeout on graceful shutdown | High |
| Slow query logging (>100ms) | High |

## Done (v0.6 NFR Medium)

| Task | Priority |
|------|----------|
| HTTP error classification metrics (4xx/5xx/timeout/network) | Medium |
| Alert send success/failure metrics | Medium |
| Max delivery duration gauge | Medium |
| RwLock poison safety in LabeledCounter | Medium |
| SIGHUP config validation (dry-run reload) | Medium |
| Label cardinality gauge (HashMap growth monitor) | Medium |

## Done (v0.7)

| Task | Priority |
|------|----------|
| JSONPath event filtering (`handler.filter`) | High |
| Payload transformation (`handler.transform`) | High |
| Unit tests for filtering + transformation (9 tests) | High |

## Done (v0.8)

| Task | Priority |
|------|----------|
| Per-IP rate limiting (`server.ip_rate_limit`) | High |
| auth_token missing startup warning | Medium |
| Cloud版有料機能ドキュメント (`docs/private/cloud-features.md`) | Low |

## Done (v0.9)

| Task | Priority |
|------|----------|
| gRPC output support (`handler.type: grpc`) | High |
| Proto file for server stub generation (`proto/qhook.proto`) | High |
| gRPC module with prost message types + tonic client | High |
| Unit tests for gRPC encode/decode + channel creation (4 tests) | High |

## Done (v1.0-docs)

| Task | Priority |
|------|----------|
| GitHub Pages docs site (Jekyll + Cayman theme) | High |
| Getting Started guide | High |
| Configuration reference (full YAML) | High |
| CLI reference | Medium |
| Feature guides (7 pages: verification, CE, SNS, filtering, gRPC, monitoring, security) | High |
| Deploy overview with platform comparison | Medium |
| Deploy guides moved to `docs/deploy/` | Low |
| Examples: quickstart, github-webhook, filter-transform | High |
| Examples guide page | Medium |
| GitHub Pages CI workflow (`pages.yml`) | Medium |
| README: docs site link, examples section | Medium |

## Done (v0.9 publish prep)

| Task | Priority |
|------|----------|
| Cargo.toml metadata (repository, homepage, keywords, categories, exclude) | High |
| Version set to 0.1.0 for initial publish | High |
| `cargo publish --dry-run` verified | High |
| GitHub issue templates (bug report, feature request) | Medium |
| GitHub PR template | Medium |

## In Progress

| Task | Priority |
|------|----------|
| (none) | |

## Backlog

| Task | Priority | Notes |
|------|----------|-------|
| `cargo publish` to crates.io | High | dry-run passed, needs `cargo login` + publish |
| AWS EventBridge input | Low | |
| GCP Pub/Sub input | Low | |
