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
qhook start --env local            # merges qhook.local.yaml + loads .env.local
qhook start --env production       # merges qhook.production.yaml + loads .env.production
QHOOK_ENV=staging qhook start      # same via environment variable
qhook start -c /path/to/qhook.yaml # custom config path
qhook start -c s3://bucket/qhook.yaml    # load from AWS S3
qhook start -c gs://bucket/qhook.yaml    # load from GCS
qhook start -c az://account/container/qhook.yaml  # load from Azure Blob
qhook start -c https://config.example.com/qhook.yaml  # load from HTTP
```

**Environment overlays:** When `--env <name>` is specified (or `QHOOK_ENV` is set), qhook:
1. Loads `.env.<name>` (if exists) into process environment variables
2. Reads base config (`qhook.yaml`)
3. Deep merges `qhook.<name>.yaml` on top (if exists)
4. Expands `${VAR}` references and validates

Remote configs are polled every 30 seconds for changes (ETag-based). If the remote config is invalid, the current config is preserved and a warning is logged.

> **Note:** Remote config currently supports public (unauthenticated) endpoints only. For private buckets, use an init container to copy the config locally, or serve it via an authenticated internal HTTP endpoint.

**Signals:**
- `SIGTERM` / `SIGINT` -- graceful shutdown (stops accepting requests, drains in-flight deliveries)
- `SIGHUP` -- validate config and log changes (added/removed sources, handlers, workflows; warns about port/database changes requiring restart)

### qhook init

Generate a starter `qhook.yaml` in the current directory.

```bash
qhook init                         # default config
qhook init --template github       # GitHub webhook template
qhook init --template stripe       # Stripe webhook template
qhook init --template sns          # AWS SNS template
qhook init --template cron         # Cron trigger template
```

### qhook validate

Validate a config file without starting the server.

```bash
qhook validate                     # uses ./qhook.yaml
qhook validate -c /path/to/config  # custom path
```

Exits 0 on success, non-zero with error details on failure.

### qhook send

Send a test event to a running qhook server.

```bash
qhook send -s app -t order.created '{"id": "123"}'       # inline JSON
qhook send -s app -t user.signup -f payload.json          # from file
qhook send -s app -t health.ping                          # empty payload ({})
qhook send -s app -t order.created '{"amount": 15000}' --dry-run  # show matches only
```

Reads the config to determine the correct endpoint, port, and authentication. Source must match a configured source name.

With `--dry-run`, shows which handlers and workflows would match without creating jobs.

**Options:**

| Flag | Description |
|------|-------------|
| `-s`, `--source` | Source name (required) |
| `-t`, `--type` | Event type (required) |
| `-f`, `--file` | Read payload from a JSON file |
| `--dry-run` | Show matching handlers/workflows without sending |
| `-c`, `--config` | Config file path (default: `qhook.yaml`) |

### qhook inspect

Show the full lifecycle of an event: payload, jobs, attempts, and workflow runs.

```bash
qhook inspect <EVENT_ID>
qhook inspect 01JQ7X...
```

Displays:
- Event metadata (type, source, timestamp, dedup key)
- Payload (truncated for display)
- All jobs with status, attempt count, and error details
- Individual delivery attempts with status codes and duration
- Related workflow runs with current step

### qhook doctor

Check server readiness and endpoint reachability.

```bash
qhook doctor                       # uses ./qhook.yaml
qhook doctor -c production.yaml    # check production config
```

Performs the following checks:
- Config file syntax and validation
- Database connectivity
- Server health endpoint
- Handler endpoint reachability
- Workflow step endpoint reachability
- Security settings (SSRF protection, auth tokens)

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

### qhook tail

Stream events and job results in real time.

```bash
qhook tail                         # all events and jobs
qhook tail --source stripe         # only events from a specific source
qhook tail --status dead           # only dead (DLQ) jobs
```

Polls the database every second and displays new events and job completions with color-coded output. Press `Ctrl+C` to stop.

### qhook export events

Export events as JSONL (one JSON object per line).

```bash
qhook export events                                    # all events (up to 1000)
qhook export events --source stripe                    # filter by source
qhook export events --event-type order.created         # filter by event type
qhook export events --since 2026-03-01T00:00:00       # events after a timestamp
qhook export events --limit 5000                       # increase limit
qhook export events > events.jsonl                     # save to file
```

**Options:**

| Flag | Description |
|------|-------------|
| `--source` | Filter by source name |
| `-t`, `--event-type` | Filter by event type |
| `--since` | Only events created after this timestamp |
| `--until` | Only events created before this timestamp |
| `-l`, `--limit` | Max events to export (default: 1000) |

## Management API

qhook exposes REST endpoints for inspecting events and jobs programmatically.

> **Authentication:** These endpoints are protected by the same `api.auth_token` configured for the `/events` endpoint. Include the token as a Bearer header.

### POST /events/:source/:type

Send an event with a source name. The source must be defined in config as `type: event`.

```bash
curl -X POST http://localhost:8888/events/platform/deploy.start \
  -H "Authorization: Bearer $QHOOK_API_TOKEN" \
  -d '{"service": "api", "version": "1.2.3"}'
```

### GET /api/events/:id

Returns event details including associated jobs and workflow runs.

```bash
curl -H "Authorization: Bearer $QHOOK_API_TOKEN" \
  http://localhost:8888/api/events/01JQ7X...
```

Response includes event metadata, payload, matched jobs (with status), and any workflow runs triggered by the event.

### GET /api/jobs/:id

Returns job details. Optionally include delivery attempts with `?include_attempts=true`.

```bash
# Job details only
curl -H "Authorization: Bearer $QHOOK_API_TOKEN" \
  http://localhost:8888/api/jobs/01JQ7X...

# Job details with delivery attempts
curl -H "Authorization: Bearer $QHOOK_API_TOKEN" \
  "http://localhost:8888/api/jobs/01JQ7X...?include_attempts=true"
```

Response includes job metadata (handler, status, attempt count, scheduled/completed timestamps). When `include_attempts=true`, each delivery attempt is included with status code, duration, and error details.

## Environment Variables

| Variable | Description |
|----------|-------------|
| `RUST_LOG` | Log level filter (e.g., `qhook=info`, `qhook=debug`) |
| `QHOOK_LOG_FORMAT` | Set to `json` for structured JSON logging |
| `QHOOK_ENV` | Environment name for config overlay (alternative to `--env` flag) |
| `QHOOK_CONFIG` | Config file path (alternative to `-c` flag) |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | OpenTelemetry OTLP endpoint (requires `otel` feature). When set, traces are exported via HTTP |
