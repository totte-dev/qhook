# Contributing to qhook

Thanks for your interest in contributing to qhook! This guide covers the development workflow and conventions.

## Getting Started

### Prerequisites

- Rust 1.85+ (edition 2024)
- Python 3 (for E2E mock server)
- Docker (optional, for SNS E2E tests with LocalStack)

### Setup

```bash
git clone https://github.com/totte-dev/qhook.git
cd qhook
cargo build
cargo test
```

## Development Workflow

### Test-Driven Development (Required)

All contributions must follow TDD. **Never implement before tests exist.**

1. Write tests first (unit tests in `#[cfg(test)] mod tests`, E2E in `tests/e2e.sh`)
2. Confirm the tests fail
3. Implement the feature or fix
4. Confirm the tests pass

For multiple features, repeat this cycle per feature — do not batch.

### Running Tests

```bash
# Unit tests
cargo test

# Lint
cargo fmt --check
cargo clippy --all-targets

# E2E tests (starts qhook + mock server)
bash tests/e2e.sh

# SNS E2E tests (requires LocalStack)
docker run -d --name localstack-qhook -p 4567:4566 \
  -e SERVICES=sns -e LOCALSTACK_HOST=localhost:4567 \
  localstack/localstack:3
LOCALSTACK_ENDPOINT=http://localhost:4567 bash tests/e2e_sns.sh
docker rm -f localstack-qhook
```

### Code Style

- `cargo fmt` — formatting is enforced in CI
- `cargo clippy` — all warnings must be resolved
- Comments and documentation in English
- IDs use ULID, timestamps use UTC

## Pull Requests

1. Fork the repository and create a feature branch from `main`
2. Follow the TDD workflow above
3. Keep changes focused — one feature or fix per PR
4. Update documentation if adding new config options, endpoints, or behavior:
   - `docs/configuration.md` for config changes
   - `CHANGELOG.md` (Keep a Changelog format)
   - Relevant guide in `docs/guides/` if applicable
5. Ensure CI passes: `cargo fmt --check && cargo clippy --all-targets && cargo test`
6. Write a clear PR description explaining what and why

### Commit Messages

Use concise, descriptive messages. Examples:

- `Add HTTP method specification for handlers`
- `Fix retry backoff calculation for 5xx errors`
- `Update configuration docs for cron sources`

## Project Structure

```
src/
  main.rs       — Entry point
  lib.rs        — Module exports
  api.rs        — HTTP handlers (webhook, event, sns, health)
  config.rs     — YAML config parsing + validation
  cron.rs       — Cron scheduler
  db.rs         — SQLite/Postgres via sqlx AnyPool
  metrics.rs    — Prometheus metrics
  queue.rs      — Job worker (poll, deliver, retry, DLQ)
  grpc.rs       — gRPC output
  verify.rs     — Signature verification
  alert.rs      — Alert notifications
  cli.rs        — CLI commands
tests/
  e2e.sh        — E2E tests
  e2e_sns.sh    — SNS E2E tests (LocalStack)
  mock_server.py — Mock HTTP server for E2E
```

## Reporting Issues

- **Bugs:** Open a [GitHub issue](https://github.com/totte-dev/qhook/issues) with steps to reproduce
- **Security:** See [SECURITY.md](SECURITY.md) — do not open public issues for vulnerabilities
- **Features:** Open an issue to discuss before implementing

## License

By contributing, you agree that your contributions will be licensed under the [Apache License 2.0](LICENSE).
