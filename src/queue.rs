use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::interval;

use crate::alert::{AlertEvent, SharedAlerter};
use crate::config::WorkerConfig;
use crate::db::Database;
use crate::metrics::Metrics;

/// Max concurrent deliveries per worker.
const MAX_CONCURRENCY: usize = 10;
/// Batch size for job fetching.
const BATCH_SIZE: i32 = 10;
/// How often to run maintenance (stale recovery + cleanup).
const MAINTENANCE_INTERVAL_SECS: u64 = 3600;

pub struct Worker {
    db: Arc<Database>,
    metrics: Arc<Metrics>,
    alerter: SharedAlerter,
    worker_config: WorkerConfig,
    http: reqwest::Client,
    poll_interval: Duration,
    shutdown: tokio::sync::watch::Receiver<bool>,
    /// Per-handler rate limiters (handler_name -> semaphore with N permits = N/sec).
    rate_limiters: HashMap<String, Arc<Semaphore>>,
    /// Per-handler payload transform templates.
    transforms: Arc<HashMap<String, String>>,
    /// Per-handler type overrides (only non-"http" entries).
    handler_types: Arc<HashMap<String, String>>,
    /// Lazily created gRPC channels keyed by URL.
    grpc_channels: Arc<std::sync::Mutex<HashMap<String, tonic::transport::Channel>>>,
}

impl Worker {
    pub fn new(
        db: Arc<Database>,
        metrics: Arc<Metrics>,
        alerter: SharedAlerter,
        worker_config: WorkerConfig,
        shutdown: tokio::sync::watch::Receiver<bool>,
        handler_rate_limits: HashMap<String, u32>,
        handler_transforms: HashMap<String, String>,
        handler_types: HashMap<String, String>,
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
            alerter,
            worker_config,
            http,
            poll_interval: Duration::from_secs(1),
            shutdown,
            rate_limiters,
            transforms: Arc::new(handler_transforms),
            handler_types: Arc::new(handler_types),
            grpc_channels: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    pub async fn run(mut self) {
        tracing::info!("Queue worker started");

        // Recover stale jobs on startup
        run_maintenance(
            &self.db,
            &self.metrics,
            self.worker_config.stale_threshold_secs,
            self.worker_config.retention_hours,
        )
        .await;

        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENCY));
        let mut in_flight = JoinSet::new();
        let mut poll_ticker = interval(self.poll_interval);
        let busy_interval = Duration::from_millis(50);
        let mut maint_ticker = interval(Duration::from_secs(MAINTENANCE_INTERVAL_SECS));
        maint_ticker.tick().await; // skip immediate first tick
        let mut consecutive_db_errors: u32 = 0;

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
                        Ok(j) => {
                            consecutive_db_errors = 0;
                            j
                        }
                        Err(e) => {
                            consecutive_db_errors += 1;
                            self.metrics.inc_db_errors();
                            let backoff = Duration::from_secs(
                                (1u64 << consecutive_db_errors.min(5)).min(30)
                            );
                            tracing::error!(
                                error = %e,
                                backoff_secs = backoff.as_secs(),
                                "Failed to fetch jobs, backing off"
                            );
                            poll_ticker.reset_after(backoff);
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
                                    self.metrics.inc_db_errors();
                                    tracing::error!(job_id = job.id, error = %e, "Failed to lock job");
                                    drop(permit);
                                    continue;
                                }
                            }
                        }

                        let db = self.db.clone();
                        let http = self.http.clone();
                        let metrics = self.metrics.clone();
                        let alerter = self.alerter.clone();
                        let transforms = self.transforms.clone();
                        let handler_types = self.handler_types.clone();
                        let grpc_channels = self.grpc_channels.clone();
                        let rate_sem = self.rate_limiters.get(&job.handler).cloned();

                        in_flight.spawn(async move {
                            // Acquire rate limit permit (blocks if at limit)
                            let rate_permit = if let Some(ref sem) = rate_sem {
                                sem.clone().acquire_owned().await.ok()
                            } else {
                                None
                            };

                            deliver_job(&db, &http, &metrics, &alerter, &transforms, &handler_types, &grpc_channels, &job).await;
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
                    run_maintenance(&self.db, &self.metrics, self.worker_config.stale_threshold_secs, self.worker_config.retention_hours).await;
                }
            }
        }

        // Drain in-flight deliveries with timeout
        let remaining = in_flight.len();
        if remaining > 0 {
            let timeout = Duration::from_secs(self.worker_config.drain_timeout_secs);
            tracing::info!(
                count = remaining,
                timeout_secs = timeout.as_secs(),
                "Draining in-flight deliveries"
            );
            let drain = async {
                while let Some(result) = in_flight.join_next().await {
                    if let Err(e) = result {
                        tracing::error!(error = %e, "Delivery task panicked during shutdown");
                    }
                }
            };
            if tokio::time::timeout(timeout, drain).await.is_err() {
                let abandoned = in_flight.len();
                tracing::warn!(
                    count = abandoned,
                    "Drain timeout reached, abandoning remaining deliveries"
                );
            }
        }
        tracing::info!("Worker stopped");
    }
}

