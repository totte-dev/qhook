use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;
use axum::{
    Router,
    body::Bytes,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::IntoResponse,
    routing::post,
};
use serde_json::Value;

use crate::alert::{AlertEvent, Alerter, SharedAlerter};
use crate::config::Config;
use crate::db::Database;
use crate::metrics::Metrics;
use crate::queue::Worker;

pub struct AppState {
    pub config: Config,
    pub db: Arc<Database>,
    pub http: reqwest::Client,
    pub metrics: Arc<Metrics>,
    pub alerter: SharedAlerter,
}

impl AppState {
    pub fn new(config: Config, db: Database) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to build HTTP client");
        let metrics = Arc::new(Metrics::new());
        let alerter = config
            .alerts
            .clone()
            .map(|c| Arc::new(Alerter::new(c, metrics.clone())));
        Self {
            config,
            db: Arc::new(db),
            http,
            metrics,
            alerter,
        }
    }
}

type SharedState = Arc<AppState>;

/// Per-IP rate limiter using a sliding window counter.
/// Tracks request counts per IP within 1-second windows.
/// Bounded to MAX_ENTRIES to prevent memory exhaustion under DDoS.
struct IpRateLimiter {
    limit: u32,
    entries: Mutex<HashMap<IpAddr, (u64, Instant)>>,
}

/// Maximum tracked IPs. Beyond this, new IPs are rate-limited by default.
const MAX_IP_ENTRIES: usize = 100_000;

impl IpRateLimiter {
    fn new(limit: u32) -> Self {
        Self {
            limit,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Returns true if the request is allowed, false if rate-limited.
    fn check(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let mut map = self.entries.lock().unwrap_or_else(|e| e.into_inner());

        // If at capacity and this is a new IP, reject to prevent unbounded growth
        if map.len() >= MAX_IP_ENTRIES && !map.contains_key(&ip) {
            return false;
        }

        let entry = map.entry(ip).or_insert((0, now));
        if now.duration_since(entry.1).as_secs() >= 1 {
            // New window
            entry.0 = 1;
            entry.1 = now;
            true
        } else if entry.0 < self.limit as u64 {
            entry.0 += 1;
            true
        } else {
            false
        }
    }

    /// Remove entries older than 60 seconds to prevent unbounded growth.
    fn cleanup(&self) {
        let now = Instant::now();
        let mut map = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, (_, ts)| now.duration_since(*ts).as_secs() < 60);
    }
}

pub async fn serve(state: AppState, config_path: std::path::PathBuf) -> Result<()> {
    let port = state.config.server.port;
    let shared = Arc::new(state);

    // SIGHUP: validate config without restart (dry-run reload)
    #[cfg(unix)]
    {
        let path = config_path.clone();
        tokio::spawn(async move {
            let mut sig = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
                .expect("failed to listen for SIGHUP");
            loop {
                sig.recv().await;
                match crate::config::Config::load(&path) {
                    Ok(_) => tracing::info!("SIGHUP: config is valid (restart to apply)"),
                    Err(e) => tracing::error!(error = %e, "SIGHUP: config validation failed"),
                }
            }
        });
    }

    // Print registered endpoints
    for (name, source) in &shared.config.sources {
        match source.source_type.as_str() {
            "webhook" => {
                tracing::info!("  POST http://localhost:{port}/webhooks/{name}");
            }
            "sns" => {
                tracing::info!("  POST http://localhost:{port}/sns/{name}");
            }
            _ => {}
        }
    }

    // Warn if auth_token is not configured
    if shared.config.api.auth_token.is_none() {
        tracing::warn!(
            "No auth_token configured — /events endpoint is unauthenticated. \
             Set api.auth_token in config for production use."
        );
    }

    // Shutdown signal for the worker
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let rate_limits: std::collections::HashMap<String, u32> = shared
        .config
        .handlers
        .iter()
        .filter_map(|(name, h)| h.rate_limit.map(|r| (name.clone(), r)))
        .collect();

    let handler_transforms: std::collections::HashMap<String, String> = shared
        .config
        .handlers
        .iter()
        .filter_map(|(name, h)| h.transform.as_ref().map(|t| (name.clone(), t.clone())))
        .collect();

    let handler_types: std::collections::HashMap<String, String> = shared
        .config
        .handlers
        .iter()
        .filter(|(_, h)| h.handler_type != "http")
        .map(|(name, h)| (name.clone(), h.handler_type.clone()))
        .collect();

    let handler_headers: std::collections::HashMap<
        String,
        std::collections::HashMap<String, String>,
    > = shared
        .config
        .handlers
        .iter()
        .filter(|(_, h)| !h.headers.is_empty())
        .map(|(name, h)| (name.clone(), h.headers.clone()))
        .collect();

    let handler_methods: std::collections::HashMap<String, String> = shared
        .config
        .handlers
        .iter()
        .filter(|(_, h)| h.method != "POST")
        .map(|(name, h)| (name.clone(), h.method.clone()))
        .collect();

    let worker = Worker::new(
        shared.db.clone(),
        shared.metrics.clone(),
        shared.alerter.clone(),
        shared.config.worker.clone(),
        shutdown_rx,
        rate_limits,
        handler_transforms,
        handler_types,
        handler_headers,
        handler_methods,
        shared.config.workflows.clone(),
        shared.config.delivery.default_retry.max,
    );
    let worker_handle = tokio::spawn(async move {
        worker.run().await;
    });

    // Spawn cron scheduler if any cron sources exist
    let has_cron = shared
        .config
        .sources
        .values()
        .any(|s| s.source_type == "cron");
    let cron_shutdown_rx = shutdown_tx.subscribe();
    let cron_handle = if has_cron {
        let cron_state = shared.clone();
        Some(tokio::spawn(async move {
            crate::cron::run(cron_state, cron_shutdown_rx).await;
        }))
    } else {
        None
    };

    let body_limit = shared.config.server.max_body_size;
    let max_inbound = shared.config.server.max_inbound;
    let ip_rate_limit = shared.config.server.ip_rate_limit;
    let trust_proxy = shared.config.server.trust_proxy;
    let inbound_semaphore = Arc::new(tokio::sync::Semaphore::new(max_inbound as usize));

    let sem = inbound_semaphore.clone();
    let concurrency_limit = middleware::from_fn(move |req, next: middleware::Next| {
        let sem = sem.clone();
        async move {
            match sem.try_acquire() {
                Ok(_permit) => next.run(req).await,
                Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
            }
        }
    });

    let security_headers = middleware::from_fn(|req, next: middleware::Next| async move {
        let mut resp = next.run(req).await;
        let headers = resp.headers_mut();
        headers.insert("x-content-type-options", "nosniff".parse().unwrap());
        headers.insert("x-frame-options", "DENY".parse().unwrap());
        headers.insert("cache-control", "no-store".parse().unwrap());
        resp
    });

    let mut app = Router::new()
        .route("/webhooks/{source}", post(handle_webhook))
        .route("/events/{event_type}", post(handle_event))
        .route("/sns/{source}", post(handle_sns))
        .route("/callback/{token}", post(handle_callback))
        .route("/health", axum::routing::get(handle_health))
        .route("/metrics", axum::routing::get(handle_metrics))
        .layer(security_headers)
        .layer(concurrency_limit)
        .layer(tower_http::limit::RequestBodyLimitLayer::new(body_limit))
        .with_state(shared);

    // Per-IP rate limiting middleware (if configured)
    if ip_rate_limit > 0 {
        let limiter = Arc::new(IpRateLimiter::new(ip_rate_limit));
        tracing::info!(
            limit = ip_rate_limit,
            trust_proxy,
            "Per-IP rate limiting enabled (req/s)"
        );

        // Background cleanup task
        let cleanup_limiter = limiter.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                cleanup_limiter.cleanup();
            }
        });

        app = app.layer(middleware::from_fn(
            move |req: axum::extract::Request, next: middleware::Next| {
                let limiter = limiter.clone();
                async move {
                    // Extract IP: prefer proxy headers when trust_proxy is enabled
                    let ip = if trust_proxy {
                        req.headers()
                            .get("x-forwarded-for")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.split(',').next())
                            .and_then(|s| s.trim().parse::<IpAddr>().ok())
                            .or_else(|| {
                                req.headers()
                                    .get("x-real-ip")
                                    .and_then(|v| v.to_str().ok())
                                    .and_then(|s| s.trim().parse::<IpAddr>().ok())
                            })
                    } else {
                        None
                    }
                    .or_else(|| {
                        req.extensions()
                            .get::<ConnectInfo<std::net::SocketAddr>>()
                            .map(|ci| ci.0.ip())
                    });

                    match ip {
                        Some(ip) => {
                            if !limiter.check(ip) {
                                tracing::debug!(ip = %ip, "IP rate limited");
                                return StatusCode::TOO_MANY_REQUESTS.into_response();
                            }
                        }
                        None => {
                            // Cannot determine IP — deny to prevent bypass
                            tracing::warn!(
                                "Cannot determine client IP for rate limiting, denying request"
                            );
                            return StatusCode::TOO_MANY_REQUESTS.into_response();
                        }
                    }
                    next.run(req).await
                }
            },
        ));
    }

    let addr = format!("0.0.0.0:{port}");
    tracing::info!("qhook running on :{port}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    // HTTP server stopped, now stop the worker
    tracing::info!("HTTP server stopped, shutting down worker...");
    let _ = shutdown_tx.send(true);
    worker_handle.await?;
    if let Some(handle) = cron_handle {
        handle.await?;
    }

    tracing::info!("qhook stopped");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to listen for ctrl+c");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to listen for SIGTERM")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutdown signal received");
}

