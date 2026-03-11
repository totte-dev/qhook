use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::interval;

use crate::alert::{AlertEvent, SharedAlerter};
use crate::config::{self, WorkerConfig};
use crate::db::Database;
use crate::metrics::Metrics;

/// Default circuit breaker threshold (consecutive failures to open).
const DEFAULT_CB_THRESHOLD: u32 = 5;
/// Default circuit breaker cooldown (seconds before half-open).
const DEFAULT_CB_COOLDOWN_SECS: u64 = 60;
/// How often to run maintenance (stale recovery + cleanup).
const MAINTENANCE_INTERVAL_SECS: u64 = 3600;

/// Per-handler circuit breaker state.
/// Tracks consecutive failures and transitions between Closed → Open → HalfOpen → Closed.
#[derive(Debug)]
pub struct CircuitBreaker {
    /// Consecutive failure count.
    failures: std::sync::atomic::AtomicU32,
    /// Threshold to trip the circuit.
    threshold: u32,
    /// When the circuit was opened (None = closed).
    opened_at: std::sync::Mutex<Option<std::time::Instant>>,
    /// Cooldown duration before transitioning to half-open.
    cooldown: Duration,
    /// Whether a half-open test is in progress.
    half_open_in_progress: std::sync::atomic::AtomicBool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

impl CircuitBreaker {
    pub fn new(threshold: u32, cooldown: Duration) -> Self {
        Self {
            failures: std::sync::atomic::AtomicU32::new(0),
            threshold,
            opened_at: std::sync::Mutex::new(None),
            cooldown,
            half_open_in_progress: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Get the current state of the circuit.
    pub fn state(&self) -> CircuitState {
        let opened = self.opened_at.lock().unwrap_or_else(|e| e.into_inner());
        match *opened {
            None => CircuitState::Closed,
            Some(at) => {
                if at.elapsed() >= self.cooldown {
                    CircuitState::HalfOpen
                } else {
                    CircuitState::Open
                }
            }
        }
    }

    /// Check if delivery should proceed. Returns true if allowed.
    /// In HalfOpen state, only one test request is allowed at a time.
    pub fn allow_request(&self) -> bool {
        match self.state() {
            CircuitState::Closed => true,
            CircuitState::Open => false,
            CircuitState::HalfOpen => {
                // Only allow one test request via CAS
                !self
                    .half_open_in_progress
                    .swap(true, std::sync::atomic::Ordering::AcqRel)
            }
        }
    }

    /// Record a successful delivery. Resets the circuit to Closed.
    pub fn record_success(&self) {
        self.failures.store(0, std::sync::atomic::Ordering::Release);
        *self.opened_at.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.half_open_in_progress
            .store(false, std::sync::atomic::Ordering::Release);
    }

    /// Record a failed delivery. Opens the circuit if threshold is reached.
    /// Returns true if the circuit was just opened (transitioned to Open).
    pub fn record_failure(&self) -> bool {
        self.half_open_in_progress
            .store(false, std::sync::atomic::Ordering::Release);
        let prev = self
            .failures
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        let count = prev + 1;
        if count >= self.threshold {
            let mut opened = self.opened_at.lock().unwrap_or_else(|e| e.into_inner());
            let was_half_open = match *opened {
                Some(at) => at.elapsed() >= self.cooldown,
                None => false,
            };
            if opened.is_none() || was_half_open {
                *opened = Some(std::time::Instant::now());
                return count == self.threshold && !was_half_open;
            }
        }
        false
    }
}

pub struct Worker {
    db: Arc<Database>,
    metrics: Arc<Metrics>,
    alerter: SharedAlerter,
    worker_config: WorkerConfig,
    http: reqwest::Client,
    poll_interval: Duration,
    shutdown: tokio::sync::watch::Receiver<bool>,
    /// Per-handler rate limiters (handler_name -> governor rate limiter).
    rate_limiters: HashMap<
        String,
        Arc<
            governor::RateLimiter<
                governor::state::NotKeyed,
                governor::state::InMemoryState,
                governor::clock::DefaultClock,
            >,
        >,
    >,
    /// Per-handler payload transform templates.
    transforms: Arc<HashMap<String, String>>,
    /// Per-handler custom HTTP headers.
    handler_headers: Arc<HashMap<String, HashMap<String, String>>>,
    /// Per-handler HTTP method overrides (only non-"POST" entries).
    handler_methods: Arc<HashMap<String, String>>,
    /// Per-handler circuit breakers.
    circuit_breakers: HashMap<String, Arc<CircuitBreaker>>,
    /// Workflow configs for step progression.
    workflows: Arc<HashMap<String, config::WorkflowConfig>>,
    /// Default retry config.
    default_retry_max: u32,
}

impl Worker {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Arc<Database>,
        metrics: Arc<Metrics>,
        alerter: SharedAlerter,
        worker_config: WorkerConfig,
        shutdown: tokio::sync::watch::Receiver<bool>,
        handler_rate_limits: HashMap<String, u32>,
        handler_transforms: HashMap<String, String>,
        handler_headers: HashMap<String, HashMap<String, String>>,
        handler_methods: HashMap<String, String>,
        workflows: HashMap<String, config::WorkflowConfig>,
        default_retry_max: u32,
        handler_names: Vec<String>,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        let rate_limiters = handler_rate_limits
            .into_iter()
            .filter(|(_, rate)| *rate > 0)
            .map(|(name, rate)| {
                let quota = governor::Quota::per_second(std::num::NonZeroU32::new(rate).unwrap());
                let limiter = governor::RateLimiter::direct(quota);
                (name, Arc::new(limiter))
            })
            .collect();

        let circuit_breakers: HashMap<String, Arc<CircuitBreaker>> = handler_names
            .into_iter()
            .map(|name| {
                (
                    name,
                    Arc::new(CircuitBreaker::new(
                        DEFAULT_CB_THRESHOLD,
                        Duration::from_secs(DEFAULT_CB_COOLDOWN_SECS),
                    )),
                )
            })
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
            circuit_breakers,
            transforms: Arc::new(handler_transforms),
            handler_headers: Arc::new(handler_headers),
            handler_methods: Arc::new(handler_methods),
            workflows: Arc::new(workflows),
            default_retry_max,
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

        let semaphore = Arc::new(Semaphore::new(self.worker_config.max_concurrency));
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
                    let batch = self.worker_config.batch_size.min(
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
                        let handler_headers = self.handler_headers.clone();
                        let handler_methods = self.handler_methods.clone();
                        let workflows = self.workflows.clone();
                        let default_retry_max = self.default_retry_max;
                        let rate_limiter = self.rate_limiters.get(&job.handler).cloned();
                        let cb = self.circuit_breakers.get(&job.handler).cloned();

                        in_flight.spawn(async move {
                            // Circuit breaker check: skip delivery if circuit is open
                            if let Some(ref cb) = cb {
                                if !cb.allow_request() {
                                    tracing::warn!(
                                        handler = job.handler,
                                        job_id = job.id,
                                        "Circuit open, rescheduling job"
                                    );
                                    let next_at = (Utc::now() + chrono::Duration::seconds(10)).naive_utc();
                                    let _ = db.mark_job_retryable(&job.id, next_at, "circuit breaker open").await;
                                    metrics.inc_circuit_rejected(&job.handler);
                                    drop(permit);
                                    return;
                                }
                            }

                            // Wait for rate limiter (GCRA — blocks until quota allows)
                            if let Some(ref rl) = rate_limiter {
                                rl.until_ready().await;
                            }

                            deliver_job(&db, &http, &metrics, &alerter, &transforms, &handler_headers, &handler_methods, &workflows, default_retry_max, &job, cb.as_deref()).await;
                            drop(permit);
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

#[allow(clippy::too_many_arguments)]
async fn deliver_job(
    db: &Database,
    http: &reqwest::Client,
    metrics: &Metrics,
    alerter: &SharedAlerter,
    transforms: &HashMap<String, String>,
    handler_headers: &HashMap<String, HashMap<String, String>>,
    handler_methods: &HashMap<String, String>,
    workflows: &HashMap<String, config::WorkflowConfig>,
    default_retry_max: u32,
    job: &crate::db::JobRow,
    circuit_breaker: Option<&CircuitBreaker>,
) {
    let start = std::time::Instant::now();
    let is_outbound = job.handler.starts_with("outbound/");
    let is_workflow = !is_outbound && job.handler.contains('/');

    // Check workflow timeout before executing any step
    if is_workflow {
        if let Ok(Some(wf_data)) = db.get_workflow_job_data(&job.id).await {
            if let Some(ref run_id) = wf_data.workflow_run_id {
                if let Ok(Some(timeout_at)) = db.get_workflow_timeout(run_id).await {
                    let now = crate::db::format_now();
                    if now > timeout_at {
                        let workflow_name = job.handler.split('/').next().unwrap_or("");
                        tracing::warn!(
                            workflow = workflow_name,
                            run_id = run_id.as_str(),
                            job_id = job.id,
                            "Workflow timed out, skipping step"
                        );
                        let _ = db.mark_job_dead(&job.id, "workflow timeout").await;
                        let _ = db.fail_workflow_run(run_id).await;
                        metrics.inc_workflow_failed(workflow_name);
                        return;
                    }
                }
            }
        }
    }

    // Check if this is a choice/parallel/map step that doesn't need HTTP delivery.
    // Branch jobs (parallel/map branches) have the same handler name but need HTTP delivery,
    // so we must check the job's branch_name to distinguish them.
    if is_workflow {
        let is_branch_job = if let Ok(Some(wf_check)) = db.get_workflow_job_data(&job.id).await {
            wf_check.branch_name.is_some()
        } else {
            false
        };
        let workflow_name = job.handler.split('/').next().unwrap_or("");
        let step_name_part = job.handler.split('/').nth(1).unwrap_or("");
        if !is_branch_job {
            if let Some(wf) = workflows.get(workflow_name) {
                if let Some(step) = wf.steps.iter().find(|s| s.name == step_name_part) {
                    match step.handler_type.as_str() {
                        "choice" => {
                            // Choice step: evaluate conditions and route (no HTTP call)
                            if let Err(e) = db.mark_job_completed(&job.id).await {
                                tracing::error!(job_id = job.id, error = %e, "Failed to mark choice job completed");
                                return;
                            }
                            if let Err(e) = handle_choice_step(
                                db,
                                metrics,
                                workflows,
                                default_retry_max,
                                job,
                                step,
                            )
                            .await
                            {
                                tracing::error!(job_id = job.id, error = %e, "Failed to handle choice step");
                            }
                            return;
                        }
                        "parallel" => {
                            // Parallel step: create branch jobs (no HTTP call for the step itself)
                            if let Err(e) = db.mark_job_completed(&job.id).await {
                                tracing::error!(job_id = job.id, error = %e, "Failed to mark parallel job completed");
                                return;
                            }
                            if let Err(e) =
                                handle_parallel_step(db, metrics, default_retry_max, job, step)
                                    .await
                            {
                                tracing::error!(job_id = job.id, error = %e, "Failed to handle parallel step");
                            }
                            return;
                        }
                        "map" => {
                            // Map step: create jobs for each item (no HTTP call for the step itself)
                            if let Err(e) = db.mark_job_completed(&job.id).await {
                                tracing::error!(job_id = job.id, error = %e, "Failed to mark map job completed");
                                return;
                            }
                            if let Err(e) =
                                handle_map_step(db, metrics, default_retry_max, job, step).await
                            {
                                tracing::error!(job_id = job.id, error = %e, "Failed to handle map step");
                            }
                            return;
                        }
                        "wait" => {
                            // Wait step: complete immediately, create next step with delayed scheduled_at
                            if let Err(e) = db.mark_job_completed(&job.id).await {
                                tracing::error!(job_id = job.id, error = %e, "Failed to mark wait job completed");
                                return;
                            }
                            if let Err(e) = handle_wait_step(
                                db,
                                metrics,
                                workflows,
                                default_retry_max,
                                job,
                                step,
                            )
                            .await
                            {
                                tracing::error!(job_id = job.id, error = %e, "Failed to handle wait step");
                            }
                            return;
                        }
                        "callback" => {
                            // Callback step: create a waiting job with a token (no HTTP call)
                            if let Err(e) = db.mark_job_completed(&job.id).await {
                                tracing::error!(job_id = job.id, error = %e, "Failed to mark callback job completed");
                                return;
                            }
                            if let Err(e) = handle_callback_step(
                                db,
                                http,
                                metrics,
                                workflows,
                                default_retry_max,
                                job,
                                step,
                            )
                            .await
                            {
                                tracing::error!(job_id = job.id, error = %e, "Failed to handle callback step");
                            }
                            return;
                        }
                        "workflow" => {
                            // Sub-workflow step: launch child workflow
                            if let Err(e) = db.mark_job_completed(&job.id).await {
                                tracing::error!(job_id = job.id, error = %e, "Failed to mark sub-workflow job completed");
                                return;
                            }
                            if let Err(e) = handle_subworkflow_step(
                                db,
                                metrics,
                                workflows,
                                default_retry_max,
                                job,
                                step,
                            )
                            .await
                            {
                                tracing::error!(job_id = job.id, error = %e, "Failed to handle sub-workflow step");
                            }
                            return;
                        }
                        _ => {} // regular HTTP/gRPC step, continue below
                    }
                }
            }
        } // end !is_branch_job
    }

    // For workflow jobs, use step_input as payload; for regular jobs, use event payload
    let (transform, custom_headers, method) = if is_workflow {
        // Get custom headers and method from step/branch config
        let workflow_name = job.handler.split('/').next().unwrap_or("");
        let step_name = job.handler.split('/').nth(1).unwrap_or("");
        let wf = workflows.get(workflow_name);
        let step = wf.and_then(|wf| wf.steps.iter().find(|s| s.name == step_name));

        // For branch jobs, look up method from the branch config
        let wf_data = db.get_workflow_job_data(&job.id).await.ok().flatten();
        let branch_name = wf_data.as_ref().and_then(|d| d.branch_name.as_deref());

        let (headers, method) = if let Some(bn) = branch_name {
            // Branch job: get method from branch config
            let branch = step
                .and_then(|s| s.branches.as_ref())
                .and_then(|bs| bs.iter().find(|b| b.name == bn));
            let h = branch.map(|b| &b.headers).filter(|h| !h.is_empty());
            let m = branch.map(|b| b.method.as_str()).unwrap_or("POST");
            (h.cloned(), m.to_string())
        } else {
            // Regular step job
            let h = step.map(|s| &s.headers).filter(|h| !h.is_empty());
            let m = step.map(|s| s.method.as_str()).unwrap_or("POST");
            (h.cloned(), m.to_string())
        };
        (None, headers, method)
    } else {
        (
            transforms.get(&job.handler).map(|s| s.as_str()),
            handler_headers.get(&job.handler).cloned(),
            handler_methods
                .get(&job.handler)
                .cloned()
                .unwrap_or_else(|| "POST".into()),
        )
    };

    let result = if is_workflow {
        deliver_workflow_step(db, http, job, custom_headers.as_ref(), &method).await
    } else if is_outbound {
        deliver_outbound(db, http, job).await
    } else {
        deliver(db, http, job, transform, custom_headers.as_ref(), &method).await
    };
    let duration_ms = start.elapsed().as_millis() as i64;

    match result {
        Ok(delivery_result) => {
            let status_code = delivery_result.status_code;
            let attempt_id = ulid::Ulid::new().to_string();
            if let Err(e) = db
                .insert_attempt(
                    &attempt_id,
                    &job.id,
                    job.attempt + 1,
                    Some(status_code as i32),
                    delivery_result.response_body.as_deref(),
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
                if let Some(cb) = circuit_breaker {
                    cb.record_success();
                }
                if let Err(e) = db.mark_job_completed(&job.id).await {
                    metrics.inc_db_errors();
                    tracing::error!(job_id = job.id, error = %e, "Failed to mark completed");
                }

                // For workflow jobs, save output and advance to next step
                if is_workflow {
                    if let Err(e) = handle_workflow_step_success(
                        db,
                        metrics,
                        workflows,
                        default_retry_max,
                        job,
                        delivery_result.response_body.as_deref(),
                    )
                    .await
                    {
                        tracing::error!(job_id = job.id, error = %e, "Failed to advance workflow");
                    }
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

                if is_workflow {
                    handle_workflow_step_failure(
                        db,
                        metrics,
                        alerter,
                        workflows,
                        default_retry_max,
                        job,
                        &error,
                        error_type,
                    )
                    .await;
                } else {
                    if let Some(cb) = circuit_breaker {
                        if cb.record_failure() {
                            tracing::warn!(handler = job.handler, "Circuit breaker opened");
                            metrics.inc_circuit_opened(&job.handler);
                        }
                    }
                    match handle_failure(db, job, &error, delivery_result.retry_after_secs).await {
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

            if is_workflow {
                handle_workflow_step_failure(
                    db,
                    metrics,
                    alerter,
                    workflows,
                    default_retry_max,
                    job,
                    &error,
                    error_type,
                )
                .await;
            } else {
                if let Some(cb) = circuit_breaker {
                    if cb.record_failure() {
                        tracing::warn!(handler = job.handler, "Circuit breaker opened");
                        metrics.inc_circuit_opened(&job.handler);
                    }
                }
                match handle_failure(db, job, &error, None).await {
                    Ok(true) => metrics.inc_dlq(&job.handler),
                    Ok(false) => {}
                    Err(e) => {
                        tracing::error!(job_id = job.id, error = %e, "Failed to handle failure")
                    }
                }
            }
        }
    }
}

struct DeliveryResult {
    status_code: u16,
    response_body: Option<String>,
    /// Retry-After header value in seconds (from downstream 429/503 response).
    retry_after_secs: Option<i64>,
}

/// Deliver a workflow step job using the step_input payload.
async fn deliver_workflow_step(
    db: &Database,
    http: &reqwest::Client,
    job: &crate::db::JobRow,
    custom_headers: Option<&HashMap<String, String>>,
    method: &str,
) -> Result<DeliveryResult> {
    // Get step_input from workflow job data
    let wf_data = db.get_workflow_job_data(&job.id).await?;
    let payload = wf_data
        .and_then(|d| d.step_input)
        .unwrap_or_else(|| "{}".to_string());

    let reqwest_method = match method {
        "GET" => reqwest::Method::GET,
        "PUT" => reqwest::Method::PUT,
        "PATCH" => reqwest::Method::PATCH,
        "DELETE" => reqwest::Method::DELETE,
        _ => reqwest::Method::POST,
    };

    let mut request = http
        .request(reqwest_method.clone(), &job.url)
        .header("Content-Type", "application/json")
        .header("X-Qhook-Job-ID", &job.id)
        .header("X-Qhook-Event-ID", &job.event_id)
        .header("X-Qhook-Handler", &job.handler)
        .header("X-Qhook-Attempt", (job.attempt + 1).to_string());

    // Apply custom headers from step config
    if let Some(ch) = custom_headers {
        for (key, value) in ch {
            request = request.header(key.as_str(), value.as_str());
        }
    }

    // Skip body for GET requests
    let response = if reqwest_method == reqwest::Method::GET {
        request.send().await?
    } else {
        request.body(payload).send().await?
    };

    let status_code = response.status().as_u16();
    let retry_after_secs = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_retry_after);
    let body = response.text().await.ok();

    Ok(DeliveryResult {
        status_code,
        response_body: body,
        retry_after_secs,
    })
}

/// Handle successful workflow step completion: save output, advance to next step.
async fn handle_workflow_step_success(
    db: &Database,
    metrics: &Metrics,
    workflows: &HashMap<String, config::WorkflowConfig>,
    default_retry_max: u32,
    job: &crate::db::JobRow,
    response_body: Option<&str>,
) -> Result<()> {
    let wf_data = db.get_workflow_job_data(&job.id).await?;
    let wf_data = match wf_data {
        Some(d) if d.workflow_run_id.is_some() => d,
        _ => return Ok(()), // not a workflow job
    };

    // If this is a branch job (parallel/map), use branch completion logic
    if wf_data.branch_name.is_some() {
        return handle_branch_completion(
            db,
            metrics,
            workflows,
            default_retry_max,
            job,
            &wf_data,
            response_body,
        )
        .await;
    }

    let run_id = wf_data.workflow_run_id.as_ref().unwrap();
    let step_index = wf_data.step_index.unwrap_or(0);

    // Save step output
    if let Some(body) = response_body {
        db.save_step_output(&job.id, body).await?;
    }

    // Find the workflow config
    // handler format: "workflow_name/step_name"
    let workflow_name = job.handler.split('/').next().unwrap_or("");
    let workflow = match workflows.get(workflow_name) {
        Some(w) => w,
        None => {
            tracing::error!(workflow = workflow_name, "Workflow config not found");
            return Ok(());
        }
    };

    // Find current step config
    let current_step = workflow
        .steps
        .iter()
        .find(|s| Some(s.name.as_str()) == wf_data.step_name.as_deref());

    // Apply result_path merge
    let step_input_payload = wf_data.step_input.as_deref().unwrap_or("{}");
    let merged_payload = if let Some(step) = current_step {
        merge_result_path(
            step_input_payload,
            response_body.unwrap_or("{}"),
            step.result_path.as_deref(),
        )
    } else {
        response_body.unwrap_or("{}").to_string()
    };

    // Apply output transform
    let output_payload = if let Some(step) = current_step
        && let Some(ref output_template) = step.output
    {
        crate::api::apply_transform(&merged_payload, output_template)
    } else {
        merged_payload
    };

    metrics.inc_workflow_step_completed(workflow_name);

    // Check workflow timeout
    if let Ok(Some(timeout_at)) = db.get_workflow_timeout(run_id).await {
        let now = crate::db::format_now();
        if now > timeout_at {
            db.fail_workflow_run(run_id).await?;
            metrics.inc_workflow_failed(workflow_name);
            tracing::warn!(workflow = workflow_name, run_id, "Workflow timed out");
            return Ok(());
        }
    }

    // Check if current step has end: true
    if current_step.is_some_and(|s| s.end) {
        db.complete_workflow_run(run_id).await?;
        metrics.inc_workflow_completed(workflow_name);
        tracing::info!(
            workflow = workflow_name,
            run_id,
            "Workflow completed (end step)"
        );
        resume_parent_workflow(
            db,
            metrics,
            workflows,
            default_retry_max,
            run_id,
            &job.event_id,
            &output_payload,
        )
        .await?;
        return Ok(());
    }

    // Find next step
    let next_index = (step_index + 1) as usize;
    if next_index >= workflow.steps.len() {
        // No more steps — workflow complete
        db.complete_workflow_run(run_id).await?;
        metrics.inc_workflow_completed(workflow_name);
        tracing::info!(workflow = workflow_name, run_id, "Workflow completed");
        resume_parent_workflow(
            db,
            metrics,
            workflows,
            default_retry_max,
            run_id,
            &job.event_id,
            &output_payload,
        )
        .await?;
        return Ok(());
    }

    let next_step = &workflow.steps[next_index];

    // Update workflow_run current_step
    db.update_workflow_run_step(run_id, &next_step.name).await?;

    // Create next step's job
    let next_job_id = ulid::Ulid::new().to_string();
    let next_handler = format!("{}/{}", workflow_name, next_step.name);
    let max_attempts = next_step
        .retry
        .as_ref()
        .map(|r| r.max)
        .unwrap_or(default_retry_max);

    // Apply next step's input transform
    let next_input = match &next_step.input {
        Some(template) => crate::api::apply_transform(&output_payload, template),
        None => output_payload,
    };

    db.insert_workflow_job(
        &next_job_id,
        &job.event_id,
        &next_handler,
        next_step.url.as_deref().unwrap_or(""),
        max_attempts,
        run_id,
        &next_step.name,
        next_index as i32,
        Some(&next_input),
    )
    .await?;

    metrics.inc_jobs_created();
    tracing::info!(
        workflow = workflow_name,
        run_id,
        job_id = next_job_id,
        step = next_step.name,
        "Next workflow step job created"
    );

    Ok(())
}

/// Merge response into payload according to result_path.
fn merge_result_path(input: &str, response: &str, result_path: Option<&str>) -> String {
    match result_path {
        None | Some("$") => {
            // Default: replace entirely with response
            response.to_string()
        }
        Some("null") => {
            // Discard response, keep input
            input.to_string()
        }
        Some(path) if path.starts_with("$.") => {
            // Merge response as a field in the input
            let field = &path[2..];
            let mut input_json: serde_json::Value = serde_json::from_str(input)
                .unwrap_or(serde_json::Value::Object(Default::default()));
            let response_json: serde_json::Value =
                serde_json::from_str(response).unwrap_or(serde_json::Value::Null);

            if let serde_json::Value::Object(ref mut map) = input_json {
                map.insert(field.to_string(), response_json);
            }
            serde_json::to_string(&input_json).unwrap_or_else(|_| input.to_string())
        }
        _ => response.to_string(),
    }
}

/// Handle workflow step failure: check catch rules, on_failure, etc.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn handle_workflow_step_failure(
    db: &Database,
    metrics: &Metrics,
    alerter: &SharedAlerter,
    workflows: &HashMap<String, config::WorkflowConfig>,
    default_retry_max: u32,
    job: &crate::db::JobRow,
    error: &str,
    error_type: &str,
) {
    let workflow_name = job.handler.split('/').next().unwrap_or("");
    let step_name = job.handler.split('/').nth(1).unwrap_or("");

    let workflow = match workflows.get(workflow_name) {
        Some(w) => w,
        None => {
            // Fallback to default failure handling
            let _ = handle_failure(db, job, error, None).await;
            return;
        }
    };

    let step = workflow.steps.iter().find(|s| s.name == step_name);

    // Check if error type should be retried (only when retry.errors is configured)
    let should_retry = step
        .and_then(|s| s.retry.as_ref())
        .map(|r| error_type_matches(&r.errors, error_type))
        .unwrap_or(true); // default: retry all errors

    let current_attempt = job.attempt + 1;

    if should_retry && current_attempt < job.max_attempts {
        // Schedule retry
        let backoff_secs = 30i64 * (1i64 << current_attempt.min(10));
        let next_at = Utc::now().naive_utc() + chrono::Duration::seconds(backoff_secs);
        if let Err(e) = db.mark_job_retryable(&job.id, next_at, error).await {
            tracing::error!(job_id = job.id, error = %e, "Failed to schedule retry");
        }
        return;
    }

    // Retries exhausted or error type not retryable — check catch
    if let Some(step) = step
        && let Some(ref catches) = step.catch
    {
        for c in catches {
            if error_type_matches(&c.errors, error_type) {
                // Route to catch target
                if let Err(e) = route_to_catch_step(
                    db,
                    metrics,
                    workflows,
                    default_retry_max,
                    workflow_name,
                    &c.goto,
                    job,
                    error,
                )
                .await
                {
                    tracing::error!(
                        job_id = job.id,
                        goto = c.goto,
                        error = %e,
                        "Failed to route to catch step"
                    );
                }
                return;
            }
        }
    }

    // No catch matched — check on_failure
    let on_failure = step
        .map(|s| &s.on_failure)
        .unwrap_or(&config::OnFailure::Stop);

    match on_failure {
        config::OnFailure::Continue => {
            // Mark job as completed (failed but continuing) and advance
            if let Err(e) = db.mark_job_completed(&job.id).await {
                tracing::error!(job_id = job.id, error = %e, "Failed to mark completed");
            }
            let error_payload = serde_json::json!({
                "error": error,
                "failed_step": step_name,
            })
            .to_string();
            if let Err(e) = handle_workflow_step_success(
                db,
                metrics,
                workflows,
                default_retry_max,
                job,
                Some(&error_payload),
            )
            .await
            {
                tracing::error!(job_id = job.id, error = %e, "Failed to continue after failure");
            }
        }
        config::OnFailure::Stop => {
            // Mark job as dead and fail the workflow
            if let Err(e) = db.mark_job_dead(&job.id, error).await {
                tracing::error!(job_id = job.id, error = %e, "Failed to mark dead");
            }
            metrics.inc_dlq(&job.handler);
            if let Some(a) = alerter {
                a.send(AlertEvent::Dlq {
                    job_id: job.id.clone(),
                    handler: job.handler.clone(),
                    attempts: current_attempt,
                });
            }
            // Fail the workflow run
            if let Ok(Some(wf_data)) = db.get_workflow_job_data(&job.id).await {
                if let Some(run_id) = &wf_data.workflow_run_id {
                    let _ = db.fail_workflow_run(run_id).await;
                    metrics.inc_workflow_failed(workflow_name);
                    tracing::warn!(
                        workflow = workflow_name,
                        run_id,
                        step = step_name,
                        "Workflow failed"
                    );
                }
            }
        }
    }
}

/// Route to a catch step in the workflow.
#[allow(clippy::too_many_arguments)]
async fn route_to_catch_step(
    db: &Database,
    metrics: &Metrics,
    workflows: &HashMap<String, config::WorkflowConfig>,
    default_retry_max: u32,
    workflow_name: &str,
    goto_step_name: &str,
    job: &crate::db::JobRow,
    error: &str,
) -> Result<()> {
    // Mark current job as completed (caught)
    db.mark_job_completed(&job.id).await?;

    let workflow = workflows
        .get(workflow_name)
        .ok_or_else(|| anyhow::anyhow!("Workflow '{}' not found", workflow_name))?;

    let (goto_index, goto_step) = workflow
        .steps
        .iter()
        .enumerate()
        .find(|(_, s)| s.name == goto_step_name)
        .ok_or_else(|| anyhow::anyhow!("Step '{}' not found", goto_step_name))?;

    let wf_data = db.get_workflow_job_data(&job.id).await?;
    let run_id = wf_data
        .as_ref()
        .and_then(|d| d.workflow_run_id.as_deref())
        .unwrap_or("");

    // Update workflow_run current_step
    db.update_workflow_run_step(run_id, goto_step_name).await?;

    // Build error payload as input to catch step
    let error_payload = serde_json::json!({
        "error": error,
        "failed_step": job.handler.split('/').nth(1).unwrap_or(""),
        "job_id": job.id,
    })
    .to_string();

    let next_input = match &goto_step.input {
        Some(template) => crate::api::apply_transform(&error_payload, template),
        None => error_payload,
    };

    let next_job_id = ulid::Ulid::new().to_string();
    let next_handler = format!("{}/{}", workflow_name, goto_step.name);
    let max_attempts = goto_step
        .retry
        .as_ref()
        .map(|r| r.max)
        .unwrap_or(default_retry_max);

    db.insert_workflow_job(
        &next_job_id,
        &job.event_id,
        &next_handler,
        goto_step.url.as_deref().unwrap_or(""),
        max_attempts,
        run_id,
        &goto_step.name,
        goto_index as i32,
        Some(&next_input),
    )
    .await?;

    metrics.inc_jobs_created();
    tracing::info!(
        workflow = workflow_name,
        run_id,
        job_id = next_job_id,
        step = goto_step.name,
        "Routed to catch step"
    );

    Ok(())
}

/// Handle a choice step: evaluate conditions and route to matching step.
async fn handle_choice_step(
    db: &Database,
    metrics: &Metrics,
    workflows: &HashMap<String, config::WorkflowConfig>,
    default_retry_max: u32,
    job: &crate::db::JobRow,
    step: &config::StepConfig,
) -> Result<()> {
    let wf_data = db.get_workflow_job_data(&job.id).await?;
    let wf_data = match wf_data {
        Some(d) if d.workflow_run_id.is_some() => d,
        _ => return Ok(()),
    };
    let run_id = wf_data.workflow_run_id.as_ref().unwrap();
    let payload = wf_data.step_input.as_deref().unwrap_or("{}");

    let workflow_name = job.handler.split('/').next().unwrap_or("");
    let workflow = workflows
        .get(workflow_name)
        .ok_or_else(|| anyhow::anyhow!("Workflow '{}' not found", workflow_name))?;

    // Evaluate choices using the filter evaluator
    let mut goto = None;
    if let Some(ref choices) = step.choices {
        for choice in choices {
            if crate::api::evaluate_filter_pub(payload, &choice.when) {
                goto = Some(choice.goto.as_str());
                break;
            }
        }
    }

    // Fall back to default
    if goto.is_none() {
        goto = step.default.as_deref();
    }

    let goto = match goto {
        Some(g) => g,
        None => {
            // No match and no default — fail the workflow
            db.fail_workflow_run(run_id).await?;
            tracing::warn!(
                workflow = workflow_name,
                run_id,
                step = step.name,
                "Choice step: no matching condition and no default"
            );
            return Ok(());
        }
    };

    // Find the goto step
    let (goto_index, goto_step) = workflow
        .steps
        .iter()
        .enumerate()
        .find(|(_, s)| s.name == goto)
        .ok_or_else(|| anyhow::anyhow!("Step '{}' not found", goto))?;

    db.update_workflow_run_step(run_id, goto).await?;

    let next_input = match &goto_step.input {
        Some(template) => crate::api::apply_transform(payload, template),
        None => payload.to_string(),
    };

    let next_job_id = ulid::Ulid::new().to_string();
    let next_handler = format!("{}/{}", workflow_name, goto_step.name);
    let max_attempts = goto_step
        .retry
        .as_ref()
        .map(|r| r.max)
        .unwrap_or(default_retry_max);

    db.insert_workflow_job(
        &next_job_id,
        &job.event_id,
        &next_handler,
        goto_step.url.as_deref().unwrap_or(""),
        max_attempts,
        run_id,
        &goto_step.name,
        goto_index as i32,
        Some(&next_input),
    )
    .await?;

    metrics.inc_jobs_created();
    tracing::info!(
        workflow = workflow_name,
        run_id,
        job_id = next_job_id,
        step = goto_step.name,
        choice_from = step.name,
        "Choice step routed"
    );

    Ok(())
}

/// Handle a parallel step: create branch jobs for concurrent execution.
async fn handle_parallel_step(
    db: &Database,
    metrics: &Metrics,
    default_retry_max: u32,
    job: &crate::db::JobRow,
    step: &config::StepConfig,
) -> Result<()> {
    let wf_data = db.get_workflow_job_data(&job.id).await?;
    let wf_data = match wf_data {
        Some(d) if d.workflow_run_id.is_some() => d,
        _ => return Ok(()),
    };
    let run_id = wf_data.workflow_run_id.as_ref().unwrap();
    let payload = wf_data.step_input.as_deref().unwrap_or("{}");
    let step_index = wf_data.step_index.unwrap_or(0);
    let workflow_name = job.handler.split('/').next().unwrap_or("");

    let branches = match &step.branches {
        Some(b) => b,
        None => return Ok(()),
    };

    // Set parallel state on workflow run
    db.set_parallel_state(run_id, &step.name, branches.len() as i32)
        .await?;

    let max_attempts = step
        .retry
        .as_ref()
        .map(|r| r.max)
        .unwrap_or(default_retry_max);

    // Create a job for each branch
    for branch in branches {
        let branch_job_id = ulid::Ulid::new().to_string();
        let handler = format!("{}/{}", workflow_name, step.name);

        // Apply input transform per branch (using the same payload)
        let branch_input = match &step.input {
            Some(template) => crate::api::apply_transform(payload, template),
            None => payload.to_string(),
        };

        db.insert_branch_job(
            &branch_job_id,
            &job.event_id,
            &handler,
            &branch.url,
            max_attempts,
            run_id,
            &step.name,
            step_index,
            Some(&branch_input),
            &branch.name,
        )
        .await?;

        metrics.inc_jobs_created();
        tracing::info!(
            workflow = workflow_name,
            run_id,
            job_id = branch_job_id,
            step = step.name,
            branch = branch.name,
            "Parallel branch job created"
        );
    }

    Ok(())
}

/// Handle a map step: create jobs for each item in the array.
async fn handle_map_step(
    db: &Database,
    metrics: &Metrics,
    default_retry_max: u32,
    job: &crate::db::JobRow,
    step: &config::StepConfig,
) -> Result<()> {
    let wf_data = db.get_workflow_job_data(&job.id).await?;
    let wf_data = match wf_data {
        Some(d) if d.workflow_run_id.is_some() => d,
        _ => return Ok(()),
    };
    let run_id = wf_data.workflow_run_id.as_ref().unwrap();
    let payload = wf_data.step_input.as_deref().unwrap_or("{}");
    let step_index = wf_data.step_index.unwrap_or(0);
    let workflow_name = job.handler.split('/').next().unwrap_or("");

    let items_path = match &step.items_path {
        Some(p) => p,
        None => return Ok(()),
    };

    let url = step.url.as_deref().unwrap_or("");

    // Extract array from payload
    let json: serde_json::Value = serde_json::from_str(payload)?;
    let items = crate::api::resolve_path_pub(&json, items_path);
    let items = match items {
        Some(serde_json::Value::Array(arr)) => arr.clone(),
        _ => {
            tracing::warn!(
                workflow = workflow_name,
                run_id,
                items_path,
                "Map step: items_path does not resolve to an array"
            );
            vec![]
        }
    };

    if items.is_empty() {
        // No items — treat as complete, advance to next step
        // We use handle_workflow_step_success with empty array as response
        db.save_step_output(&job.id, "[]").await?;
        // Clear any parallel state and continue
        return handle_workflow_step_success(
            db,
            metrics,
            &HashMap::new(), // no workflows needed for empty result
            default_retry_max,
            job,
            Some("[]"),
        )
        .await;
    }

    // Set parallel state (map uses the same parallel tracking)
    db.set_parallel_state(run_id, &step.name, items.len() as i32)
        .await?;

    let max_attempts = step
        .retry
        .as_ref()
        .map(|r| r.max)
        .unwrap_or(default_retry_max);

    for (i, item) in items.iter().enumerate() {
        let item_job_id = ulid::Ulid::new().to_string();
        let handler = format!("{}/{}", workflow_name, step.name);
        let item_payload = serde_json::to_string(item)?;
        let branch_name = format!("{}", i);

        db.insert_branch_job(
            &item_job_id,
            &job.event_id,
            &handler,
            url,
            max_attempts,
            run_id,
            &step.name,
            step_index,
            Some(&item_payload),
            &branch_name,
        )
        .await?;

        metrics.inc_jobs_created();
    }

    tracing::info!(
        workflow = workflow_name,
        run_id,
        step = step.name,
        count = items.len(),
        "Map step: created jobs for items"
    );

    Ok(())
}

/// Handle completion of a parallel/map branch: check if all branches done, merge and advance.
async fn handle_branch_completion(
    db: &Database,
    metrics: &Metrics,
    workflows: &HashMap<String, config::WorkflowConfig>,
    default_retry_max: u32,
    job: &crate::db::JobRow,
    wf_data: &crate::db::WorkflowJobRow,
    response_body: Option<&str>,
) -> Result<()> {
    let run_id = wf_data.workflow_run_id.as_ref().unwrap();
    let step_name = wf_data.step_name.as_deref().unwrap_or("");
    let step_index = wf_data.step_index.unwrap_or(0);
    let workflow_name = job.handler.split('/').next().unwrap_or("");

    // Save branch output
    if let Some(body) = response_body {
        db.save_step_output(&job.id, body).await?;
    }

    // Increment parallel completed
    let (completed, total) = db.increment_parallel_completed(run_id).await?;
    tracing::info!(
        workflow = workflow_name,
        run_id,
        branch = wf_data.branch_name.as_deref().unwrap_or("?"),
        completed,
        total,
        "Branch completed"
    );

    if completed < total {
        return Ok(()); // Still waiting for other branches
    }

    // All branches done — merge results
    let branch_outputs = db.get_branch_outputs(run_id, step_name).await?;

    let workflow = workflows.get(workflow_name);
    let step = workflow.and_then(|w| w.steps.iter().find(|s| s.name == step_name));

    // Determine if this is a parallel or map step for merge strategy
    let is_map = step.is_some_and(|s| s.handler_type == "map");

    let merged = if is_map {
        // Map: collect outputs as an array
        let arr: Vec<serde_json::Value> = branch_outputs
            .iter()
            .map(|(_, output)| {
                output
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(serde_json::Value::Null)
            })
            .collect();
        serde_json::to_string(&arr)?
    } else {
        // Parallel: collect outputs as object keyed by branch name
        let mut obj = serde_json::Map::new();
        for (branch_name, output) in &branch_outputs {
            let val = output
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(serde_json::Value::Null);
            obj.insert(branch_name.clone(), val);
        }
        serde_json::to_string(&serde_json::Value::Object(obj))?
    };

    // Clear parallel state
    db.clear_parallel_state(run_id).await?;

    // Apply result_path merge
    let original_input = wf_data.step_input.as_deref().unwrap_or("{}");
    let result_path = step.and_then(|s| s.result_path.as_deref());
    let merged_payload = merge_result_path(original_input, &merged, result_path);

    // Apply output transform
    let output_payload = if let Some(step) = step
        && let Some(ref output_template) = step.output
    {
        crate::api::apply_transform(&merged_payload, output_template)
    } else {
        merged_payload
    };

    metrics.inc_workflow_step_completed(workflow_name);

    // Check end
    if step.is_some_and(|s| s.end) {
        db.complete_workflow_run(run_id).await?;
        metrics.inc_workflow_completed(workflow_name);
        tracing::info!(
            workflow = workflow_name,
            run_id,
            "Workflow completed (parallel/map end step)"
        );
        resume_parent_workflow(
            db,
            metrics,
            workflows,
            default_retry_max,
            run_id,
            &job.event_id,
            &output_payload,
        )
        .await?;
        return Ok(());
    }

    // Advance to next step
    let workflow = match workflows.get(workflow_name) {
        Some(w) => w,
        None => return Ok(()),
    };
    let next_index = (step_index + 1) as usize;
    if next_index >= workflow.steps.len() {
        db.complete_workflow_run(run_id).await?;
        metrics.inc_workflow_completed(workflow_name);
        tracing::info!(workflow = workflow_name, run_id, "Workflow completed");
        resume_parent_workflow(
            db,
            metrics,
            workflows,
            default_retry_max,
            run_id,
            &job.event_id,
            &output_payload,
        )
        .await?;
        return Ok(());
    }

    let next_step = &workflow.steps[next_index];
    db.update_workflow_run_step(run_id, &next_step.name).await?;

    let next_input = match &next_step.input {
        Some(template) => crate::api::apply_transform(&output_payload, template),
        None => output_payload,
    };

    let next_job_id = ulid::Ulid::new().to_string();
    let next_handler = format!("{}/{}", workflow_name, next_step.name);
    let max_attempts = next_step
        .retry
        .as_ref()
        .map(|r| r.max)
        .unwrap_or(default_retry_max);

    db.insert_workflow_job(
        &next_job_id,
        &job.event_id,
        &next_handler,
        next_step.url.as_deref().unwrap_or(""),
        max_attempts,
        run_id,
        &next_step.name,
        next_index as i32,
        Some(&next_input),
    )
    .await?;

    metrics.inc_jobs_created();
    tracing::info!(
        workflow = workflow_name,
        run_id,
        job_id = next_job_id,
        step = next_step.name,
        "Next step after parallel/map"
    );

    Ok(())
}

/// Handle a wait step: schedule the next step with a delayed scheduled_at.
async fn handle_wait_step(
    db: &Database,
    metrics: &Metrics,
    workflows: &HashMap<String, config::WorkflowConfig>,
    default_retry_max: u32,
    job: &crate::db::JobRow,
    step: &config::StepConfig,
) -> Result<()> {
    let wf_data = db.get_workflow_job_data(&job.id).await?;
    let wf_data = match wf_data {
        Some(d) if d.workflow_run_id.is_some() => d,
        _ => return Ok(()),
    };
    let run_id = wf_data.workflow_run_id.as_ref().unwrap();
    let step_index = wf_data.step_index.unwrap_or(0);
    let payload = wf_data.step_input.as_deref().unwrap_or("{}");
    let workflow_name = job.handler.split('/').next().unwrap_or("");

    let workflow = match workflows.get(workflow_name) {
        Some(w) => w,
        None => return Ok(()),
    };

    // Determine the wait duration
    let wait_until = if let Some(seconds) = step.seconds {
        Utc::now().naive_utc() + chrono::Duration::seconds(seconds as i64)
    } else if let Some(ref ts_path) = step.timestamp_path {
        // Extract timestamp from payload
        let json: serde_json::Value = serde_json::from_str(payload)?;
        let ts_value = crate::api::resolve_path_pub(&json, ts_path);
        match ts_value {
            Some(serde_json::Value::String(ts)) => {
                // Try parsing ISO 8601 timestamp
                chrono::NaiveDateTime::parse_from_str(&ts, "%Y-%m-%dT%H:%M:%S%.3f")
                    .or_else(|_| chrono::NaiveDateTime::parse_from_str(&ts, "%Y-%m-%dT%H:%M:%S"))
                    .or_else(|_| chrono::NaiveDateTime::parse_from_str(&ts, "%Y-%m-%dT%H:%M:%SZ"))
                    .or_else(|e| {
                        // Try parsing with timezone info by stripping it
                        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&ts) {
                            Ok(dt.naive_utc())
                        } else {
                            Err(e)
                        }
                    })
                    .unwrap_or_else(|_| {
                        tracing::warn!(
                            workflow = workflow_name,
                            run_id,
                            timestamp = ts,
                            "Wait step: failed to parse timestamp, using now"
                        );
                        Utc::now().naive_utc()
                    })
            }
            Some(serde_json::Value::Number(n)) => {
                // Unix timestamp (seconds)
                let ts = n.as_i64().unwrap_or(0);
                chrono::DateTime::from_timestamp(ts, 0)
                    .map(|dt| dt.naive_utc())
                    .unwrap_or_else(|| Utc::now().naive_utc())
            }
            _ => {
                tracing::warn!(
                    workflow = workflow_name,
                    run_id,
                    path = ts_path.as_str(),
                    "Wait step: timestamp_path not found or invalid"
                );
                Utc::now().naive_utc()
            }
        }
    } else {
        // No seconds or timestamp_path — shouldn't happen (validated)
        Utc::now().naive_utc()
    };

    let scheduled_at = crate::db::format_dt(wait_until);

    metrics.inc_workflow_step_completed(workflow_name);
    tracing::info!(
        workflow = workflow_name,
        run_id,
        step = step.name,
        scheduled_at = scheduled_at,
        "Wait step completed, scheduling next step"
    );

    // Advance to next step with delayed scheduled_at
    let next_index = (step_index + 1) as usize;
    if next_index >= workflow.steps.len() {
        db.complete_workflow_run(run_id).await?;
        metrics.inc_workflow_completed(workflow_name);
        resume_parent_workflow(
            db,
            metrics,
            workflows,
            default_retry_max,
            run_id,
            &job.event_id,
            payload,
        )
        .await?;
        return Ok(());
    }

    let next_step = &workflow.steps[next_index];
    db.update_workflow_run_step(run_id, &next_step.name).await?;

    let next_input = match &next_step.input {
        Some(template) => crate::api::apply_transform(payload, template),
        None => payload.to_string(),
    };

    let next_job_id = ulid::Ulid::new().to_string();
    let next_handler = format!("{}/{}", workflow_name, next_step.name);
    let max_attempts = next_step
        .retry
        .as_ref()
        .map(|r| r.max)
        .unwrap_or(default_retry_max);

    db.insert_workflow_job_delayed(
        &next_job_id,
        &job.event_id,
        &next_handler,
        next_step.url.as_deref().unwrap_or(""),
        max_attempts,
        run_id,
        &next_step.name,
        next_index as i32,
        Some(&next_input),
        &scheduled_at,
    )
    .await?;

    metrics.inc_jobs_created();
    Ok(())
}

/// Handle a callback step: create a waiting job with a unique token.
async fn handle_callback_step(
    db: &Database,
    http: &reqwest::Client,
    metrics: &Metrics,
    workflows: &HashMap<String, config::WorkflowConfig>,
    default_retry_max: u32,
    job: &crate::db::JobRow,
    step: &config::StepConfig,
) -> Result<()> {
    let wf_data = db.get_workflow_job_data(&job.id).await?;
    let wf_data = match wf_data {
        Some(d) if d.workflow_run_id.is_some() => d,
        _ => return Ok(()),
    };
    let run_id = wf_data.workflow_run_id.as_ref().unwrap();
    let step_index = wf_data.step_index.unwrap_or(0);
    let payload = wf_data.step_input.as_deref().unwrap_or("{}");
    let workflow_name = job.handler.split('/').next().unwrap_or("");

    let _ = workflows
        .get(workflow_name)
        .ok_or_else(|| anyhow::anyhow!("Workflow '{}' not found", workflow_name))?;

    // Generate cryptographically strong token: two ULIDs = 160 bits of randomness.
    // Each ULID has 80 random bits from thread_rng (ChaCha CSPRNG).
    let callback_token = format!("{}{}", ulid::Ulid::new(), ulid::Ulid::new());
    let callback_job_id = ulid::Ulid::new().to_string();
    let handler = format!("{}/{}", workflow_name, step.name);

    let max_attempts = step
        .retry
        .as_ref()
        .map(|r| r.max)
        .unwrap_or(default_retry_max);

    // Calculate callback timeout_at if configured
    let timeout_at = step.callback_timeout.map(|secs| {
        crate::db::format_dt(Utc::now().naive_utc() + chrono::Duration::seconds(secs as i64))
    });

    db.insert_callback_job(
        &callback_job_id,
        &job.event_id,
        &handler,
        max_attempts,
        run_id,
        &step.name,
        step_index,
        Some(payload),
        &callback_token,
        timeout_at.as_deref(),
    )
    .await?;

    metrics.inc_jobs_created();
    // Log only first 8 chars of token to avoid leaking the full secret
    tracing::info!(
        workflow = workflow_name,
        run_id,
        step = step.name,
        token_prefix = &callback_token[..8],
        "Callback step: waiting for external callback"
    );

    // If the step has a url configured, notify the external service with the callback token
    if let Some(ref url) = step.url {
        let notify_payload = serde_json::json!({
            "callback_token": callback_token,
            "workflow": workflow_name,
            "step": step.name,
            "run_id": run_id,
            "payload": serde_json::from_str::<serde_json::Value>(payload).unwrap_or_default(),
        });

        let mut request = http
            .post(url)
            .header("Content-Type", "application/json")
            .header("X-Qhook-Callback-Token", &callback_token);

        // Apply custom headers from step config
        for (key, value) in &step.headers {
            request = request.header(key.as_str(), value.as_str());
        }

        match request.json(&notify_payload).send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!(
                    workflow = workflow_name,
                    step = step.name,
                    url,
                    "Callback token notified to external service"
                );
            }
            Ok(resp) => {
                tracing::warn!(
                    workflow = workflow_name,
                    step = step.name,
                    url,
                    status = resp.status().as_u16(),
                    "Failed to notify callback token (non-2xx)"
                );
            }
            Err(e) => {
                tracing::warn!(
                    workflow = workflow_name,
                    step = step.name,
                    url,
                    error = %e,
                    "Failed to notify callback token"
                );
            }
        }
    }

    Ok(())
}

/// Resume a callback job after receiving an external callback.
/// Called from the API layer. Advances the workflow to the next step.
pub async fn resume_callback(
    db: &Database,
    metrics: &Metrics,
    workflows: &HashMap<String, config::WorkflowConfig>,
    default_retry_max: u32,
    token: &str,
    callback_payload: &str,
) -> Result<bool> {
    // Find and complete the callback job
    let job_id = match db.resume_callback_job(token, callback_payload).await? {
        Some(id) => id,
        None => return Ok(false),
    };

    metrics.inc_callbacks_received();

    // Get the job data to advance the workflow
    let job = db
        .get_callback_job(token)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Callback job not found after resume"))?;

    let wf_data = db.get_workflow_job_data(&job_id).await?;
    let wf_data = match wf_data {
        Some(d) if d.workflow_run_id.is_some() => d,
        _ => return Ok(true),
    };

    let run_id = wf_data.workflow_run_id.as_ref().unwrap();
    let step_index = wf_data.step_index.unwrap_or(0);
    let workflow_name = job.handler.split('/').next().unwrap_or("");

    let workflow = match workflows.get(workflow_name) {
        Some(w) => w,
        None => return Ok(true),
    };

    // Find current step for result_path/output
    let step_name = wf_data.step_name.as_deref().unwrap_or("");
    let current_step = workflow.steps.iter().find(|s| s.name == step_name);

    // Apply result_path merge
    let step_input_payload = wf_data.step_input.as_deref().unwrap_or("{}");
    let merged_payload = if let Some(step) = current_step {
        merge_result_path(
            step_input_payload,
            callback_payload,
            step.result_path.as_deref(),
        )
    } else {
        callback_payload.to_string()
    };

    // Apply output transform
    let output_payload = if let Some(step) = current_step
        && let Some(ref output_template) = step.output
    {
        crate::api::apply_transform(&merged_payload, output_template)
    } else {
        merged_payload
    };

    metrics.inc_workflow_step_completed(workflow_name);

    // Check end
    if current_step.is_some_and(|s| s.end) {
        db.complete_workflow_run(run_id).await?;
        metrics.inc_workflow_completed(workflow_name);
        resume_parent_workflow(
            db,
            metrics,
            workflows,
            default_retry_max,
            run_id,
            &job.event_id,
            &output_payload,
        )
        .await?;
        return Ok(true);
    }

    // Advance to next step
    let next_index = (step_index + 1) as usize;
    if next_index >= workflow.steps.len() {
        db.complete_workflow_run(run_id).await?;
        metrics.inc_workflow_completed(workflow_name);
        resume_parent_workflow(
            db,
            metrics,
            workflows,
            default_retry_max,
            run_id,
            &job.event_id,
            &output_payload,
        )
        .await?;
        return Ok(true);
    }

    let next_step = &workflow.steps[next_index];
    db.update_workflow_run_step(run_id, &next_step.name).await?;

    let next_input = match &next_step.input {
        Some(template) => crate::api::apply_transform(&output_payload, template),
        None => output_payload,
    };

    let next_job_id = ulid::Ulid::new().to_string();
    let next_handler = format!("{}/{}", workflow_name, next_step.name);
    let max_attempts = next_step
        .retry
        .as_ref()
        .map(|r| r.max)
        .unwrap_or(default_retry_max);

    db.insert_workflow_job(
        &next_job_id,
        &job.event_id,
        &next_handler,
        next_step.url.as_deref().unwrap_or(""),
        max_attempts,
        run_id,
        &next_step.name,
        next_index as i32,
        Some(&next_input),
    )
    .await?;

    metrics.inc_jobs_created();
    tracing::info!(
        workflow = workflow_name,
        run_id,
        step = next_step.name,
        "Callback received, advancing workflow"
    );

    Ok(true)
}

/// Handle a sub-workflow step: launch a child workflow.
async fn handle_subworkflow_step(
    db: &Database,
    metrics: &Metrics,
    workflows: &HashMap<String, config::WorkflowConfig>,
    default_retry_max: u32,
    job: &crate::db::JobRow,
    step: &config::StepConfig,
) -> Result<()> {
    let wf_data = db.get_workflow_job_data(&job.id).await?;
    let wf_data = match wf_data {
        Some(d) if d.workflow_run_id.is_some() => d,
        _ => return Ok(()),
    };
    let parent_run_id = wf_data.workflow_run_id.as_ref().unwrap();
    let parent_step_index = wf_data.step_index.unwrap_or(0);
    let payload = wf_data.step_input.as_deref().unwrap_or("{}");
    let parent_workflow_name = job.handler.split('/').next().unwrap_or("");

    let sub_workflow_name = step
        .workflow
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("Sub-workflow step has no workflow name"))?;

    let sub_workflow = workflows
        .get(sub_workflow_name)
        .ok_or_else(|| anyhow::anyhow!("Sub-workflow '{}' not found", sub_workflow_name))?;

    if sub_workflow.steps.is_empty() {
        anyhow::bail!("Sub-workflow '{}' has no steps", sub_workflow_name);
    }

    // Create a new workflow_run for the sub-workflow with parent reference
    let sub_run_id = ulid::Ulid::new().to_string();
    let first_step = &sub_workflow.steps[0];
    db.insert_sub_workflow_run(
        &sub_run_id,
        sub_workflow_name,
        &job.event_id,
        &first_step.name,
        parent_run_id,
        parent_step_index,
    )
    .await?;

    // Apply first step's input transform
    let first_input = match &first_step.input {
        Some(template) => crate::api::apply_transform(payload, template),
        None => payload.to_string(),
    };

    // Create job for the first step
    let first_job_id = ulid::Ulid::new().to_string();
    let first_handler = format!("{}/{}", sub_workflow_name, first_step.name);
    let max_attempts = first_step
        .retry
        .as_ref()
        .map(|r| r.max)
        .unwrap_or(default_retry_max);

    db.insert_workflow_job(
        &first_job_id,
        &job.event_id,
        &first_handler,
        first_step.url.as_deref().unwrap_or(""),
        max_attempts,
        &sub_run_id,
        &first_step.name,
        0,
        Some(&first_input),
    )
    .await?;

    metrics.inc_jobs_created();
    tracing::info!(
        parent_workflow = parent_workflow_name,
        parent_run_id,
        sub_workflow = sub_workflow_name,
        sub_run_id,
        step = first_step.name,
        "Sub-workflow launched"
    );

    Ok(())
}

/// After completing a workflow run, check if it's a sub-workflow and advance the parent.
async fn resume_parent_workflow(
    db: &Database,
    metrics: &Metrics,
    workflows: &HashMap<String, config::WorkflowConfig>,
    default_retry_max: u32,
    run_id: &str,
    event_id: &str,
    output_payload: &str,
) -> Result<()> {
    let parent = db.get_parent_workflow_run(run_id).await?;
    let (parent_run_id, parent_step_index) = match parent {
        Some(p) => p,
        None => return Ok(()), // Not a sub-workflow, nothing to do
    };

    // Get parent workflow info
    let parent_run = db
        .get_workflow_run(&parent_run_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Parent workflow run '{}' not found", parent_run_id))?;
    let parent_workflow_name = &parent_run.workflow;
    let parent_workflow = match workflows.get(parent_workflow_name.as_str()) {
        Some(w) => w,
        None => return Ok(()),
    };

    // Find next step in parent workflow
    let next_index = (parent_step_index + 1) as usize;
    if next_index >= parent_workflow.steps.len() {
        // Parent workflow also completed
        db.complete_workflow_run(&parent_run_id).await?;
        metrics.inc_workflow_completed(parent_workflow_name);
        tracing::info!(
            workflow = parent_workflow_name.as_str(),
            run_id = parent_run_id,
            "Parent workflow completed (sub-workflow was last step)"
        );
        // Recurse: parent may also be a sub-workflow
        return Box::pin(resume_parent_workflow(
            db,
            metrics,
            workflows,
            default_retry_max,
            &parent_run_id,
            event_id,
            output_payload,
        ))
        .await;
    }

    let next_step = &parent_workflow.steps[next_index];
    db.update_workflow_run_step(&parent_run_id, &next_step.name)
        .await?;

    let next_input = match &next_step.input {
        Some(template) => crate::api::apply_transform(output_payload, template),
        None => output_payload.to_string(),
    };

    let next_job_id = ulid::Ulid::new().to_string();
    let next_handler = format!("{}/{}", parent_workflow_name, next_step.name);
    let max_attempts = next_step
        .retry
        .as_ref()
        .map(|r| r.max)
        .unwrap_or(default_retry_max);

    db.insert_workflow_job(
        &next_job_id,
        event_id,
        &next_handler,
        next_step.url.as_deref().unwrap_or(""),
        max_attempts,
        &parent_run_id,
        &next_step.name,
        next_index as i32,
        Some(&next_input),
    )
    .await?;

    metrics.inc_jobs_created();
    tracing::info!(
        parent_workflow = parent_workflow_name.as_str(),
        parent_run_id,
        step = next_step.name,
        "Sub-workflow completed, resuming parent workflow"
    );

    Ok(())
}

/// Check if an error type matches any of the configured error types.
fn error_type_matches(errors: &[config::ErrorType], error_type: &str) -> bool {
    errors.iter().any(|e| match e {
        config::ErrorType::All => true,
        config::ErrorType::Timeout => error_type == "timeout",
        config::ErrorType::Http5xx => error_type == "5xx",
        config::ErrorType::Http4xx => error_type == "4xx",
        config::ErrorType::Network => error_type == "network",
    })
}

async fn deliver(
    db: &Database,
    http: &reqwest::Client,
    job: &crate::db::JobRow,
    transform: Option<&str>,
    custom_headers: Option<&HashMap<String, String>>,
    method: &str,
) -> Result<DeliveryResult> {
    let (raw_payload, headers_json) = db.get_event_data(&job.event_id).await?;

    // Apply transformation if configured
    let payload = match transform {
        Some(template) => crate::api::apply_transform(&raw_payload, template),
        None => raw_payload,
    };

    deliver_http(http, job, &payload, &headers_json, custom_headers, method).await
}

/// Deliver an outbound webhook with HMAC-SHA256 signature.
async fn deliver_outbound(
    db: &Database,
    http: &reqwest::Client,
    job: &crate::db::JobRow,
) -> Result<DeliveryResult> {
    let endpoint_id = job
        .handler
        .strip_prefix("outbound/")
        .unwrap_or(&job.handler);

    let (payload, _headers_json) = db.get_event_data(&job.event_id).await?;

    // Look up the endpoint's signing secret
    let signing_secret = db
        .get_endpoint_secret(endpoint_id)
        .await?
        .unwrap_or_default();

    let timestamp = chrono::Utc::now().timestamp();
    // Use job ID as the message ID for Standard Webhooks
    let msg_id = &job.id;
    let signature = crate::verify::sign_outbound_payload(
        &signing_secret,
        msg_id,
        timestamp,
        payload.as_bytes(),
    );

    // Get event type for the header
    let event_type: Option<String> =
        sqlx::query_as::<_, (String,)>("SELECT event_type FROM events WHERE id = $1")
            .bind(&job.event_id)
            .fetch_optional(&db.pool)
            .await?
            .map(|r| r.0);

    // Standard Webhooks spec headers
    let mut request = http
        .post(&job.url)
        .header("Content-Type", "application/json")
        .header("webhook-id", msg_id.as_str())
        .header("webhook-timestamp", timestamp.to_string())
        .header("webhook-signature", format!("v1,{}", signature))
        // Supplementary qhook headers (not part of Standard Webhooks spec)
        .header("X-Qhook-Event-ID", &job.event_id);

    if let Some(ref et) = event_type {
        request = request.header("X-Qhook-Event-Type", et.as_str());
    }

    let response = request.body(payload).send().await?;
    let retry_after_secs = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_retry_after);
    Ok(DeliveryResult {
        status_code: response.status().as_u16(),
        response_body: None,
        retry_after_secs,
    })
}

/// Parse `Retry-After` header value into seconds.
/// Supports integer seconds (e.g., "120") only. HTTP-date format is not supported.
fn parse_retry_after(value: &str) -> Option<i64> {
    value
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|&s| s > 0 && s <= 86400)
}

async fn deliver_http(
    http: &reqwest::Client,
    job: &crate::db::JobRow,
    payload: &str,
    headers_json: &Option<String>,
    custom_headers: Option<&HashMap<String, String>>,
    method: &str,
) -> Result<DeliveryResult> {
    let reqwest_method = match method {
        "GET" => reqwest::Method::GET,
        "PUT" => reqwest::Method::PUT,
        "PATCH" => reqwest::Method::PATCH,
        "DELETE" => reqwest::Method::DELETE,
        _ => reqwest::Method::POST,
    };

    let mut request = http
        .request(reqwest_method.clone(), &job.url)
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

    // Apply custom headers from handler/step config
    if let Some(ch) = custom_headers {
        for (key, value) in ch {
            request = request.header(key.as_str(), value.as_str());
        }
    }

    // Skip body for GET requests
    let response = if reqwest_method == reqwest::Method::GET {
        request.send().await?
    } else {
        request.body(payload.to_string()).send().await?
    };
    let retry_after_secs = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_retry_after);
    Ok(DeliveryResult {
        status_code: response.status().as_u16(),
        response_body: None,
        retry_after_secs,
    })
}

/// Handle delivery failure. Returns `true` if the job was moved to DLQ.
/// If `retry_after_secs` is provided (from downstream Retry-After header), it overrides
/// the default exponential backoff delay.
async fn handle_failure(
    db: &Database,
    job: &crate::db::JobRow,
    error: &str,
    retry_after_secs: Option<i64>,
) -> Result<bool> {
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
        // Use Retry-After header if provided, otherwise exponential backoff: 30s * 2^attempt
        let backoff_secs =
            retry_after_secs.unwrap_or_else(|| 30i64 * (1i64 << current_attempt.min(10)));
        let next_at = Utc::now().naive_utc() + chrono::Duration::seconds(backoff_secs);
        db.mark_job_retryable(&job.id, next_at, error).await?;
        tracing::info!(
            job_id = job.id,
            handler = job.handler,
            attempt = current_attempt,
            next_retry_secs = backoff_secs,
            retry_after = retry_after_secs.is_some(),
            error,
            "Job scheduled for retry"
        );
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_result_path_default_replaces() {
        let input = r#"{"id": "123", "amount": 5000}"#;
        let response = r#"{"valid": true}"#;
        assert_eq!(merge_result_path(input, response, None), response);
        assert_eq!(merge_result_path(input, response, Some("$")), response);
    }

    #[test]
    fn test_merge_result_path_null_discards() {
        let input = r#"{"id": "123"}"#;
        let response = r#"{"valid": true}"#;
        assert_eq!(merge_result_path(input, response, Some("null")), input);
    }

    #[test]
    fn test_merge_result_path_field_merges() {
        let input = r#"{"id":"123","amount":5000}"#;
        let response = r#"{"valid":true,"score":0.1}"#;
        let result = merge_result_path(input, response, Some("$.validation"));
        let json: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(json["id"], "123");
        assert_eq!(json["amount"], 5000);
        assert_eq!(json["validation"]["valid"], true);
        assert_eq!(json["validation"]["score"], 0.1);
    }

    #[test]
    fn test_error_type_matches_all() {
        assert!(error_type_matches(&[config::ErrorType::All], "timeout"));
        assert!(error_type_matches(&[config::ErrorType::All], "5xx"));
        assert!(error_type_matches(&[config::ErrorType::All], "4xx"));
        assert!(error_type_matches(&[config::ErrorType::All], "network"));
    }

    #[test]
    fn test_error_type_matches_specific() {
        let errors = vec![config::ErrorType::Timeout, config::ErrorType::Http5xx];
        assert!(error_type_matches(&errors, "timeout"));
        assert!(error_type_matches(&errors, "5xx"));
        assert!(!error_type_matches(&errors, "4xx"));
        assert!(!error_type_matches(&errors, "network"));
    }

    #[test]
    fn test_error_type_matches_empty() {
        assert!(!error_type_matches(&[], "timeout"));
    }

    #[test]
    fn test_circuit_breaker_starts_closed() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(60));
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.allow_request());
    }

