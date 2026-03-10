# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

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
