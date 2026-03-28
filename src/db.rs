use anyhow::{Context, Result};
use chrono::{NaiveDateTime, Utc};
use sqlx::{AnyPool, any::AnyPoolOptions};
use std::time::Instant;

use crate::config::DatabaseConfig;

/// Format current UTC time as TEXT for database storage.
pub(crate) fn format_now() -> String {
    Utc::now()
        .naive_utc()
        .format("%Y-%m-%dT%H:%M:%S%.3f")
        .to_string()
}

/// Format a NaiveDateTime as TEXT for database storage.
pub(crate) fn format_dt(dt: NaiveDateTime) -> String {
    dt.format("%Y-%m-%dT%H:%M:%S%.3f").to_string()
}

/// Redact credentials from a database URL.
/// Replaces `user:password@` with `***@` to prevent credential leakage in logs.
fn redact_url(url: &str) -> String {
    // Match patterns like postgres://user:pass@host or sqlite:file
    if let Some(scheme_end) = url.find("://") {
        let after_scheme = &url[scheme_end + 3..];
        if let Some(at_pos) = after_scheme.find('@') {
            // Has credentials — redact them
            return format!(
                "{}://***@{}",
                &url[..scheme_end],
                &after_scheme[at_pos + 1..]
            );
        }
    }
    url.to_string()
}

/// Log queries that take longer than this.
const SLOW_QUERY_MS: u128 = 100;

#[allow(dead_code)]
pub struct Database {
    pub pool: AnyPool,
    pub driver: String,
}

impl Database {
    pub async fn connect(config: &DatabaseConfig) -> Result<Self> {
        let url = match config.driver.as_str() {
            "sqlite" => config
                .url
                .clone()
                .unwrap_or_else(|| "sqlite:qhook.db?mode=rwc".into()),
            "postgres" => config
                .url
                .clone()
                .context("database.url is required for postgres")?,
            "mysql" => config
                .url
                .clone()
                .context("database.url is required for mysql")?,
            other => anyhow::bail!("Unsupported database driver: {other}"),
        };

        sqlx::any::install_default_drivers();

        let pool = AnyPoolOptions::new()
            .max_connections(config.max_connections)
            .connect(&url)
            .await
            .with_context(|| format!("Failed to connect to database: {}", redact_url(&url)))?;

        tracing::info!(driver = config.driver, "Database connected");

        Ok(Self {
            pool,
            driver: config.driver.clone(),
        })
    }

