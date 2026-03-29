# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [0.6.0] - 2026-03-29

### Added
- **Pull-mode queue delivery**: Consumer-side polling via `GET /api/queues/{name}/messages` with long-polling, `POST /ack`, `POST /nack`. Configurable visibility timeout, per-queue API key auth. Events matching a queue create jobs consumed by polling instead of push.
- **Cloudflare D1 database adapter**: HTTP-based adapter for Cloudflare D1. Supports REST API mode and Outbound Workers proxy mode. Enables `wrangler deploy` for serverless webhook processing.
- **At-least-once delivery guarantee**: Event and all jobs are now inserted in a single database transaction. If qhook crashes mid-insert, the DB rolls back and the webhook sender retries.
- **Database backpressure**: When 3+ consecutive DB errors occur, webhook/event/SNS handlers return `503 Service Unavailable` with `Retry-After: 30`. Auto-recovers when DB succeeds. Health endpoint also recovers the flag if the worker crashes.
- **Queue management CLI**: `qhook queues list`, `inspect`, `drain`, `dlq`, `retry`, `peek` commands for pull-mode queue operations.
- **`qhook status` command**: One-command overview — events by source, jobs by status, handler stats, queue depths, workflow counts.
- **CLI remote mode**: `--remote <URL> --token <TOKEN>` flags (or `QHOOK_REMOTE_URL`/`QHOOK_API_TOKEN` env vars) to manage deployed instances via HTTP API instead of local DB.
- **`X-Qhook-Delivery-ID` header**: Deterministic `SHA256(event_id:handler)` on every delivery for handler-side deduplication.
- **Event TTL**: `delivery.event_ttl` config (e.g., `168h`) auto-expires available/retryable jobs older than the threshold.
- **Retry jitter**: ±20% random jitter on all exponential backoff to prevent thundering herd across instances.
- **Bulk retry API**: `POST /api/jobs/retry?status=dead&handler=xxx` for batch retry operations.
- **Operational webhooks**: New alert events — `circuit_opened`, `workflow_failed`, `db_unhealthy`, `event_ttl_expired`.
- **Structured validation errors**: API errors now return `{"code", "message", "details"}` format with field-level paths.
- **Outbound endpoint verification**: Challenge-response ownership check on endpoint creation (`skip_endpoint_verification` config for dev).
- **Tracing spans**: `event_id`/`source`/`event_type` on `process_event`, `job_id`/`handler` on delivery for structured log correlation.
- **Production warnings in `qhook validate`**: Warns about missing `api.auth_token`, `allow_private_urls`, and queue `api_key`.
- **Scaling guide**: `docs/guides/scaling.md` — D1 vs Postgres decision framework with cost breakdowns.
- **Local-to-production guide**: `docs/guides/local-to-production.md` — environment overlays, DB migration, security checklist.
- **Pull-mode queue guide**: `docs/guides/pull-mode-queues.md` — API reference, config, consumer snippets in Python/TypeScript/Go/Ruby.
- **Cloudflare deploy guide**: `docs/deploy/cloudflare.md` — Containers + D1 setup.
- **Stripe pull example**: `examples/stripe-pull/` — Python and TypeScript consumer examples.
- **Benchmark scripts**: `scripts/bench-cloudflare.sh`, k6 scripts for ingestion, delivery, pull-mode, and D1 scenarios.

### Changed
- **README rewritten**: Progressive disclosure — core message ("The missing layer between webhooks and your app") with one use case. Workflows, pull-mode, outbound moved to docs.
- **`/health` endpoint**: Now includes `dead_jobs` count and returns `"degraded"` status when DLQ has items.
- **Stale recovery excludes queue jobs**: Push stale recovery no longer touches pull-mode queue messages (separate visibility timeout recovery).
- **`fetch_available_jobs` excludes queue jobs**: Push worker ignores `queue/*` handler prefix.