async fn handle_webhook(
    State(state): State<SharedState>,
    Path(source_name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Find source config
    let source = match state.config.sources.get(&source_name) {
        Some(s) if s.source_type == "webhook" => s,
        _ => return (StatusCode::NOT_FOUND, "Unknown source".to_string()),
    };

    // Verify signature
    if let Some(verify_provider) = &source.verify {
        let secret = source.secret.as_deref().unwrap_or("");
        match crate::verify::verify_signature(verify_provider, secret, &body, &headers) {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!(source = source_name, "Signature verification failed");
                state.metrics.inc_verification_failure(&source_name);
                if let Some(ref alerter) = state.alerter {
                    alerter.send(AlertEvent::VerificationFailure {
                        source: source_name.clone(),
                    });
                }
                return (StatusCode::UNAUTHORIZED, "Invalid signature".to_string());
            }
            Err(e) => {
                tracing::error!(source = source_name, error = %e, "Verification error");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Verification error".to_string(),
                );
            }
        }
    }

    // Parse payload
    let payload_str = match std::str::from_utf8(&body) {
        Ok(s) => s.to_string(),
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid UTF-8".to_string()),
    };

    // Extract event type (CloudEvents-aware)
    let event_type = extract_event_type(&source_name, &payload_str, &headers);

    // Process event
    match process_event(&state, &source_name, &event_type, &payload_str, &headers).await {
        Ok(created) => {
            if created {
                (StatusCode::OK, "Event received".to_string())
            } else {
                (StatusCode::OK, "Duplicate event ignored".to_string())
            }
        }
        Err(e) => {
            state.metrics.inc_db_errors();
            tracing::error!(error = %e, "Failed to process event");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal error".to_string(),
            )
        }
    }
}

async fn handle_event(
    State(state): State<SharedState>,
    Path(event_type): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Check auth token if configured (constant-time comparison)
    if let Some(expected_token) = &state.config.api.auth_token {
        let provided = headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));

        match provided {
            Some(token) => {
                use subtle::ConstantTimeEq;
                if !bool::from(token.as_bytes().ct_eq(expected_token.as_bytes())) {
                    tracing::warn!(
                        endpoint = "events",
                        "Authentication failed: invalid bearer token"
                    );
                    return (StatusCode::UNAUTHORIZED, "Invalid token".to_string());
                }
            }
            _ => {
                tracing::warn!(
                    endpoint = "events",
                    "Authentication failed: missing bearer token"
                );
                return (StatusCode::UNAUTHORIZED, "Invalid token".to_string());
            }
        }
    }

    let payload_str = match std::str::from_utf8(&body) {
        Ok(s) => s.to_string(),
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid UTF-8".to_string()),
    };

    // CloudEvents ce-type header overrides URL path event type
    let event_type = if let Some(ce_type) = headers.get("ce-type").and_then(|v| v.to_str().ok()) {
        ce_type.to_string()
    } else {
        event_type
    };

    match process_event(&state, "app", &event_type, &payload_str, &headers).await {
        Ok(_) => (StatusCode::ACCEPTED, "Event accepted".to_string()),
        Err(e) => {
            state.metrics.inc_db_errors();
            tracing::error!(error = %e, "Failed to process event");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal error".to_string(),
            )
        }
    }
}

