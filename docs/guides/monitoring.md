---
layout: default
title: Monitoring
---

# Monitoring

## Health Check

```bash
curl http://localhost:8888/health
# {"status":"ok","queue_depth":3}
```

Returns `200` when healthy, `503` if the database is unreachable. Use as a readiness probe in Kubernetes, ECS, or any container orchestrator.

## Prometheus Metrics

```bash
curl http://localhost:8888/metrics
```

### Available Metrics

**Counters:**

| Metric | Labels | Description |
|--------|--------|-------------|
| `qhook_events_received_total` | - | Total events received |
| `qhook_events_by_source_total` | `source` | Events per source |
| `qhook_events_duplicated_total` | - | Duplicate events ignored |
| `qhook_jobs_created_total` | - | Total jobs created |
| `qhook_deliveries_total` | `result` | Delivery attempts (success/failure) |
| `qhook_deliveries_by_handler_total` | `handler`, `result` | Delivery attempts per handler |
| `qhook_delivery_duration_seconds_sum` | - | Total delivery duration |
| `qhook_delivery_duration_seconds_count` | - | Total delivery attempts |
| `qhook_verification_failures_total` | `source` | Signature verification failures |
| `qhook_dlq_total` | `handler` | Jobs moved to DLQ |
| `qhook_delivery_errors_by_type_total` | `type` | Errors by type (4xx/5xx/timeout/network) |
| `qhook_db_errors_total` | - | Database errors |
| `qhook_alerts_sent_total` | - | Alerts sent successfully |
| `qhook_alerts_failed_total` | - | Failed alert sends |

**Gauges:**

| Metric | Description |
|--------|-------------|
| `qhook_queue_depth` | Jobs waiting to be delivered |
| `qhook_dead_jobs` | Jobs in dead letter queue |
| `qhook_delivery_duration_seconds_max` | Max single delivery duration |
| `qhook_metric_label_count` | Unique label values (monitors for label explosion) |

### Prometheus Config

```yaml
# prometheus.yml
scrape_configs:
  - job_name: qhook
    static_configs:
      - targets: ['localhost:8888']
    metrics_path: /metrics
    scrape_interval: 15s
```

### Grafana Dashboard

Key panels to set up:

- **Event rate**: `rate(qhook_events_received_total[5m])`
- **Delivery success rate**: `rate(qhook_deliveries_total{result="success"}[5m]) / rate(qhook_deliveries_total[5m])`
- **Queue depth**: `qhook_queue_depth`
- **DLQ growth**: `rate(qhook_dlq_total[5m])`
- **P99 delivery latency**: `qhook_delivery_duration_seconds_max`

## Alerts

qhook can send webhook alerts when jobs are moved to the DLQ or signature verification fails.

```yaml
alerts:
  url: https://hooks.slack.com/services/T.../B.../xxx
  type: slack
  on: [dlq, verification_failure]
```

### Alert Types

**`generic`** (default): JSON payload with structured fields.

```json
{"alert": "dlq", "message": "Job moved to DLQ", "handler": "payment", "job_id": "01JXXX"}
```

**`slack`**: Slack incoming webhook format.

```json
{"text": ":warning: [qhook] Job moved to DLQ: handler=payment job_id=01JXXX"}
```

**`discord`**: Discord webhook format with embeds.

### Alert Events

| Event | Triggered when |
|-------|---------------|
| `dlq` | A job exhausts all retry attempts and moves to the Dead Letter Queue |
| `verification_failure` | An inbound webhook fails signature verification |

## Structured Logging

For production log aggregation (CloudWatch, Datadog, ELK), enable JSON logging:

```bash
QHOOK_LOG_FORMAT=json qhook start
```

Example output:

```json
{"timestamp":"2025-01-15T10:30:00.123Z","level":"INFO","target":"qhook","message":"event received","event_id":"01JXXX","event_type":"order.created","source":"stripe"}
```

### Slow Query Logging

Database queries exceeding 100ms are logged as warnings:

```
WARN slow query: SELECT ... (152ms)
```

## Operational Signals

| Signal | Command | Effect |
|--------|---------|--------|
| `SIGTERM` | `kill <pid>` | Graceful shutdown (drain in-flight, then exit) |
| `SIGINT` | `Ctrl+C` | Same as SIGTERM |
| `SIGHUP` | `kill -HUP <pid>` | Validate config without restart (dry-run reload) |