### Fixed
- **MySQL/SQLite parallel branch completion race**: Wrapped UPDATE + SELECT in transaction to prevent duplicate workflow step execution.
- **Nack TOCTOU race**: Added `AND status = 'running'` guard to nack UPDATE queries.
- **`delete_jobs_by_handler` transaction safety**: Wrapped in explicit transaction to prevent orphaned records.
- **`retry_job` resets attempt counter**: Prevents immediate re-dead after retry.
- **SNS cert cache TTL cleanup**: `retain()` expired entries on insert.
- **Alert bounded channel**: Changed from unbounded to capacity 1000 with graceful overflow handling.
- **D1 client timeout**: Set explicit 10s timeout to prevent indefinite hangs.

### Removed
- **`thiserror` dependency**: Unused, removed.

## [0.5.0] - 2026-03-12

### Added
- **Gzip compression**: HTTP responses are now gzip-compressed via `tower-http` `CompressionLayer`, reducing bandwidth for API and metrics responses.
- **Full JSON Schema validation**: Replaced manual schema validation with the `jsonschema` crate (Draft 2020-12). Now supports `enum`, `pattern`, `minLength`, `maxLength`, and all standard JSON Schema keywords.
- **Local replay mode**: `qhook replay-local <file.jsonl>` replays exported events (from `qhook export events`) to a running server. Supports `--target`, `--token`, and stdin (`-`).
- **Twilio webhook verification**: `verify: twilio` checks `X-Twilio-Signature` header (HMAC-SHA1, base64-encoded).
- **Paddle webhook verification**: `verify: paddle` checks `Paddle-Signature` header (`ts=...;h1=...` format, HMAC-SHA256) with 5-minute replay protection.
- **MySQL support**: `database.driver: mysql` for MySQL/MariaDB backends via sqlx AnyPool. Adapts `FOR UPDATE SKIP LOCKED`, `INSERT IGNORE`, and `VARCHAR(255)` primary keys for MySQL compatibility.
- **llms.txt**: AI agent discoverability file at `docs/llms.txt` following the llms.txt specification.
- **Compliance documentation**: PCI DSS 4.0 and SOC 2 compliance framing guide (`docs/guides/compliance.md`) with audit trail mapping tables.
- **proptest property-based tests**: 29 property-based tests covering config parsing robustness, retry backoff invariants, filter evaluation safety, and ULID generation properties.
- **insta snapshot tests**: 21 snapshot tests for config validation error messages, default config template, Prometheus metrics format, and API response formats.
- **Event inspection API**: `GET /api/events`, `GET /api/events/{id}/jobs`, `GET /api/jobs`, `GET /api/jobs/{id}/attempts` with filtering, cursor-based pagination, and `has_more` support.
- **Filtered replay**: `replay-local` now supports `--source`, `--event-type`, `--since`, `--until`, `--status` filters. Event type supports prefix matching with `*`.
- **Benchmark suite**: criterion-based benchmarks for config parsing, filter evaluation, ULID generation, signature verification, and SQLite event insertion.
- **MCP server**: TypeScript MCP server (`mcp-server/`) wrapping qhook's REST API for AI agent integration (Claude Code, Claude Desktop).

### Changed
- **Per-destination rate limiting**: Switched from semaphore-hold to GCRA-based rate limiting via the `governor` crate. Provides smoother, more accurate per-handler rate limiting.

## [0.4.2] - 2026-03-11