async fn handle_sns(
    State(state): State<SharedState>,
    Path(source_name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Find source config
    let source = match state.config.sources.get(&source_name) {
        Some(s) if s.source_type == "sns" => s,
        _ => return (StatusCode::NOT_FOUND, "Unknown source".to_string()),
    };

    let body_str = match std::str::from_utf8(&body) {
        Ok(s) => s.to_string(),
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid UTF-8".to_string()),
    };

    // Parse SNS message
    let sns_msg: crate::verify::SnsMessage = match serde_json::from_str(&body_str) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to parse SNS message");
            return (StatusCode::BAD_REQUEST, "Invalid SNS message".to_string());
        }
    };

    // Verify SNS signature (can be skipped for LocalStack / testing)
    if source.skip_verify {
        tracing::debug!(source = source_name, "SNS signature verification skipped");
    } else {
        match crate::verify::verify_sns_message(&sns_msg, &state.http).await {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!(source = source_name, "SNS signature verification failed");
                state.metrics.inc_verification_failure(&source_name);
                if let Some(ref alerter) = state.alerter {
                    alerter.send(AlertEvent::VerificationFailure {
                        source: source_name.clone(),
                    });
                }
                return (StatusCode::UNAUTHORIZED, "Invalid signature".to_string());
            }
            Err(e) => {
                tracing::error!(source = source_name, error = %e, "SNS verification error");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Verification error".to_string(),
                );
            }
        }
    }

    // Handle message type
    match sns_msg.message_type.as_str() {
        "SubscriptionConfirmation" => {
            if let Some(ref subscribe_url) = sns_msg.subscribe_url {
                // Validate SubscribeURL to prevent SSRF
                if !crate::verify::is_valid_sns_url(subscribe_url) {
                    tracing::warn!(
                        source = source_name,
                        url = subscribe_url,
                        "Rejected SNS SubscribeURL: not a valid SNS endpoint"
                    );
                    return (StatusCode::BAD_REQUEST, "Invalid SubscribeURL".to_string());
                }
                tracing::info!(
                    source = source_name,
                    topic = sns_msg.topic_arn,
                    "Confirming SNS subscription"
                );
                match state.http.get(subscribe_url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        tracing::info!(source = source_name, "SNS subscription confirmed");
                        (StatusCode::OK, "Subscription confirmed".to_string())
                    }
                    Ok(resp) => {
                        tracing::error!(
                            source = source_name,
                            status = resp.status().as_u16(),
                            "Failed to confirm SNS subscription"
                        );
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Subscription confirmation failed".to_string(),
                        )
                    }
                    Err(e) => {
                        tracing::error!(source = source_name, error = %e, "Failed to confirm SNS subscription");
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Subscription confirmation failed".to_string(),
                        )
                    }
                }
            } else {
                (StatusCode::BAD_REQUEST, "Missing SubscribeURL".to_string())
            }
        }
        "Notification" => {
            // Unwrap the SNS envelope: use Message field as the actual payload
            let payload = &sns_msg.message;
            let event_type = extract_sns_event_type(payload, sns_msg.subject.as_deref());

            match process_event(&state, &source_name, &event_type, payload, &headers).await {
                Ok(created) => {
                    if created {
                        (StatusCode::OK, "Event received".to_string())
                    } else {
                        (StatusCode::OK, "Duplicate event ignored".to_string())
                    }
                }
                Err(e) => {
                    state.metrics.inc_db_errors();
                    tracing::error!(error = %e, "Failed to process SNS event");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal error".to_string(),
                    )
                }
            }
        }
        "UnsubscribeConfirmation" => {
            tracing::info!(
                source = source_name,
                "SNS unsubscribe confirmation received"
            );
            (StatusCode::OK, "Unsubscribe acknowledged".to_string())
        }
        other => {
            tracing::warn!(
                source = source_name,
                message_type = other,
                "Unknown SNS message type"
            );
            (StatusCode::BAD_REQUEST, "Unknown message type".to_string())
        }
    }
}

async fn process_event(
    state: &AppState,
    source: &str,
    event_type: &str,
    payload: &str,
    headers: &HeaderMap,
) -> Result<bool> {
    let event_id = ulid::Ulid::new().to_string();

    // Find matching handlers
    let matching_handlers: Vec<_> = state
        .config
        .handlers
        .iter()
        .filter(|(_, h)| {
            h.source == source
                && (h.events.is_empty() || h.events.iter().any(|e| event_matches(e, event_type)))
        })
        .collect();

    // Find matching workflows
    let matching_workflows: Vec<_> = state
        .config
        .workflows
        .iter()
        .filter(|(_, w)| {
            w.source == source
                && (w.events.is_empty() || w.events.iter().any(|e| event_matches(e, event_type)))
        })
        .collect();

    if matching_handlers.is_empty() && matching_workflows.is_empty() {
        tracing::debug!(source, event_type, "No matching handlers or workflows");
        return Ok(true);
    }

    // Extract idempotency key if configured
    let unique_key = matching_handlers
        .first()
        .and_then(|(_, h)| h.idempotency_key.as_ref())
        .and_then(|path| extract_json_path(payload, path));

    // Serialize relevant headers
    let headers_json = serialize_headers(headers);

    // Insert event
    let created = state
        .db
        .insert_event(
            &event_id,
            source,
            event_type,
            payload,
            Some(&headers_json),
            unique_key.as_deref(),
        )
        .await?;

    state.metrics.inc_events_received_for(source);

    if !created {
        state.metrics.inc_events_duplicated();
        tracing::info!(source, event_type, "Duplicate event");
        return Ok(false);
    }

    // Create jobs for each matching handler (apply filter if configured)
    for (handler_name, handler) in &matching_handlers {
        // Apply JSONPath filter — skip job creation if filter doesn't match
        if let Some(ref filter) = handler.filter {
            if !evaluate_filter(payload, filter) {
                tracing::debug!(handler = *handler_name, filter, "Event filtered out");
                continue;
            }
        }

        let job_id = ulid::Ulid::new().to_string();
        let max_attempts = handler
            .retry
            .as_ref()
            .map(|r| r.max)
            .unwrap_or(state.config.delivery.default_retry.max);

        state
            .db
            .insert_job(&job_id, &event_id, handler_name, &handler.url, max_attempts)
            .await?;

        state.metrics.inc_jobs_created();
        tracing::info!(
            event_id,
            job_id,
            handler = *handler_name,
            event_type,
            "Job created"
        );
    }

    // Start matching workflows
    for (workflow_name, workflow) in &matching_workflows {
        start_workflow(state, workflow_name, workflow, &event_id, payload).await?;
    }

    Ok(true)
}

