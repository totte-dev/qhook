use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::interval;

use crate::db::Database;
use crate::metrics::Metrics;

/// Max concurrent deliveries per worker.
const MAX_CONCURRENCY: usize = 10;
/// Batch size for job fetching.
const BATCH_SIZE: i32 = 10;
/// Jobs stuck in 'running' longer than this are recovered (seconds).
const STALE_THRESHOLD_SECS: i64 = 300;
/// Completed/dead records older than this are purged (hours).
const RETENTION_HOURS: i64 = 72;
/// How often to run maintenance (stale recovery + cleanup).
const MAINTENANCE_INTERVAL_SECS: u64 = 3600;

pub struct Worker {
    db: Arc<Database>,
    metrics: Arc<Metrics>,
    http: reqwest::Client,
    poll_interval: Duration,
    shutdown: tokio::sync::watch::Receiver<bool>,
    /// Per-handler rate limiters (handler_name -> semaphore with N permits = N/sec).
    rate_limiters: HashMap<String, Arc<Semaphore>>,
}

impl Worker {
    pub fn new(
        db: Arc<Database>,
        metrics: Arc<Metrics>,
        shutdown: tokio::sync::watch::Receiver<bool>,
        handler_rate_limits: HashMap<String, u32>,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        let rate_limiters: HashMap<String, Arc<Semaphore>> = handler_rate_limits
            .into_iter()
            .map(|(name, rate)| (name, Arc::new(Semaphore::new(rate as usize))))
            .collect();

        Self {
            db,
            metrics,
            http,
            poll_interval: Duration::from_secs(1),
            shutdown,
            rate_limiters,
        }
    }

    pub async fn run(mut self) {
        tracing::info!("Queue worker started");

        // Recover stale jobs on startup
        run_maintenance(&self.db).await;

        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENCY));
        let mut in_flight = JoinSet::new();
        let mut poll_ticker = interval(self.poll_interval);
        let busy_interval = Duration::from_millis(50);
        let mut maint_ticker = interval(Duration::from_secs(MAINTENANCE_INTERVAL_SECS));
        maint_ticker.tick().await; // skip immediate first tick

        loop {
            tokio::select! {
                _ = self.shutdown.changed() => {
                    if *self.shutdown.borrow() {
                        tracing::info!("Worker received shutdown signal");
                        break;
                    }
                }
                _ = poll_ticker.tick() => {
                    let is_postgres = self.db.driver == "postgres";
                    let batch = BATCH_SIZE.min(
                        semaphore.available_permits() as i32
                    ).max(1);
                    let jobs = match self.db.fetch_available_jobs(batch).await {
                        Ok(j) => j,
                        Err(e) => {
                            tracing::error!(error = %e, "Failed to fetch jobs");
                            continue;
                        }
                    };

                    let found_jobs = !jobs.is_empty();
                    for job in jobs {
                        let permit = match semaphore.clone().try_acquire_owned() {
                            Ok(p) => p,
                            Err(_) => break, // at capacity
                        };

                        // Postgres already locked jobs in fetch_available_jobs
                        if !is_postgres {
                            match self.db.mark_job_running(&job.id).await {
                                Ok(true) => {}
                                Ok(false) => { drop(permit); continue; }
                                Err(e) => {
                                    tracing::error!(job_id = job.id, error = %e, "Failed to lock job");
                                    drop(permit);
                                    continue;
                                }
                            }
                        }

                        let db = self.db.clone();
                        let http = self.http.clone();
                        let metrics = self.metrics.clone();
                        let rate_sem = self.rate_limiters.get(&job.handler).cloned();

                        in_flight.spawn(async move {
                            // Acquire rate limit permit (blocks if at limit)
                            let rate_permit = if let Some(ref sem) = rate_sem {
                                sem.clone().acquire_owned().await.ok()
                            } else {
                                None
                            };

                            deliver_job(&db, &http, &metrics, &job).await;
                            drop(permit);

                            // Hold rate permit for 1s to enforce per-second limit
                            if let Some(rp) = rate_permit {
                                tokio::spawn(async move {
                                    tokio::time::sleep(Duration::from_secs(1)).await;
                                    drop(rp);
                                });
                            }
                        });
                    }

                    // If we found jobs, poll again quickly instead of waiting the full interval
                    if found_jobs {
                        poll_ticker.reset_after(busy_interval);
                    }
                }
                Some(result) = in_flight.join_next() => {
                    if let Err(e) = result {
                        tracing::error!(error = %e, "Delivery task panicked");
                    }
                }
                _ = maint_ticker.tick() => {
                    run_maintenance(&self.db).await;
                }
            }
        }

        // Drain in-flight deliveries before stopping
        let remaining = in_flight.len();
        if remaining > 0 {
            tracing::info!(count = remaining, "Draining in-flight deliveries");
        }
        while let Some(result) = in_flight.join_next().await {
            if let Err(e) = result {
                tracing::error!(error = %e, "Delivery task panicked during shutdown");
            }
        }
        tracing::info!("Worker stopped");
    }
}

