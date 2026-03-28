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

### qhook queues list

List all configured queues with pending, processing, and dead counts.

```bash
qhook queues list
```

### qhook queues inspect

Show detailed stats for a queue.

```bash
qhook queues inspect billing
```

### qhook queues peek

View the next available message without consuming it.

```bash
qhook queues peek billing
```

### qhook queues dlq

List dead-letter messages for a queue.

```bash
qhook queues dlq billing
```

### qhook queues retry

Retry dead jobs for a queue.

```bash
qhook queues retry billing                 # retry all dead jobs
qhook queues retry billing --id <JOB_ID>   # retry a specific job
```

### qhook queues drain

Delete all jobs for a queue.

```bash
qhook queues drain billing                 # interactive confirmation
qhook queues drain billing --force         # skip confirmation
```

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

### qhook replay-local

Replay events from a JSONL file to a running qhook server. Reads the output of `qhook export events` and sends each event to the server's `/events/{source}/{event_type}` endpoint.

```bash
# Export from production, replay locally
qhook export events --source stripe > events.jsonl
qhook replay-local events.jsonl                          # replay to localhost (port from config)
qhook replay-local events.jsonl --target http://localhost:9999  # custom target
qhook replay-local events.jsonl --token my-api-token -y  # with auth, skip confirmation
cat events.jsonl | qhook replay-local -                  # read from stdin

# Filtered replay
qhook replay-local events.jsonl --source stripe                 # only stripe events
qhook replay-local events.jsonl --event-type order.created      # exact event type match
qhook replay-local events.jsonl --event-type payment.*          # prefix match (payment.created, payment.refunded, etc.)
qhook replay-local events.jsonl --since 2026-03-01T00:00:00     # events after a timestamp
qhook replay-local events.jsonl --until 2026-03-10T00:00:00     # events before a timestamp
qhook replay-local events.jsonl --status failed                 # only failed deliveries
qhook replay-local events.jsonl --source stripe --since 2026-03-01T00:00:00 -y  # combine filters
```

**Options:**

| Flag | Description |
|------|-------------|
| `--target` | Target server URL (default: `http://localhost:{port}` from config) |
| `--token` | API auth token (overrides config `api.auth_token`) |
| `-y`, `--yes` | Skip confirmation prompt |
| `-c`, `--config` | Config file path (default: `qhook.yaml`) |
| `--source` | Filter by source name (exact match) |
| `--event-type` | Filter by event type (exact match, or prefix match with trailing `*`) |
| `--since` | Only events created after this timestamp (ISO 8601) |
| `--until` | Only events created before this timestamp (ISO 8601) |
| `--status` | Only events with matching status (e.g. `failed`, `completed`) |

When filters are applied, the confirmation prompt shows: `Replaying 15 of 100 events (filtered by source=stripe, since=2026-03-01)`.

**Use cases:**
- Reproduce production issues locally using real event data
- Test new handler/filter configurations against historical events
- Migration testing with actual payloads
- Replay only failed deliveries after fixing a downstream service

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

### GET /api/events

List events with optional filters. Returns events without payload for efficiency.

```bash
# List all events (default limit: 50)
curl -H "Authorization: Bearer $QHOOK_API_TOKEN" \
  http://localhost:8888/api/events

# Filter by source and time range
curl -H "Authorization: Bearer $QHOOK_API_TOKEN" \
  "http://localhost:8888/api/events?source=stripe&since=2026-03-01T00:00:00&limit=100"

# Cursor-based pagination
curl -H "Authorization: Bearer $QHOOK_API_TOKEN" \
  "http://localhost:8888/api/events?after=01JQ7X...&limit=50"
```

Query parameters: `source`, `event_type`, `since`, `until`, `limit` (max 1000), `after` (cursor).

Response: `{"events": [...], "has_more": true/false}`

### GET /api/events/:id/jobs

List all jobs created for a specific event.

```bash
curl -H "Authorization: Bearer $QHOOK_API_TOKEN" \
  http://localhost:8888/api/events/01JQ7X.../jobs
```

### GET /api/jobs

List jobs with optional filters.

```bash
# List dead-letter jobs
curl -H "Authorization: Bearer $QHOOK_API_TOKEN" \
  "http://localhost:8888/api/jobs?status=dead"

# Filter by handler
curl -H "Authorization: Bearer $QHOOK_API_TOKEN" \
  "http://localhost:8888/api/jobs?handler=deploy&limit=100"
```

Query parameters: `status`, `handler`, `limit` (max 1000), `after` (cursor).

### GET /api/jobs/:id/attempts

List delivery attempts for a specific job.

```bash
curl -H "Authorization: Bearer $QHOOK_API_TOKEN" \
  http://localhost:8888/api/jobs/01JQ7X.../attempts
```

Each attempt includes: `attempt` number, `status_code`, `error`, `duration_ms`, `created_at`.

## Environment Variables

| Variable | Description |
|----------|-------------|
| `RUST_LOG` | Log level filter (e.g., `qhook=info`, `qhook=debug`) |
| `QHOOK_LOG_FORMAT` | Set to `json` for structured JSON logging |
| `QHOOK_ENV` | Environment name for config overlay (alternative to `--env` flag) |
| `QHOOK_CONFIG` | Config file path (alternative to `-c` flag) |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | OpenTelemetry OTLP endpoint (requires `otel` feature). When set, traces are exported via HTTP |