/// Start a workflow by creating a workflow_run and the first step's job.
pub async fn start_workflow(
    state: &AppState,
    workflow_name: &str,
    workflow: &crate::config::WorkflowConfig,
    event_id: &str,
    payload: &str,
) -> Result<()> {
    // Validate input params if defined
    if !workflow.params.is_empty() {
        validate_workflow_params(workflow_name, &workflow.params, payload)?;
    }

    let first_step = &workflow.steps[0];
    let run_id = ulid::Ulid::new().to_string();

    state
        .db
        .insert_workflow_run(&run_id, workflow_name, event_id, &first_step.name)
        .await?;

    // Set workflow timeout if configured
    if let Some(timeout_secs) = workflow.timeout {
        let timeout_at = (chrono::Utc::now().naive_utc()
            + chrono::Duration::seconds(timeout_secs as i64))
        .format("%Y-%m-%dT%H:%M:%S%.3f")
        .to_string();
        state.db.set_workflow_timeout(&run_id, &timeout_at).await?;
    }

    state.metrics.inc_workflow_started(workflow_name);

    // Create the first step's job
    create_step_job(
        state,
        workflow_name,
        &run_id,
        event_id,
        first_step,
        0,
        payload,
    )
    .await?;

    tracing::info!(
        workflow = workflow_name,
        run_id,
        event_id,
        first_step = first_step.name,
        "Workflow started"
    );
    Ok(())
}

/// Create a job for a workflow step.
async fn create_step_job(
    state: &AppState,
    workflow_name: &str,
    run_id: &str,
    event_id: &str,
    step: &crate::config::StepConfig,
    step_index: i32,
    input_payload: &str,
) -> Result<()> {
    let url = step.url.as_deref().unwrap_or("");
    let max_attempts = step
        .retry
        .as_ref()
        .map(|r| r.max)
        .unwrap_or(state.config.delivery.default_retry.max);

    let job_id = ulid::Ulid::new().to_string();
    let handler_name = format!("{}/{}", workflow_name, step.name);

    // Apply input transform if configured
    let step_input = match &step.input {
        Some(template) => apply_transform(input_payload, template),
        None => input_payload.to_string(),
    };

    state
        .db
        .insert_workflow_job(
            &job_id,
            event_id,
            &handler_name,
            url,
            max_attempts,
            run_id,
            &step.name,
            step_index,
            Some(&step_input),
        )
        .await?;

    state.metrics.inc_jobs_created();
    tracing::info!(
        workflow = workflow_name,
        run_id,
        job_id,
        step = step.name,
        step_index,
        "Workflow step job created"
    );
    Ok(())
}

/// Handle callback webhook: resume a waiting workflow step.
/// The token itself serves as authentication — it is a 160-bit cryptographic random value.
async fn handle_callback(
    State(state): State<SharedState>,
    Path(token): Path<String>,
    body: Bytes,
) -> impl IntoResponse {
    // Reject obviously invalid tokens early (valid tokens are 52 chars: two ULIDs)
    if token.len() < 26 {
        // Return same 404 as invalid/expired to prevent enumeration
        let body = serde_json::json!({"error": "not found"});
        return (StatusCode::NOT_FOUND, axum::Json(body)).into_response();
    }

    let payload = String::from_utf8_lossy(&body).to_string();

    // Build workflow configs map
    let workflows: std::collections::HashMap<String, crate::config::WorkflowConfig> = state
        .config
        .workflows
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let default_retry_max = state.config.delivery.default_retry.max;

    // Log only token prefix to avoid leaking the full secret
    let token_prefix = &token[..token.len().min(8)];

    match crate::queue::resume_callback(
        &state.db,
        &state.metrics,
        &workflows,
        default_retry_max,
        &token,
        &payload,
    )
    .await
    {
        Ok(true) => {
            let body = serde_json::json!({"status": "ok", "message": "callback received"});
            (StatusCode::OK, axum::Json(body)).into_response()
        }
        Ok(false) => {
            tracing::debug!(token_prefix, "Callback token not found or already used");
            // Uniform 404 for invalid, expired, and already-used tokens (no enumeration)
            let body = serde_json::json!({"error": "not found"});
            (StatusCode::NOT_FOUND, axum::Json(body)).into_response()
        }
        Err(e) => {
            tracing::error!(token_prefix, error = %e, "Callback processing failed");
            let body = serde_json::json!({"error": "internal error"});
            (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response()
        }
    }
}

async fn handle_health(State(state): State<SharedState>) -> impl IntoResponse {
    match state.db.queue_depth().await {
        Ok(depth) => {
            let body = serde_json::json!({
                "status": "ok",
                "queue_depth": depth,
            });
            (StatusCode::OK, axum::Json(body)).into_response()
        }
        Err(_) => {
            let body = serde_json::json!({ "status": "error", "detail": "database unreachable" });
            (StatusCode::SERVICE_UNAVAILABLE, axum::Json(body)).into_response()
        }
    }
}

async fn handle_metrics(State(state): State<SharedState>, headers: HeaderMap) -> impl IntoResponse {
    // Check metrics auth token if configured
    if let Some(ref expected_token) = state.config.api.metrics_auth_token {
        let provided = headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));

        match provided {
            Some(token) => {
                use subtle::ConstantTimeEq;
                if !bool::from(token.as_bytes().ct_eq(expected_token.as_bytes())) {
                    return (StatusCode::UNAUTHORIZED, "Invalid token".to_string()).into_response();
                }
            }
            _ => {
                return (StatusCode::UNAUTHORIZED, "Invalid token".to_string()).into_response();
            }
        }
    }

    let queue_depth = state.db.queue_depth().await.unwrap_or(0);
    let dead_jobs = state.db.dead_job_count().await.unwrap_or(0);
    let body = state.metrics.to_prometheus(queue_depth, dead_jobs);

    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
        .into_response()
}