async fn run_maintenance(db: &Database) {
    match db.recover_stale_jobs(STALE_THRESHOLD_SECS).await {
        Ok(0) => {}
        Ok(n) => tracing::info!(count = n, "Recovered stale running jobs"),
        Err(e) => tracing::error!(error = %e, "Failed to recover stale jobs"),
    }
    match db.cleanup_old_records(RETENTION_HOURS).await {
        Ok((0, 0)) => {}
        Ok((jobs, attempts)) => tracing::info!(jobs, attempts, "Cleaned up old records"),
        Err(e) => tracing::error!(error = %e, "Failed to cleanup old records"),
    }
}

async fn deliver_job(
    db: &Database,
    http: &reqwest::Client,
    metrics: &Metrics,
    job: &crate::db::JobRow,
) {
    let start = std::time::Instant::now();
    let result = deliver(db, http, job).await;
    let duration_ms = start.elapsed().as_millis() as i64;

    match result {
        Ok(status_code) => {
            let attempt_id = ulid::Ulid::new().to_string();
            if let Err(e) = db
                .insert_attempt(
                    &attempt_id,
                    &job.id,
                    job.attempt + 1,
                    Some(status_code as i32),
                    None,
                    None,
                    duration_ms,
                )
                .await
            {
                tracing::error!(job_id = job.id, error = %e, "Failed to insert attempt");
                return;
            }

            if (200..300).contains(&status_code) {
                metrics.inc_delivery_success(duration_ms as u64);
                if let Err(e) = db.mark_job_completed(&job.id).await {
                    tracing::error!(job_id = job.id, error = %e, "Failed to mark completed");
                }
                tracing::info!(
                    job_id = job.id,
                    handler = job.handler,
                    status = status_code,
                    duration_ms,
                    "Job completed"
                );
            } else {
                metrics.inc_delivery_failure(duration_ms as u64);
                let error = format!("HTTP {status_code}");
                if let Err(e) = handle_failure(db, job, &error).await {
                    tracing::error!(job_id = job.id, error = %e, "Failed to handle failure");
                }
            }
        }
        Err(e) => {
            let error = e.to_string();
            let attempt_id = ulid::Ulid::new().to_string();
            metrics.inc_delivery_failure(duration_ms as u64);
            let _ = db
                .insert_attempt(
                    &attempt_id,
                    &job.id,
                    job.attempt + 1,
                    None,
                    None,
                    Some(&error),
                    duration_ms,
                )
                .await;
            if let Err(e) = handle_failure(db, job, &error).await {
                tracing::error!(job_id = job.id, error = %e, "Failed to handle failure");
            }
        }
    }
}

async fn deliver(db: &Database, http: &reqwest::Client, job: &crate::db::JobRow) -> Result<u16> {
    let payload = db.get_event_payload(&job.event_id).await?;
    let headers_json = db.get_event_headers(&job.event_id).await?;

    let mut request = http
        .post(&job.url)
        .header("Content-Type", "application/json")
        .header("X-Qhook-Job-ID", &job.id)
        .header("X-Qhook-Event-ID", &job.event_id)
        .header("X-Qhook-Handler", &job.handler)
        .header("X-Qhook-Attempt", (job.attempt + 1).to_string());

    // Forward CloudEvents headers from the original event
    if let Some(ref hj) = headers_json
        && let Ok(headers) = serde_json::from_str::<std::collections::HashMap<String, String>>(hj)
    {
        for (key, value) in &headers {
            if key.starts_with("ce-") {
                request = request.header(key.as_str(), value.as_str());
            }
        }
    }

    let response = request.body(payload).send().await?;
    Ok(response.status().as_u16())
}

async fn handle_failure(db: &Database, job: &crate::db::JobRow, error: &str) -> Result<()> {
    let current_attempt = job.attempt + 1;

    if current_attempt >= job.max_attempts {
        db.mark_job_dead(&job.id, error).await?;
        tracing::warn!(
            job_id = job.id,
            handler = job.handler,
            attempts = current_attempt,
            "Job moved to DLQ"
        );
    } else {
        // Exponential backoff: 30s * 2^attempt
        let backoff_secs = 30i64 * (1i64 << current_attempt.min(10));
        let next_at = Utc::now().naive_utc() + chrono::Duration::seconds(backoff_secs);
        db.mark_job_retryable(&job.id, next_at, error).await?;
        tracing::info!(
            job_id = job.id,
            handler = job.handler,
            attempt = current_attempt,
            next_retry_secs = backoff_secs,
            error,
            "Job scheduled for retry"
        );
    }

    Ok(())
}
