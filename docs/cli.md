---
layout: default
title: CLI Reference
---

# CLI Reference

## Commands

### qhook start

Start the event gateway server.

```bash
qhook start                        # uses ./qhook.yaml
qhook start -c /path/to/qhook.yaml # custom config path
```

**Signals:**
- `SIGTERM` / `SIGINT` -- graceful shutdown (stops accepting requests, drains in-flight deliveries)
- `SIGHUP` -- validate config and log changes (added/removed sources, handlers, workflows; warns about port/database changes requiring restart)

### qhook init

Generate a starter `qhook.yaml` in the current directory.

```bash
qhook init
```

### qhook validate

Validate a config file without starting the server.

```bash
qhook validate                     # uses ./qhook.yaml
qhook validate -c /path/to/config  # custom path
```

Exits 0 on success, non-zero with error details on failure.

### qhook jobs list

List jobs in the queue.

```bash
qhook jobs list                    # all jobs
qhook jobs list --status dead      # only dead (DLQ) jobs
qhook jobs list --status completed # only completed jobs
qhook jobs list --limit 50         # limit results
```

**Job statuses:** `available`, `running`, `completed`, `retryable`, `dead`

### qhook jobs retry

Retry failed jobs.

```bash
qhook jobs retry                   # retry all dead jobs
qhook jobs retry <JOB_ID>         # retry a specific job
```

Moves jobs from `dead` back to `available` for redelivery.

### qhook events list

List received events.

```bash
qhook events list                  # recent events
qhook events list --limit 50      # limit results
```

### qhook events replay

Replay historical events by re-creating jobs for matching handlers.

```bash
qhook events replay                                    # replay all events (with confirmation)
qhook events replay --source stripe                    # only events from a specific source
qhook events replay --event-type order.created         # only a specific event type
qhook events replay --since 2026-03-01T00:00:00       # events after a timestamp
qhook events replay --until 2026-03-10T00:00:00       # events before a timestamp
qhook events replay --source stripe --limit 50 -y     # combine filters, skip confirmation
```

**Options:**

| Flag | Description |
|------|-------------|
| `--source` | Filter by source name |
| `-t`, `--event-type` | Filter by event type |
| `--since` | Only events created after this timestamp |
| `--until` | Only events created before this timestamp |
| `-l`, `--limit` | Max events to replay (default: 100) |
| `-y`, `--yes` | Skip confirmation prompt |

Replay respects the current config: only handlers matching the event's source and type (including filters) will have jobs created.

### qhook workflow-runs list

List workflow runs.

```bash
qhook workflow-runs list                    # all workflow runs
qhook workflow-runs list --status completed # filter by status
qhook workflow-runs list --status failed    # failed workflows
qhook workflow-runs list --limit 50         # limit results
```

**Workflow run statuses:** `pending`, `running`, `completed`, `failed`

### qhook workflow-runs redrive

Redrive a failed workflow run from the beginning.

```bash
qhook workflow-runs redrive <RUN_ID>       # redrive a specific workflow run
```

Resets the workflow run to `pending` and creates a new job for the first step.

## Environment Variables

| Variable | Description |
|----------|-------------|
| `RUST_LOG` | Log level filter (e.g., `qhook=info`, `qhook=debug`) |
| `QHOOK_LOG_FORMAT` | Set to `json` for structured JSON logging |
| `QHOOK_CONFIG` | Config file path (alternative to `-c` flag) |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | OpenTelemetry OTLP endpoint (requires `otel` feature). When set, traces are exported via gRPC |