fn extract_event_type(source: &str, payload: &str, headers: &HeaderMap) -> String {
    // CloudEvents binary mode: ce-type header takes precedence
    if let Some(ce_type) = headers.get("ce-type").and_then(|v| v.to_str().ok()) {
        return ce_type.to_string();
    }

    // Parse JSON once and reuse
    let json: Value = serde_json::from_str(payload).unwrap_or(Value::Null);

    // CloudEvents structured mode: application/cloudevents+json
    if let Some(ct) = headers.get("content-type").and_then(|v| v.to_str().ok())
        && ct.contains("application/cloudevents+json")
    {
        if let Some(ce_type) = json.get("type").and_then(|v| v.as_str()) {
            return ce_type.to_string();
        }
    }

    // Provider-specific extraction

    match source {
        // Stripe: { "type": "invoice.paid" }
        "stripe" => json
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        // GitHub: event type is in X-GitHub-Event header, but also check payload
        "github" => json
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("push")
            .to_string(),
        // Shopify: { "topic": "orders/create" } or from header
        "shopify" => json
            .get("topic")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        _ => "event".to_string(),
    }
}

fn extract_sns_event_type(message: &str, subject: Option<&str>) -> String {
    if let Ok(json) = serde_json::from_str::<Value>(message) {
        // CloudEvents structured mode
        if json.get("specversion").is_some()
            && let Some(t) = json.get("type").and_then(|v| v.as_str())
        {
            return t.to_string();
        }
        // Generic type field
        if let Some(t) = json.get("type").and_then(|v| v.as_str()) {
            return t.to_string();
        }
        // AWS EventBridge detail-type (often forwarded via SNS)
        if let Some(t) = json.get("detail-type").and_then(|v| v.as_str()) {
            return t.to_string();
        }
    }

    // Subject as event type
    if let Some(s) = subject
        && !s.is_empty()
    {
        return s.to_string();
    }

    "sns.notification".to_string()
}

pub fn event_matches(pattern: &str, event_type: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix(".*") {
        return event_type.starts_with(prefix);
    }
    pattern == event_type
}

fn extract_json_path(payload: &str, path: &str) -> Option<String> {
    extract_json_path_value(payload, path).and_then(|v| v.as_str().map(|s| s.to_string()))
}

fn extract_json_path_value(payload: &str, path: &str) -> Option<Value> {
    let json: Value = serde_json::from_str(payload).ok()?;
    resolve_path(&json, path)
}

/// Public wrapper for resolve_path (used by queue.rs for map step).
pub fn resolve_path_pub(json: &Value, path: &str) -> Option<Value> {
    resolve_path(json, path)
}

fn resolve_path(json: &Value, path: &str) -> Option<Value> {
    let path = path.strip_prefix("$.").unwrap_or(path);
    let mut current = json;

    for part in path.split('.') {
        current = current.get(part)?;
    }

    Some(current.clone())
}

/// Evaluate a filter expression against a JSON payload.
/// Supported formats:
///   "$.path"           — truthy check (exists and not null/false/0/"")
///   "$.path == value"  — equality
///   "$.path != value"  — inequality
///   "$.path in [a,b]"  — set membership
/// Numeric comparison helper for filter expressions.
fn compare_numeric(payload: &str, path: &str, expected: &str, cmp: fn(f64, f64) -> bool) -> bool {
    let expected_num = match expected.parse::<f64>() {
        Ok(n) => n,
        Err(_) => return false,
    };
    match extract_json_path_value(payload, path) {
        Some(Value::Number(n)) => n.as_f64().is_some_and(|v| cmp(v, expected_num)),
        _ => false,
    }
}

/// Public wrapper for evaluate_filter (used by queue.rs for choice steps).
pub fn evaluate_filter_pub(payload: &str, filter: &str) -> bool {
    evaluate_filter(payload, filter)
}

pub fn evaluate_filter(payload: &str, filter: &str) -> bool {
    let filter = filter.trim();

    // "$.path >= value" (must check before > and ==)
    if let Some((path, value)) = filter.split_once(">=") {
        let path = path.trim();
        let expected = value.trim().trim_matches('"');
        return compare_numeric(payload, path, expected, |a, b| a >= b);
    }

    // "$.path <= value" (must check before < and ==)
    if let Some((path, value)) = filter.split_once("<=") {
        let path = path.trim();
        let expected = value.trim().trim_matches('"');
        return compare_numeric(payload, path, expected, |a, b| a <= b);
    }

    // "$.path > value"
    if let Some((path, value)) = filter.split_once(">") {
        let path = path.trim();
        let expected = value.trim().trim_matches('"');
        return compare_numeric(payload, path, expected, |a, b| a > b);
    }

    // "$.path < value"
    if let Some((path, value)) = filter.split_once("<") {
        let path = path.trim();
        let expected = value.trim().trim_matches('"');
        return compare_numeric(payload, path, expected, |a, b| a < b);
    }

    // "$.path == value"
    if let Some((path, value)) = filter.split_once("==") {
        let path = path.trim();
        let expected = value.trim().trim_matches('"');
        return match extract_json_path_value(payload, path) {
            Some(Value::String(s)) => s == expected,
            Some(Value::Number(n)) => n.to_string() == expected,
            Some(Value::Bool(b)) => b.to_string() == expected,
            _ => false,
        };
    }

    // "$.path != value"
    if let Some((path, value)) = filter.split_once("!=") {
        let path = path.trim();
        let expected = value.trim().trim_matches('"');
        return match extract_json_path_value(payload, path) {
            Some(Value::String(s)) => s != expected,
            Some(Value::Number(n)) => n.to_string() != expected,
            Some(Value::Bool(b)) => b.to_string() != expected,
            Some(Value::Null) => true,
            None => true,
            _ => true,
        };
    }

    // "$.path in [a, b, c]"
    if let Some((path, set_str)) = filter.split_once(" in ") {
        let path = path.trim();
        let set_str = set_str.trim();
        if let Some(inner) = set_str.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let values: Vec<&str> = inner
                .split(',')
                .map(|v| v.trim().trim_matches('"'))
                .collect();
            return match extract_json_path_value(payload, path) {
                Some(Value::String(s)) => values.contains(&s.as_str()),
                Some(Value::Number(n)) => values.contains(&n.to_string().as_str()),
                _ => false,
            };
        }
    }

    // "$.path" — truthy check
    match extract_json_path_value(payload, filter) {
        Some(Value::Null) | None => false,
        Some(Value::Bool(b)) => b,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        _ => true, // arrays, objects are truthy
    }
}

