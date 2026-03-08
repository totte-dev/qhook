# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Added
- **Stale job recovery**: Jobs stuck in `running` for over 5 minutes are automatically recovered to `retryable` on startup and every hour.
- **Concurrent delivery**: Up to 10 jobs are delivered in parallel (was sequential).
- **Graceful shutdown**: SIGTERM/SIGINT stops accepting new requests, drains in-flight deliveries, then exits cleanly.
- **Auto cleanup**: Completed/dead jobs and their attempt records older than 72 hours are automatically purged every hour.
- **Postgres atomic locking**: `SELECT ... FOR UPDATE SKIP LOCKED` eliminates race conditions when running multiple qhook instances.
- **Adaptive polling**: Worker polls every 50ms while jobs are available, drops to 1s when idle. ~10x throughput improvement.
- **Benchmark script**: `tests/bench.sh` measures receive RPS and delivery throughput.
- **Prometheus metrics**: `GET /metrics` exposes counters (events, jobs, deliveries, duration) and gauges (queue depth, dead jobs) in Prometheus text format. Zero external dependencies.
- **Health check**: `GET /health` returns JSON with `status` and `queue_depth`, returns 503 if DB is unreachable.
- **GitHub Actions CI**: Format, clippy, unit tests, and E2E tests on push/PR to main.
- **Rate limiting**: Per-handler `rate_limit` config (max deliveries/sec). Uses semaphore with 1-second hold to enforce sliding window.

## [0.2.0] - 2025-03-02

### Added
- **CloudEvents support**: Binary mode (`ce-type` header) and structured mode (`application/cloudevents+json`). `ce-*` headers are forwarded to handlers on delivery.
- **AWS SNS input**: `POST /sns/{source}` endpoint with automatic subscription confirmation, message envelope unwrapping, and X.509 signature verification (SHA1/SHA256).
- **`skip_verify` option**: Bypass SNS signature verification for LocalStack / testing.
- Unit tests (34 tests) and E2E tests (14 + 8 SNS tests).
- devcontainer with LocalStack for SNS integration testing.

### Changed
- Project description updated from "webhook receiver" to "event gateway".

## [0.1.0] - 2025-02-22

### Added
- Webhook receive, queue, and retry with exponential backoff.
- Signature verification: GitHub, Stripe, Shopify, generic HMAC.
- Idempotency via configurable JSONPath dedup key.
- Dead Letter Queue for exhausted jobs.
- CLI commands: `init`, `start`, `validate`, `jobs list/retry`, `events list`.
- SQLite and Postgres support via sqlx AnyPool.
- Docker image and Compose files.
- Deployment guides: AWS (ECS Fargate / EC2), Railway, Fly.io, Render.
- Stripe checkout example app.