async fn run_maintenance(db: &Database, metrics: &Metrics, stale_secs: i64, retention_hours: i64) {
    match db.recover_stale_jobs(stale_secs).await {
        Ok(0) => {}
        Ok(n) => tracing::info!(count = n, "Recovered stale running jobs"),
        Err(e) => {
            metrics.inc_db_errors();
            tracing::error!(error = %e, "Failed to recover stale jobs");
        }
    }
    match db.cleanup_old_records(retention_hours).await {
        Ok((0, 0)) => {}
        Ok((jobs, attempts)) => tracing::info!(jobs, attempts, "Cleaned up old records"),
        Err(e) => {
            metrics.inc_db_errors();
            tracing::error!(error = %e, "Failed to cleanup old records");
        }
    }
}

async fn deliver_job(
    db: &Database,
    http: &reqwest::Client,
    metrics: &Metrics,
    alerter: &SharedAlerter,
    transforms: &HashMap<String, String>,
    handler_types: &HashMap<String, String>,
    grpc_channels: &std::sync::Mutex<HashMap<String, tonic::transport::Channel>>,
    job: &crate::db::JobRow,
) {
    let start = std::time::Instant::now();
    let transform = transforms.get(&job.handler);
    let is_grpc = handler_types.get(&job.handler).is_some_and(|t| t == "grpc");
    let result = deliver(
        db,
        http,
        grpc_channels,
        job,
        transform.map(|s| s.as_str()),
        is_grpc,
    )
    .await;
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
                metrics.inc_db_errors();
                tracing::error!(job_id = job.id, error = %e, "Failed to insert attempt");
                return;
            }

            if (200..300).contains(&status_code) {
                metrics.inc_delivery_success_for(&job.handler, duration_ms as u64);
                if let Err(e) = db.mark_job_completed(&job.id).await {
                    metrics.inc_db_errors();
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
                metrics.inc_delivery_failure_for(&job.handler, duration_ms as u64);
                let error_type = if (400..500).contains(&status_code) {
                    "4xx"
                } else {
                    "5xx"
                };
                metrics.inc_delivery_error_type(error_type);
                let error = format!("HTTP {status_code}");
                match handle_failure(db, job, &error).await {
                    Ok(true) => {
                        metrics.inc_dlq(&job.handler);
                        if let Some(a) = alerter {
                            a.send(AlertEvent::Dlq {
                                job_id: job.id.clone(),
                                handler: job.handler.clone(),
                                attempts: job.attempt + 1,
                            });
                        }
                    }
                    Ok(false) => {}
                    Err(e) => {
                        tracing::error!(job_id = job.id, error = %e, "Failed to handle failure")
                    }
                }
            }
        }
        Err(e) => {
            let error = e.to_string();
            let attempt_id = ulid::Ulid::new().to_string();
            metrics.inc_delivery_failure_for(&job.handler, duration_ms as u64);
            let error_type = if e
                .downcast_ref::<reqwest::Error>()
                .is_some_and(|re| re.is_timeout())
            {
                "timeout"
            } else {
                "network"
            };
            metrics.inc_delivery_error_type(error_type);
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
            match handle_failure(db, job, &error).await {
                Ok(true) => metrics.inc_dlq(&job.handler),
                Ok(false) => {}
                Err(e) => tracing::error!(job_id = job.id, error = %e, "Failed to handle failure"),
            }
        }
    }
}

