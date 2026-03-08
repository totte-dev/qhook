use anyhow::{Context, Result};
use chrono::{NaiveDateTime, Utc};
use sqlx::{AnyPool, any::AnyPoolOptions};
use std::time::Instant;

use crate::config::DatabaseConfig;

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
            other => anyhow::bail!("Unsupported database driver: {other}"),
        };

        sqlx::any::install_default_drivers();

        let pool = AnyPoolOptions::new()
            .max_connections(config.max_connections)
            .connect(&url)
            .await
            .with_context(|| format!("Failed to connect to database: {url}"))?;

        tracing::info!(driver = config.driver, "Database connected");

        Ok(Self {
            pool,
            driver: config.driver.clone(),
        })
    }

    pub async fn migrate(&self) -> Result<()> {
        // Create tables if they don't exist
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS events (
                id          TEXT PRIMARY KEY,
                source      TEXT NOT NULL,
                event_type  TEXT NOT NULL,
                payload     TEXT NOT NULL,
                headers     TEXT,
                unique_key  TEXT,
                created_at  TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Unique constraint on (source, unique_key) where unique_key is not null
        // SQLite and Postgres both support CREATE UNIQUE INDEX IF NOT EXISTS
        sqlx::query(
            r#"
            CREATE UNIQUE INDEX IF NOT EXISTS idx_events_unique
            ON events (source, unique_key)
            "#,
        )
        .execute(&self.pool)
        .await
        .ok(); // Ignore if already exists with different definition

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS jobs (
                id           TEXT PRIMARY KEY,
                event_id     TEXT NOT NULL,
                handler      TEXT NOT NULL,
                url          TEXT NOT NULL,
                status       TEXT NOT NULL DEFAULT 'available',
                attempt      INTEGER NOT NULL DEFAULT 0,
                max_attempts INTEGER NOT NULL DEFAULT 5,
                scheduled_at TEXT NOT NULL,
                started_at   TEXT,
                completed_at TEXT,
                created_at   TEXT NOT NULL,
                last_error   TEXT
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_jobs_fetch
            ON jobs (status, scheduled_at)
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS job_attempts (
                id            TEXT PRIMARY KEY,
                job_id        TEXT NOT NULL,
                attempt       INTEGER NOT NULL,
                status_code   INTEGER,
                response_body TEXT,
                error         TEXT,
                duration_ms   INTEGER,
                created_at    TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        tracing::info!("Database migrated");
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
        let now = Utc::now()
            .naive_utc()
            .format("%Y-%m-%dT%H:%M:%S%.3f")
            .to_string();

        // Try insert; if unique_key conflicts, return false (duplicate)
        if unique_key.is_some() {
            let result = sqlx::query(
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
            .await?;

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
        let now = Utc::now()
            .naive_utc()
            .format("%Y-%m-%dT%H:%M:%S%.3f")
            .to_string();

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
    /// For SQLite: plain SELECT (use mark_job_running separately).
    pub async fn fetch_available_jobs(&self, limit: i32) -> Result<Vec<JobRow>> {
        let now = Utc::now()
            .naive_utc()
            .format("%Y-%m-%dT%H:%M:%S%.3f")
            .to_string();

        let start = Instant::now();
        let rows = if self.driver == "postgres" {
            sqlx::query_as::<_, JobRow>(
                "UPDATE jobs SET status = 'running', started_at = $1, attempt = attempt + 1 \
                 WHERE id IN ( \
                     SELECT id FROM jobs \
                     WHERE status IN ('available', 'retryable') AND scheduled_at <= $1 \
                     ORDER BY scheduled_at ASC \
                     LIMIT $2 \
                     FOR UPDATE SKIP LOCKED \
                 ) \
                 RETURNING id, event_id, handler, url, status, attempt - 1 AS attempt, max_attempts, scheduled_at, last_error",
            )
            .bind(&now)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, JobRow>(
                "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error \
                 FROM jobs \
                 WHERE status IN ('available', 'retryable') AND scheduled_at <= $1 \
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
            tracing::warn!(query = "fetch_available_jobs", duration_ms = elapsed, rows = rows.len(), "Slow query");
        }

        Ok(rows)
    }

    /// Mark a job as running (SQLite only — Postgres does this in fetch_available_jobs).
    pub async fn mark_job_running(&self, job_id: &str) -> Result<bool> {
        let now = Utc::now()
            .naive_utc()
            .format("%Y-%m-%dT%H:%M:%S%.3f")
            .to_string();

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
        let now = Utc::now()
            .naive_utc()
            .format("%Y-%m-%dT%H:%M:%S%.3f")
            .to_string();

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
        let scheduled = next_attempt_at.format("%Y-%m-%dT%H:%M:%S%.3f").to_string();

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
        let now = Utc::now()
            .naive_utc()
            .format("%Y-%m-%dT%H:%M:%S%.3f")
            .to_string();

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
        let now = Utc::now()
            .naive_utc()
            .format("%Y-%m-%dT%H:%M:%S%.3f")
            .to_string();

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
                "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error \
                 FROM jobs WHERE status = $1 ORDER BY scheduled_at DESC LIMIT $2",
            )
            .bind(status)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, JobRow>(
                "SELECT id, event_id, handler, url, status, attempt, max_attempts, scheduled_at, last_error \
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

    pub async fn retry_dead_jobs(&self) -> Result<u64> {
        let now = Utc::now()
            .naive_utc()
            .format("%Y-%m-%dT%H:%M:%S%.3f")
            .to_string();
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
        let cutoff = (now - chrono::Duration::seconds(stale_secs))
            .format("%Y-%m-%dT%H:%M:%S%.3f")
            .to_string();
        let now_str = now.format("%Y-%m-%dT%H:%M:%S%.3f").to_string();

        let start = Instant::now();
        let result = sqlx::query(
            "UPDATE jobs SET status = 'retryable', scheduled_at = $1 \
             WHERE status = 'running' AND started_at <= $2",
        )
        .bind(&now_str)
        .bind(&cutoff)
        .execute(&self.pool)
        .await?;
        let elapsed = start.elapsed().as_millis();
        if elapsed > SLOW_QUERY_MS {
            tracing::warn!(query = "recover_stale_jobs", duration_ms = elapsed, "Slow query");
        }

        Ok(result.rows_affected())
    }

    /// Delete completed/dead jobs (and their attempts) older than `retention_hours`.
    pub async fn cleanup_old_records(&self, retention_hours: i64) -> Result<(u64, u64)> {
        let cutoff = (Utc::now().naive_utc() - chrono::Duration::hours(retention_hours))
            .format("%Y-%m-%dT%H:%M:%S%.3f")
            .to_string();

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
            tracing::warn!(query = "cleanup_old_records", duration_ms = elapsed, "Slow query");
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

    pub async fn retry_job(&self, job_id: &str) -> Result<bool> {
        let now = Utc::now()
            .naive_utc()
            .format("%Y-%m-%dT%H:%M:%S%.3f")
            .to_string();
        let result = sqlx::query(
            "UPDATE jobs SET status = 'available', scheduled_at = $1, last_error = NULL \
             WHERE id = $2 AND status IN ('dead', 'retryable')",
        )
        .bind(&now)
        .bind(job_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
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
}

#[derive(Debug, sqlx::FromRow)]
pub struct EventRow {
    pub id: String,
    pub source: String,
    pub event_type: String,
    pub unique_key: Option<String>,
    pub created_at: String,
}
