# qhook Development Guide

## Project Overview

qhook is a lightweight webhook receiver with built-in queue and retry. Rust (axum + sqlx + reqwest).

## Development Workflow

### 1. Task Management

- **Tasks live in `KANBAN.md`** at the project root.
- Before starting work, read KANBAN.md to pick the next task from "In Progress" or "Todo".
- Move the task to "In Progress" when starting.
- Move to "Done" when complete, then pick the next task.
- Add new tasks as they emerge during development.

### 2. Test-Driven Development

Write or update tests **before** implementing features:

1. Write unit tests in `#[cfg(test)] mod tests` within the relevant source file.
2. Write E2E tests in `tests/e2e.sh` (basic) or `tests/e2e_sns.sh` (SNS/LocalStack).
3. Run tests to confirm they fail.
4. Implement the feature.
5. Run tests to confirm they pass.

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
  db.rs         — SQLite/Postgres via sqlx AnyPool
  queue.rs      — Job worker (poll, deliver, retry, DLQ)
  verify.rs     — Signature verification (GitHub, Stripe, Shopify, HMAC, SNS X.509)
  cli.rs        — CLI commands (start, init, validate, jobs, events)
tests/
  e2e.sh        — E2E tests (14 tests)
  e2e_sns.sh    — SNS E2E tests with LocalStack (8 tests)
  mock_server.py — Python mock HTTP server for E2E
docs/           — Deployment guides, why-qhook comparison
examples/       — Example configs, stripe-checkout sample app
```

## Conventions

- **Language**: Code and comments in English. User communication in Japanese.
- **Source type strings**: `webhook`, `event`, `sns` (in config YAML and source_type field).
- **Event type extraction order**: CloudEvents `ce-type` header → structured mode `type` field → provider-specific logic.
- **Database**: SQLite for dev/testing, Postgres for production. Both via sqlx AnyPool.
- **IDs**: ULID for event_id and job_id.
- **Timestamps**: UTC, format `%Y-%m-%dT%H:%M:%S%.3f` stored as TEXT.
- **Config env vars**: `${VAR}` or `${VAR:-default}` syntax.
- **Version**: Currently v0.2.0. Update in Cargo.toml for releases.
- **Docker image**: `ghcr.io/totte-dev/qhook`. Multi-stage build, ~119MB.
- **Commit**: Include `Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>`.

## Key Design Decisions

- **Single binary, no external deps** — no Redis, no RabbitMQ. The queue is built into qhook.
- **lib.rs exports all modules** — enables future Cloud version to depend on qhook as a library crate.
- **skip_verify on SourceConfig** — allows testing SNS with LocalStack (no real X.509 signatures).
- **CloudEvents headers forwarded** — `ce-*` headers stored with event and forwarded on delivery.
- **Open Core model** — OSS base (this repo), paid Cloud version planned (UI, analytics, multi-tenant).
