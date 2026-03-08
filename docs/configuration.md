---
layout: default
title: Configuration
---

# Configuration Reference

qhook is configured via a single YAML file. Environment variables are expanded using `${VAR}` or `${VAR:-default}` syntax.

## Full Example

```yaml
database:
  driver: sqlite                    # sqlite (default) or postgres
  # url: ${DATABASE_URL}            # required for postgres
  max_connections: 10               # DB pool size (default: 10)

server:
  port: 8888                        # listen port (default: 8888)
  max_body_size: 1048576            # max request body in bytes (default: 1MB)
  max_inbound: 100                  # max concurrent inbound requests (default: 100)
  ip_rate_limit: 100                # per-IP requests/sec limit (default: 0 = disabled)

api:
  auth_token: ${QHOOK_API_TOKEN}    # bearer token for /events endpoint (optional)

delivery:
  signing_secret: ${QHOOK_SIGNING_SECRET}  # sign outgoing deliveries (optional)
  timeout: 30s                             # delivery HTTP timeout (default: 30s)
  default_retry:
    max: 5                          # max delivery attempts (default: 5)
    backoff: exponential            # exponential (default) or fixed
    interval: 30s                   # base retry interval (default: 30s)

worker:
  stale_threshold_secs: 300         # recover stuck jobs after N seconds (default: 300)
  retention_hours: 72               # purge completed/dead records after N hours (default: 72)
  drain_timeout_secs: 30            # max wait for in-flight deliveries on shutdown (default: 30)

sources:
  stripe:
    type: webhook
    verify: stripe
    secret: ${STRIPE_WEBHOOK_SECRET}

  app:
    type: event

  my-sns:
    type: sns
    # skip_verify: true             # for LocalStack testing

handlers:
  payment-success:
    source: stripe
    events: [checkout.session.completed, invoice.paid]
    url: http://backend:3000/jobs/payment
    type: http                      # http (default) or grpc
    retry: { max: 8 }
    timeout: 60s
    idempotency_key: "$.id"
    rate_limit: 10
    filter: "$.data.object.status == paid"
    transform: |
      {"order_id": "{{$.data.object.id}}", "amount": {{$.data.object.amount}}}

alerts:
  url: ${SLACK_WEBHOOK_URL}
  type: slack                       # slack / discord / generic
  on: [dlq, verification_failure]
```

## Sections

### database

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `driver` | string | `sqlite` | Database driver: `sqlite` or `postgres` |
| `url` | string | - | Connection string. Required for postgres. For sqlite, defaults to `./qhook.db` |
| `max_connections` | integer | `10` | Connection pool size |

### server

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `port` | integer | `8888` | HTTP listen port |
| `max_body_size` | integer | `1048576` | Max request body size in bytes (1MB) |
| `max_inbound` | integer | `100` | Max concurrent inbound requests. Returns 503 when exceeded |
| `ip_rate_limit` | integer | `0` | Per-IP requests/sec limit. 0 = disabled. Returns 429 when exceeded |

### api

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `auth_token` | string | - | Bearer token for the `/events` endpoint. If not set, the endpoint is open (with a startup warning) |

### delivery

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `signing_secret` | string | - | HMAC-SHA256 key for signing outgoing deliveries |
| `timeout` | duration | `30s` | Default HTTP timeout for delivery requests |
| `default_retry.max` | integer | `5` | Max delivery attempts |
| `default_retry.backoff` | string | `exponential` | `exponential` or `fixed` |
| `default_retry.interval` | duration | `30s` | Base retry interval |

### worker

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `stale_threshold_secs` | integer | `300` | Jobs stuck in `running` longer than this are recovered |
| `retention_hours` | integer | `72` | Completed/dead records older than this are purged |
| `drain_timeout_secs` | integer | `30` | Max seconds to wait for in-flight deliveries on shutdown |

### sources

Each source is a named entry under `sources:`.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `type` | string | required | `webhook`, `event`, or `sns` |
| `verify` | string | - | Signature verification: `github`, `stripe`, `shopify`, or `hmac` |
| `secret` | string | - | Shared secret for signature verification. Required when `verify` is set |
| `skip_verify` | boolean | `false` | Skip SNS X.509 verification (for testing with LocalStack) |

**Source types:**

| Type | Endpoint | Description |
|------|----------|-------------|
| `webhook` | `POST /webhooks/{source}` | External webhooks with signature verification |
| `event` | `POST /events/{event_type}` | Internal events with optional bearer token auth |
| `sns` | `POST /sns/{source}` | AWS SNS with auto-confirmation and envelope unwrapping |

### handlers

Each handler is a named entry under `handlers:`.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `source` | string | required | Source name to subscribe to |
| `events` | list | `[]` (all) | Event types to match. Empty = all events from this source |
| `url` | string | required | Delivery target URL |
| `type` | string | `http` | Delivery protocol: `http` or `grpc` |
| `retry` | object | - | Override `default_retry` for this handler |
| `timeout` | duration | - | Override delivery timeout for this handler |
| `idempotency_key` | string | - | JSONPath to dedup key in payload (e.g., `$.id`) |
| `rate_limit` | integer | - | Max deliveries/sec to this handler |
| `filter` | string | - | JSONPath filter expression. Job created only if filter matches |
| `transform` | string | - | Payload transformation template with `{{$.path}}` placeholders |

### alerts

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `url` | string | required | Alert webhook URL |
| `type` | string | `generic` | `generic`, `slack`, or `discord` |
| `on` | list | required | Events to alert on: `dlq`, `verification_failure` |

## Environment Variables

Use `${VAR}` or `${VAR:-default}` syntax in any string field:

```yaml
server:
  port: ${PORT:-8888}

sources:
  stripe:
    secret: ${STRIPE_WEBHOOK_SECRET}
```

## Validation

Validate your config without starting the server:

```bash
qhook validate
qhook validate -c /path/to/qhook.yaml
```

Checks performed:
- YAML syntax
- Source type is valid (`webhook`, `event`, `sns`)
- Handler type is valid (`http`, `grpc`)
- Handler references an existing source
- `verify` requires `secret` to be set (non-empty)
- Handler URLs use http/https scheme (private IPs trigger warnings)
- Alert config has valid `on` events
