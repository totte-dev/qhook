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
  # allow_private_urls: true        # allow localhost/private IPs in handler URLs (dev only)
  # trust_proxy: true               # trust X-Forwarded-For for rate limiting

api:
  auth_token: ${QHOOK_API_TOKEN}    # bearer token for /events endpoint (optional)
  # metrics_auth_token: ${METRICS_TOKEN}  # bearer token for /metrics endpoint (optional)

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

  daily-report:
    type: cron
    schedule: "0 9 * * MON-FRI"      # 9 AM on weekdays
    # timezone: "+09:00"             # default: UTC

handlers:
  payment-success:
    source: stripe
    events: [checkout.session.completed, invoice.paid]
    url: http://backend:3000/jobs/payment
    type: http                      # http (default) or grpc
    method: POST                    # GET, POST (default), PUT, PATCH, DELETE
    retry: { max: 8 }
    timeout: 60s
    idempotency_key: "$.id"
    rate_limit: 10
    filter: "$.data.object.status == paid"
    transform: |
      {"order_id": "{{$.data.object.id}}", "amount": {{$.data.object.amount}}}
    headers:                            # custom HTTP headers (optional)
      Authorization: "Bearer ${API_TOKEN}"

alerts:
  url: ${SLACK_WEBHOOK_URL}
  type: slack                       # slack / discord / generic
  on: [dlq, verification_failure]

workflows:
  order-pipeline:
    source: app
    events: [order.created]
    timeout: 3600                     # workflow timeout in seconds (optional)
    params:                           # input parameter validation (optional)
      - name: id
        type: string
      - name: amount
        type: number
    steps:
      - name: validate
        url: http://backend:3000/validate
        retry: { max: 3, errors: [5xx, timeout] }
        catch:
          - errors: [4xx]
            goto: handle-error
      - name: fulfill
        url: http://backend:3000/fulfill
        result_path: "$.fulfillment"
      - name: wait-for-shipping
        type: wait
        seconds: 300                  # fixed delay
      - name: notify
        url: http://backend:3000/notify
        end: true
      - name: handle-error
        url: http://backend:3000/error-handler
        end: true
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
| `allow_private_urls` | boolean | `false` | Allow handler URLs pointing to private/loopback IPs. Enable for local development |
| `trust_proxy` | boolean | `false` | Trust `X-Forwarded-For` / `X-Real-IP` headers for IP rate limiting. Enable when behind a reverse proxy |

### api

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `auth_token` | string | - | Bearer token for the `/events` endpoint. If not set, the endpoint is open (with a startup warning) |
| `metrics_auth_token` | string | - | Bearer token for the `/metrics` endpoint. If not set, the endpoint is open |

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
| `type` | string | required | `webhook`, `event`, `sns`, or `cron` |
| `verify` | string | - | Signature verification: `github`, `stripe`, `shopify`, or `hmac` |
| `secret` | string | - | Shared secret for signature verification. Required when `verify` is set |
| `skip_verify` | boolean | `false` | Skip SNS X.509 verification (for testing with LocalStack) |
| `schedule` | string | - | Cron expression (required for `cron` sources). 5-field standard or 6-field with seconds |
| `timezone` | string | `UTC` | Timezone for cron evaluation. `UTC` or fixed offset like `+09:00` |

**Source types:**

| Type | Endpoint | Description |
|------|----------|-------------|
| `webhook` | `POST /webhooks/{source}` | External webhooks with signature verification |
| `event` | `POST /events/{event_type}` | Internal events with optional bearer token auth |
| `sns` | `POST /sns/{source}` | AWS SNS with auto-confirmation and envelope unwrapping |
| `cron` | *(internal)* | Time-based trigger. Fires `cron.tick` events on schedule |

### handlers

Each handler is a named entry under `handlers:`.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `source` | string | required | Source name to subscribe to |
| `events` | list | `[]` (all) | Event types to match. Empty = all events from this source |
| `url` | string | required | Delivery target URL |
| `type` | string | `http` | Delivery protocol: `http` or `grpc` |
| `method` | string | `POST` | HTTP method: `GET`, `POST`, `PUT`, `PATCH`, `DELETE` |
| `retry` | object | - | Override `default_retry` for this handler |
| `timeout` | duration | - | Override delivery timeout for this handler |
| `idempotency_key` | string | - | JSONPath to dedup key in payload (e.g., `$.id`) |
| `rate_limit` | integer | - | Max deliveries/sec to this handler |
| `filter` | string | - | JSONPath filter expression. Job created only if filter matches |
| `transform` | string | - | Payload transformation template with `{{$.path}}` placeholders |
| `headers` | map | `{}` | Custom HTTP headers to send with delivery (e.g., `Authorization`) |