/// Apply a JSON template transformation to a payload.
/// Replaces `{{$.path}}` references with values from the original payload.
/// Validate event payload against workflow input parameters.
fn validate_workflow_params(
    workflow_name: &str,
    params: &[crate::config::ParamConfig],
    payload: &str,
) -> Result<()> {
    let json: Value = serde_json::from_str(payload).map_err(|e| {
        anyhow::anyhow!(
            "workflow '{}' param validation: invalid JSON: {}",
            workflow_name,
            e
        )
    })?;

    let obj = json.as_object().ok_or_else(|| {
        anyhow::anyhow!(
            "workflow '{}' param validation: payload must be a JSON object",
            workflow_name
        )
    })?;

    for param in params {
        match obj.get(&param.name) {
            None if param.required => {
                anyhow::bail!(
                    "workflow '{}' missing required param '{}'",
                    workflow_name,
                    param.name
                );
            }
            None => continue,
            Some(value) => {
                let type_ok = match param.param_type.as_str() {
                    "string" => value.is_string(),
                    "number" => value.is_number(),
                    "boolean" => value.is_boolean(),
                    "object" => value.is_object(),
                    "array" => value.is_array(),
                    _ => true,
                };
                if !type_ok {
                    anyhow::bail!(
                        "workflow '{}' param '{}' expected type '{}', got {:?}",
                        workflow_name,
                        param.name,
                        param.param_type,
                        value
                    );
                }
            }
        }
    }
    Ok(())
}

/// If the template is valid JSON with `{{...}}` placeholders, returns transformed JSON.
/// Otherwise returns the original payload unchanged.
pub fn apply_transform(payload: &str, template: &str) -> String {
    let json: Value = match serde_json::from_str(payload) {
        Ok(v) => v,
        Err(_) => return payload.to_string(),
    };

    let mut result = template.to_string();
    // Find all {{$.path}} references and replace them
    while let Some(start) = result.find("{{") {
        let Some(end) = result[start..].find("}}") else {
            break;
        };
        let placeholder = &result[start + 2..start + end];
        let replacement = match resolve_path(&json, placeholder.trim()) {
            Some(Value::String(s)) => {
                // JSON-escape the string to prevent injection
                let escaped = serde_json::to_string(&s).unwrap_or_else(|_| format!("\"{}\"", s));
                // Remove surrounding quotes — the template controls quoting
                escaped[1..escaped.len() - 1].to_string()
            }
            Some(Value::Number(n)) => n.to_string(),
            Some(Value::Bool(b)) => b.to_string(),
            Some(Value::Null) => "null".to_string(),
            Some(v) => v.to_string(), // arrays/objects as JSON
            None => "null".to_string(),
        };
        result = format!(
            "{}{}{}",
            &result[..start],
            replacement,
            &result[start + end + 2..]
        );
    }

    result
}

