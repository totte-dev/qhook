---
layout: default
title: Compliance & Audit Trail
---

# Compliance & Audit Trail

qhook's queue-first architecture provides built-in audit trail capabilities suitable for regulated environments (PCI DSS, SOC 2, HIPAA, GDPR).

## Event Immutability

Every webhook and API event is **persisted before acknowledgment**:

```
webhook arrives → verify signature → write to DB → return 200
```

Events are stored with:
- **Unique event ID** (ULID, sortable by time)
- **Full payload** (original request body)
- **Source and event type** metadata
- **Timestamp** (UTC, millisecond precision)
- **Idempotency key** (prevents duplicate processing)

Events are retained until the configured cleanup interval (default: 72 hours). Extend this for compliance:

```yaml
worker:
  retention_hours: 8760  # 1 year
```

## Delivery Audit Trail

Every delivery attempt is recorded individually:

| Field | Description |
|-------|-------------|
| `job_id` | Unique delivery job identifier |
| `event_id` | Link to originating event |
| `handler` | Target handler name |
| `attempt` | Attempt number (1, 2, 3, ...) |
| `status_code` | HTTP response code from downstream |
| `duration_ms` | Response time in milliseconds |
| `error` | Error details (if failed) |
| `created_at` | Timestamp of this attempt |

Query delivery history via the Management API:

```bash
# Event details with all jobs
curl -H "Authorization: Bearer $TOKEN" \
  http://localhost:8888/api/events/01JQ7X...

# Job details with delivery attempts
curl -H "Authorization: Bearer $TOKEN" \
  "http://localhost:8888/api/jobs/01JQ7X...?include_attempts=true"
```

## Export for Auditors

Export events as JSONL for external audit systems:

```bash
# Export all events from a time range
qhook export events --since 2026-01-01T00:00:00 --until 2026-03-31T23:59:59 > q1-events.jsonl

# Filter by source
qhook export events --source stripe --limit 10000 > stripe-events.jsonl
```

Each line contains: `id`, `source`, `event_type`, `payload`, `created_at`, `unique_key`.

## PCI DSS 4.0 Considerations

| Requirement | qhook Feature |
|-------------|--------------|
| **6.2.4** — Software attack prevention | SSRF protection, URL validation, body size limits |
| **6.3** — Security vulnerabilities identified and managed | Webhook signature verification (11+ providers) |
| **10.2** — Audit logs capture events | Full event + delivery attempt logging |
| **10.3** — Audit logs protected | SQLite/Postgres with file system permissions |
| **10.5** — Audit log retention | Configurable `retention_hours` (default 72h, set higher for compliance) |
| **11.6** — Detect unauthorized changes | Config validation at startup, SIGHUP reload validation |

**Important:** If webhook payloads contain cardholder data (PAN, CVV), the entire qhook deployment is in PCI scope. Consider:
- Using Postgres with encryption at rest
- Restricting database access
- Running qhook in an isolated network segment

## SOC 2 Considerations

| Trust Service Criteria | qhook Feature |
|----------------------|--------------|
| **CC6.1** — Logical access controls | `api.auth_token`, `metrics_auth_token`, webhook signature verification |
| **CC7.2** — System monitoring | Prometheus metrics, Slack/Discord/webhook alerts on failures |
| **CC7.3** — Evaluate security events | `qhook_verification_failures_total` metric, DLQ alerts |
| **CC8.1** — Change management | YAML config (version-controlled), `qhook validate` pre-deploy checks |
| **A1.2** — Recovery objectives | Event replay (`qhook events replay`), DLQ retry (`qhook jobs retry`) |

## Structured Logging

Enable JSON structured logging for log aggregation (ELK, Datadog, Splunk):

```bash
RUST_LOG=info qhook start  # default: human-readable
```

Log entries include: timestamp, level, span context (job_id, event_id, handler), and message.

With the `otel` feature flag, traces are exported via OTLP for distributed tracing correlation:

```bash
cargo install qhook --features otel
OTEL_EXPORTER_OTLP_ENDPOINT=http://collector:4317 qhook start
```

## Self-Hosting Advantages

For regulated environments, self-hosting qhook provides:

- **Data sovereignty** — events never leave your infrastructure
- **Network isolation** — run in a private subnet with no internet access
- **Audit control** — full access to database and logs
- **No vendor dependency** — Apache 2.0 license, no phone-home
- **Reproducible deployments** — single binary, deterministic behavior