**Circuit breaker:** Each handler has an automatic circuit breaker. After 5 consecutive delivery failures, the circuit opens and deliveries are temporarily rejected (jobs are rescheduled). After 60 seconds, a single probe request is allowed through (half-open state). If it succeeds, the circuit closes; if it fails, it stays open for another cooldown period.

### alerts

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `url` | string | required | Alert webhook URL |
| `type` | string | `generic` | `generic`, `slack`, or `discord` |
| `on` | list | required | Events to alert on: `dlq`, `verification_failure` |

### workflows

Each workflow is a named entry under `workflows:`.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `source` | string | required | Source name to subscribe to |
| `events` | list | `[]` (all) | Event types to trigger this workflow |
| `timeout` | integer | - | Workflow timeout in seconds. Steps are skipped after expiry |
| `params` | list | `[]` | Input parameters for validation (see below) |
| `steps` | list | required | Ordered list of steps |

**Param fields:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | string | required | Parameter name (top-level key in event payload) |
| `type` | string | `string` | Expected type: `string`, `number`, `boolean`, `object`, `array` |
| `required` | boolean | `true` | Whether this parameter must be present |

**Step fields:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | string | required | Unique step name (used for `goto` references) |
| `type` | string | `http` | Step type: `http`, `choice`, `parallel`, `map`, `wait`, `callback` |
| `url` | string | - | Delivery target URL (required for `http` steps). For `callback` steps, URL to notify with the callback token |
| `method` | string | `POST` | HTTP method: `GET`, `POST`, `PUT`, `PATCH`, `DELETE` |
| `headers` | map | `{}` | Custom HTTP headers to send with this step's request |
| `retry` | object | - | Override retry config. Supports `errors` field for error type matching (`5xx`, `4xx`, `timeout`, `network`, `all`) |
| `catch` | list | - | Error routing rules. Each entry has `errors` (list) and `goto` (step name) |
| `on_failure` | string | - | Set to `continue` to proceed to next step on failure |
| `input` | string | - | JSONPath transformation applied before the step |
| `result_path` | string | - | JSONPath to merge step output into (e.g., `$.fulfillment`) |
| `output` | string | - | JSONPath transformation applied after the step |
| `end` | boolean | `false` | Mark this step as a terminal step |
| `goto` | string | - | Jump to a named step instead of the next sequential step |
| `seconds` | integer | - | Fixed delay in seconds (for `wait` steps) |
| `timestamp_path` | string | - | JSONPath to a timestamp in the payload (for `wait` steps). Supports ISO 8601, RFC 3339, and Unix timestamps |
| `callback_timeout` | integer | - | Timeout in seconds for callback steps (optional) |
| `choices` | list | - | Conditions for `choice` steps. Each entry has `condition`, `goto` |
| `default` | string | - | Default `goto` for `choice` steps when no condition matches |
| `branches` | list | - | Branch definitions for `parallel` steps |
| `items_path` | string | - | JSONPath to array for `map` steps |
| `iterator` | object | - | Step definition applied to each item in `map` steps |

> See the [Workflows guide](https://totte-dev.github.io/qhook/guides/workflows) for detailed examples of each step type.

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
- Source type is valid (`webhook`, `event`, `sns`, `cron`)
- Handler type is valid (`http`, `grpc`)
- HTTP method is valid (`GET`, `POST`, `PUT`, `PATCH`, `DELETE`)
- Handler references an existing source
- `verify` requires `secret` to be set (non-empty)
- Cron sources have a valid `schedule` and `timezone`
- Handler URLs use http/https scheme (private IPs trigger warnings)
- Alert config has valid `on` events
- Workflow references an existing source
- Workflow steps have valid types and required fields
- Wait steps have `seconds` or `timestamp_path`
- Choice steps have at least one condition
- Step `goto` and `catch` targets reference existing step names