    #[test]
    fn test_circuit_breaker_opens_after_threshold() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(60));
        assert!(!cb.record_failure()); // 1
        assert!(!cb.record_failure()); // 2
        assert!(cb.record_failure()); // 3 -> opens, returns true
        assert_eq!(cb.state(), CircuitState::Open);
        assert!(!cb.allow_request());
    }

    #[test]
    fn test_circuit_breaker_success_resets() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(60));
        cb.record_failure();
        cb.record_failure();
        cb.record_success();
        assert_eq!(cb.state(), CircuitState::Closed);
        // Should need 3 more failures to open
        assert!(!cb.record_failure());
        assert!(!cb.record_failure());
        assert!(cb.record_failure());
        assert_eq!(cb.state(), CircuitState::Open);
    }

    #[test]
    fn test_circuit_breaker_half_open_after_cooldown() {
        let cb = CircuitBreaker::new(2, Duration::from_millis(1));
        cb.record_failure();
        cb.record_failure(); // opens
        assert_eq!(cb.state(), CircuitState::Open);

        // Wait for cooldown
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        // Only one request allowed in half-open
        assert!(cb.allow_request());
        assert!(!cb.allow_request()); // second blocked
    }

    #[test]
    fn test_circuit_breaker_half_open_success_closes() {
        let cb = CircuitBreaker::new(2, Duration::from_millis(1));
        cb.record_failure();
        cb.record_failure();
        std::thread::sleep(Duration::from_millis(5));

        assert!(cb.allow_request()); // half-open test
        cb.record_success();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.allow_request());
    }

    #[test]
    fn test_circuit_breaker_half_open_failure_reopens() {
        let cb = CircuitBreaker::new(2, Duration::from_millis(1));
        cb.record_failure();
        cb.record_failure();
        std::thread::sleep(Duration::from_millis(5));

        assert!(cb.allow_request()); // half-open test
        cb.record_failure(); // fails again -> reopen
        assert_eq!(cb.state(), CircuitState::Open);
        assert!(!cb.allow_request());
    }

    #[test]
    fn test_parse_retry_after_valid() {
        assert_eq!(parse_retry_after("120"), Some(120));
        assert_eq!(parse_retry_after("1"), Some(1));
        assert_eq!(parse_retry_after(" 30 "), Some(30));
    }

    #[test]
    fn test_parse_retry_after_invalid() {
        assert_eq!(parse_retry_after("0"), None);
        assert_eq!(parse_retry_after("-1"), None);
        assert_eq!(parse_retry_after("abc"), None);
        assert_eq!(parse_retry_after(""), None);
    }

    #[test]
    fn test_parse_retry_after_clamped() {
        // Over 24h is rejected
        assert_eq!(parse_retry_after("86401"), None);
        assert_eq!(parse_retry_after("86400"), Some(86400));
    }
}