async fn deliver(
    db: &Database,
    http: &reqwest::Client,
    grpc_channels: &std::sync::Mutex<HashMap<String, tonic::transport::Channel>>,
    job: &crate::db::JobRow,
    transform: Option<&str>,
    is_grpc: bool,
) -> Result<u16> {
    let (raw_payload, headers_json) = db.get_event_data(&job.event_id).await?;

    // Apply transformation if configured
    let payload = match transform {
        Some(template) => crate::api::apply_transform(&raw_payload, template),
        None => raw_payload,
    };

    if is_grpc {
        deliver_grpc(grpc_channels, job, &payload, &headers_json).await
    } else {
        deliver_http(http, job, &payload, &headers_json).await
    }
}

async fn deliver_http(
    http: &reqwest::Client,
    job: &crate::db::JobRow,
    payload: &str,
    headers_json: &Option<String>,
) -> Result<u16> {
    let mut request = http
        .post(&job.url)
        .header("Content-Type", "application/json")
        .header("X-Qhook-Job-ID", &job.id)
        .header("X-Qhook-Event-ID", &job.event_id)
        .header("X-Qhook-Handler", &job.handler)
        .header("X-Qhook-Attempt", (job.attempt + 1).to_string());

    // Forward CloudEvents headers from the original event
    if let Some(hj) = headers_json
        && let Ok(headers) = serde_json::from_str::<std::collections::HashMap<String, String>>(hj)
    {
        for (key, value) in &headers {
            if key.starts_with("ce-") {
                request = request.header(key.as_str(), value.as_str());
            }
        }
    }

    let response = request.body(payload.to_string()).send().await?;
    Ok(response.status().as_u16())
}

async fn deliver_grpc(
    grpc_channels: &std::sync::Mutex<HashMap<String, tonic::transport::Channel>>,
    job: &crate::db::JobRow,
    payload: &str,
    headers_json: &Option<String>,
) -> Result<u16> {
    // Get or create channel for this URL
    let channel = {
        let mut channels = grpc_channels.lock().unwrap_or_else(|e| e.into_inner());
        match channels.get(&job.url) {
            Some(ch) => ch.clone(),
            None => {
                let ch = crate::grpc::create_channel(&job.url)?;
                channels.insert(job.url.clone(), ch.clone());
                ch
            }
        }
    };

    // Build metadata from CloudEvents headers
    let mut metadata = HashMap::new();
    metadata.insert("job_id".to_string(), job.id.clone());
    if let Some(hj) = headers_json
        && let Ok(headers) = serde_json::from_str::<HashMap<String, String>>(hj)
    {
        for (key, value) in headers {
            if key.starts_with("ce-") {
                metadata.insert(key, value);
            }
        }
    }

    // Get event_type from the event
    let event_type = metadata
        .get("ce-type")
        .cloned()
        .unwrap_or_else(|| "event".to_string());

    let request = crate::grpc::DeliverRequest {
        event_id: job.event_id.clone(),
        event_type,
        handler: job.handler.clone(),
        payload: payload.to_string(),
        metadata,
        attempt: job.attempt + 1,
    };

    match crate::grpc::deliver(&channel, request).await {
        Ok(response) => {
            if response.success {
                Ok(200)
            } else {
                tracing::warn!(
                    job_id = job.id,
                    handler = job.handler,
                    message = response.message,
                    "gRPC handler returned failure"
                );
                Ok(500) // Treat as server error for retry logic
            }
        }
        Err(e) => Err(e),
    }
}

/// Handle delivery failure. Returns `true` if the job was moved to DLQ.
async fn handle_failure(db: &Database, job: &crate::db::JobRow, error: &str) -> Result<bool> {
    let current_attempt = job.attempt + 1;

    if current_attempt >= job.max_attempts {
        db.mark_job_dead(&job.id, error).await?;
        tracing::warn!(
            job_id = job.id,
            handler = job.handler,
            attempts = current_attempt,
            "Job moved to DLQ"
        );
        Ok(true)
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
        Ok(false)
    }
}