fn serialize_headers(headers: &HeaderMap) -> String {
    // Only store CloudEvents and content-type headers to reduce DB bloat
    let map: std::collections::HashMap<String, String> = headers
        .iter()
        .filter_map(|(k, v)| {
            let name = k.as_str();
            if name.starts_with("ce-") || name == "content-type" {
                v.to_str().ok().map(|v| (name.to_string(), v.to_string()))
            } else {
                None
            }
        })
        .collect();

    serde_json::to_string(&map).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    // --- CloudEvents event type extraction ---

    #[test]
    fn test_cloudevents_binary_mode() {
        let mut headers = HeaderMap::new();
        headers.insert("ce-type", "com.example.order.created".parse().unwrap());
        headers.insert("ce-specversion", "1.0".parse().unwrap());

        let result = extract_event_type("my-source", "{}", &headers);
        assert_eq!(result, "com.example.order.created");
    }

    #[test]
    fn test_cloudevents_structured_mode() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-type",
            "application/cloudevents+json".parse().unwrap(),
        );

        let payload = r#"{
            "specversion": "1.0",
            "type": "com.example.user.signup",
            "source": "/myapp",
            "id": "abc-123",
            "data": {"name": "Alice"}
        }"#;

        let result = extract_event_type("my-source", payload, &headers);
        assert_eq!(result, "com.example.user.signup");
    }

    #[test]
    fn test_cloudevents_binary_takes_precedence() {
        let mut headers = HeaderMap::new();
        headers.insert("ce-type", "from.header".parse().unwrap());
        headers.insert(
            "content-type",
            "application/cloudevents+json".parse().unwrap(),
        );

        let payload = r#"{"type": "from.body"}"#;
        let result = extract_event_type("source", payload, &headers);
        assert_eq!(result, "from.header");
    }

    // --- Provider-specific event type extraction ---

    #[test]
    fn test_stripe_event_type() {
        let headers = HeaderMap::new();
        let payload = r#"{"type": "invoice.paid", "id": "evt_123"}"#;
        assert_eq!(
            extract_event_type("stripe", payload, &headers),
            "invoice.paid"
        );
    }

    #[test]
    fn test_github_event_type() {
        let headers = HeaderMap::new();
        let payload = r#"{"action": "opened", "number": 1}"#;
        assert_eq!(extract_event_type("github", payload, &headers), "opened");
    }

    #[test]
    fn test_shopify_event_type() {
        let headers = HeaderMap::new();
        let payload = r#"{"topic": "orders/create"}"#;
        assert_eq!(
            extract_event_type("shopify", payload, &headers),
            "orders/create"
        );
    }

    #[test]
    fn test_unknown_source_event_type() {
        let headers = HeaderMap::new();
        assert_eq!(extract_event_type("custom", "{}", &headers), "event");
    }

    // --- SNS event type extraction ---

    #[test]
    fn test_sns_event_type_from_json_type() {
        let message = r#"{"type": "order.created", "data": {}}"#;
        assert_eq!(extract_sns_event_type(message, None), "order.created");
    }

    #[test]
    fn test_sns_event_type_from_cloudevents() {
        let message = r#"{"specversion": "1.0", "type": "com.example.event", "source": "/"}"#;
        assert_eq!(extract_sns_event_type(message, None), "com.example.event");
    }

    #[test]
    fn test_sns_event_type_from_eventbridge() {
        let message = r#"{"detail-type": "EC2 Instance State-change", "source": "aws.ec2"}"#;
        assert_eq!(
            extract_sns_event_type(message, None),
            "EC2 Instance State-change"
        );
    }

    #[test]
    fn test_sns_event_type_from_subject() {
        let message = "plain text message";
        assert_eq!(
            extract_sns_event_type(message, Some("my-subject")),
            "my-subject"
        );
    }

    #[test]
    fn test_sns_event_type_fallback() {
        assert_eq!(extract_sns_event_type("not json", None), "sns.notification");
    }

    // --- Event matching ---

    #[test]
    fn test_event_matches_exact() {
        assert!(event_matches("order.created", "order.created"));
        assert!(!event_matches("order.created", "order.updated"));
    }

    #[test]
    fn test_event_matches_wildcard() {
        assert!(event_matches("*", "anything"));
        assert!(event_matches("*", ""));
    }

    #[test]
    fn test_event_matches_prefix() {
        assert!(event_matches("order.*", "order.created"));
        assert!(event_matches("order.*", "order.updated"));
        assert!(!event_matches("order.*", "user.created"));
    }

    // --- JSON path extraction ---

    #[test]
    fn test_json_path_simple() {
        let payload = r#"{"id": "evt_123"}"#;
        assert_eq!(extract_json_path(payload, "$.id"), Some("evt_123".into()));
    }

    #[test]
    fn test_json_path_nested() {
        let payload = r#"{"data": {"order": {"id": "ord_456"}}}"#;
        assert_eq!(
            extract_json_path(payload, "$.data.order.id"),
            Some("ord_456".into())
        );
    }

    #[test]
    fn test_json_path_missing() {
        let payload = r#"{"id": "evt_123"}"#;
        assert_eq!(extract_json_path(payload, "$.missing"), None);
    }

    #[test]
    fn test_json_path_without_dollar_prefix() {
        let payload = r#"{"id": "evt_123"}"#;
        assert_eq!(extract_json_path(payload, "id"), Some("evt_123".into()));
    }

    // --- Header serialization ---

    #[test]
    fn test_serialize_headers_includes_ce_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("ce-type", "test.event".parse().unwrap());
        headers.insert("ce-source", "/myapp".parse().unwrap());
        headers.insert("content-type", "application/json".parse().unwrap());

        let json = serialize_headers(&headers);
        let map: std::collections::HashMap<String, String> = serde_json::from_str(&json).unwrap();

        assert_eq!(map.get("ce-type").unwrap(), "test.event");
        assert_eq!(map.get("ce-source").unwrap(), "/myapp");
        assert_eq!(map.get("content-type").unwrap(), "application/json");
    }

    // --- IP rate limiter ---

    #[test]
    fn test_ip_rate_limiter_allows_within_limit() {
        let limiter = IpRateLimiter::new(3);
        let ip: IpAddr = "1.2.3.4".parse().unwrap();
        assert!(limiter.check(ip));
        assert!(limiter.check(ip));
        assert!(limiter.check(ip));
        // 4th request should be rejected
        assert!(!limiter.check(ip));
    }

    #[test]
    fn test_ip_rate_limiter_separate_ips() {
        let limiter = IpRateLimiter::new(1);
        let ip1: IpAddr = "1.2.3.4".parse().unwrap();
        let ip2: IpAddr = "5.6.7.8".parse().unwrap();
        assert!(limiter.check(ip1));
        assert!(!limiter.check(ip1)); // over limit
        assert!(limiter.check(ip2)); // different IP, ok
    }

    // --- Filter evaluation ---

    #[test]
    fn test_filter_equality() {
        let payload = r#"{"status": "paid", "amount": 100}"#;
        assert!(evaluate_filter(payload, r#"$.status == paid"#));
        assert!(evaluate_filter(payload, r#"$.status == "paid""#));
        assert!(!evaluate_filter(payload, r#"$.status == pending"#));
        assert!(evaluate_filter(payload, r#"$.amount == 100"#));
    }

    #[test]
    fn test_filter_inequality() {
        let payload = r#"{"status": "failed"}"#;
        assert!(evaluate_filter(payload, r#"$.status != paid"#));
        assert!(!evaluate_filter(payload, r#"$.status != failed"#));
    }

    #[test]
    fn test_filter_in_set() {
        let payload = r#"{"type": "order.created"}"#;
        assert!(evaluate_filter(
            payload,
            r#"$.type in [order.created, order.updated]"#
        ));
        assert!(!evaluate_filter(payload, r#"$.type in [payment.success]"#));
    }

    #[test]
    fn test_filter_truthy() {
        assert!(evaluate_filter(r#"{"active": true}"#, "$.active"));
        assert!(!evaluate_filter(r#"{"active": false}"#, "$.active"));
        assert!(!evaluate_filter(r#"{"val": null}"#, "$.val"));
        assert!(!evaluate_filter(r#"{"val": ""}"#, "$.val"));
        assert!(evaluate_filter(r#"{"val": "yes"}"#, "$.val"));
        assert!(!evaluate_filter(r#"{}"#, "$.missing"));
        assert!(evaluate_filter(r#"{"n": 42}"#, "$.n"));
        assert!(!evaluate_filter(r#"{"n": 0}"#, "$.n"));
    }

    #[test]
    fn test_filter_nested_path() {
        let payload = r#"{"data": {"object": {"status": "active"}}}"#;
        assert!(evaluate_filter(payload, "$.data.object.status == active"));
        assert!(!evaluate_filter(
            payload,
            "$.data.object.status == inactive"
        ));
    }

    #[test]
    fn test_filter_numeric_comparisons() {
        let payload = r#"{"amount": 5000, "score": 0.8}"#;
        assert!(evaluate_filter(payload, "$.amount >= 5000"));
        assert!(evaluate_filter(payload, "$.amount >= 4999"));
        assert!(!evaluate_filter(payload, "$.amount >= 5001"));
        assert!(evaluate_filter(payload, "$.amount > 4999"));
        assert!(!evaluate_filter(payload, "$.amount > 5000"));
        assert!(evaluate_filter(payload, "$.amount <= 5000"));
        assert!(evaluate_filter(payload, "$.amount < 5001"));
        assert!(!evaluate_filter(payload, "$.amount < 5000"));
        assert!(evaluate_filter(payload, "$.score >= 0.5"));
        assert!(!evaluate_filter(payload, "$.score >= 0.9"));
    }

    // --- Payload transformation ---

    #[test]
    fn test_transform_simple() {
        let payload = r#"{"id": "evt_1", "data": {"name": "Alice", "amount": 42}}"#;
        let template =
            r#"{"event_id": "{{$.id}}", "user": "{{$.data.name}}", "total": {{$.data.amount}}}"#;
        let result = apply_transform(payload, template);
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["event_id"], "evt_1");
        assert_eq!(v["user"], "Alice");
        assert_eq!(v["total"], 42);
    }

    #[test]
    fn test_transform_missing_field() {
        let payload = r#"{"id": "evt_1"}"#;
        let template = r#"{"id": "{{$.id}}", "missing": "{{$.nonexistent}}"}"#;
        let result = apply_transform(payload, template);
        assert!(result.contains("null"));
        assert!(result.contains("evt_1"));
    }

    #[test]
    fn test_transform_passthrough_on_no_placeholders() {
        let payload = r#"{"id": "evt_1"}"#;
        let template = r#"{"static": "value"}"#;
        let result = apply_transform(payload, template);
        assert_eq!(result, r#"{"static": "value"}"#);
    }

    #[test]
    fn test_transform_nested_object() {
        let payload = r#"{"meta": {"tags": ["a", "b"]}}"#;
        let template = r#"{"labels": {{$.meta.tags}}}"#;
        let result = apply_transform(payload, template);
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["labels"], serde_json::json!(["a", "b"]));
    }

    #[test]
    fn test_transform_escapes_special_chars() {
        // Payload with quotes and backslashes in string values
        let payload = r#"{"name": "foo\"bar\\baz"}"#;
        let template = r#"{"user": "{{$.name}}"}"#;
        let result = apply_transform(payload, template);
        // Must produce valid JSON
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["user"], r#"foo"bar\baz"#);
    }

    // --- IP rate limiter ---

    #[test]
    fn test_ip_rate_limiter_max_entries() {
        let limiter = IpRateLimiter::new(100);
        // Fill to MAX_IP_ENTRIES
        for i in 0..MAX_IP_ENTRIES {
            let ip: IpAddr = std::net::Ipv4Addr::from((i as u32).to_be_bytes()).into();
            assert!(limiter.check(ip));
        }
        // New IP beyond cap should be rejected
        let new_ip: IpAddr = "255.255.255.255".parse().unwrap();
        assert!(!limiter.check(new_ip));
    }

    // --- Workflow param validation ---

    #[test]
    fn test_validate_params_required_present() {
        let params = vec![crate::config::ParamConfig {
            name: "tenant_id".into(),
            param_type: "string".into(),
            required: true,
        }];
        let payload = r#"{"tenant_id": "t-123"}"#;
        assert!(validate_workflow_params("test", &params, payload).is_ok());
    }

    #[test]
    fn test_validate_params_required_missing() {
        let params = vec![crate::config::ParamConfig {
            name: "tenant_id".into(),
            param_type: "string".into(),
            required: true,
        }];
        let payload = r#"{"other": "value"}"#;
        let err = validate_workflow_params("test", &params, payload).unwrap_err();
        assert!(err.to_string().contains("missing required param"));
    }

    #[test]
    fn test_validate_params_optional_missing() {
        let params = vec![crate::config::ParamConfig {
            name: "region".into(),
            param_type: "string".into(),
            required: false,
        }];
        let payload = r#"{}"#;
        assert!(validate_workflow_params("test", &params, payload).is_ok());
    }

    #[test]
    fn test_validate_params_type_mismatch() {
        let params = vec![crate::config::ParamConfig {
            name: "count".into(),
            param_type: "number".into(),
            required: true,
        }];
        let payload = r#"{"count": "not-a-number"}"#;
        let err = validate_workflow_params("test", &params, payload).unwrap_err();
        assert!(err.to_string().contains("expected type 'number'"));
    }

    #[test]
    fn test_validate_params_multiple() {
        let params = vec![
            crate::config::ParamConfig {
                name: "tenant_id".into(),
                param_type: "string".into(),
                required: true,
            },
            crate::config::ParamConfig {
                name: "config".into(),
                param_type: "object".into(),
                required: true,
            },
            crate::config::ParamConfig {
                name: "tags".into(),
                param_type: "array".into(),
                required: false,
            },
        ];
        let payload = r#"{"tenant_id": "t-1", "config": {"key": "val"}}"#;
        assert!(validate_workflow_params("test", &params, payload).is_ok());
    }

    #[test]
    fn test_filter_inequality_missing_field() {
        // != with missing field returns true (field doesn't equal the value)
        assert!(evaluate_filter(r#"{"a": 1}"#, "$.missing != x"));
    }

    #[test]
    fn test_filter_in_numeric() {
        assert!(evaluate_filter(
            r#"{"code": 200}"#,
            "$.code in [200, 201, 202]"
        ));
        assert!(!evaluate_filter(r#"{"code": 404}"#, "$.code in [200, 201]"));
    }

    #[test]
    fn test_validate_params_invalid_json() {
        let params = vec![crate::config::ParamConfig {
            name: "id".into(),
            param_type: "string".into(),
            required: true,
        }];
        assert!(validate_workflow_params("test", &params, "not json").is_err());
    }

    #[test]
    fn test_validate_params_boolean_type() {
        let params = vec![crate::config::ParamConfig {
            name: "enabled".into(),
            param_type: "boolean".into(),
            required: true,
        }];
        assert!(validate_workflow_params("test", &params, r#"{"enabled": true}"#).is_ok());
        assert!(validate_workflow_params("test", &params, r#"{"enabled": "yes"}"#).is_err());
    }
}
