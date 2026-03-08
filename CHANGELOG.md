# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

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
