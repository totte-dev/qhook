---
layout: default
title: Local to Production
---

# Local to Production

This guide walks through the changes needed to take a qhook setup from local development to production.

## Environment Overlays

qhook supports environment-specific configuration without duplicating your entire config file. The `--env` flag (or `QHOOK_ENV` environment variable) loads an overlay on top of your base config:

```bash
qhook start --env production     # merges qhook.production.yaml + loads .env.production
qhook start --env staging        # merges qhook.staging.yaml + loads .env.staging
QHOOK_ENV=production qhook start # same, via environment variable
```

**Resolution order:**

1. Load `.env.<name>` (if exists) -- sets environment variables (does not override existing ones)
2. Read base config (`qhook.yaml`)
3. Deep merge `qhook.<name>.yaml` on top (if exists) -- overlay values override base, mappings merge recursively
4. Expand `${VAR}` references and validate

The overlay only needs to contain the fields you want to change. Everything else is inherited from the base config.

### File layout

```
project/
  qhook.yaml                # base config (shared structure)
  qhook.local.yaml           # local overrides (allow_private_urls, etc.)
  qhook.production.yaml      # production overrides (Postgres, auth, etc.)
  .env.local                  # local secrets
  .env.production             # production secrets (or use real env vars)
```

`qhook init` generates `qhook.yaml` and `qhook.local.yaml` automatically.

## Database Migration

### SQLite to Postgres

qhook runs database migrations automatically on startup -- no manual migration step is needed. To switch from SQLite to Postgres:

1. **Install Postgres** and create a database:

```bash
createdb qhook
```

2. **Update your production overlay** (`qhook.production.yaml`):

```yaml
database:
  driver: postgres
  url: ${DATABASE_URL}
  max_connections: 20
```

3. **Set the connection string** in `.env.production`:

```
DATABASE_URL=postgres://user:password@localhost:5432/qhook
```

4. **Start qhook** with the production environment:

```bash
qhook start --env production
```

qhook creates all tables and indexes on first startup. Existing events in SQLite are not migrated -- this is a fresh start.

**Multi-instance deployments:** Postgres uses `FOR UPDATE SKIP LOCKED` for job claiming, which supports running multiple qhook instances against the same database safely.

### SQLite to MySQL

The process is the same as Postgres:

1. **Create a MySQL database:**

```sql
CREATE DATABASE qhook CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
```

2. **Update your production overlay:**

```yaml
database:
  driver: mysql
  url: ${DATABASE_URL}
  max_connections: 20
```

3. **Set the connection string** in `.env.production`:

```
DATABASE_URL=mysql://user:password@localhost:3306/qhook
```

4. **Start with `--env production`** -- tables are created automatically.

## Security Checklist

These settings should be changed before running in production:

### 1. Set API authentication

Without `auth_token`, the `/events` and Management API endpoints are open. qhook logs a warning at startup if this is unset.

```yaml
# qhook.production.yaml
api:
  auth_token: ${QHOOK_API_TOKEN}
  metrics_auth_token: ${QHOOK_METRICS_TOKEN}
```

### 2. Disable private URL access

`allow_private_urls: true` disables SSRF protection. In production, leave it at the default (`false`) so handler URLs pointing to `localhost`, `127.0.0.1`, `10.x.x.x`, `172.16-31.x.x`, `192.168.x.x`, and `169.254.x.x` are rejected.

```yaml
# qhook.local.yaml (dev only)
server:
  allow_private_urls: true

# qhook.production.yaml -- omit allow_private_urls (defaults to false)
```

### 3. Enable trust_proxy

When running behind a reverse proxy (nginx, ALB, Caddy), set `trust_proxy: true` so rate limiting and IP allowlists use the real client IP from `X-Forwarded-For` / `X-Real-IP`:

```yaml
# qhook.production.yaml
server:
  trust_proxy: true
```

### 4. Configure rate limiting

```yaml
# qhook.production.yaml
server:
  ip_rate_limit: 100    # requests/sec per IP
```

Returns `429 Too Many Requests` when exceeded. Without this, there is no per-IP throttling.

### 5. Use environment variables for secrets

Never commit secrets to config files. Use `${VAR}` or `${VAR:-default}` syntax:

```yaml
sources:
  stripe:
    secret: ${STRIPE_WEBHOOK_SECRET}    # expanded at load time

api:
  auth_token: ${QHOOK_API_TOKEN}

alerts:
  url: ${SLACK_WEBHOOK_URL}
```

Set the actual values in `.env.production` (loaded automatically with `--env production`) or in your deployment platform's environment variable settings.

### 6. Set up alerts

Configure alerts for dead-letter queue events and verification failures:

```yaml
# qhook.production.yaml
alerts:
  url: ${SLACK_WEBHOOK_URL}
  type: slack
  on: [dlq, verification_failure]
```

## Pull-Mode Queues in Production

Pull-mode queues let your app fetch events on its own schedule via `GET /api/queues/{name}/messages`. For production:

### Set an API key per queue

Without `api_key`, queue endpoints are open. Set a per-queue key:

```yaml
queues:
  orders:
    source: stripe
    events: [checkout.session.completed]
    api_key: ${ORDERS_QUEUE_API_KEY}
    visibility_timeout: 60s
    max_attempts: 5
```

Consumers authenticate with `Authorization: Bearer <key>`.

### Use Postgres for multi-instance

SQLite supports a single writer. If you run multiple qhook instances (for availability or throughput), use Postgres so that `FOR UPDATE SKIP LOCKED` prevents duplicate message delivery across instances.

## Pre-Deploy: `qhook doctor`

Run `qhook doctor` against your production config before deploying:

```bash
qhook doctor -c qhook.yaml
```

Doctor checks:

| Check | What it verifies |
|-------|-----------------|
| Config valid | YAML syntax, source/handler references, URL schemes |
| Database connection | Connects to the configured database |
| Server reachable | `GET /health` returns 200 (run against a running instance) |
| Handler endpoints | Each handler URL is reachable (no 5xx) |
| Workflow step endpoints | Each workflow step URL is reachable |

If running doctor before the server is started, the server reachability check will fail -- that is expected. The config validation and database connection checks still run.

## Example: Complete Production Overlay

```yaml
# qhook.production.yaml
database:
  driver: postgres
  url: ${DATABASE_URL}
  max_connections: 20

server:
  ip_rate_limit: 100
  trust_proxy: true

api:
  auth_token: ${QHOOK_API_TOKEN}
  metrics_auth_token: ${QHOOK_METRICS_TOKEN}

delivery:
  signing_secret: ${QHOOK_SIGNING_SECRET}
  default_retry:
    max: 8
    interval: 60s

worker:
  max_concurrency: 20
  batch_size: 20
  retention_hours: 168    # 7 days

handlers:
  billing:
    url: https://api.internal/billing/webhook
  analytics:
    url: https://api.internal/analytics/ingest

alerts:
  url: ${SLACK_WEBHOOK_URL}
  type: slack
  on: [dlq, verification_failure]
```

```bash
# .env.production
DATABASE_URL=postgres://qhook:secret@db.internal:5432/qhook
QHOOK_API_TOKEN=tok_prod_abc123
QHOOK_METRICS_TOKEN=tok_metrics_xyz
QHOOK_SIGNING_SECRET=whsec_prod_signing_key
STRIPE_WEBHOOK_SECRET=whsec_stripe_live_key
SLACK_WEBHOOK_URL=https://hooks.slack.com/services/T00/B00/xxx
```

The base `qhook.yaml` keeps sources, handler routing, and workflow definitions. The production overlay changes only what differs: database, URLs, security settings, and tuning parameters.
