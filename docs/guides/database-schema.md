---
layout: default
title: Database Schema
---

# Database Schema

qhook uses 4 tables to manage events, jobs, job attempts, and workflow runs. Tables are created automatically on startup (`migrate()`). SQLite and Postgres share the same schema.

## Tables

### events

Stores all received events. One row per inbound webhook/event/SNS message.

| Column | Type | Description |
|--------|------|-------------|
| `id` | TEXT PK | ULID, globally unique |
| `source` | TEXT NOT NULL | Source name from config (e.g. `github`, `stripe`) |
| `event_type` | TEXT NOT NULL | Event type (e.g. `push`, `order.created`) |
| `payload` | TEXT NOT NULL | Raw JSON body |
| `headers` | TEXT | JSON object of forwarded headers (CloudEvents `ce-*` headers) |
| `unique_key` | TEXT | Idempotency key extracted via `dedup_key` JSONPath |
| `created_at` | TEXT NOT NULL | UTC timestamp (`%Y-%m-%dT%H:%M:%S%.3f`) |

**Indexes:**
- `idx_events_unique` — UNIQUE on `(source, unique_key)` for deduplication

### jobs

Work queue. Each job represents a single delivery attempt to a handler or workflow step endpoint.

| Column | Type | Description |
|--------|------|-------------|
| `id` | TEXT PK | ULID |
| `event_id` | TEXT NOT NULL | FK to `events.id` |
| `handler` | TEXT NOT NULL | Handler or step name |
| `url` | TEXT NOT NULL | Delivery endpoint URL |
| `status` | TEXT NOT NULL | `available`, `running`, `completed`, `retryable`, `dead` |
| `attempt` | INTEGER NOT NULL | Current attempt number (0-based) |
| `max_attempts` | INTEGER NOT NULL | Max retry attempts |
| `scheduled_at` | TEXT NOT NULL | When the job should be picked up |
| `started_at` | TEXT | When delivery started |
| `completed_at` | TEXT | When delivery finished |
| `created_at` | TEXT NOT NULL | Row creation time |
| `last_error` | TEXT | Last error message (on failure) |
| `workflow_run_id` | TEXT | FK to `workflow_runs.id` (workflow jobs only) |
| `step_name` | TEXT | Workflow step name |
| `step_index` | INTEGER | Step position in workflow |
| `step_input` | TEXT | Transformed input payload for this step |
| `step_output` | TEXT | Output transform template |
| `branch_name` | TEXT | Parallel branch name |
| `callback_token` | TEXT | Unique token for callback steps |

**Indexes:**
- `idx_jobs_fetch` — on `(status, scheduled_at)` for queue polling
- `idx_jobs_callback_token` — UNIQUE on `callback_token` for callback lookup

**Job status lifecycle:**

```
available → running → completed
                   → retryable → available (retry)
                   → dead (DLQ, max attempts exhausted)
```

### job_attempts

Audit log of each delivery attempt. One row per HTTP request to a handler.

| Column | Type | Description |
|--------|------|-------------|
| `id` | TEXT PK | ULID |
| `job_id` | TEXT NOT NULL | FK to `jobs.id` |
| `attempt` | INTEGER NOT NULL | Attempt number |
| `status_code` | INTEGER | HTTP response status code |
| `response_body` | TEXT | Response body (truncated) |
| `error` | TEXT | Error message if request failed |
| `duration_ms` | INTEGER | Request duration in milliseconds |
| `created_at` | TEXT NOT NULL | Attempt timestamp |

### workflow_runs

Tracks the state of workflow executions.

| Column | Type | Description |
|--------|------|-------------|
| `id` | TEXT PK | ULID |
| `workflow` | TEXT NOT NULL | Workflow name from config |
| `event_id` | TEXT NOT NULL | FK to `events.id` (triggering event) |
| `status` | TEXT NOT NULL | `running`, `completed`, `failed` |
| `current_step` | TEXT | Name of the step currently executing |
| `created_at` | TEXT NOT NULL | Run start time |
| `completed_at` | TEXT | Run completion time |
| `parallel_step` | TEXT | Step name that initiated parallel execution |
| `parallel_count` | INTEGER | Total number of parallel branches |
| `parallel_completed` | INTEGER | Number of completed branches |
| `timeout_at` | TEXT | Workflow timeout deadline |
| `parent_run_id` | TEXT | FK to `workflow_runs.id` (parent, for sub-workflows) |
| `parent_step_index` | INTEGER | Step index in parent workflow to resume after completion |

**Indexes:**
- `idx_workflow_runs_status` — on `status`

### outbound_endpoints

Registered customer webhook endpoints for outbound delivery.

| Column | Type | Description |
|--------|------|-------------|
| `id` | TEXT PK | ULID |
| `source` | TEXT NOT NULL | Source name this endpoint receives events from |
| `url` | TEXT NOT NULL | Customer's webhook URL |
| `description` | TEXT | Human-readable description |
| `signing_secret` | TEXT NOT NULL | Per-endpoint HMAC secret (`whsec_` prefix, base64-encoded key) |
| `status` | TEXT NOT NULL | `active` or `disabled` |
| `created_at` | TEXT NOT NULL | Creation timestamp |
| `updated_at` | TEXT NOT NULL | Last update timestamp |

**Indexes:**
- `idx_outbound_endpoints_source` — on `(source, status)` for fan-out lookups

### outbound_subscriptions

Event type subscriptions for outbound endpoints. Only subscribed event types are delivered.

| Column | Type | Description |
|--------|------|-------------|
| `id` | TEXT PK | ULID |
| `endpoint_id` | TEXT NOT NULL | FK to `outbound_endpoints.id` |
| `event_type` | TEXT NOT NULL | Subscribed event type (or `*` for all) |
| `created_at` | TEXT NOT NULL | Creation timestamp |

**Indexes:**
- `idx_outbound_sub_unique` — UNIQUE on `(endpoint_id, event_type)` to prevent duplicate subscriptions

### _migrations

Tracks applied database migrations for safe schema upgrades across versions.

| Column | Type | Description |
|--------|------|-------------|
| `version` | INTEGER PK | Migration version number |
| `name` | TEXT NOT NULL | Human-readable migration name |
| `applied_at` | TEXT NOT NULL | When the migration was applied |

**Current migrations:**

| Version | Name | Description |
|---------|------|-------------|
| 1 | Core tables | `events`, `jobs`, `job_attempts` tables and indexes |
| 2 | Workflow engine | `workflow_runs` table |
| 3 | Workflow extensions | `parallel_step`, `parallel_count`, `parallel_completed`, `timeout_at`, `parent_run_id`, `parent_step_index` columns |
| 4 | Outbound webhooks | `outbound_endpoints`, `outbound_subscriptions` tables |

## Conventions

- **IDs**: ULID format (lexicographically sortable, unique)
- **Timestamps**: UTC, stored as TEXT in `%Y-%m-%dT%H:%M:%S%.3f` format
- **JSON fields**: `payload`, `headers`, `step_input` store JSON as TEXT
- **Null vs empty**: Optional fields use NULL, not empty strings

## Multi-instance (Postgres)

When running multiple qhook instances against the same Postgres database:

- Job polling uses `SELECT ... FOR UPDATE SKIP LOCKED` to prevent double-processing
- Parallel branch completion uses atomic `UPDATE ... RETURNING` to prevent race conditions
- SQLite is single-writer only — use Postgres for multi-instance deployments

## Data retention

By default, completed jobs and events older than 72 hours are automatically cleaned up. Configure via `worker.retention_hours`.
