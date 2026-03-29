//! D1-specific Database method implementations.
//!
//! Each method corresponds to a method on `Database` and is called when
//! the backend is D1. SQL uses SQLite dialect (D1 is SQLite-based).
//! Parameters use `?N` positional syntax (D1/SQLite style) instead of `$N`.

use anyhow::Result;
use chrono::Utc;

use crate::d1;
use crate::db::*;

/// Convert `$N` parameter placeholders to `?N` (D1/SQLite native format).
/// D1 HTTP API expects `?1`, `?2`, etc. instead of `$1`, `$2`.
fn rewrite_params(sql: &str) -> String {
    let mut result = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '$' {
            // Check if next chars are digits
            let mut digits = String::new();
            while let Some(&next) = chars.peek() {
                if next.is_ascii_digit() {
                    digits.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            if digits.is_empty() {
                result.push(ch);
            } else {
                result.push('?');
                result.push_str(&digits);
            }
        } else {
            result.push(ch);
        }
    }
    result
}

/// Helper: build params vec from bindings.
macro_rules! params {
    ($($val:expr),* $(,)?) => {
        vec![$( serde_json::json!($val) ),*]
    };
}

impl Database {
    // ── Migration ──────────────────────────────────────────────────────

    pub(crate) async fn migrate_d1(&self) -> Result<()> {
        let d1 = self.d1();

        // Create migration tracking table
        d1.execute(
            "CREATE TABLE IF NOT EXISTS _migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL)",
            vec![],
        ).await?;

        // Get current version
        let current_version: i32 = match d1
            .query_optional("SELECT COALESCE(MAX(version), 0) as v FROM _migrations", vec![])
            .await?
        {
            Some(row) => d1::row_get_i32(&row, "v").unwrap_or(0),
            None => 0,
        };

        // Detect pre-existing tables
        let has_events_table = d1
            .execute("SELECT 1 FROM events LIMIT 0", vec![])
            .await
            .is_ok();

        let effective_version = if current_version == 0 && has_events_table {
            tracing::info!("Pre-existing database detected, initializing migration tracking");
            for v in 1..=4i32 {
                d1.execute(
                    &rewrite_params("INSERT INTO _migrations (version, applied_at) VALUES (?1, ?2)"),
                    params![v, format_now()],
                )
                .await
                .ok();
            }
            4
        } else {
            current_version
        };

        // D1 uses SQLite syntax (same as sqlite path, no mysql branching needed)
        let migrations: Vec<(i32, &str, Vec<&str>)> = vec![
            (
                1,
                "Core tables",
                vec![
                    "CREATE TABLE IF NOT EXISTS events (
                        id TEXT PRIMARY KEY, source TEXT NOT NULL, event_type TEXT NOT NULL,
                        payload TEXT NOT NULL, headers TEXT, unique_key TEXT, created_at TEXT NOT NULL
                    )",
                    "CREATE UNIQUE INDEX IF NOT EXISTS idx_events_unique ON events (source, unique_key)",
                    "CREATE TABLE IF NOT EXISTS jobs (
                        id TEXT PRIMARY KEY, event_id TEXT NOT NULL, handler TEXT NOT NULL,
                        url TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'available',
                        attempt INTEGER NOT NULL DEFAULT 0, max_attempts INTEGER NOT NULL DEFAULT 5,
                        scheduled_at TEXT NOT NULL, started_at TEXT, completed_at TEXT,
                        created_at TEXT NOT NULL, last_error TEXT
                    )",
                    "CREATE INDEX IF NOT EXISTS idx_jobs_fetch ON jobs (status, scheduled_at)",
                    "CREATE TABLE IF NOT EXISTS job_attempts (
                        id TEXT PRIMARY KEY, job_id TEXT NOT NULL, attempt INTEGER NOT NULL,
                        status_code INTEGER, response_body TEXT, error TEXT,
                        duration_ms INTEGER, created_at TEXT NOT NULL
                    )",
                ],
            ),
            (
                2,
                "Workflow tables",
                vec![
                    "CREATE TABLE IF NOT EXISTS workflow_runs (
                        id TEXT PRIMARY KEY, workflow TEXT NOT NULL, event_id TEXT NOT NULL,
                        status TEXT NOT NULL DEFAULT 'running', current_step TEXT,
                        created_at TEXT NOT NULL, completed_at TEXT
                    )",
                    "CREATE INDEX IF NOT EXISTS idx_workflow_runs_status ON workflow_runs (status)",
                    "ALTER TABLE jobs ADD COLUMN workflow_run_id TEXT",
                    "ALTER TABLE jobs ADD COLUMN step_name TEXT",
                    "ALTER TABLE jobs ADD COLUMN step_index INTEGER",
                    "ALTER TABLE jobs ADD COLUMN step_input TEXT",
                    "ALTER TABLE jobs ADD COLUMN step_output TEXT",
                    "ALTER TABLE jobs ADD COLUMN branch_name TEXT",
                ],
            ),
            (
                3,
                "Workflow extensions",
                vec![
                    "ALTER TABLE workflow_runs ADD COLUMN parallel_step TEXT",
                    "ALTER TABLE workflow_runs ADD COLUMN parallel_count INTEGER DEFAULT 0",
                    "ALTER TABLE workflow_runs ADD COLUMN parallel_completed INTEGER DEFAULT 0",
                    "ALTER TABLE workflow_runs ADD COLUMN timeout_at TEXT",
                    "ALTER TABLE workflow_runs ADD COLUMN parent_run_id TEXT",
                    "ALTER TABLE workflow_runs ADD COLUMN parent_step_index INTEGER",
                    "ALTER TABLE jobs ADD COLUMN callback_token TEXT",
                    "CREATE UNIQUE INDEX IF NOT EXISTS idx_jobs_callback_token ON jobs (callback_token)",
                ],
            ),
            (
                4,
                "Outbound webhooks",
                vec![
                    "CREATE TABLE IF NOT EXISTS outbound_endpoints (
                        id TEXT PRIMARY KEY, source TEXT NOT NULL, url TEXT NOT NULL,
                        description TEXT, signing_secret TEXT NOT NULL,
                        status TEXT NOT NULL DEFAULT 'active',
                        created_at TEXT NOT NULL, updated_at TEXT NOT NULL
                    )",
                    "CREATE TABLE IF NOT EXISTS outbound_subscriptions (
                        id TEXT PRIMARY KEY, endpoint_id TEXT NOT NULL,
                        event_type TEXT NOT NULL, created_at TEXT NOT NULL
                    )",
                    "CREATE UNIQUE INDEX IF NOT EXISTS idx_outbound_sub_unique ON outbound_subscriptions (endpoint_id, event_type)",
                    "CREATE INDEX IF NOT EXISTS idx_outbound_endpoints_source ON outbound_endpoints (source, status)",
                ],
            ),
        ];

        for (version, name, queries) in &migrations {
            if *version <= effective_version {
                continue;
            }
            tracing::info!(version, name, "Applying migration (D1)");
            for sql in queries {
                if sql.contains("ALTER TABLE") {
                    // ALTER TABLE may fail if column already exists
                    d1.execute(sql, vec![]).await.ok();
                } else {
                    d1.execute(sql, vec![]).await?;
                }
            }
            d1.execute(
                &rewrite_params("INSERT INTO _migrations (version, applied_at) VALUES (?1, ?2)"),
                params![*version, format_now()],
            )
            .await?;
        }

        tracing::info!(
            version = migrations.last().map_or(0, |m| m.0),
            "D1 database migrated"
        );
        Ok(())
    }

    // ── Events ────────────────────────────────────────────────────────

    pub(crate) async fn d1_insert_event(
        &self,
        id: &str,
        source: &str,
        event_type: &str,
        payload: &str,
        headers: Option<&str>,
        unique_key: Option<&str>,
    ) -> Result<bool> {
        let d1 = self.d1();
        let now = format_now();

        if unique_key.is_some() {
            let result = d1
                .execute(
                    "INSERT INTO events (id, source, event_type, payload, headers, unique_key, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                     ON CONFLICT (source, unique_key) DO NOTHING",
                    params![id, source, event_type, payload, headers, unique_key, now],
                )
                .await?;
            Ok(result.rows_affected > 0)
        } else {
            d1.execute(
                "INSERT INTO events (id, source, event_type, payload, headers, unique_key, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id, source, event_type, payload, headers, unique_key, now],
            )
            .await?;
            Ok(true)
        }
    }

    /// D1: Insert event + jobs. D1 is single-writer so individual INSERTs are
    /// sequentially consistent. No explicit transaction available via REST API,
    /// but D1's single-writer model prevents partial writes from concurrent requests.
    pub(crate) async fn d1_insert_event_and_jobs(
        &self,
        event_id: &str,
        source: &str,
        event_type: &str,
        payload: &str,
        headers: Option<&str>,
        unique_key: Option<&str>,
        jobs: &[(String, String, String, u32)],
    ) -> Result<bool> {
        // Insert event first
        let created = self
            .d1_insert_event(event_id, source, event_type, payload, headers, unique_key)
            .await?;
        if !created {
            return Ok(false);
        }
        // Insert jobs sequentially (D1 single writer ensures consistency)
        for (job_id, handler, url, max_attempts) in jobs {
            self.d1_insert_job(job_id, event_id, handler, url, *max_attempts)
                .await?;
        }
        Ok(true)
    }

    pub(crate) async fn d1_get_event_payload(&self, event_id: &str) -> Result<String> {
        let row = self
            .d1()
            .query_one("SELECT payload FROM events WHERE id = ?1", params![event_id])
            .await?;
        d1::row_get_string(&row, "payload")
    }

    pub(crate) async fn d1_get_event_headers(&self, event_id: &str) -> Result<Option<String>> {
        let row = self
            .d1()
            .query_one("SELECT headers FROM events WHERE id = ?1", params![event_id])
            .await?;
        Ok(d1::row_get_opt_string(&row, "headers"))
    }

    pub(crate) async fn d1_get_event_data(
        &self,
        event_id: &str,
    ) -> Result<(String, Option<String>)> {
        let row = self
            .d1()
            .query_one(
                "SELECT payload, headers FROM events WHERE id = ?1",
                params![event_id],
            )
            .await?;
        Ok((
            d1::row_get_string(&row, "payload")?,
            d1::row_get_opt_string(&row, "headers"),
        ))
    }

    pub(crate) async fn d1_list_events(&self, limit: i32) -> Result<Vec<EventRow>> {
        let rows = self
            .d1()
            .query(
                "SELECT id, source, event_type, unique_key, created_at \
                 FROM events ORDER BY created_at DESC LIMIT ?1",
                params![limit],
            )
            .await?;
        rows.iter().map(d1::row_to_event).collect()
    }

    pub(crate) async fn d1_list_events_filtered(
        &self,
        source: Option<&str>,
        event_type: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
        limit: i32,
    ) -> Result<Vec<EventRowFull>> {
        let mut conditions = Vec::new();
        let mut values: Vec<serde_json::Value> = Vec::new();
        let mut idx = 1;

        if let Some(v) = source {
            conditions.push(format!("source = ?{idx}"));
            values.push(serde_json::json!(v));
            idx += 1;
        }
        if let Some(v) = event_type {
            conditions.push(format!("event_type = ?{idx}"));
            values.push(serde_json::json!(v));
            idx += 1;
        }
        if let Some(v) = since {
            conditions.push(format!("created_at >= ?{idx}"));
            values.push(serde_json::json!(v));
            idx += 1;
        }
        if let Some(v) = until {
            conditions.push(format!("created_at <= ?{idx}"));
            values.push(serde_json::json!(v));
            idx += 1;
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT id, source, event_type, payload, headers, unique_key, created_at \
             FROM events{where_clause} ORDER BY created_at ASC LIMIT ?{idx}"
        );
        values.push(serde_json::json!(limit));

        let rows = self.d1().query(&sql, values).await?;
        rows.iter().map(d1::row_to_event_full).collect()
    }

    pub(crate) async fn d1_list_events_after(
        &self,
        after_id: Option<&str>,
        source: Option<&str>,
        limit: i32,
    ) -> Result<Vec<EventRow>> {
        let rows = match (after_id, source) {
            (Some(id), Some(src)) => {
                self.d1()
                    .query(
                        "SELECT id, source, event_type, unique_key, created_at \
                         FROM events WHERE id > ?1 AND source = ?2 ORDER BY id ASC LIMIT ?3",
                        params![id, src, limit],
                    )
                    .await?
            }
            (Some(id), None) => {
                self.d1()
                    .query(
                        "SELECT id, source, event_type, unique_key, created_at \
                         FROM events WHERE id > ?1 ORDER BY id ASC LIMIT ?2",
                        params![id, limit],
                    )
                    .await?
            }
            (None, Some(src)) => {
                self.d1()
                    .query(
                        "SELECT id, source, event_type, unique_key, created_at \
                         FROM events WHERE source = ?1 ORDER BY id DESC LIMIT ?2",
                        params![src, limit],
                    )
                    .await?
            }
            (None, None) => {
                self.d1()
                    .query(
                        "SELECT id, source, event_type, unique_key, created_at \
                         FROM events ORDER BY id DESC LIMIT ?1",
                        params![limit],
                    )
                    .await?
            }
        };
        rows.iter().map(d1::row_to_event).collect()
    }

    pub(crate) async fn d1_get_event_by_id(
        &self,
        event_id: &str,
    ) -> Result<Option<EventRowFull>> {
        let row = self
            .d1()
            .query_optional(
                "SELECT id, source, event_type, payload, headers, unique_key, created_at \
                 FROM events WHERE id = ?1",
                params![event_id],
            )
            .await?;
        row.as_ref().map(d1::row_to_event_full).transpose()
    }

    pub(crate) async fn d1_list_events_for_api(
        &self,
        source: Option<&str>,
        event_type: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
        limit: i32,
        after: Option<&str>,
    ) -> Result<Vec<EventRow>> {
        let mut conditions = Vec::new();
        let mut values: Vec<serde_json::Value> = Vec::new();
        let mut idx = 1;

        if let Some(v) = after {
            conditions.push(format!("id > ?{idx}"));
            values.push(serde_json::json!(v));
            idx += 1;
        }
        if let Some(v) = source {
            conditions.push(format!("source = ?{idx}"));
            values.push(serde_json::json!(v));
            idx += 1;
        }
        if let Some(v) = event_type {
            conditions.push(format!("event_type = ?{idx}"));
            values.push(serde_json::json!(v));
            idx += 1;
        }
        if let Some(v) = since {
            conditions.push(format!("created_at >= ?{idx}"));
            values.push(serde_json::json!(v));
            idx += 1;
        }
        if let Some(v) = until {
            conditions.push(format!("created_at <= ?{idx}"));
            values.push(serde_json::json!(v));
            idx += 1;
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT id, source, event_type, unique_key, created_at \
             FROM events{where_clause} ORDER BY id ASC LIMIT ?{idx}"
        );
        values.push(serde_json::json!(limit));

        let rows = self.d1().query(&sql, values).await?;
        rows.iter().map(d1::row_to_event).collect()
    }

    // ── Jobs ──────────────────────────────────────────────────────────

    pub(crate) async fn d1_insert_job(
        &self,
        id: &str,
        event_id: &str,
        handler: &str,
        url: &str,
        max_attempts: u32,
    ) -> Result<()> {
        let now = format_now();
        self.d1()
            .execute(
                "INSERT INTO jobs (id, event_id, handler, url, status, max_attempts, scheduled_at, created_at) \
                 VALUES (?1, ?2, ?3, ?4, 'available', ?5, ?6, ?6)",
                params![id, event_id, handler, url, i32::try_from(max_attempts).unwrap_or(i32::MAX), now],
            )
            .await?;
        Ok(())
    }

    /// D1: Fetch available jobs (no SKIP LOCKED — single writer like SQLite).
    pub(crate) async fn d1_fetch_available_jobs(&self, limit: i32) -> Result<Vec<JobRow>> {
        let now = format_now();
        let rows = self
            .d1()
            .query(
                "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                 FROM jobs \
                 WHERE status IN ('available', 'retryable') AND scheduled_at <= ?1 \
                 AND handler NOT LIKE 'queue/%' \
                 ORDER BY scheduled_at ASC \
                 LIMIT ?2",
                params![now, limit],
            )
            .await?;
        rows.iter().map(d1::row_to_job).collect()
    }

    pub(crate) async fn d1_mark_job_running(&self, job_id: &str) -> Result<bool> {
        let now = format_now();
        let result = self
            .d1()
            .execute(
                "UPDATE jobs SET status = 'running', started_at = ?1, attempt = attempt + 1 \
                 WHERE id = ?2 AND status IN ('available', 'retryable')",
                params![now, job_id],
            )
            .await?;
        Ok(result.rows_affected > 0)
    }

    pub(crate) async fn d1_mark_job_completed(&self, job_id: &str) -> Result<()> {
        let now = format_now();
        self.d1()
            .execute(
                "UPDATE jobs SET status = 'completed', completed_at = ?1 WHERE id = ?2",
                params![now, job_id],
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn d1_mark_job_retryable(
        &self,
        job_id: &str,
        next_attempt_at: chrono::NaiveDateTime,
        error: &str,
    ) -> Result<()> {
        let scheduled = format_dt(next_attempt_at);
        self.d1()
            .execute(
                "UPDATE jobs SET status = 'retryable', scheduled_at = ?1, last_error = ?2 WHERE id = ?3",
                params![scheduled, error, job_id],
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn d1_mark_job_dead(&self, job_id: &str, error: &str) -> Result<()> {
        let now = format_now();
        self.d1()
            .execute(
                "UPDATE jobs SET status = 'dead', completed_at = ?1, last_error = ?2 WHERE id = ?3",
                params![now, error, job_id],
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn d1_insert_attempt(
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
        self.d1()
            .execute(
                "INSERT INTO job_attempts (id, job_id, attempt, status_code, response_body, error, duration_ms, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![id, job_id, attempt, status_code, response_body, error, duration_ms, now],
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn d1_list_jobs(
        &self,
        status: Option<&str>,
        limit: i32,
    ) -> Result<Vec<JobRow>> {
        let rows = if let Some(status) = status {
            self.d1()
                .query(
                    "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                     FROM jobs WHERE status = ?1 ORDER BY scheduled_at DESC LIMIT ?2",
                    params![status, limit],
                )
                .await?
        } else {
            self.d1()
                .query(
                    "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                     FROM jobs ORDER BY scheduled_at DESC LIMIT ?1",
                    params![limit],
                )
                .await?
        };
        rows.iter().map(d1::row_to_job).collect()
    }

    pub(crate) async fn d1_list_jobs_after(
        &self,
        after_id: Option<&str>,
        status: Option<&str>,
        limit: i32,
    ) -> Result<Vec<JobRow>> {
        let rows = match (after_id, status) {
            (Some(id), Some(st)) => {
                self.d1()
                    .query(
                        "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                         FROM jobs WHERE id > ?1 AND status = ?2 ORDER BY id ASC LIMIT ?3",
                        params![id, st, limit],
                    )
                    .await?
            }
            (Some(id), None) => {
                self.d1()
                    .query(
                        "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                         FROM jobs WHERE id > ?1 AND status IN ('completed', 'dead', 'retryable') ORDER BY id ASC LIMIT ?2",
                        params![id, limit],
                    )
                    .await?
            }
            (None, Some(st)) => {
                self.d1()
                    .query(
                        "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                         FROM jobs WHERE status = ?1 ORDER BY id DESC LIMIT ?2",
                        params![st, limit],
                    )
                    .await?
            }
            (None, None) => {
                self.d1()
                    .query(
                        "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                         FROM jobs WHERE status IN ('completed', 'dead', 'retryable') ORDER BY id DESC LIMIT ?1",
                        params![limit],
                    )
                    .await?
            }
        };
        rows.iter().map(d1::row_to_job).collect()
    }

    pub(crate) async fn d1_retry_dead_jobs(&self) -> Result<u64> {
        let now = format_now();
        let result = self
            .d1()
            .execute(
                "UPDATE jobs SET status = 'available', scheduled_at = ?1, last_error = NULL WHERE status = 'dead'",
                params![now],
            )
            .await?;
        Ok(result.rows_affected)
    }

    pub(crate) async fn d1_recover_stale_jobs(&self, stale_secs: i64) -> Result<u64> {
        let now = Utc::now().naive_utc();
        let cutoff = format_dt(now - chrono::Duration::seconds(stale_secs));
        let now_str = format_dt(now);
        let result = self
            .d1()
            .execute(
                "UPDATE jobs SET status = 'retryable', scheduled_at = ?1 \
                 WHERE status = 'running' AND started_at <= ?2 \
                 AND handler NOT LIKE 'queue/%'",
                params![now_str, cutoff],
            )
            .await?;
        Ok(result.rows_affected)
    }

    pub(crate) async fn d1_cleanup_old_records(
        &self,
        retention_hours: i64,
    ) -> Result<(u64, u64)> {
        let cutoff = format_dt(Utc::now().naive_utc() - chrono::Duration::hours(retention_hours));
        let attempts = self
            .d1()
            .execute(
                "DELETE FROM job_attempts WHERE job_id IN \
                 (SELECT id FROM jobs WHERE status IN ('completed', 'dead') AND completed_at < ?1)",
                params![cutoff],
            )
            .await?;
        let jobs = self
            .d1()
            .execute(
                "DELETE FROM jobs WHERE status IN ('completed', 'dead') AND completed_at < ?1",
                params![cutoff],
            )
            .await?;
        Ok((jobs.rows_affected, attempts.rows_affected))
    }

    /// D1 version: expire available/retryable jobs older than `ttl_secs`.
    pub(crate) async fn d1_expire_old_jobs(&self, ttl_secs: i64) -> Result<u64> {
        let cutoff = format_dt(Utc::now().naive_utc() - chrono::Duration::seconds(ttl_secs));
        let now = format_now();
        let result = self
            .d1()
            .execute(
                "UPDATE jobs SET status = 'dead', completed_at = ?1, last_error = 'event TTL expired' \
                 WHERE status IN ('available', 'retryable') AND created_at <= ?2",
                params![now, cutoff],
            )
            .await?;
        Ok(result.rows_affected)
    }

    pub(crate) async fn d1_queue_depth(&self) -> Result<i64> {
        let row = self
            .d1()
            .query_one(
                "SELECT COUNT(*) as cnt FROM jobs WHERE status IN ('available', 'retryable')",
                vec![],
            )
            .await?;
        d1::row_get_i64(&row, "cnt")
    }

    pub(crate) async fn d1_dead_job_count(&self) -> Result<i64> {
        let row = self
            .d1()
            .query_one(
                "SELECT COUNT(*) as cnt FROM jobs WHERE status = 'dead'",
                vec![],
            )
            .await?;
        d1::row_get_i64(&row, "cnt")
    }

    pub(crate) async fn d1_get_job_by_id(&self, job_id: &str) -> Result<Option<JobRow>> {
        let row = self
            .d1()
            .query_optional(
                "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                 FROM jobs WHERE id = ?1",
                params![job_id],
            )
            .await?;
        row.as_ref().map(d1::row_to_job).transpose()
    }

    pub(crate) async fn d1_list_jobs_by_event(&self, event_id: &str) -> Result<Vec<JobRow>> {
        let rows = self
            .d1()
            .query(
                "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                 FROM jobs WHERE event_id = ?1 ORDER BY created_at",
                params![event_id],
            )
            .await?;
        rows.iter().map(d1::row_to_job).collect()
    }

    pub(crate) async fn d1_list_job_attempts(&self, job_id: &str) -> Result<Vec<JobAttemptRow>> {
        let rows = self
            .d1()
            .query(
                "SELECT attempt, status_code, error, duration_ms, created_at \
                 FROM job_attempts WHERE job_id = ?1 ORDER BY attempt",
                params![job_id],
            )
            .await?;
        rows.iter().map(d1::row_to_job_attempt).collect()
    }

    pub(crate) async fn d1_list_jobs_filtered(
        &self,
        status: Option<&str>,
        handler: Option<&str>,
        limit: i32,
        after: Option<&str>,
    ) -> Result<Vec<JobRow>> {
        let mut conditions = Vec::new();
        let mut values: Vec<serde_json::Value> = Vec::new();
        let mut idx = 1;

        if let Some(v) = after {
            conditions.push(format!("id > ?{idx}"));
            values.push(serde_json::json!(v));
            idx += 1;
        }
        if let Some(v) = status {
            conditions.push(format!("status = ?{idx}"));
            values.push(serde_json::json!(v));
            idx += 1;
        }
        if let Some(v) = handler {
            conditions.push(format!("handler = ?{idx}"));
            values.push(serde_json::json!(v));
            idx += 1;
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
             FROM jobs{where_clause} ORDER BY id ASC LIMIT ?{idx}"
        );
        values.push(serde_json::json!(limit));

        let rows = self.d1().query(&sql, values).await?;
        rows.iter().map(d1::row_to_job).collect()
    }

    pub(crate) async fn d1_retry_job(&self, job_id: &str) -> Result<bool> {
        let now = format_now();
        let result = self
            .d1()
            .execute(
                "UPDATE jobs SET status = 'available', scheduled_at = ?1, last_error = NULL, attempt = 0 \
                 WHERE id = ?2 AND status IN ('dead', 'retryable')",
                params![now, job_id],
            )
            .await?;
        Ok(result.rows_affected > 0)
    }

    // ── Workflows ─────────────────────────────────────────────────────

    pub(crate) async fn d1_insert_workflow_run(
        &self,
        id: &str,
        workflow: &str,
        event_id: &str,
        first_step: &str,
    ) -> Result<()> {
        let now = format_now();
        self.d1()
            .execute(
                "INSERT INTO workflow_runs (id, workflow, event_id, status, current_step, created_at) \
                 VALUES (?1, ?2, ?3, 'running', ?4, ?5)",
                params![id, workflow, event_id, first_step, now],
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn d1_update_workflow_run_step(
        &self,
        run_id: &str,
        step_name: &str,
    ) -> Result<()> {
        self.d1()
            .execute(
                "UPDATE workflow_runs SET current_step = ?1 WHERE id = ?2",
                params![step_name, run_id],
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn d1_complete_workflow_run(&self, run_id: &str) -> Result<()> {
        let now = format_now();
        self.d1()
            .execute(
                "UPDATE workflow_runs SET status = 'completed', completed_at = ?1 WHERE id = ?2",
                params![now, run_id],
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn d1_fail_workflow_run(&self, run_id: &str) -> Result<()> {
        let now = format_now();
        self.d1()
            .execute(
                "UPDATE workflow_runs SET status = 'failed', completed_at = ?1 WHERE id = ?2",
                params![now, run_id],
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn d1_insert_workflow_job(
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
        self.d1()
            .execute(
                "INSERT INTO jobs (id, event_id, handler, url, status, max_attempts, scheduled_at, created_at, \
                 workflow_run_id, step_name, step_index, step_input) \
                 VALUES (?1, ?2, ?3, ?4, 'available', ?5, ?6, ?6, ?7, ?8, ?9, ?10)",
                params![id, event_id, handler, url, i32::try_from(max_attempts).unwrap_or(i32::MAX), now, workflow_run_id, step_name, step_index, step_input],
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn d1_save_step_output(&self, job_id: &str, output: &str) -> Result<()> {
        self.d1()
            .execute(
                "UPDATE jobs SET step_output = ?1 WHERE id = ?2",
                params![output, job_id],
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn d1_get_workflow_job_data(
        &self,
        job_id: &str,
    ) -> Result<Option<WorkflowJobRow>> {
        let row = self
            .d1()
            .query_optional(
                "SELECT workflow_run_id, step_name, step_index, step_input, step_output, branch_name \
                 FROM jobs WHERE id = ?1",
                params![job_id],
            )
            .await?;
        row.as_ref().map(d1::row_to_workflow_job).transpose()
    }

    pub(crate) async fn d1_get_workflow_run(
        &self,
        run_id: &str,
    ) -> Result<Option<WorkflowRunRow>> {
        let row = self
            .d1()
            .query_optional(
                "SELECT id, workflow, event_id, status, current_step, created_at, completed_at \
                 FROM workflow_runs WHERE id = ?1",
                params![run_id],
            )
            .await?;
        row.as_ref().map(d1::row_to_workflow_run).transpose()
    }

    pub(crate) async fn d1_list_workflow_runs(
        &self,
        status: Option<&str>,
        limit: i32,
    ) -> Result<Vec<WorkflowRunRow>> {
        let rows = if let Some(status) = status {
            self.d1()
                .query(
                    "SELECT id, workflow, event_id, status, current_step, created_at, completed_at \
                     FROM workflow_runs WHERE status = ?1 ORDER BY created_at DESC LIMIT ?2",
                    params![status, limit],
                )
                .await?
        } else {
            self.d1()
                .query(
                    "SELECT id, workflow, event_id, status, current_step, created_at, completed_at \
                     FROM workflow_runs ORDER BY created_at DESC LIMIT ?1",
                    params![limit],
                )
                .await?
        };
        rows.iter().map(d1::row_to_workflow_run).collect()
    }

    pub(crate) async fn d1_list_workflow_runs_by_event(
        &self,
        event_id: &str,
    ) -> Result<Vec<WorkflowRunRow>> {
        let rows = self
            .d1()
            .query(
                "SELECT id, workflow, event_id, status, current_step, created_at, completed_at \
                 FROM workflow_runs WHERE event_id = ?1 ORDER BY created_at",
                params![event_id],
            )
            .await?;
        rows.iter().map(d1::row_to_workflow_run).collect()
    }

    pub(crate) async fn d1_redrive_workflow_run(&self, run_id: &str) -> Result<bool> {
        let result = self
            .d1()
            .execute(
                "UPDATE workflow_runs SET status = 'running' WHERE id = ?1 AND status = 'failed'",
                params![run_id],
            )
            .await?;
        Ok(result.rows_affected > 0)
    }

    pub(crate) async fn d1_set_parallel_state(
        &self,
        run_id: &str,
        parallel_step: &str,
        count: i32,
    ) -> Result<()> {
        self.d1()
            .execute(
                "UPDATE workflow_runs SET parallel_step = ?1, parallel_count = ?2, parallel_completed = 0 \
                 WHERE id = ?3",
                params![parallel_step, count, run_id],
            )
            .await?;
        Ok(())
    }

    /// D1: increment parallel completed. No RETURNING, use UPDATE + SELECT (like SQLite).
    pub(crate) async fn d1_increment_parallel_completed(
        &self,
        run_id: &str,
    ) -> Result<(i32, i32)> {
        self.d1()
            .execute(
                "UPDATE workflow_runs SET parallel_completed = parallel_completed + 1 WHERE id = ?1",
                params![run_id],
            )
            .await?;
        let row = self
            .d1()
            .query_one(
                "SELECT parallel_completed, parallel_count FROM workflow_runs WHERE id = ?1",
                params![run_id],
            )
            .await?;
        Ok((
            d1::row_get_i32(&row, "parallel_completed")?,
            d1::row_get_i32(&row, "parallel_count")?,
        ))
    }

    pub(crate) async fn d1_clear_parallel_state(&self, run_id: &str) -> Result<()> {
        self.d1()
            .execute(
                "UPDATE workflow_runs SET parallel_step = NULL, parallel_count = 0, parallel_completed = 0 \
                 WHERE id = ?1",
                params![run_id],
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn d1_insert_branch_job(
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
        self.d1()
            .execute(
                "INSERT INTO jobs (id, event_id, handler, url, status, max_attempts, scheduled_at, created_at, \
                 workflow_run_id, step_name, step_index, step_input, branch_name) \
                 VALUES (?1, ?2, ?3, ?4, 'available', ?5, ?6, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![id, event_id, handler, url, i32::try_from(max_attempts).unwrap_or(i32::MAX), now, workflow_run_id, step_name, step_index, step_input, branch_name],
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn d1_get_branch_outputs(
        &self,
        run_id: &str,
        step_name: &str,
    ) -> Result<Vec<(String, Option<String>)>> {
        let rows = self
            .d1()
            .query(
                "SELECT branch_name, step_output FROM jobs \
                 WHERE workflow_run_id = ?1 AND step_name = ?2 AND branch_name IS NOT NULL AND status = 'completed' \
                 ORDER BY branch_name",
                params![run_id, step_name],
            )
            .await?;
        rows.iter()
            .map(|r| {
                Ok((
                    d1::row_get_string(r, "branch_name")?,
                    d1::row_get_opt_string(r, "step_output"),
                ))
            })
            .collect()
    }

    pub(crate) async fn d1_insert_workflow_job_delayed(
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
        self.d1()
            .execute(
                "INSERT INTO jobs (id, event_id, handler, url, status, max_attempts, scheduled_at, created_at, \
                 workflow_run_id, step_name, step_index, step_input) \
                 VALUES (?1, ?2, ?3, ?4, 'available', ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![id, event_id, handler, url, i32::try_from(max_attempts).unwrap_or(i32::MAX), scheduled_at, now, workflow_run_id, step_name, step_index, step_input],
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn d1_insert_callback_job(
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
        let far_future = "9999-12-31T23:59:59.999";
        self.d1()
            .execute(
                "INSERT INTO jobs (id, event_id, handler, url, status, max_attempts, scheduled_at, created_at, \
                 workflow_run_id, step_name, step_index, step_input, callback_token) \
                 VALUES (?1, ?2, ?3, 'callback', 'waiting', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![id, event_id, handler, i32::try_from(max_attempts).unwrap_or(i32::MAX), far_future, now, workflow_run_id, step_name, step_index, step_input, callback_token],
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn d1_resume_callback_job(
        &self,
        token: &str,
        payload: &str,
    ) -> Result<Option<String>> {
        let now = format_now();
        let result = self
            .d1()
            .execute(
                "UPDATE jobs SET status = 'completed', completed_at = ?1, step_output = ?2 \
                 WHERE callback_token = ?3 AND status = 'waiting'",
                params![now, payload, token],
            )
            .await?;
        if result.rows_affected == 0 {
            return Ok(None);
        }
        let row = self
            .d1()
            .query_one(
                "SELECT id FROM jobs WHERE callback_token = ?1",
                params![token],
            )
            .await?;
        Ok(Some(d1::row_get_string(&row, "id")?))
    }

    pub(crate) async fn d1_get_callback_job(&self, token: &str) -> Result<Option<JobRow>> {
        let row = self
            .d1()
            .query_optional(
                "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                 FROM jobs WHERE callback_token = ?1",
                params![token],
            )
            .await?;
        row.as_ref().map(d1::row_to_job).transpose()
    }

    pub(crate) async fn d1_set_workflow_timeout(
        &self,
        run_id: &str,
        timeout_at: &str,
    ) -> Result<()> {
        self.d1()
            .execute(
                "UPDATE workflow_runs SET timeout_at = ?1 WHERE id = ?2",
                params![timeout_at, run_id],
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn d1_get_workflow_timeout(&self, run_id: &str) -> Result<Option<String>> {
        let row = self
            .d1()
            .query_optional(
                "SELECT timeout_at FROM workflow_runs WHERE id = ?1",
                params![run_id],
            )
            .await?;
        Ok(row.and_then(|r| d1::row_get_opt_string(&r, "timeout_at")))
    }

    pub(crate) async fn d1_expire_timed_out_callbacks(&self) -> Result<u64> {
        let now = format_now();
        let result = self
            .d1()
            .execute(
                "UPDATE jobs SET status = 'dead', completed_at = ?1, last_error = 'callback timeout' \
                 WHERE status = 'waiting' AND callback_token IS NOT NULL \
                 AND workflow_run_id IN ( \
                     SELECT id FROM workflow_runs WHERE timeout_at IS NOT NULL AND timeout_at <= ?1 AND status = 'running' \
                 )",
                params![now],
            )
            .await?;
        Ok(result.rows_affected)
    }

    pub(crate) async fn d1_insert_sub_workflow_run(
        &self,
        id: &str,
        workflow: &str,
        event_id: &str,
        first_step: &str,
        parent_run_id: &str,
        parent_step_index: i32,
    ) -> Result<()> {
        let now = format_now();
        self.d1()
            .execute(
                "INSERT INTO workflow_runs (id, workflow, event_id, status, current_step, created_at, parent_run_id, parent_step_index) \
                 VALUES (?1, ?2, ?3, 'running', ?4, ?5, ?6, ?7)",
                params![id, workflow, event_id, first_step, now, parent_run_id, parent_step_index],
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn d1_get_parent_workflow_run(
        &self,
        run_id: &str,
    ) -> Result<Option<(String, i32)>> {
        let row = self
            .d1()
            .query_optional(
                "SELECT parent_run_id, parent_step_index FROM workflow_runs WHERE id = ?1",
                params![run_id],
            )
            .await?;
        Ok(row.and_then(|r| {
            let id = d1::row_get_opt_string(&r, "parent_run_id")?;
            let idx = d1::row_get_opt_i32(&r, "parent_step_index")?;
            Some((id, idx))
        }))
    }

    // ── Queue (Pull-mode) ─────────────────────────────────────────────

    pub(crate) async fn d1_fetch_queue_messages(
        &self,
        handler: &str,
        limit: i32,
    ) -> Result<Vec<QueueMessageRow>> {
        let now = format_now();
        // Two-step like SQLite: SELECT then mark running
        let job_rows = self
            .d1()
            .query(
                "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                 FROM jobs \
                 WHERE handler = ?1 AND status IN ('available', 'retryable') AND scheduled_at <= ?2 \
                 ORDER BY scheduled_at ASC \
                 LIMIT ?3",
                params![handler, now, limit],
            )
            .await?;

        let mut msgs = Vec::with_capacity(job_rows.len());
        for job_row in &job_rows {
            let job = d1::row_to_job(job_row)?;
            if !self.d1_mark_job_running(&job.id).await? {
                continue;
            }
            let evt = self
                .d1()
                .query_one(
                    "SELECT event_type, payload, headers FROM events WHERE id = ?1",
                    params![job.event_id],
                )
                .await?;
            msgs.push(QueueMessageRow {
                id: job.id,
                event_id: job.event_id,
                event_type: d1::row_get_string(&evt, "event_type")?,
                payload: d1::row_get_string(&evt, "payload")?,
                headers: d1::row_get_opt_string(&evt, "headers"),
                attempt: job.attempt,
                created_at: job.created_at,
            });
        }
        Ok(msgs)
    }

    pub(crate) async fn d1_ack_queue_messages(
        &self,
        handler: &str,
        ids: &[String],
    ) -> Result<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let now = format_now();
        let mut total = 0u64;
        for id in ids {
            let result = self
                .d1()
                .execute(
                    "UPDATE jobs SET status = 'completed', completed_at = ?1 \
                     WHERE id = ?2 AND handler = ?3 AND status = 'running'",
                    params![now, id, handler],
                )
                .await?;
            if result.rows_affected > 0 {
                let attempt_id = ulid::Ulid::new().to_string();
                let _ = self
                    .d1_insert_attempt(&attempt_id, id, 1, Some(200), None, None, 0)
                    .await;
                total += 1;
            }
        }
        Ok(total)
    }

    pub(crate) async fn d1_nack_queue_messages(
        &self,
        handler: &str,
        ids: &[String],
    ) -> Result<(u64, u64)> {
        if ids.is_empty() {
            return Ok((0, 0));
        }
        let now = Utc::now().naive_utc();
        let now_str = format_dt(now);
        let mut retried = 0u64;
        let mut dead = 0u64;
        for id in ids {
            let row = self
                .d1()
                .query_optional(
                    "SELECT attempt, max_attempts FROM jobs WHERE id = ?1 AND handler = ?2 AND status = 'running'",
                    params![id, handler],
                )
                .await?;
            let Some(r) = row else { continue };
            let attempt = d1::row_get_i32(&r, "attempt")?;
            let max_attempts = d1::row_get_i32(&r, "max_attempts")?;

            if attempt < max_attempts {
                let backoff_secs = (30i64 * (1i64 << attempt.min(20))).min(3600);
                let next_at = format_dt(now + chrono::Duration::seconds(backoff_secs));
                self.d1()
                    .execute(
                        "UPDATE jobs SET status = 'retryable', scheduled_at = ?1, last_error = 'nack' \
                         WHERE id = ?2 AND status = 'running'",
                        params![next_at, id],
                    )
                    .await?;
                retried += 1;
            } else {
                self.d1()
                    .execute(
                        "UPDATE jobs SET status = 'dead', completed_at = ?1, last_error = 'nack: max attempts exceeded' \
                         WHERE id = ?2 AND status = 'running'",
                        params![now_str, id],
                    )
                    .await?;
                dead += 1;
            }
        }
        Ok((retried, dead))
    }

    pub(crate) async fn d1_recover_expired_queue_messages(
        &self,
        handler: &str,
        visibility_timeout_secs: u64,
    ) -> Result<u64> {
        let now = Utc::now().naive_utc();
        let cutoff = format_dt(now - chrono::Duration::seconds(i64::try_from(visibility_timeout_secs).unwrap_or(i64::MAX)));
        let now_str = format_dt(now);
        let result = self
            .d1()
            .execute(
                "UPDATE jobs SET status = 'retryable', scheduled_at = ?1 \
                 WHERE handler = ?2 AND status = 'running' AND started_at <= ?3",
                params![now_str, handler, cutoff],
            )
            .await?;
        Ok(result.rows_affected)
    }

    pub(crate) async fn d1_count_queue_depth(&self, handler: &str) -> Result<i64> {
        let now = format_now();
        let row = self
            .d1()
            .query_one(
                "SELECT COUNT(*) as cnt FROM jobs WHERE handler = ?1 AND status IN ('available', 'retryable') AND scheduled_at <= ?2",
                params![handler, now],
            )
            .await?;
        d1::row_get_i64(&row, "cnt")
    }

    // ── Outbound webhooks ─────────────────────────────────────────────

    pub(crate) async fn d1_insert_endpoint(
        &self,
        id: &str,
        source: &str,
        url: &str,
        description: Option<&str>,
        signing_secret: &str,
        status: &str,
    ) -> Result<()> {
        let now = format_now();
        self.d1()
            .execute(
                "INSERT INTO outbound_endpoints (id, source, url, description, signing_secret, status, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![id, source, url, description, signing_secret, status, now],
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn d1_get_endpoint(&self, id: &str) -> Result<Option<EndpointRow>> {
        let row = self
            .d1()
            .query_optional(
                "SELECT id, source, url, description, signing_secret, status, created_at, updated_at \
                 FROM outbound_endpoints WHERE id = ?1",
                params![id],
            )
            .await?;
        row.as_ref().map(d1::row_to_endpoint).transpose()
    }

    pub(crate) async fn d1_list_endpoints(
        &self,
        source: Option<&str>,
    ) -> Result<Vec<EndpointRow>> {
        let rows = if let Some(src) = source {
            self.d1()
                .query(
                    "SELECT id, source, url, description, signing_secret, status, created_at, updated_at \
                     FROM outbound_endpoints WHERE source = ?1 ORDER BY created_at",
                    params![src],
                )
                .await?
        } else {
            self.d1()
                .query(
                    "SELECT id, source, url, description, signing_secret, status, created_at, updated_at \
                     FROM outbound_endpoints ORDER BY created_at",
                    vec![],
                )
                .await?
        };
        rows.iter().map(d1::row_to_endpoint).collect()
    }

    pub(crate) async fn d1_update_endpoint(
        &self,
        id: &str,
        url: Option<&str>,
        description: Option<&str>,
        status: Option<&str>,
    ) -> Result<bool> {
        let now = format_now();
        let current = self.d1_get_endpoint(id).await?;
        let Some(current) = current else {
            return Ok(false);
        };
        let new_url = url.unwrap_or(&current.url);
        let new_desc = description.or(current.description.as_deref());
        let new_status = status.unwrap_or(&current.status);
        let result = self
            .d1()
            .execute(
                "UPDATE outbound_endpoints SET url = ?1, description = ?2, status = ?3, updated_at = ?4 WHERE id = ?5",
                params![new_url, new_desc, new_status, now, id],
            )
            .await?;
        Ok(result.rows_affected > 0)
    }

    pub(crate) async fn d1_delete_endpoint(&self, id: &str) -> Result<bool> {
        self.d1()
            .execute(
                "DELETE FROM outbound_subscriptions WHERE endpoint_id = ?1",
                params![id],
            )
            .await?;
        let result = self
            .d1()
            .execute(
                "DELETE FROM outbound_endpoints WHERE id = ?1",
                params![id],
            )
            .await?;
        Ok(result.rows_affected > 0)
    }

    pub(crate) async fn d1_rotate_endpoint_secret(
        &self,
        id: &str,
        new_secret: &str,
    ) -> Result<bool> {
        let now = format_now();
        let result = self
            .d1()
            .execute(
                "UPDATE outbound_endpoints SET signing_secret = ?1, updated_at = ?2 WHERE id = ?3",
                params![new_secret, now, id],
            )
            .await?;
        Ok(result.rows_affected > 0)
    }

    pub(crate) async fn d1_insert_subscriptions(
        &self,
        endpoint_id: &str,
        event_types: &[String],
    ) -> Result<Vec<SubscriptionRow>> {
        let now = format_now();
        let mut created = Vec::new();
        for event_type in event_types {
            let id = ulid::Ulid::new().to_string();
            let result = self
                .d1()
                .execute(
                    "INSERT INTO outbound_subscriptions (id, endpoint_id, event_type, created_at) \
                     VALUES (?1, ?2, ?3, ?4) \
                     ON CONFLICT (endpoint_id, event_type) DO NOTHING",
                    params![id, endpoint_id, event_type, now],
                )
                .await?;
            if result.rows_affected > 0 {
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

    pub(crate) async fn d1_list_subscriptions(
        &self,
        endpoint_id: &str,
    ) -> Result<Vec<SubscriptionRow>> {
        let rows = self
            .d1()
            .query(
                "SELECT id, endpoint_id, event_type, created_at \
                 FROM outbound_subscriptions WHERE endpoint_id = ?1 ORDER BY created_at",
                params![endpoint_id],
            )
            .await?;
        rows.iter().map(d1::row_to_subscription).collect()
    }

    pub(crate) async fn d1_delete_subscription(&self, id: &str) -> Result<bool> {
        let result = self
            .d1()
            .execute(
                "DELETE FROM outbound_subscriptions WHERE id = ?1",
                params![id],
            )
            .await?;
        Ok(result.rows_affected > 0)
    }

    pub(crate) async fn d1_find_subscribed_endpoints(
        &self,
        source: &str,
        event_type: &str,
    ) -> Result<Vec<SubscribedEndpoint>> {
        let rows: Vec<d1::D1Row> = self
            .d1()
            .query(
                "SELECT DISTINCT e.id, e.url, e.signing_secret \
                 FROM outbound_endpoints e \
                 JOIN outbound_subscriptions s ON s.endpoint_id = e.id \
                 WHERE e.source = ?1 AND e.status = 'active' \
                   AND (s.event_type = ?2 OR s.event_type = '*') \
                 ORDER BY e.id",
                params![source, event_type],
            )
            .await?;
        rows.iter().map(d1::row_to_subscribed_endpoint).collect()
    }

    // ── Queue CLI helpers ─────────────────────────────────────────────

    pub(crate) async fn d1_count_jobs_by_handler_status(
        &self,
        handler: &str,
    ) -> Result<Vec<(String, i64)>> {
        let rows = self
            .d1()
            .query(
                "SELECT status, COUNT(*) as cnt FROM jobs WHERE handler = ?1 GROUP BY status",
                params![handler],
            )
            .await?;
        rows.iter()
            .map(|r| Ok((d1::row_get_string(r, "status")?, d1::row_get_i64(r, "cnt")?)))
            .collect()
    }

    pub(crate) async fn d1_list_jobs_by_handler(
        &self,
        handler: &str,
        status: Option<&str>,
        limit: i32,
    ) -> Result<Vec<JobRow>> {
        let rows = if let Some(status) = status {
            self.d1()
                .query(
                    "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                     FROM jobs WHERE handler = ?1 AND status = ?2 ORDER BY scheduled_at DESC LIMIT ?3",
                    params![handler, status, limit],
                )
                .await?
        } else {
            self.d1()
                .query(
                    "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                     FROM jobs WHERE handler = ?1 ORDER BY scheduled_at DESC LIMIT ?2",
                    params![handler, limit],
                )
                .await?
        };
        rows.iter().map(d1::row_to_job).collect()
    }

    /// D1: delete jobs by handler. No transaction support — execute sequentially.
    pub(crate) async fn d1_delete_jobs_by_handler(&self, handler: &str) -> Result<u64> {
        // D1 does not support transactions, so we run these sequentially.
        // If the second query fails, we may have orphaned job_attempts.
        self.d1()
            .execute(
                "DELETE FROM job_attempts WHERE job_id IN (SELECT id FROM jobs WHERE handler = ?1)",
                params![handler],
            )
            .await?;
        let result = self
            .d1()
            .execute("DELETE FROM jobs WHERE handler = ?1", params![handler])
            .await?;
        Ok(result.rows_affected)
    }

    pub(crate) async fn d1_retry_dead_jobs_by_handler(&self, handler: &str) -> Result<u64> {
        let now = format_now();
        let result = self
            .d1()
            .execute(
                "UPDATE jobs SET status = 'available', scheduled_at = ?1, last_error = NULL, attempt = 0 \
                 WHERE handler = ?2 AND status = 'dead'",
                params![now, handler],
            )
            .await?;
        Ok(result.rows_affected)
    }

    pub(crate) async fn d1_peek_queue_job(&self, handler: &str) -> Result<Option<JobRow>> {
        let row = self
            .d1()
            .query_optional(
                "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error, created_at \
                 FROM jobs WHERE handler = ?1 AND status IN ('available', 'retryable') \
                 ORDER BY scheduled_at ASC LIMIT 1",
                params![handler],
            )
            .await?;
        row.as_ref().map(d1::row_to_job).transpose()
    }

    pub(crate) async fn d1_get_endpoint_secret(
        &self,
        endpoint_id: &str,
    ) -> Result<Option<String>> {
        let row = self
            .d1()
            .query_optional(
                "SELECT signing_secret FROM outbound_endpoints WHERE id = ?1",
                params![endpoint_id],
            )
            .await?;
        Ok(row.and_then(|r| d1::row_get_opt_string(&r, "signing_secret")))
    }

    // --- Status command queries ---

    pub(crate) async fn d1_count_events_by_source(&self) -> Result<Vec<(String, i64)>> {
        let rows = self
            .d1()
            .query(
                "SELECT source, COUNT(*) as cnt FROM events GROUP BY source ORDER BY cnt DESC",
                vec![],
            )
            .await?;
        rows.iter()
            .map(|r| Ok((d1::row_get_string(r, "source")?, d1::row_get_i64(r, "cnt")?)))
            .collect()
    }

    pub(crate) async fn d1_count_jobs_by_status(&self) -> Result<Vec<(String, i64)>> {
        let rows = self
            .d1()
            .query(
                "SELECT status, COUNT(*) as cnt FROM jobs GROUP BY status ORDER BY cnt DESC",
                vec![],
            )
            .await?;
        rows.iter()
            .map(|r| Ok((d1::row_get_string(r, "status")?, d1::row_get_i64(r, "cnt")?)))
            .collect()
    }

    pub(crate) async fn d1_count_handler_stats(&self) -> Result<Vec<(String, i64, i64, i64)>> {
        let rows = self
            .d1()
            .query(
                "SELECT handler, \
                 SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) as ok, \
                 SUM(CASE WHEN status IN ('available', 'retryable', 'running') THEN 1 ELSE 0 END) as fail, \
                 SUM(CASE WHEN status = 'dead' THEN 1 ELSE 0 END) as dead \
                 FROM jobs GROUP BY handler ORDER BY ok DESC",
                vec![],
            )
            .await?;
        rows.iter()
            .map(|r| {
                Ok((
                    d1::row_get_string(r, "handler")?,
                    d1::row_get_i64(r, "ok")?,
                    d1::row_get_i64(r, "fail")?,
                    d1::row_get_i64(r, "dead")?,
                ))
            })
            .collect()
    }

    pub(crate) async fn d1_count_workflow_stats(&self) -> Result<Vec<(String, i64, i64, i64)>> {
        let rows = self
            .d1()
            .query(
                "SELECT workflow, \
                 SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) as completed, \
                 SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END) as running, \
                 SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END) as failed \
                 FROM workflow_runs GROUP BY workflow ORDER BY completed DESC",
                vec![],
            )
            .await?;
        rows.iter()
            .map(|r| {
                Ok((
                    d1::row_get_string(r, "workflow")?,
                    d1::row_get_i64(r, "completed")?,
                    d1::row_get_i64(r, "running")?,
                    d1::row_get_i64(r, "failed")?,
                ))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rewrite_params_basic() {
        assert_eq!(rewrite_params("SELECT * FROM t WHERE id = $1"), "SELECT * FROM t WHERE id = ?1");
    }

    #[test]
    fn test_rewrite_params_multiple() {
        assert_eq!(
            rewrite_params("INSERT INTO t (a, b) VALUES ($1, $2)"),
            "INSERT INTO t (a, b) VALUES (?1, ?2)"
        );
    }

    #[test]
    fn test_rewrite_params_no_params() {
        assert_eq!(rewrite_params("SELECT 1"), "SELECT 1");
    }

    #[test]
    fn test_rewrite_params_double_digit() {
        assert_eq!(
            rewrite_params("SELECT $10, $11"),
            "SELECT ?10, ?11"
        );
    }

    #[test]
    fn test_rewrite_params_preserves_dollar_non_digit() {
        assert_eq!(rewrite_params("SELECT $abc"), "SELECT $abc");
    }
}
