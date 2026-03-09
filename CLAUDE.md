# qhook Development Guide

## Project Overview

qhook is a lightweight event gateway with built-in queue and retry. Rust (axum + sqlx + reqwest).

## Development Workflow

### 1. Task Management

- **Tasks live in `KANBAN.md`** at the project root.
- Before starting work, read KANBAN.md to pick the next task from "In Progress" or "Todo".
- Move the task to "In Progress" when starting.
- Move to "Done" when complete, then pick the next task.
- Add new tasks as they emerge during development.
- **Update `CHANGELOG.md`** when completing features (Keep a Changelog format).
- **Update `README.md` and docs** when adding features — new config options, endpoints, sections as needed.

### 2. Test-Driven Development (MANDATORY)

**NEVER implement before tests exist.** Always:

1. Write unit tests in `#[cfg(test)] mod tests` within the relevant source file.
2. Write E2E tests in `tests/e2e.sh` (basic) or `tests/e2e_sns.sh` (SNS/LocalStack).
3. Run tests to **confirm they fail**.
4. Implement the feature.
5. Run tests to **confirm they pass**.

If implementing multiple features, repeat this cycle per feature — do not batch implementations before writing tests.

### 3. Test Commands

```bash
# Unit tests (fast, always run)
cargo test

# E2E tests (starts qhook + mock server, no external deps)
bash tests/e2e.sh

# SNS E2E tests (requires LocalStack)
docker run -d --name localstack-qhook -p 4567:4566 -e SERVICES=sns -e LOCALSTACK_HOST=localhost:4567 localstack/localstack:3
LOCALSTACK_ENDPOINT=http://localhost:4567 bash tests/e2e_sns.sh
docker rm -f localstack-qhook
```

### 4. Build

```bash
cargo build            # debug
cargo build --release  # release
```

## Project Structure

```
src/
  main.rs       — Entry point (minimal)
  lib.rs        — Module exports (for Cloud version dependency)
  api.rs        — HTTP handlers (webhook, event, sns, health)
  config.rs     — YAML config parsing + env var expansion
  cron.rs       — Cron scheduler (periodic event generation)
  db.rs         — SQLite/Postgres via sqlx AnyPool
  metrics.rs    — Prometheus metrics (atomic counters, no deps)
  queue.rs      — Job worker (poll, deliver, retry, DLQ)
  grpc.rs       — gRPC output (prost types, tonic client, no codegen)
  verify.rs     — Signature verification (GitHub, Stripe, Shopify, HMAC, SNS X.509)
  cli.rs        — CLI commands (start, init, validate, jobs, events)
tests/
  e2e.sh        — E2E tests (26 tests)
  e2e_sns.sh    — SNS E2E tests with LocalStack (8 tests)
  mock_server.py — Python mock HTTP server for E2E
  bench.sh      — Benchmark script (receive RPS + delivery throughput)
.github/workflows/
  ci.yml        — GitHub Actions CI (fmt, clippy, test, E2E)
  pages.yml     — GitHub Pages deploy (docs/ on push to main)
examples/
  quickstart/       — Minimal setup, no Docker (qhook binary + curl)
  github-webhook/   — GitHub push/PR with verification + fan-out
  filter-transform/ — Event filtering + payload transformation
  stripe-checkout/  — Stripe checkout with dual handlers
docs/               — GitHub Pages user guide (Jekyll, Cayman theme)
  index.md          — Top page with navigation
  getting-started.md, configuration.md, cli.md, examples.md
  guides/           — Feature guides (webhook-verification, cloudevents, sns, filtering, grpc, monitoring, security)
  deploy/           — Platform deploy guides (aws, flyio, railway, render)
  why-qhook.md      — DIY vs qhook comparison
```

## Conventions

- **Language**: Code, comments, and all documentation in English. Exception: `docs/private/` may be in Japanese. User communication in Japanese.
- **Source type strings**: `webhook`, `event`, `sns`, `cron` (in config YAML and source_type field).
- **Event type extraction order**: CloudEvents `ce-type` header → structured mode `type` field → provider-specific logic.
- **Database**: SQLite for dev/testing, Postgres for production. Both via sqlx AnyPool.
- **IDs**: ULID for event_id and job_id.
- **Timestamps**: UTC, format `%Y-%m-%dT%H:%M:%S%.3f` stored as TEXT.
- **Config env vars**: `${VAR}` or `${VAR:-default}` syntax.
- **Rust version**: 1.85+ (edition 2024). Use latest stable features (e.g., let chains).
- **Version**: Currently v0.1.0. Update in Cargo.toml for releases.
- **Docker image**: `ghcr.io/totte-dev/qhook`. Multi-stage build, ~119MB.

## Key Design Decisions

- **Single binary, no external deps** — no Redis, no RabbitMQ. The queue is built into qhook.
- **lib.rs exports all modules** — enables future Cloud version to depend on qhook as a library crate.
- **skip_verify on SourceConfig** — allows testing SNS with LocalStack (no real X.509 signatures).
- **CloudEvents headers forwarded** — `ce-*` headers stored with event and forwarded on delivery.
- **Open Core model** — OSS base (this repo), paid Cloud version planned (UI, analytics, multi-tenant).
- **Lightweight always** — no new crate dependencies for metrics/rate-limiting/shutdown. Use stdlib atomics, tokio semaphores, and manual Prometheus text format.
- **Zero-config safety** — stale recovery (5min), cleanup (72h), concurrent delivery (10), graceful shutdown all work with no config.
- **Adaptive polling** — 50ms when busy, 1s when idle. Balances throughput (~100 deliveries/sec) with low CPU usage at rest.
- **Postgres-optimized** — `FOR UPDATE SKIP LOCKED` for multi-instance deployments. SQLite uses optimistic locking (single writer).
- **Rate limiting via semaphore hold** — permits held for 1s after delivery to enforce per-second rate. Simple, no token bucket needed.