    pub async fn migrate(&self) -> Result<()> {
        // Create migration tracking table
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS _migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL)",
        )
        .execute(&self.pool)
        .await?;

        let current_version: i32 =
            sqlx::query_as::<_, (i32,)>("SELECT COALESCE(MAX(version), 0) FROM _migrations")
                .fetch_one(&self.pool)
                .await
                .map(|r| r.0)
                .unwrap_or(0);

        // Detect if tables exist but migration tracking is new (upgrade from pre-migration)
        let has_events_table = sqlx::query("SELECT 1 FROM events LIMIT 0")
            .execute(&self.pool)
            .await
            .is_ok();

        let effective_version = if current_version == 0 && has_events_table {
            // Pre-existing database without migration tracking — mark all existing migrations as applied
            tracing::info!("Pre-existing database detected, initializing migration tracking");
            for v in 1..=4i32 {
                sqlx::query("INSERT INTO _migrations (version, applied_at) VALUES ($1, $2)")
                    .bind(v)
                    .bind(format_now())
                    .execute(&self.pool)
                    .await
                    .ok();
            }
            4
        } else {
            current_version
        };

        let is_mysql = self.driver == "mysql";

        // MySQL requires VARCHAR for primary keys and indexed columns (TEXT cannot be indexed).
        // SQLite and PostgreSQL use TEXT for all string columns.
        let migrations: Vec<(i32, &str, Vec<String>)> = vec![
            // v1: Core tables (events, jobs, job_attempts)
            (
                1,
                "Core tables",
                if is_mysql {
                    vec![
                        "CREATE TABLE IF NOT EXISTS events (
                        id VARCHAR(255) PRIMARY KEY, source VARCHAR(255) NOT NULL, event_type VARCHAR(255) NOT NULL,
                        payload TEXT NOT NULL, headers TEXT, unique_key VARCHAR(255), created_at VARCHAR(255) NOT NULL
                    )".into(),
                        "CREATE UNIQUE INDEX idx_events_unique ON events (source, unique_key)".into(),
                        "CREATE TABLE IF NOT EXISTS jobs (
                        id VARCHAR(255) PRIMARY KEY, event_id VARCHAR(255) NOT NULL, handler VARCHAR(255) NOT NULL,
                        url TEXT NOT NULL, status VARCHAR(255) NOT NULL DEFAULT 'available',
                        attempt INTEGER NOT NULL DEFAULT 0, max_attempts INTEGER NOT NULL DEFAULT 5,
                        scheduled_at VARCHAR(255) NOT NULL, started_at VARCHAR(255), completed_at VARCHAR(255),
                        created_at VARCHAR(255) NOT NULL, last_error TEXT
                    )".into(),
                        "CREATE INDEX idx_jobs_fetch ON jobs (status, scheduled_at)".into(),
                        "CREATE TABLE IF NOT EXISTS job_attempts (
                        id VARCHAR(255) PRIMARY KEY, job_id VARCHAR(255) NOT NULL, attempt INTEGER NOT NULL,
                        status_code INTEGER, response_body TEXT, error TEXT,
                        duration_ms INTEGER, created_at VARCHAR(255) NOT NULL
                    )".into(),
                    ]
                } else {
                    vec![
                        "CREATE TABLE IF NOT EXISTS events (
                        id TEXT PRIMARY KEY, source TEXT NOT NULL, event_type TEXT NOT NULL,
                        payload TEXT NOT NULL, headers TEXT, unique_key TEXT, created_at TEXT NOT NULL
                    )".into(),
                        "CREATE UNIQUE INDEX IF NOT EXISTS idx_events_unique ON events (source, unique_key)".into(),
                        "CREATE TABLE IF NOT EXISTS jobs (
                        id TEXT PRIMARY KEY, event_id TEXT NOT NULL, handler TEXT NOT NULL,
                        url TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'available',
                        attempt INTEGER NOT NULL DEFAULT 0, max_attempts INTEGER NOT NULL DEFAULT 5,
                        scheduled_at TEXT NOT NULL, started_at TEXT, completed_at TEXT,
                        created_at TEXT NOT NULL, last_error TEXT
                    )".into(),
                        "CREATE INDEX IF NOT EXISTS idx_jobs_fetch ON jobs (status, scheduled_at)".into(),
                        "CREATE TABLE IF NOT EXISTS job_attempts (
                        id TEXT PRIMARY KEY, job_id TEXT NOT NULL, attempt INTEGER NOT NULL,
                        status_code INTEGER, response_body TEXT, error TEXT,
                        duration_ms INTEGER, created_at TEXT NOT NULL
                    )".into(),
                    ]
                },
            ),
            // v2: Workflow engine
            (
                2,
                "Workflow tables",
                if is_mysql {
                    vec![
                        "CREATE TABLE IF NOT EXISTS workflow_runs (
                        id VARCHAR(255) PRIMARY KEY, workflow VARCHAR(255) NOT NULL, event_id VARCHAR(255) NOT NULL,
                        status VARCHAR(255) NOT NULL DEFAULT 'running', current_step VARCHAR(255),
                        created_at VARCHAR(255) NOT NULL, completed_at VARCHAR(255)
                    )".into(),
                        "CREATE INDEX idx_workflow_runs_status ON workflow_runs (status)".into(),
                        "ALTER TABLE jobs ADD COLUMN workflow_run_id VARCHAR(255)".into(),
                        "ALTER TABLE jobs ADD COLUMN step_name VARCHAR(255)".into(),
                        "ALTER TABLE jobs ADD COLUMN step_index INTEGER".into(),
                        "ALTER TABLE jobs ADD COLUMN step_input TEXT".into(),
                        "ALTER TABLE jobs ADD COLUMN step_output TEXT".into(),
                        "ALTER TABLE jobs ADD COLUMN branch_name VARCHAR(255)".into(),
                    ]
                } else {
                    vec![
                        "CREATE TABLE IF NOT EXISTS workflow_runs (
                        id TEXT PRIMARY KEY, workflow TEXT NOT NULL, event_id TEXT NOT NULL,
                        status TEXT NOT NULL DEFAULT 'running', current_step TEXT,
                        created_at TEXT NOT NULL, completed_at TEXT
                    )".into(),
                        "CREATE INDEX IF NOT EXISTS idx_workflow_runs_status ON workflow_runs (status)".into(),
                        "ALTER TABLE jobs ADD COLUMN workflow_run_id TEXT".into(),
                        "ALTER TABLE jobs ADD COLUMN step_name TEXT".into(),
                        "ALTER TABLE jobs ADD COLUMN step_index INTEGER".into(),
                        "ALTER TABLE jobs ADD COLUMN step_input TEXT".into(),
                        "ALTER TABLE jobs ADD COLUMN step_output TEXT".into(),
                        "ALTER TABLE jobs ADD COLUMN branch_name TEXT".into(),
                    ]
                },
            ),
            // v3: Workflow extensions (parallel, timeout, sub-workflow, callback)
            (
                3,
                "Workflow extensions",
                if is_mysql {
                    vec![
                        "ALTER TABLE workflow_runs ADD COLUMN parallel_step VARCHAR(255)".into(),
                        "ALTER TABLE workflow_runs ADD COLUMN parallel_count INTEGER DEFAULT 0"
                            .into(),
                        "ALTER TABLE workflow_runs ADD COLUMN parallel_completed INTEGER DEFAULT 0"
                            .into(),
                        "ALTER TABLE workflow_runs ADD COLUMN timeout_at VARCHAR(255)".into(),
                        "ALTER TABLE workflow_runs ADD COLUMN parent_run_id VARCHAR(255)".into(),
                        "ALTER TABLE workflow_runs ADD COLUMN parent_step_index INTEGER".into(),
                        "ALTER TABLE jobs ADD COLUMN callback_token VARCHAR(255)".into(),
                        "CREATE UNIQUE INDEX idx_jobs_callback_token ON jobs (callback_token)"
                            .into(),
                    ]
                } else {
                    vec![
                        "ALTER TABLE workflow_runs ADD COLUMN parallel_step TEXT".into(),
                        "ALTER TABLE workflow_runs ADD COLUMN parallel_count INTEGER DEFAULT 0".into(),
                        "ALTER TABLE workflow_runs ADD COLUMN parallel_completed INTEGER DEFAULT 0".into(),
                        "ALTER TABLE workflow_runs ADD COLUMN timeout_at TEXT".into(),
                        "ALTER TABLE workflow_runs ADD COLUMN parent_run_id TEXT".into(),
                        "ALTER TABLE workflow_runs ADD COLUMN parent_step_index INTEGER".into(),
                        "ALTER TABLE jobs ADD COLUMN callback_token TEXT".into(),
                        "CREATE UNIQUE INDEX IF NOT EXISTS idx_jobs_callback_token ON jobs (callback_token)".into(),
                    ]
                },
            ),
            // v4: Outbound webhooks
            (
                4,
                "Outbound webhooks",
                if is_mysql {
                    vec![
                        "CREATE TABLE IF NOT EXISTS outbound_endpoints (
                        id VARCHAR(255) PRIMARY KEY, source VARCHAR(255) NOT NULL, url TEXT NOT NULL,
                        description TEXT, signing_secret VARCHAR(255) NOT NULL,
                        status VARCHAR(255) NOT NULL DEFAULT 'active',
                        created_at VARCHAR(255) NOT NULL, updated_at VARCHAR(255) NOT NULL
                    )".into(),
                        "CREATE TABLE IF NOT EXISTS outbound_subscriptions (
                        id VARCHAR(255) PRIMARY KEY, endpoint_id VARCHAR(255) NOT NULL,
                        event_type VARCHAR(255) NOT NULL, created_at VARCHAR(255) NOT NULL
                    )".into(),
                        "CREATE UNIQUE INDEX idx_outbound_sub_unique ON outbound_subscriptions (endpoint_id, event_type)".into(),
                        "CREATE INDEX idx_outbound_endpoints_source ON outbound_endpoints (source, status)".into(),
                    ]
                } else {
                    vec![
                        "CREATE TABLE IF NOT EXISTS outbound_endpoints (
                        id TEXT PRIMARY KEY, source TEXT NOT NULL, url TEXT NOT NULL,
                        description TEXT, signing_secret TEXT NOT NULL,
                        status TEXT NOT NULL DEFAULT 'active',
                        created_at TEXT NOT NULL, updated_at TEXT NOT NULL
                    )".into(),
                        "CREATE TABLE IF NOT EXISTS outbound_subscriptions (
                        id TEXT PRIMARY KEY, endpoint_id TEXT NOT NULL,
                        event_type TEXT NOT NULL, created_at TEXT NOT NULL
                    )".into(),
                        "CREATE UNIQUE INDEX IF NOT EXISTS idx_outbound_sub_unique ON outbound_subscriptions (endpoint_id, event_type)".into(),
                        "CREATE INDEX IF NOT EXISTS idx_outbound_endpoints_source ON outbound_endpoints (source, status)".into(),
                    ]
                },
            ),
        ];

        for (version, name, queries) in &migrations {
            if *version <= effective_version {
                continue;
            }
            tracing::info!(version, name, "Applying migration");
            for sql in queries {
                // ALTER TABLE ADD COLUMN may fail if column already exists — that's OK
                // MySQL CREATE INDEX (without IF NOT EXISTS) may also fail if index exists
                if sql.contains("ALTER TABLE") || (is_mysql && sql.contains("CREATE INDEX")) {
                    sqlx::query(sql).execute(&self.pool).await.ok();
                } else {
                    sqlx::query(sql).execute(&self.pool).await?;
                }
            }
            sqlx::query("INSERT INTO _migrations (version, applied_at) VALUES ($1, $2)")
                .bind(*version)
                .bind(format_now())
                .execute(&self.pool)
                .await?;
        }

        tracing::info!(
            version = migrations.last().map(|m| m.0).unwrap_or(0),
            "Database migrated"
        );
        Ok(())
    }

    pub async fn insert_event(
        &self,
        id: &str,
        source: &str,
        event_type: &str,
        payload: &str,
        headers: Option<&str>,
        unique_key: Option<&str>,
    ) -> Result<bool> {
        let now = format_now();

        // Try insert; if unique_key conflicts, return false (duplicate)
        if unique_key.is_some() {
            let result = if self.driver == "mysql" {
                sqlx::query(
                    "INSERT IGNORE INTO events (id, source, event_type, payload, headers, unique_key, created_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7)",
                )
                .bind(id)
                .bind(source)
                .bind(event_type)
                .bind(payload)
                .bind(headers)
                .bind(unique_key)
                .bind(&now)
                .execute(&self.pool)
                .await?
            } else {
                sqlx::query(
                    "INSERT INTO events (id, source, event_type, payload, headers, unique_key, created_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7) \
                     ON CONFLICT (source, unique_key) DO NOTHING",
                )
                .bind(id)
                .bind(source)
                .bind(event_type)
                .bind(payload)
                .bind(headers)
                .bind(unique_key)
                .bind(&now)
                .execute(&self.pool)
                .await?
            };

            Ok(result.rows_affected() > 0)
        } else {
            sqlx::query(
                "INSERT INTO events (id, source, event_type, payload, headers, unique_key, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(id)
            .bind(source)
            .bind(event_type)
            .bind(payload)
            .bind(headers)
            .bind(unique_key)
            .bind(&now)
            .execute(&self.pool)
            .await?;

            Ok(true)
        }
    }

    pub async fn insert_job(
        &self,
        id: &str,
        event_id: &str,
        handler: &str,
        url: &str,
        max_attempts: u32,
    ) -> Result<()> {
        let now = format_now();

        sqlx::query(
            "INSERT INTO jobs (id, event_id, handler, url, status, max_attempts, scheduled_at, created_at) \
             VALUES ($1, $2, $3, $4, 'available', $5, $6, $6)",
        )
        .bind(id)
        .bind(event_id)
        .bind(handler)
        .bind(url)
        .bind(max_attempts as i32)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Fetch jobs ready for processing.
    /// For Postgres: uses FOR UPDATE SKIP LOCKED + immediate status update in one query.
    /// For MySQL: uses FOR UPDATE SKIP LOCKED but without RETURNING (separate SELECT).
    /// For SQLite: plain SELECT (use mark_job_running separately).
    pub async fn fetch_available_jobs(&self, limit: i32) -> Result<Vec<JobRow>> {
        let now = format_now();

        let start = Instant::now();
        let rows = if self.driver == "postgres" {
            sqlx::query_as::<_, JobRow>(
                "UPDATE jobs SET status = 'running', started_at = $1, attempt = attempt + 1 \
                 WHERE id IN ( \
                     SELECT id FROM jobs \
                     WHERE status IN ('available', 'retryable') AND scheduled_at <= $1 \
                     AND handler NOT LIKE 'queue/%' \
                     ORDER BY scheduled_at ASC \
                     LIMIT $2 \
                     FOR UPDATE SKIP LOCKED \
                 ) \
                 RETURNING id, event_id, handler, url, status, attempt - 1 AS attempt, max_attempts, scheduled_at, last_error, created_at",
            )
            .bind(&now)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, JobRow>(
                "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                 FROM jobs \
                 WHERE status IN ('available', 'retryable') AND scheduled_at <= $1 \
                 AND handler NOT LIKE 'queue/%' \
                 ORDER BY scheduled_at ASC \
                 LIMIT $2",
            )
            .bind(&now)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };
        let elapsed = start.elapsed().as_millis();
        if elapsed > SLOW_QUERY_MS {
            tracing::warn!(
                query = "fetch_available_jobs",
                duration_ms = elapsed,
                rows = rows.len(),
                "Slow query"
            );
        }

        Ok(rows)
    }

    /// Mark a job as running (SQLite/MySQL only — Postgres does this in fetch_available_jobs).
    ///
    /// TODO(perf): This is called per-job in a loop (N+1 problem). Ideally we'd batch these
    /// into a single `UPDATE ... WHERE id IN (...)` query, but sqlx's AnyPool doesn't support
    /// dynamic bind parameter lists for IN clauses. Options to fix:
    /// - Build raw SQL with comma-separated IDs (risk: SQL injection if IDs aren't validated)
    /// - Use a CTE or temp table approach
    /// - Use sqlx's `QueryBuilder` with `push_values` (requires separate sqlite/mysql pool types)
    /// In practice, the loop is bounded by `concurrent_delivery` (default 10), so impact is low.
    pub async fn mark_job_running(&self, job_id: &str) -> Result<bool> {
        let now = format_now();

        let result = sqlx::query(
            "UPDATE jobs SET status = 'running', started_at = $1, attempt = attempt + 1 \
             WHERE id = $2 AND status IN ('available', 'retryable')",
        )
        .bind(&now)
        .bind(job_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn mark_job_completed(&self, job_id: &str) -> Result<()> {
        let now = format_now();

        sqlx::query("UPDATE jobs SET status = 'completed', completed_at = $1 WHERE id = $2")
            .bind(&now)
            .bind(job_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn mark_job_retryable(
        &self,
        job_id: &str,
        next_attempt_at: NaiveDateTime,
        error: &str,
    ) -> Result<()> {
        let scheduled = format_dt(next_attempt_at);

        sqlx::query(
            "UPDATE jobs SET status = 'retryable', scheduled_at = $1, last_error = $2 WHERE id = $3",
        )
        .bind(&scheduled)
        .bind(error)
        .bind(job_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn mark_job_dead(&self, job_id: &str, error: &str) -> Result<()> {
        let now = format_now();

        sqlx::query(
            "UPDATE jobs SET status = 'dead', completed_at = $1, last_error = $2 WHERE id = $3",
        )
        .bind(&now)
        .bind(error)
        .bind(job_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_attempt(
        &self,
        id: &str,
        job_id: &str,
        attempt: i32,
        status_code: Option<i32>,
        response_body: Option<&str>,
        error: Option<&str>,
        duration_ms: i64,
    ) -> Result<()> {
        let now = format_now();

        sqlx::query(
            "INSERT INTO job_attempts (id, job_id, attempt, status_code, response_body, error, duration_ms, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(id)
        .bind(job_id)
        .bind(attempt)
        .bind(status_code)
        .bind(response_body)
        .bind(error)
        .bind(duration_ms)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_event_payload(&self, event_id: &str) -> Result<String> {
        let row: (String,) = sqlx::query_as("SELECT payload FROM events WHERE id = $1")
            .bind(event_id)
            .fetch_one(&self.pool)
            .await?;

        Ok(row.0)
    }

    pub async fn get_event_headers(&self, event_id: &str) -> Result<Option<String>> {
        let row: (Option<String>,) = sqlx::query_as("SELECT headers FROM events WHERE id = $1")
            .bind(event_id)
            .fetch_one(&self.pool)
            .await?;

        Ok(row.0)
    }

    /// Fetch payload and headers in a single query.
    pub async fn get_event_data(&self, event_id: &str) -> Result<(String, Option<String>)> {
        let row: (String, Option<String>) =
            sqlx::query_as("SELECT payload, headers FROM events WHERE id = $1")
                .bind(event_id)
                .fetch_one(&self.pool)
                .await?;

        Ok(row)
    }

    pub async fn list_jobs(&self, status: Option<&str>, limit: i32) -> Result<Vec<JobRow>> {
        let rows = if let Some(status) = status {
            sqlx::query_as::<_, JobRow>(
                "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                 FROM jobs WHERE status = $1 ORDER BY scheduled_at DESC LIMIT $2",
            )
            .bind(status)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, JobRow>(
                "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                 FROM jobs ORDER BY scheduled_at DESC LIMIT $1",
            )
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };
        Ok(rows)
    }

    pub async fn list_events(&self, limit: i32) -> Result<Vec<EventRow>> {
        let rows = sqlx::query_as::<_, EventRow>(
            "SELECT id, source, event_type, unique_key, created_at \
             FROM events ORDER BY created_at DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// List events with optional filters for replay.
    pub async fn list_events_filtered(
        &self,
        source: Option<&str>,
        event_type: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
        limit: i32,
    ) -> Result<Vec<EventRowFull>> {
        // Build query dynamically based on filters
        let mut conditions = Vec::new();
        let mut param_idx = 1;

        if source.is_some() {
            conditions.push(format!("source = ${param_idx}"));
            param_idx += 1;
        }
        if event_type.is_some() {
            conditions.push(format!("event_type = ${param_idx}"));
            param_idx += 1;
        }
        if since.is_some() {
            conditions.push(format!("created_at >= ${param_idx}"));
            param_idx += 1;
        }
        if until.is_some() {
            conditions.push(format!("created_at <= ${param_idx}"));
            param_idx += 1;
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT id, source, event_type, payload, headers, unique_key, created_at \
             FROM events{where_clause} ORDER BY created_at ASC LIMIT ${param_idx}"
        );

        let mut query = sqlx::query_as::<_, EventRowFull>(&sql);
        if let Some(v) = source {
            query = query.bind(v.to_string());
        }
        if let Some(v) = event_type {
            query = query.bind(v.to_string());
        }
        if let Some(v) = since {
            query = query.bind(v.to_string());
        }
        if let Some(v) = until {
            query = query.bind(v.to_string());
        }
        query = query.bind(limit);

        let rows = query.fetch_all(&self.pool).await?;
        Ok(rows)
    }

    /// List events created after the given ID, optionally filtered by source.
    /// Used by `qhook tail` for polling.
    pub async fn list_events_after(
        &self,
        after_id: Option<&str>,
        source: Option<&str>,
        limit: i32,
    ) -> Result<Vec<EventRow>> {
        let rows = match (after_id, source) {
            (Some(id), Some(src)) => {
                sqlx::query_as::<_, EventRow>(
                    "SELECT id, source, event_type, unique_key, created_at \
                     FROM events WHERE id > $1 AND source = $2 ORDER BY id ASC LIMIT $3",
                )
                .bind(id)
                .bind(src)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            (Some(id), None) => {
                sqlx::query_as::<_, EventRow>(
                    "SELECT id, source, event_type, unique_key, created_at \
                     FROM events WHERE id > $1 ORDER BY id ASC LIMIT $2",
                )
                .bind(id)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            (None, Some(src)) => {
                sqlx::query_as::<_, EventRow>(
                    "SELECT id, source, event_type, unique_key, created_at \
                     FROM events WHERE source = $1 ORDER BY id DESC LIMIT $2",
                )
                .bind(src)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            (None, None) => {
                sqlx::query_as::<_, EventRow>(
                    "SELECT id, source, event_type, unique_key, created_at \
                     FROM events ORDER BY id DESC LIMIT $1",
                )
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
        };
        Ok(rows)
    }

    /// List jobs updated after the given ID, optionally filtered by status.
    /// Used by `qhook tail` for polling.
    pub async fn list_jobs_after(
        &self,
        after_id: Option<&str>,
        status: Option<&str>,
        limit: i32,
    ) -> Result<Vec<JobRow>> {
        let rows = match (after_id, status) {
            (Some(id), Some(st)) => {
                sqlx::query_as::<_, JobRow>(
                    "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                     FROM jobs WHERE id > $1 AND status = $2 ORDER BY id ASC LIMIT $3",
                )
                .bind(id)
                .bind(st)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            (Some(id), None) => {
                sqlx::query_as::<_, JobRow>(
                    "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                     FROM jobs WHERE id > $1 AND status IN ('completed', 'dead', 'retryable') ORDER BY id ASC LIMIT $2",
                )
                .bind(id)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            (None, Some(st)) => {
                sqlx::query_as::<_, JobRow>(
                    "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                     FROM jobs WHERE status = $1 ORDER BY id DESC LIMIT $2",
                )
                .bind(st)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            (None, None) => {
                sqlx::query_as::<_, JobRow>(
                    "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                     FROM jobs WHERE status IN ('completed', 'dead', 'retryable') ORDER BY id DESC LIMIT $1",
                )
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
        };
        Ok(rows)
    }

    pub async fn retry_dead_jobs(&self) -> Result<u64> {
        let now = format_now();
        let result = sqlx::query(
            "UPDATE jobs SET status = 'available', scheduled_at = $1, last_error = NULL WHERE status = 'dead'",
        )
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Reset jobs stuck in 'running' for longer than `stale_secs` back to 'retryable'.
    pub async fn recover_stale_jobs(&self, stale_secs: i64) -> Result<u64> {
        let now = Utc::now().naive_utc();
        let cutoff = format_dt(now - chrono::Duration::seconds(stale_secs));
        let now_str = format_dt(now);

        let start = Instant::now();
        let result = sqlx::query(
            "UPDATE jobs SET status = 'retryable', scheduled_at = $1 \
             WHERE status = 'running' AND started_at <= $2 \
             AND handler NOT LIKE 'queue/%'",
        )
        .bind(&now_str)
        .bind(&cutoff)
        .execute(&self.pool)
        .await?;
        let elapsed = start.elapsed().as_millis();
        if elapsed > SLOW_QUERY_MS {
            tracing::warn!(
                query = "recover_stale_jobs",
                duration_ms = elapsed,
                "Slow query"
            );
        }

        Ok(result.rows_affected())
    }

    /// Delete completed/dead jobs (and their attempts) older than `retention_hours`.
    pub async fn cleanup_old_records(&self, retention_hours: i64) -> Result<(u64, u64)> {
        let cutoff = format_dt(Utc::now().naive_utc() - chrono::Duration::hours(retention_hours));

        let start = Instant::now();
        let attempts = sqlx::query(
            "DELETE FROM job_attempts WHERE job_id IN \
             (SELECT id FROM jobs WHERE status IN ('completed', 'dead') AND completed_at < $1)",
        )
        .bind(&cutoff)
        .execute(&self.pool)
        .await?;

        let jobs = sqlx::query(
            "DELETE FROM jobs WHERE status IN ('completed', 'dead') AND completed_at < $1",
        )
        .bind(&cutoff)
        .execute(&self.pool)
        .await?;
        let elapsed = start.elapsed().as_millis();
        if elapsed > SLOW_QUERY_MS {
            tracing::warn!(
                query = "cleanup_old_records",
                duration_ms = elapsed,
                "Slow query"
            );
        }

        Ok((jobs.rows_affected(), attempts.rows_affected()))
    }

    pub async fn queue_depth(&self) -> Result<i64> {
        let row: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM jobs WHERE status IN ('available', 'retryable')")
                .fetch_one(&self.pool)
                .await?;
        Ok(row.0)
    }

    pub async fn dead_job_count(&self) -> Result<i64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM jobs WHERE status = 'dead'")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }

    // --- v0.2: Workflow operations ---

    pub async fn insert_workflow_run(
        &self,
        id: &str,
        workflow: &str,
        event_id: &str,
        first_step: &str,
    ) -> Result<()> {
        let now = format_now();

        sqlx::query(
            "INSERT INTO workflow_runs (id, workflow, event_id, status, current_step, created_at) \
             VALUES ($1, $2, $3, 'running', $4, $5)",
        )
        .bind(id)
        .bind(workflow)
        .bind(event_id)
        .bind(first_step)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn update_workflow_run_step(&self, run_id: &str, step_name: &str) -> Result<()> {
        sqlx::query("UPDATE workflow_runs SET current_step = $1 WHERE id = $2")
            .bind(step_name)
            .bind(run_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn complete_workflow_run(&self, run_id: &str) -> Result<()> {
        let now = format_now();

        sqlx::query(
            "UPDATE workflow_runs SET status = 'completed', completed_at = $1 WHERE id = $2",
        )
        .bind(&now)
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn fail_workflow_run(&self, run_id: &str) -> Result<()> {
        let now = format_now();

        sqlx::query("UPDATE workflow_runs SET status = 'failed', completed_at = $1 WHERE id = $2")
            .bind(&now)
            .bind(run_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_workflow_job(
        &self,
        id: &str,
        event_id: &str,
        handler: &str,
        url: &str,
        max_attempts: u32,
        workflow_run_id: &str,
        step_name: &str,
        step_index: i32,
        step_input: Option<&str>,
    ) -> Result<()> {
        let now = format_now();

        sqlx::query(
            "INSERT INTO jobs (id, event_id, handler, url, status, max_attempts, scheduled_at, created_at, \
             workflow_run_id, step_name, step_index, step_input) \
             VALUES ($1, $2, $3, $4, 'available', $5, $6, $6, $7, $8, $9, $10)",
        )
        .bind(id)
        .bind(event_id)
        .bind(handler)
        .bind(url)
        .bind(max_attempts as i32)
        .bind(&now)
        .bind(workflow_run_id)
        .bind(step_name)
        .bind(step_index)
        .bind(step_input)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn save_step_output(&self, job_id: &str, output: &str) -> Result<()> {
        sqlx::query("UPDATE jobs SET step_output = $1 WHERE id = $2")
            .bind(output)
            .bind(job_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_workflow_job_data(&self, job_id: &str) -> Result<Option<WorkflowJobRow>> {
        let row = sqlx::query_as::<_, WorkflowJobRow>(
            "SELECT workflow_run_id, step_name, step_index, step_input, step_output, branch_name \
             FROM jobs WHERE id = $1",
        )
        .bind(job_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_workflow_run(&self, run_id: &str) -> Result<Option<WorkflowRunRow>> {
        let row = sqlx::query_as::<_, WorkflowRunRow>(
            "SELECT id, workflow, event_id, status, current_step, created_at, completed_at \
             FROM workflow_runs WHERE id = $1",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_workflow_runs(
        &self,
        status: Option<&str>,
        limit: i32,
    ) -> Result<Vec<WorkflowRunRow>> {
        let rows = if let Some(status) = status {
            sqlx::query_as::<_, WorkflowRunRow>(
                "SELECT id, workflow, event_id, status, current_step, created_at, completed_at \
                 FROM workflow_runs WHERE status = $1 ORDER BY created_at DESC LIMIT $2",
            )
            .bind(status)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, WorkflowRunRow>(
                "SELECT id, workflow, event_id, status, current_step, created_at, completed_at \
                 FROM workflow_runs ORDER BY created_at DESC LIMIT $1",
            )
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };
        Ok(rows)
    }

    pub async fn redrive_workflow_run(&self, run_id: &str) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE workflow_runs SET status = 'running' WHERE id = $1 AND status = 'failed'",
        )
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Set parallel execution state on a workflow run.
    pub async fn set_parallel_state(
        &self,
        run_id: &str,
        parallel_step: &str,
        count: i32,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE workflow_runs SET parallel_step = $1, parallel_count = $2, parallel_completed = 0 \
             WHERE id = $3",
        )
        .bind(parallel_step)
        .bind(count)
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Increment parallel completed count. Returns (completed, total).
    /// Uses a single atomic query to prevent race conditions when multiple branches
    /// complete concurrently.
    pub async fn increment_parallel_completed(&self, run_id: &str) -> Result<(i32, i32)> {
        if self.driver == "postgres" {
            // Postgres: atomic UPDATE ... RETURNING
            let row: (i32, i32) = sqlx::query_as(
                "UPDATE workflow_runs SET parallel_completed = parallel_completed + 1 \
                 WHERE id = $1 \
                 RETURNING parallel_completed, parallel_count",
            )
            .bind(run_id)
            .fetch_one(&self.pool)
            .await?;
            Ok(row)
        } else {
            // SQLite: single writer guarantees atomicity.
            // MySQL: no RETURNING clause, so use UPDATE + SELECT.
            // For MySQL, callers should use appropriate locking at the application level.
            sqlx::query(
                "UPDATE workflow_runs SET parallel_completed = parallel_completed + 1 WHERE id = $1",
            )
            .bind(run_id)
            .execute(&self.pool)
            .await?;

            let row: (i32, i32) = sqlx::query_as(
                "SELECT parallel_completed, parallel_count FROM workflow_runs WHERE id = $1",
            )
            .bind(run_id)
            .fetch_one(&self.pool)
            .await?;
            Ok(row)
        }
    }

    /// Clear parallel state after all branches complete.
    pub async fn clear_parallel_state(&self, run_id: &str) -> Result<()> {
        sqlx::query(
            "UPDATE workflow_runs SET parallel_step = NULL, parallel_count = 0, parallel_completed = 0 \
             WHERE id = $1",
        )
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Insert a branch job for parallel/map execution.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_branch_job(
        &self,
        id: &str,
        event_id: &str,
        handler: &str,
        url: &str,
        max_attempts: u32,
        workflow_run_id: &str,
        step_name: &str,
        step_index: i32,
        step_input: Option<&str>,
        branch_name: &str,
    ) -> Result<()> {
        let now = format_now();

        sqlx::query(
            "INSERT INTO jobs (id, event_id, handler, url, status, max_attempts, scheduled_at, created_at, \
             workflow_run_id, step_name, step_index, step_input, branch_name) \
             VALUES ($1, $2, $3, $4, 'available', $5, $6, $6, $7, $8, $9, $10, $11)",
        )
        .bind(id)
        .bind(event_id)
        .bind(handler)
        .bind(url)
        .bind(max_attempts as i32)
        .bind(&now)
        .bind(workflow_run_id)
        .bind(step_name)
        .bind(step_index)
        .bind(step_input)
        .bind(branch_name)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get all completed branch outputs for a parallel/map step.
    pub async fn get_branch_outputs(
        &self,
        run_id: &str,
        step_name: &str,
    ) -> Result<Vec<(String, Option<String>)>> {
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT branch_name, step_output FROM jobs \
             WHERE workflow_run_id = $1 AND step_name = $2 AND branch_name IS NOT NULL AND status = 'completed' \
             ORDER BY branch_name",
        )
        .bind(run_id)
        .bind(step_name)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Insert a workflow job with a future scheduled_at (for wait steps).
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_workflow_job_delayed(
        &self,
        id: &str,
        event_id: &str,
        handler: &str,
        url: &str,
        max_attempts: u32,
        workflow_run_id: &str,
        step_name: &str,
        step_index: i32,
        step_input: Option<&str>,
        scheduled_at: &str,
    ) -> Result<()> {
        let now = format_now();

        sqlx::query(
            "INSERT INTO jobs (id, event_id, handler, url, status, max_attempts, scheduled_at, created_at, \
             workflow_run_id, step_name, step_index, step_input) \
             VALUES ($1, $2, $3, $4, 'available', $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(id)
        .bind(event_id)
        .bind(handler)
        .bind(url)
        .bind(max_attempts as i32)
        .bind(scheduled_at)
        .bind(&now)
        .bind(workflow_run_id)
        .bind(step_name)
        .bind(step_index)
        .bind(step_input)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Insert a callback job that waits for an external callback.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_callback_job(
        &self,
        id: &str,
        event_id: &str,
        handler: &str,
        max_attempts: u32,
        workflow_run_id: &str,
        step_name: &str,
        step_index: i32,
        step_input: Option<&str>,
        callback_token: &str,
        _timeout_at: Option<&str>,
    ) -> Result<()> {
        let now = format_now();

        // Use a far-future scheduled_at so it's never picked up by the worker.
        // It will be resumed via the callback API.
        let far_future = "9999-12-31T23:59:59.999";

        sqlx::query(
            "INSERT INTO jobs (id, event_id, handler, url, status, max_attempts, scheduled_at, created_at, \
             workflow_run_id, step_name, step_index, step_input, callback_token) \
             VALUES ($1, $2, $3, 'callback', 'waiting', $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(id)
        .bind(event_id)
        .bind(handler)
        .bind(max_attempts as i32)
        .bind(far_future)
        .bind(&now)
        .bind(workflow_run_id)
        .bind(step_name)
        .bind(step_index)
        .bind(step_input)
        .bind(callback_token)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Find a waiting callback job by token and resume it with payload.
    /// Atomic: UPDATE with WHERE status='waiting' prevents double-resume race conditions.
    pub async fn resume_callback_job(&self, token: &str, payload: &str) -> Result<Option<String>> {
        let now = format_now();

        // Atomic update: only succeeds if job exists AND is still waiting.
        // Concurrent requests will see rows_affected=0 for the loser.
        let result = sqlx::query(
            "UPDATE jobs SET status = 'completed', completed_at = $1, step_output = $2 \
             WHERE callback_token = $3 AND status = 'waiting'",
        )
        .bind(&now)
        .bind(payload)
        .bind(token)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        // Get the job_id (safe: callback_token has UNIQUE index)
        let row: (String,) = sqlx::query_as("SELECT id FROM jobs WHERE callback_token = $1")
            .bind(token)
            .fetch_one(&self.pool)
            .await?;

        Ok(Some(row.0))
    }

    /// Get callback job data for advancing the workflow after callback.
    pub async fn get_callback_job(&self, token: &str) -> Result<Option<JobRow>> {
        let row = sqlx::query_as::<_, JobRow>(
            "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
             FROM jobs WHERE callback_token = $1",
        )
        .bind(token)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Set timeout_at on a workflow run.
    pub async fn set_workflow_timeout(&self, run_id: &str, timeout_at: &str) -> Result<()> {
        sqlx::query("UPDATE workflow_runs SET timeout_at = $1 WHERE id = $2")
            .bind(timeout_at)
            .bind(run_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Get workflow run timeout_at.
    pub async fn get_workflow_timeout(&self, run_id: &str) -> Result<Option<String>> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT timeout_at FROM workflow_runs WHERE id = $1")
                .bind(run_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.and_then(|(t,)| t))
    }

    /// Expire waiting callback jobs whose workflow has timed out.
    pub async fn expire_timed_out_callbacks(&self) -> Result<u64> {
        let now = format_now();

        let result = sqlx::query(
            "UPDATE jobs SET status = 'dead', completed_at = $1, last_error = 'callback timeout' \
             WHERE status = 'waiting' AND callback_token IS NOT NULL \
             AND workflow_run_id IN ( \
                 SELECT id FROM workflow_runs WHERE timeout_at IS NOT NULL AND timeout_at <= $1 AND status = 'running' \
             )",
        )
        .bind(&now)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    /// Insert a sub-workflow run with parent reference.
    pub async fn insert_sub_workflow_run(
        &self,
        id: &str,
        workflow: &str,
        event_id: &str,
        first_step: &str,
        parent_run_id: &str,
        parent_step_index: i32,
    ) -> Result<()> {
        let now = format_now();

        sqlx::query(
            "INSERT INTO workflow_runs (id, workflow, event_id, status, current_step, created_at, parent_run_id, parent_step_index) \
             VALUES ($1, $2, $3, 'running', $4, $5, $6, $7)",
        )
        .bind(id)
        .bind(workflow)
        .bind(event_id)
        .bind(first_step)
        .bind(&now)
        .bind(parent_run_id)
        .bind(parent_step_index)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get parent workflow info for a sub-workflow run.
    pub async fn get_parent_workflow_run(&self, run_id: &str) -> Result<Option<(String, i32)>> {
        let row: Option<(Option<String>, Option<i32>)> = sqlx::query_as(
            "SELECT parent_run_id, parent_step_index FROM workflow_runs WHERE id = $1",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|(id, idx)| id.zip(idx)))
    }

    pub async fn retry_job(&self, job_id: &str) -> Result<bool> {
        let now = format_now();
        let result = sqlx::query(
            "UPDATE jobs SET status = 'available', scheduled_at = $1, last_error = NULL, attempt = 0 \
             WHERE id = $2 AND status IN ('dead', 'retryable')",
        )
        .bind(&now)
        .bind(job_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    // ── Queue (Pull-mode) methods ──────────────────────────────────────

    /// Fetch available queue messages, atomically marking them as running.
    /// Returns jobs joined with event payload/headers/event_type for the response.
    pub async fn fetch_queue_messages(
        &self,
        handler: &str,
        limit: i32,
    ) -> Result<Vec<QueueMessageRow>> {
        let now = format_now();

        let rows = if self.driver == "postgres" {
            sqlx::query_as::<_, QueueMessageRow>(
                "UPDATE jobs SET status = 'running', started_at = $1, attempt = attempt + 1 \
                 WHERE id IN ( \
                     SELECT id FROM jobs \
                     WHERE handler = $2 AND status IN ('available', 'retryable') AND scheduled_at <= $1 \
                     ORDER BY scheduled_at ASC \
                     LIMIT $3 \
                     FOR UPDATE SKIP LOCKED \
                 ) \
                 RETURNING id, event_id, \
                     (SELECT event_type FROM events WHERE events.id = jobs.event_id) AS event_type, \
                     (SELECT payload FROM events WHERE events.id = jobs.event_id) AS payload, \
                     (SELECT headers FROM events WHERE events.id = jobs.event_id) AS headers, \
                     attempt - 1 AS attempt, created_at",
            )
            .bind(&now)
            .bind(handler)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            // SQLite/MySQL: two-step — SELECT then mark running individually.
            let job_rows = sqlx::query_as::<_, JobRow>(
                "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                 FROM jobs \
                 WHERE handler = $1 AND status IN ('available', 'retryable') AND scheduled_at <= $2 \
                 ORDER BY scheduled_at ASC \
                 LIMIT $3",
            )
            .bind(handler)
            .bind(&now)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;

            let mut msgs = Vec::with_capacity(job_rows.len());
            for job in job_rows {
                if !self.mark_job_running(&job.id).await? {
                    continue; // another consumer grabbed it
                }
                // Fetch event data
                let evt: (String, String, Option<String>) = sqlx::query_as(
                    "SELECT event_type, payload, headers FROM events WHERE id = $1",
                )
                .bind(&job.event_id)
                .fetch_one(&self.pool)
                .await?;

                msgs.push(QueueMessageRow {
                    id: job.id,
                    event_id: job.event_id,
                    event_type: evt.0,
                    payload: evt.1,
                    headers: evt.2,
                    attempt: job.attempt,
                    created_at: job.created_at,
                });
            }
            msgs
        };

        Ok(rows)
    }

    /// Acknowledge queue messages (mark as completed).
    /// Returns the number of jobs actually acked.
    pub async fn ack_queue_messages(&self, handler: &str, ids: &[String]) -> Result<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let now = format_now();
        let mut total = 0u64;
        for id in ids {
            let result = sqlx::query(
                "UPDATE jobs SET status = 'completed', completed_at = $1 \
                 WHERE id = $2 AND handler = $3 AND status = 'running'",
            )
            .bind(&now)
            .bind(id)
            .bind(handler)
            .execute(&self.pool)
            .await?;
            if result.rows_affected() > 0 {
                // Record a successful attempt
                let attempt_id = ulid::Ulid::new().to_string();
                let _ = self
                    .insert_attempt(&attempt_id, id, 1, Some(200), None, None, 0)
                    .await;
                total += 1;
            }
        }
        Ok(total)
    }

    /// Negative-acknowledge queue messages.
    /// Jobs below max_attempts are retried with backoff; others go to DLQ.
    /// Returns (retried, dead) counts.
    pub async fn nack_queue_messages(&self, handler: &str, ids: &[String]) -> Result<(u64, u64)> {
        if ids.is_empty() {
            return Ok((0, 0));
        }
        let now = Utc::now().naive_utc();
        let now_str = format_dt(now);
        let mut retried = 0u64;
        let mut dead = 0u64;
        for id in ids {
            // Fetch current attempt and max_attempts
            let row: Option<(i32, i32)> = sqlx::query_as(
                "SELECT attempt, max_attempts FROM jobs WHERE id = $1 AND handler = $2 AND status = 'running'",
            )
            .bind(id)
            .bind(handler)
            .fetch_optional(&self.pool)
            .await?;

            let Some((attempt, max_attempts)) = row else {
                continue;
            };

            if attempt < max_attempts {
                // Retry with exponential backoff: 30s * 2^attempt, capped at 1 hour
                let backoff_secs = (30i64 * (1i64 << attempt.min(20))).min(3600);
                let next_at = format_dt(now + chrono::Duration::seconds(backoff_secs));
                sqlx::query(
                    "UPDATE jobs SET status = 'retryable', scheduled_at = $1, last_error = 'nack' \
                     WHERE id = $2 AND status = 'running'",
                )
                .bind(&next_at)
                .bind(id)
                .execute(&self.pool)
                .await?;
                retried += 1;
            } else {
                // Move to DLQ
                sqlx::query(
                    "UPDATE jobs SET status = 'dead', completed_at = $1, last_error = 'nack: max attempts exceeded' \
                     WHERE id = $2 AND status = 'running'",
                )
                .bind(&now_str)
                .bind(id)
                .execute(&self.pool)
                .await?;
                dead += 1;
            }
        }
        Ok((retried, dead))
    }

    /// Recover queue messages that exceeded their visibility timeout.
    /// Unlike stale recovery (which uses global threshold), this is per-queue.
    pub async fn recover_expired_queue_messages(
        &self,
        handler: &str,
        visibility_timeout_secs: u64,
    ) -> Result<u64> {
        let now = Utc::now().naive_utc();
        let cutoff = format_dt(now - chrono::Duration::seconds(visibility_timeout_secs as i64));
        let now_str = format_dt(now);

        let result = sqlx::query(
            "UPDATE jobs SET status = 'retryable', scheduled_at = $1 \
             WHERE handler = $2 AND status = 'running' AND started_at <= $3",
        )
        .bind(&now_str)
        .bind(handler)
        .bind(&cutoff)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    /// Count available + retryable messages in a queue (queue depth).
    pub async fn count_queue_depth(&self, handler: &str) -> Result<i64> {
        let now = format_now();
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM jobs WHERE handler = $1 AND status IN ('available', 'retryable') AND scheduled_at <= $2",
        )
        .bind(handler)
        .bind(&now)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }
}

#[derive(Debug, sqlx::FromRow)]
pub struct JobRow {
    pub id: String,
    pub event_id: String,
    pub handler: String,
    pub url: String,
    pub status: String,
    pub attempt: i32,
    pub max_attempts: i32,
    pub scheduled_at: String,
    pub last_error: Option<String>,
    pub created_at: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct EventRow {
    pub id: String,
    pub source: String,
    pub event_type: String,
    pub unique_key: Option<String>,
    pub created_at: String,
}

/// Row returned by fetch_queue_messages — job + event data for API response.
#[derive(Debug, sqlx::FromRow)]
pub struct QueueMessageRow {
    pub id: String,
    pub event_id: String,
    pub event_type: String,
    pub payload: String,
    pub headers: Option<String>,
    pub attempt: i32,
    pub created_at: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct EventRowFull {
    pub id: String,
    pub source: String,
    pub event_type: String,
    pub payload: String,
    pub headers: Option<String>,
    pub unique_key: Option<String>,
    pub created_at: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct WorkflowJobRow {
    pub workflow_run_id: Option<String>,
    pub step_name: Option<String>,
    pub step_index: Option<i32>,
    pub step_input: Option<String>,
    pub step_output: Option<String>,
    pub branch_name: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct WorkflowRunRow {
    pub id: String,
    pub workflow: String,
    pub event_id: String,
    pub status: String,
    pub current_step: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct JobAttemptRow {
    pub attempt: i32,
    pub status_code: Option<i32>,
    pub error: Option<String>,
    pub duration_ms: Option<i32>,
    pub created_at: String,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct EndpointRow {
    pub id: String,
    pub source: String,
    pub url: String,
    pub description: Option<String>,
    pub signing_secret: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct SubscriptionRow {
    pub id: String,
    pub endpoint_id: String,
    pub event_type: String,
    pub created_at: String,
}

/// Minimal endpoint info needed for outbound job creation.
#[derive(Debug, sqlx::FromRow)]
pub struct SubscribedEndpoint {
    pub id: String,
    pub url: String,
    pub signing_secret: String,
}

impl Database {
    /// Get a single job by ID.
    pub async fn get_job_by_id(&self, job_id: &str) -> Result<Option<JobRow>> {
        let row = sqlx::query_as::<_, JobRow>(
            "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
             FROM jobs WHERE id = $1",
        )
        .bind(job_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Get a single event by ID with full data.
    pub async fn get_event_by_id(&self, event_id: &str) -> Result<Option<EventRowFull>> {
        let row = sqlx::query_as::<_, EventRowFull>(
            "SELECT id, source, event_type, payload, headers, unique_key, created_at \
             FROM events WHERE id = $1",
        )
        .bind(event_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// List jobs for a specific event.
    pub async fn list_jobs_by_event(&self, event_id: &str) -> Result<Vec<JobRow>> {
        let rows = sqlx::query_as::<_, JobRow>(
            "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
             FROM jobs WHERE event_id = $1 ORDER BY created_at",
        )
        .bind(event_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// List attempts for a specific job.
    pub async fn list_job_attempts(&self, job_id: &str) -> Result<Vec<JobAttemptRow>> {
        let rows = sqlx::query_as::<_, JobAttemptRow>(
            "SELECT attempt, status_code, error, duration_ms, created_at \
             FROM job_attempts WHERE job_id = $1 ORDER BY attempt",
        )
        .bind(job_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// List workflow runs for a specific event.
    pub async fn list_workflow_runs_by_event(&self, event_id: &str) -> Result<Vec<WorkflowRunRow>> {
        let rows = sqlx::query_as::<_, WorkflowRunRow>(
            "SELECT id, workflow, event_id, status, current_step, created_at, completed_at \
             FROM workflow_runs WHERE event_id = $1 ORDER BY created_at",
        )
        .bind(event_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// List events for the inspection API (no payload, with cursor pagination).
    pub async fn list_events_for_api(
        &self,
        source: Option<&str>,
        event_type: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
        limit: i32,
        after: Option<&str>,
    ) -> Result<Vec<EventRow>> {
        let mut conditions = Vec::new();
        let mut param_idx = 1;

        if after.is_some() {
            conditions.push(format!("id > ${param_idx}"));
            param_idx += 1;
        }
        if source.is_some() {
            conditions.push(format!("source = ${param_idx}"));
            param_idx += 1;
        }
        if event_type.is_some() {
            conditions.push(format!("event_type = ${param_idx}"));
            param_idx += 1;
        }
        if since.is_some() {
            conditions.push(format!("created_at >= ${param_idx}"));
            param_idx += 1;
        }
        if until.is_some() {
            conditions.push(format!("created_at <= ${param_idx}"));
            param_idx += 1;
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT id, source, event_type, unique_key, created_at \
             FROM events{where_clause} ORDER BY id ASC LIMIT ${param_idx}"
        );

        let mut query = sqlx::query_as::<_, EventRow>(&sql);
        if let Some(v) = after {
            query = query.bind(v.to_string());
        }
        if let Some(v) = source {
            query = query.bind(v.to_string());
        }
        if let Some(v) = event_type {
            query = query.bind(v.to_string());
        }
        if let Some(v) = since {
            query = query.bind(v.to_string());
        }
        if let Some(v) = until {
            query = query.bind(v.to_string());
        }
        query = query.bind(limit);

        let rows = query.fetch_all(&self.pool).await?;
        Ok(rows)
    }

    /// List jobs with optional filters and cursor pagination for the inspection API.
    pub async fn list_jobs_filtered(
        &self,
        status: Option<&str>,
        handler: Option<&str>,
        limit: i32,
        after: Option<&str>,
    ) -> Result<Vec<JobRow>> {
        let mut conditions = Vec::new();
        let mut param_idx = 1;

        if after.is_some() {
            conditions.push(format!("id > ${param_idx}"));
            param_idx += 1;
        }
        if status.is_some() {
            conditions.push(format!("status = ${param_idx}"));
            param_idx += 1;
        }
        if handler.is_some() {
            conditions.push(format!("handler = ${param_idx}"));
            param_idx += 1;
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
             FROM jobs{where_clause} ORDER BY id ASC LIMIT ${param_idx}"
        );

        let mut query = sqlx::query_as::<_, JobRow>(&sql);
        if let Some(v) = after {
            query = query.bind(v.to_string());
        }
        if let Some(v) = status {
            query = query.bind(v.to_string());
        }
        if let Some(v) = handler {
            query = query.bind(v.to_string());
        }
        query = query.bind(limit);

        let rows = query.fetch_all(&self.pool).await?;
        Ok(rows)
    }

    // --- Outbound webhook endpoint CRUD ---

    pub async fn insert_endpoint(
        &self,
        id: &str,
        source: &str,
        url: &str,
        description: Option<&str>,
        signing_secret: &str,
    ) -> Result<()> {
        let now = format_now();
        sqlx::query(
            "INSERT INTO outbound_endpoints (id, source, url, description, signing_secret, status, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, 'active', $6, $6)",
        )
        .bind(id)
        .bind(source)
        .bind(url)
        .bind(description)
        .bind(signing_secret)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_endpoint(&self, id: &str) -> Result<Option<EndpointRow>> {
        let row = sqlx::query_as::<_, EndpointRow>(
            "SELECT id, source, url, description, signing_secret, status, created_at, updated_at \
             FROM outbound_endpoints WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_endpoints(&self, source: Option<&str>) -> Result<Vec<EndpointRow>> {
        let rows = if let Some(src) = source {
            sqlx::query_as::<_, EndpointRow>(
                "SELECT id, source, url, description, signing_secret, status, created_at, updated_at \
                 FROM outbound_endpoints WHERE source = $1 ORDER BY created_at",
            )
            .bind(src)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, EndpointRow>(
                "SELECT id, source, url, description, signing_secret, status, created_at, updated_at \
                 FROM outbound_endpoints ORDER BY created_at",
            )
            .fetch_all(&self.pool)
            .await?
        };
        Ok(rows)
    }

    pub async fn update_endpoint(
        &self,
        id: &str,
        url: Option<&str>,
        description: Option<&str>,
        status: Option<&str>,
    ) -> Result<bool> {
        let now = format_now();
        // Build dynamic update — always update updated_at
        let current = self.get_endpoint(id).await?;
        let Some(current) = current else {
            return Ok(false);
        };
        let new_url = url.unwrap_or(&current.url);
        let new_desc = description.or(current.description.as_deref());
        let new_status = status.unwrap_or(&current.status);

        let result = sqlx::query(
            "UPDATE outbound_endpoints SET url = $1, description = $2, status = $3, updated_at = $4 WHERE id = $5",
        )
        .bind(new_url)
        .bind(new_desc)
        .bind(new_status)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_endpoint(&self, id: &str) -> Result<bool> {
        // Delete subscriptions first
        sqlx::query("DELETE FROM outbound_subscriptions WHERE endpoint_id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        let result = sqlx::query("DELETE FROM outbound_endpoints WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn rotate_endpoint_secret(&self, id: &str, new_secret: &str) -> Result<bool> {
        let now = format_now();
        let result = sqlx::query(
            "UPDATE outbound_endpoints SET signing_secret = $1, updated_at = $2 WHERE id = $3",
        )
        .bind(new_secret)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    // --- Outbound subscription CRUD ---

    pub async fn insert_subscriptions(
        &self,
        endpoint_id: &str,
        event_types: &[String],
    ) -> Result<Vec<SubscriptionRow>> {
        let now = format_now();
        let mut created = Vec::new();
        for event_type in event_types {
            let id = ulid::Ulid::new().to_string();
            let result = if self.driver == "mysql" {
                sqlx::query(
                    "INSERT IGNORE INTO outbound_subscriptions (id, endpoint_id, event_type, created_at) \
                     VALUES ($1, $2, $3, $4)",
                )
                .bind(&id)
                .bind(endpoint_id)
                .bind(event_type)
                .bind(&now)
                .execute(&self.pool)
                .await?
            } else {
                sqlx::query(
                    "INSERT INTO outbound_subscriptions (id, endpoint_id, event_type, created_at) \
                     VALUES ($1, $2, $3, $4) \
                     ON CONFLICT (endpoint_id, event_type) DO NOTHING",
                )
                .bind(&id)
                .bind(endpoint_id)
                .bind(event_type)
                .bind(&now)
                .execute(&self.pool)
                .await?
            };
            if result.rows_affected() > 0 {
                created.push(SubscriptionRow {
                    id,
                    endpoint_id: endpoint_id.to_string(),
                    event_type: event_type.clone(),
                    created_at: now.clone(),
                });
            }
        }
        Ok(created)
    }

    pub async fn list_subscriptions(&self, endpoint_id: &str) -> Result<Vec<SubscriptionRow>> {
        let rows = sqlx::query_as::<_, SubscriptionRow>(
            "SELECT id, endpoint_id, event_type, created_at \
             FROM outbound_subscriptions WHERE endpoint_id = $1 ORDER BY created_at",
        )
        .bind(endpoint_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn delete_subscription(&self, id: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM outbound_subscriptions WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Find all active endpoints subscribed to this source + event_type.
    pub async fn find_subscribed_endpoints(
        &self,
        source: &str,
        event_type: &str,
    ) -> Result<Vec<SubscribedEndpoint>> {
        let rows = sqlx::query_as::<_, SubscribedEndpoint>(
            "SELECT DISTINCT e.id, e.url, e.signing_secret \
             FROM outbound_endpoints e \
             JOIN outbound_subscriptions s ON s.endpoint_id = e.id \
             WHERE e.source = $1 AND e.status = 'active' \
               AND (s.event_type = $2 OR s.event_type = '*') \
             ORDER BY e.id",
        )
        .bind(source)
        .bind(event_type)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ── Queue CLI helper methods ──────────────────────────────────────

    /// Count jobs by status for a specific handler pattern (e.g. "queue/{name}").
    pub async fn count_jobs_by_handler_status(
        &self,
        handler: &str,
    ) -> Result<Vec<(String, i64)>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT status, COUNT(*) as cnt FROM jobs WHERE handler = $1 GROUP BY status",
        )
        .bind(handler)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// List jobs for a specific handler with optional status filter.
    pub async fn list_jobs_by_handler(
        &self,
        handler: &str,
        status: Option<&str>,
        limit: i32,
    ) -> Result<Vec<JobRow>> {
        let rows = if let Some(status) = status {
            sqlx::query_as::<_, JobRow>(
                "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                 FROM jobs WHERE handler = $1 AND status = $2 ORDER BY scheduled_at DESC LIMIT $3",
            )
            .bind(handler)
            .bind(status)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, JobRow>(
                "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                 FROM jobs WHERE handler = $1 ORDER BY scheduled_at DESC LIMIT $2",
            )
            .bind(handler)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };
        Ok(rows)
    }

    /// Delete all jobs for a specific handler.
    pub async fn delete_jobs_by_handler(&self, handler: &str) -> Result<u64> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "DELETE FROM job_attempts WHERE job_id IN (SELECT id FROM jobs WHERE handler = $1)",
        )
        .bind(handler)
        .execute(&mut *tx)
        .await?;

        let result = sqlx::query("DELETE FROM jobs WHERE handler = $1")
            .bind(handler)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(result.rows_affected())
    }

    /// Retry all dead jobs for a specific handler. Resets attempt counter.
    pub async fn retry_dead_jobs_by_handler(&self, handler: &str) -> Result<u64> {
        let now = format_now();
        let result = sqlx::query(
            "UPDATE jobs SET status = 'available', scheduled_at = $1, last_error = NULL, attempt = 0 \
             WHERE handler = $2 AND status = 'dead'",
        )
        .bind(&now)
        .bind(handler)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Peek at the first available job for a handler without consuming it.
    pub async fn peek_queue_job(&self, handler: &str) -> Result<Option<JobRow>> {
        let row = sqlx::query_as::<_, JobRow>(
            "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
             FROM jobs WHERE handler = $1 AND status IN ('available', 'retryable') \
             ORDER BY scheduled_at ASC LIMIT 1",
        )
        .bind(handler)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Get signing secret for an outbound endpoint.
    pub async fn get_endpoint_secret(&self, endpoint_id: &str) -> Result<Option<String>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT signing_secret FROM outbound_endpoints WHERE id = $1")
                .bind(endpoint_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|r| r.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_url_with_credentials() {
        assert_eq!(
            redact_url("postgres://user:password@host:5432/db"),
            "postgres://***@host:5432/db"
        );
    }

    #[test]
    fn test_redact_url_with_user_only() {
        assert_eq!(
            redact_url("postgres://admin@host:5432/db"),
            "postgres://***@host:5432/db"
        );
    }

    #[test]
    fn test_redact_url_without_credentials() {
        assert_eq!(
            redact_url("sqlite:qhook.db?mode=rwc"),
            "sqlite:qhook.db?mode=rwc"
        );
    }

    #[test]
    fn test_redact_url_no_scheme() {
        assert_eq!(redact_url("just-a-path"), "just-a-path");
    }

    #[test]
    fn test_redact_url_scheme_no_credentials() {
        assert_eq!(
            redact_url("postgres://db.example.com:5432/mydb"),
            "postgres://db.example.com:5432/mydb"
        );
    }

    #[test]
    fn test_redact_url_complex_password() {
        assert_eq!(
            redact_url("postgres://user:p%40ss%3Dw0rd@db.example.com:5432/mydb"),
            "postgres://***@db.example.com:5432/mydb"
        );
    }
}