### Added
- **Standard Webhooks compliance**: Outbound webhooks now use `webhook-id`, `webhook-timestamp`, `webhook-signature` headers per the [Standard Webhooks](https://www.standardwebhooks.com/) spec. Signed content format: `{msg_id}.{timestamp}.{body}` with base64 output and `v1,` prefix.
- **Retry-After header support**: Respects `Retry-After` from downstream 429/503 responses to override exponential backoff delay (capped at 86400s).
- **DB migration versioning**: Migrations tracked in `_migrations` table with auto-detection of pre-existing databases. Enables safe schema upgrades across versions.
- **Configurable worker concurrency**: `worker.max_concurrency` and `worker.batch_size` config fields replace hardcoded constants (both default to 10).
- **CI code coverage**: `cargo-llvm-cov` integrated in CI pipeline with LCOV output.
- **Cost comparison documentation**: Added infrastructure cost comparison vs Svix, Hookdeck, and AWS Step Functions in `docs/why-qhook.md`.
- **Standard Webhooks inbound verification**: `verify: standard-webhooks` for providers using the Standard Webhooks spec (Clerk, Resend, Lemon Squeezy, Orb). Includes 5-minute replay protection and multi-signature support for key rotation.
- **Linear webhook verification**: `verify: linear` checks `Linear-Signature` header (HMAC-SHA256).
- **IP allowlisting**: `allowed_ips` field on sources restricts webhook reception to specific IPs/CIDRs. Supports IPv4, IPv6, and proxy-aware IP extraction.

### Changed
- Outbound webhook headers changed from `X-Qhook-Signature`/`X-Qhook-Timestamp` to Standard Webhooks format (outbound webhooks were added in v0.4.1, minimal impact).

## [0.4.1] - 2026-03-11

### Added
- **Outbound webhooks**: Send webhooks to your customers with per-endpoint HMAC-SHA256 signatures. New `type: outbound` source with dynamic endpoint management via REST API.
  - `POST/GET /api/outbound/endpoints` — create and list customer endpoints
  - `GET/PUT/DELETE /api/outbound/endpoints/:id` — manage individual endpoints
  - `POST /api/outbound/endpoints/:id/rotate-secret` — rotate signing secret
  - `POST/GET/DELETE /api/outbound/endpoints/:id/subscriptions` — event type subscriptions
  - Per-endpoint signing secret (`whsec_` prefix) with `X-Qhook-Signature: v1=<hmac>` and `X-Qhook-Timestamp` headers
  - Wildcard (`*`) subscriptions, fan-out to multiple endpoints, disable/enable toggle
  - Reuses existing retry, DLQ, and circuit breaker infrastructure
- **Example**: `examples/outbound-webhook/` — customer webhook receiver with signature verification
- 10 outbound E2E tests + 1 full lifecycle scenario test (endpoint → subscribe → deliver → verify signature → disable → rotate secret)
- TypeScript and Python SDK generation from OpenAPI spec (`sdks/generate.sh`)

## [0.4.0] - 2026-03-11

### Added
- JSON response for webhook and event endpoints — returns `event_id`, `jobs_created`, and `duplicate` status
- Management API: `GET /api/events/:id` returns event details with associated jobs and workflow runs
- Management API: `GET /api/jobs/:id` returns job details with optional delivery attempts (`?include_attempts=true`)
- `cargo-deny` security checks (advisories, licenses, bans, sources) in CI and weekly schedule
- Rust integration tests replacing shell-based E2E tests (30 tests: 12 e2e + 12 workflow + 6 scenarios)

### Changed
- `POST /webhooks/:source` now returns JSON instead of plain text (breaking change for clients parsing text responses)
- Event endpoint is now `POST /events/{source}/{event_type}` — the old `POST /events/{event_type}` route was removed
- `POST /events/:source/:type` now returns JSON `{"event_id": "...", "jobs_created": N}` instead of plain text
- Updated product positioning to "lightweight workflow engine"

## [0.3.1] - 2026-03-11

### Added
- **`qhook send` CLI**: Send test events to a running server without curl. Supports inline JSON, file input, and auto-detects source type/port from config.
- **`qhook doctor` CLI**: Pre-production readiness check. Validates config, database connectivity, handler/workflow endpoint reachability, and security settings.
- **Echo endpoint**: Built-in `/_echo` endpoint returns request headers and body as JSON. Eliminates the need for a mock server during development.
- **Local development guide**: New `docs/guides/local-development.md` covering echo endpoint, test events, tunnels (GitHub CLI, LocalStack, cloudflared).
- **Remote config loading**: `qhook start -c s3://...`, `-c gs://...`, `-c az://...`, or `-c https://...` loads config from AWS S3, GCS, Azure Blob, or HTTP. Polls every 30s for changes (ETag-based). Invalid config is rejected with current config preserved and logged. Public endpoints only (private bucket support tracked in #26).
- **`qhook inspect` CLI**: Show an event's full lifecycle — payload, matched jobs with status/attempts, workflow runs. Debug event flow in one command.
- **`qhook send --dry-run`**: Show which handlers and workflows would match an event without creating jobs.
- **`qhook init --template`**: Scaffold config from templates (`github`, `stripe`, `sns`, `cron`).
- **JSON Schema**: `docs/schema.json` for editor autocomplete and validation. Add `# yaml-language-server: $schema=https://totte-dev.github.io/qhook/schema.json` to `qhook.yaml`.
- **Database schema guide**: `docs/guides/database-schema.md` — full table/column/index reference.
- **Config overlay (`--env`)**: `qhook start --env production` merges `qhook.production.yaml` on top of `qhook.yaml`. Also loads `.env.production` for environment variables. Supports `QHOOK_ENV` environment variable. `qhook init` now generates `qhook.local.yaml` alongside `qhook.yaml`.
- **`qhook tail`**: Real-time event and job stream in the terminal. Filter by `--source` or `--status`. Color-coded output.
- **`qhook export events`**: Export events as JSONL for portability between environments. Supports `--source`, `--event-type`, `--since`, `--until` filters.

### Changed
- **Dependency reduction**: Replaced `regex` with `regex-lite` (smaller binary, no Unicode tables). Trimmed `tokio` features from `full` to only required features (`rt-multi-thread`, `macros`, `sync`, `signal`, `net`, `time`). ~1MB binary size reduction.
- **OTLP exporter**: Switched from gRPC (tonic) to HTTP JSON transport. Set `OTEL_EXPORTER_OTLP_ENDPOINT` to an HTTP endpoint.

### Removed
- **gRPC output**: Removed `type: grpc` handler support and `tonic`/`prost` dependencies. gRPC added significant binary size (~3MB) for limited use cases — HTTP handlers with Envoy or gRPC-gateway cover the same scenarios. If demand exists, gRPC support will be available in the Cloud version.
- **Helm chart**: Removed `charts/qhook/`. qhook is a single-binary application and doesn't benefit from Helm's complexity. For Kubernetes, use a simple Deployment manifest with the Docker image directly.

## [0.3.0] - 2026-03-10

### Added
- **Circuit breaker**: Per-handler circuit breaker. Opens after 5 consecutive failures, closes after 60s cooldown with half-open probe. New metrics: `qhook_circuit_breaker_opened_total`, `qhook_circuit_breaker_rejected_total`.
- **OpenTelemetry tracing**: Optional `otel` feature flag (`cargo build --features otel`). Exports traces via OTLP when `OTEL_EXPORTER_OTLP_ENDPOINT` is set. Falls back to standard tracing otherwise.
- **Event replay CLI**: `qhook events replay` re-creates jobs for historical events. Supports `--source`, `--event-type`, `--since`, `--until` filters.
- **Helm chart**: Kubernetes deployment via `charts/qhook/`. Includes Deployment, Service, ConfigMap, Ingress, PVC, HPA, and ServiceAccount.
- **SIGHUP config diff**: SIGHUP now logs added/removed/changed sources, handlers, and workflows. Warns about changes requiring restart (port, database driver).
- **Advanced filter operators**: `contains` (substring + array membership), `starts_with`, `ends_with`, `matches` (regex), `exists`, and `not` (negation).
- **Event schema validation**: `schema` field on sources for lightweight JSON Schema validation (`type`, `required`, `properties`). Rejects non-conforming events with 400.
- **Sub-workflow step**: `type: workflow` step invokes another workflow as a child. Parent workflow resumes after sub-workflow completes. Supports nesting.

### Changed
- **Performance**: Extracted `format_now()`/`format_dt()` helpers to eliminate 15+ repeated datetime formatting calls in DB layer. Optimized Stripe HMAC to avoid intermediate String allocation. Pre-allocated SNS signature string builder. Replaced `from_utf8` + `to_string()` with `String::from_utf8` in HTTP handlers.

## [0.2.2] - 2026-03-10

### Added
- **HTTP method specification**: `method` field on handlers, workflow steps, and parallel branches. Supports `GET`, `POST` (default), `PUT`, `PATCH`, `DELETE`. GET requests omit the body.
- **Cron triggers**: New `cron` source type with `schedule` (cron expression) and optional `timezone`. Fires `cron.tick` events on schedule, matching handlers and workflows.

### Changed
- Test coverage improvements: 14 gap-coverage tests added, 4 redundant tests removed (174 total unit tests).

## [0.2.0] - 2026-03-09

### Added
- **Workflow engine**: Event-driven multi-step pipelines defined in YAML.
  - Sequential workflows with response chaining (step N's response → step N+1's input).
  - Data flow control: `input` (transform before call), `result_path` (merge response), `output` (transform after).
  - Per-step retry with error type matching (`timeout`, `5xx`, `4xx`, `network`, `all`).
  - `catch` blocks for error routing to named fallback steps after retries exhausted.
  - `on_failure: continue` to proceed to next step with error info on failure.
  - `end: true` to terminate workflow early.
  - **Choice step** (`type: choice`): conditional routing with `when` conditions and `default` fallback. Reuses filter syntax (`==`, `!=`, `>=`, `>`, `<=`, `<`, `in`).
  - **Parallel step** (`type: parallel`): concurrent branch execution. Results merged as object keyed by branch name.
  - **Map step** (`type: map`): iterate over array items in payload. Results collected as array.
  - **Wait step** (`type: wait`): pause workflow for a fixed `seconds` delay or until a dynamic `timestamp_path` from the payload. No HTTP call — next step's job is scheduled with a future `scheduled_at`.
  - **Callback step** (`type: callback`): pause workflow and wait for an external system to call `POST /callback/:token` with a JSON body to resume. Optional `callback_timeout` to expire waiting callbacks.
  - **Workflow timeout**: `timeout` field on workflow config sets an overall time limit. If exceeded, subsequent steps are skipped and the workflow is marked as failed.
  - Workflows and handlers coexist — same event can trigger both.
- **Workflow metrics**: `qhook_workflow_runs_total` (by workflow + status), `qhook_workflow_steps_completed_total` (by workflow), `qhook_callbacks_received_total`, `qhook_callbacks_expired_total`.
- **Filter operators**: Added `>=`, `>`, `<=`, `<` numeric comparisons (handler filters + choice conditions).
- **CLI**: `workflow-runs list` and `workflow-runs redrive` commands.
- **DB**: `workflow_runs` table with parallel tracking (`parallel_count`, `parallel_completed`).
- **Config**: `workflows` section in YAML with full validation (step name uniqueness, catch goto targets, branch names, source references).
- **Example**: `examples/workflow/` — order processing pipeline with catch routing.
- **Docs**: Workflow guide at `docs/guides/workflows.md`.
- **Custom headers**: `headers` field on handlers, workflow steps, and parallel branches for authenticated API calls (e.g., `Authorization: Bearer ${TOKEN}`).
- **Callback URL notification**: Callback steps with a `url` field POST the callback token to the external service before waiting.
- **Workflow input parameters**: `params` field on workflows for runtime payload validation (type checking: string/number/boolean/object/array, required/optional).
- **Signature verification**: PagerDuty (`X-PagerDuty-Signature`, HMAC-SHA256), Grafana (`X-Grafana-Alerting-Signature`, HMAC-SHA256), Terraform Cloud (`X-TFE-Notification-Signature`, HMAC-SHA512), GitLab (`X-Gitlab-Token`, constant-time comparison).
- **Examples**: `examples/tenant-provision/` (params + headers + rollback), `examples/alert-remediation/` (PagerDuty + choice + wait + escalation).

### Security
- **SSRF protection**: Handler/workflow URLs pointing to private/loopback IPs are now rejected by default. Set `server.allow_private_urls: true` for local development.
- **Metrics endpoint authentication**: Optional `api.metrics_auth_token` to protect the `/metrics` endpoint with a bearer token.
- **DB credential redaction**: Database connection URLs are redacted (credentials removed) before logging on connection errors.
- **Proxy-aware rate limiting**: `server.trust_proxy` enables extraction of client IP from `X-Forwarded-For` / `X-Real-IP` headers when behind a reverse proxy. Requests with no determinable IP are now denied instead of bypassing the rate limiter.
- **Parallel branch race condition**: Fixed potential double-execution of the next workflow step when parallel branches complete concurrently, by using atomic `UPDATE ... RETURNING` on Postgres.

## [0.1.0] - 2025-03-08

Initial release.

### Core
- Webhook receive, queue, and retry with exponential backoff.
- Signature verification: Stripe (`t=...,v1=...`), GitHub (`sha256=...`), Shopify (Base64 HMAC), generic HMAC.
- Idempotency via configurable JSONPath dedup key.
- Dead Letter Queue for exhausted jobs.
- SQLite and Postgres support via sqlx AnyPool.

### Event Sources
- **CloudEvents**: Binary mode (`ce-type` header) and structured mode (`application/cloudevents+json`). `ce-*` headers forwarded to handlers.
- **AWS SNS**: Automatic subscription confirmation, envelope unwrapping, X.509 signature verification (SHA1/SHA256). `skip_verify` option for LocalStack testing.

### Processing
- **Event filtering**: `handler.filter` with JSONPath-like syntax (`==`, `!=`, `in [a, b]`, truthy).
- **Payload transformation**: `handler.transform` with `{{$.path}}` placeholders. Applied at delivery time, original payload preserved.
- **gRPC output**: `handler.type: grpc` with `qhook.v1.EventReceiver/Deliver` unary RPC. Proto file included.

### Production
- Concurrent delivery (max 10 parallel).
- Adaptive polling (50ms busy / 1s idle).
- Stale job recovery (5min threshold).
- Auto cleanup (72h retention).
- Graceful shutdown (SIGTERM/SIGINT + drain with configurable timeout).
- Postgres `SELECT FOR UPDATE SKIP LOCKED` for multi-instance deployments.
- Prometheus metrics (`/metrics`) with per-source and per-handler labels.
- Health check (`/health`) with queue depth.
- Per-handler rate limiting.
- Per-IP rate limiting (`server.ip_rate_limit`).

### Security
- Stripe replay protection (5min signature timestamp).
- Request body size limit (default 1MB).
- Inbound concurrency limit (default 100).
- Constant-time auth token comparison.
- Security headers (nosniff, DENY, no-store).
- SNS cert URL domain validation.
- Transform JSON injection prevention.

### Operations
- Alert system (Slack, Discord, generic webhook) on DLQ and verification failures.
- Structured JSON logging (`QHOOK_LOG_FORMAT=json`).
- Slow query logging (>100ms).
- SIGHUP config validation (dry-run reload).
- Configurable DB pool size, stale threshold, retention hours, drain timeout.

### CLI
- Commands: `init`, `start`, `validate`, `jobs list/retry`, `events list`.

### Deployment
- Docker image and Compose files.
- Deployment guides: AWS (ECS Fargate / EC2), Railway, Fly.io, Render.
- GitHub Actions CI (fmt + clippy + test + E2E).
- Documentation site (GitHub Pages).
- Examples: quickstart, github-webhook, filter-transform, stripe-checkout.
