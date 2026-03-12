use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;
use axum::{
    Router,
    body::Bytes,
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::IntoResponse,
    routing::{delete, get, post},
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

pub async fn serve(state: AppState, config_path: String) -> Result<()> {
    let port = state.config.server.port;
    let shared = Arc::new(state);
    let is_remote = crate::config::Config::is_remote(&config_path);

    // SIGHUP: validate config and log changes (restart to apply)
    #[cfg(unix)]
    {
        let path = config_path.clone();
        let current_cfg = shared.config.clone();
        tokio::spawn(async move {
            let mut sig = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
                .expect("failed to listen for SIGHUP");
            loop {
                sig.recv().await;
                let result = if crate::config::Config::is_remote(&path) {
                    crate::config::Config::load_from(&path, None).await
                } else {
                    crate::config::Config::load(std::path::Path::new(&path))
                };
                match result {
                    Ok(new_cfg) => {
                        log_config_diff(&current_cfg, &new_cfg);
                        tracing::info!("SIGHUP: config is valid (restart to apply)");
                    }
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
            "event" => {
                tracing::info!("  POST http://localhost:{port}/events/{name}/{{event_type}}");
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
        handler_headers,
        handler_methods,
        shared.config.workflows.clone(),
        shared.config.delivery.default_retry.max,
        shared.config.handlers.keys().cloned().collect(),
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

    // Remote config polling (S3/HTTP only)
    if is_remote {
        let poll_path = config_path.clone();
        let current_cfg = shared.config.clone();
        let poll_alerter = shared.alerter.clone();
        tokio::spawn(async move {
            // Get initial ETag
            let mut last_etag: Option<String> =
                match crate::config::Config::fetch_remote(&poll_path).await {
                    Ok((_, etag)) => etag,
                    Err(_) => None,
                };
            let poll_interval = std::time::Duration::from_secs(30);
            tracing::info!(
                path = %poll_path,
                interval = ?poll_interval,
                "Remote config polling enabled"
            );
            loop {
                tokio::time::sleep(poll_interval).await;
                match crate::config::Config::fetch_remote(&poll_path).await {
                    Ok((content, new_etag)) => {
                        if new_etag == last_etag && last_etag.is_some() {
                            continue; // No change
                        }
                        match crate::config::Config::from_yaml(&content) {
                            Ok(new_cfg) => {
                                // Check for restart-required changes
                                if new_cfg.server.port != current_cfg.server.port
                                    || new_cfg.database.driver != current_cfg.database.driver
                                {
                                    tracing::warn!(
                                        "Remote config changed port or database driver — restart required to apply"
                                    );
                                    last_etag = new_etag;
                                    continue;
                                }
                                log_config_diff(&current_cfg, &new_cfg);
                                tracing::info!("Remote config updated (restart to apply)");
                                last_etag = new_etag;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    error = %e,
                                    "Remote config validation failed — keeping current config"
                                );
                                if let Some(ref alerter) = poll_alerter {
                                    alerter.send(crate::alert::AlertEvent::Custom(format!(
                                        "Config validation failed: {}",
                                        e
                                    )));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "Failed to fetch remote config — keeping current config"
                        );
                    }
                }
            }
        });
    }

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
        headers.insert("x-api-version", "1".parse().unwrap());
        resp
    });

    let mut app = Router::new()
        .route("/webhooks/{source}", post(handle_webhook))
        .route("/events/{source}/{event_type}", post(handle_event))
        .route("/sns/{source}", post(handle_sns))
        .route("/callback/{token}", post(handle_callback))
        .route("/api/events", get(handle_list_events))
        .route("/api/events/{event_id}", get(handle_get_event))
        .route("/api/events/{event_id}/jobs", get(handle_list_event_jobs))
        .route("/api/jobs", get(handle_list_jobs))
        .route("/api/jobs/{job_id}", get(handle_get_job))
        .route("/api/jobs/{job_id}/attempts", get(handle_list_job_attempts))
        // Outbound webhook management
        .route(
            "/api/outbound/endpoints",
            post(handle_create_endpoint).get(handle_list_endpoints),
        )
        .route(
            "/api/outbound/endpoints/{endpoint_id}",
            get(handle_get_endpoint)
                .put(handle_update_endpoint)
                .delete(handle_delete_endpoint),
        )
        .route(
            "/api/outbound/endpoints/{endpoint_id}/rotate-secret",
            post(handle_rotate_secret),
        )
        .route(
            "/api/outbound/endpoints/{endpoint_id}/subscriptions",
            post(handle_create_subscriptions).get(handle_list_subscriptions),
        )
        .route(
            "/api/outbound/endpoints/{endpoint_id}/subscriptions/{subscription_id}",
            delete(handle_delete_subscription),
        )
        .route("/_echo", axum::routing::any(handle_echo))
        .route("/health", axum::routing::get(handle_health))
        .route("/metrics", axum::routing::get(handle_metrics))
        .layer(tower_http::compression::CompressionLayer::new())
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
    req: axum::extract::Request,
) -> axum::response::Response {
    let connect_info = req
        .extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .map(|ci| ci.0);
    let (parts, body_stream) = req.into_parts();
    let headers = parts.headers;
    let source_name = match parts.uri.path().strip_prefix("/webhooks/") {
        Some(name) => name.to_string(),
        None => return (StatusCode::NOT_FOUND, "Unknown source".to_string()).into_response(),
    };
    let body = match axum::body::to_bytes(
        axum::body::Body::new(body_stream),
        state.config.server.max_body_size,
    )
    .await
    {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                "Payload too large".to_string(),
            )
                .into_response();
        }
    };

    // Find source config
    let source = match state.config.sources.get(&source_name) {
        Some(s) if s.source_type == "webhook" => s,
        _ => return (StatusCode::NOT_FOUND, "Unknown source".to_string()).into_response(),
    };

    // IP allowlist check
    if !source.allowed_ips.is_empty() {
        let client_ip = if state.config.server.trust_proxy {
            headers
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(',').next())
                .and_then(|s| s.trim().parse::<std::net::IpAddr>().ok())
                .or_else(|| {
                    headers
                        .get("x-real-ip")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.trim().parse::<std::net::IpAddr>().ok())
                })
        } else {
            None
        }
        .or_else(|| connect_info.map(|ci| ci.ip()));

        match client_ip {
            Some(ip) if source.is_ip_allowed(ip) => {}
            Some(ip) => {
                tracing::warn!(source = source_name, ip = %ip, "IP not in allowlist");
                return (StatusCode::FORBIDDEN, "IP not allowed".to_string()).into_response();
            }
            None => {
                tracing::warn!(
                    source = source_name,
                    "Cannot determine client IP for allowlist check"
                );
                return (StatusCode::FORBIDDEN, "IP not allowed".to_string()).into_response();
            }
        }
    }

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
                return (StatusCode::UNAUTHORIZED, "Invalid signature".to_string()).into_response();
            }
            Err(e) => {
                tracing::error!(source = source_name, error = %e, "Verification error");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Verification error".to_string(),
                )
                    .into_response();
            }
        }
    }

    // Parse payload
    let payload_str = match String::from_utf8(body.to_vec()) {
        Ok(s) => s,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid UTF-8".to_string()).into_response(),
    };

    // Validate against schema if configured
    if let Some(ref schema) = source.schema {
        if let Err(e) = validate_event_schema(&payload_str, schema) {
            tracing::warn!(source = source_name, error = %e, "Schema validation failed");
            return (
                StatusCode::BAD_REQUEST,
                format!("Schema validation failed: {e}"),
            )
                .into_response();
        }
    }

    // Extract event type (CloudEvents-aware)
    let event_type = extract_event_type(&source_name, &payload_str, &headers);

    // Process event
    match process_event(&state, &source_name, &event_type, &payload_str, &headers).await {
        Ok(result) => {
            let body = serde_json::json!({
                "event_id": result.event_id,
                "duplicate": !result.created,
                "jobs_created": result.jobs_created,
            });
            (StatusCode::OK, axum::Json(body)).into_response()
        }
        Err(e) => {
            state.metrics.inc_db_errors();
            tracing::error!(error = %e, "Failed to process event");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal error".to_string(),
            )
                .into_response()
        }
    }
}

async fn handle_event(
    State(state): State<SharedState>,
    Path((source, event_type)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    // Validate that source exists and is type "event" or "outbound"
    match state.config.sources.get(&source) {
        Some(s) if s.source_type == "event" || s.source_type == "outbound" => {}
        _ => {
            return (StatusCode::NOT_FOUND, "Unknown event source".to_string()).into_response();
        }
    }

    let source = source.as_str();
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
                    return (StatusCode::UNAUTHORIZED, "Invalid token".to_string()).into_response();
                }
            }
            _ => {
                tracing::warn!(
                    endpoint = "events",
                    "Authentication failed: missing bearer token"
                );
                return (StatusCode::UNAUTHORIZED, "Invalid token".to_string()).into_response();
            }
        }
    }

    let payload_str = match String::from_utf8(body.to_vec()) {
        Ok(s) => s,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid UTF-8".to_string()).into_response(),
    };

    // Validate against schema if configured on the specific source
    if let Some(src_config) = state.config.sources.get(source) {
        if let Some(ref schema) = src_config.schema {
            if let Err(e) = validate_event_schema(&payload_str, schema) {
                tracing::warn!(error = %e, "Schema validation failed");
                return (
                    StatusCode::BAD_REQUEST,
                    format!("Schema validation failed: {e}"),
                )
                    .into_response();
            }
        }
    }

    // CloudEvents ce-type header overrides URL path event type
    let event_type = if let Some(ce_type) = headers.get("ce-type").and_then(|v| v.to_str().ok()) {
        ce_type.to_string()
    } else {
        event_type
    };

    match process_event(&state, source, &event_type, &payload_str, &headers).await {
        Ok(result) => {
            let body = serde_json::json!({
                "event_id": result.event_id,
                "duplicate": !result.created,
                "jobs_created": result.jobs_created,
            });
            (StatusCode::ACCEPTED, axum::Json(body)).into_response()
        }
        Err(e) => {
            state.metrics.inc_db_errors();
            tracing::error!(error = %e, "Failed to process event");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal error".to_string(),
            )
                .into_response()
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

    let body_str = match String::from_utf8(body.to_vec()) {
        Ok(s) => s,
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
                Ok(result) => {
                    let body = serde_json::json!({
                        "event_id": result.event_id,
                        "duplicate": !result.created,
                        "jobs_created": result.jobs_created,
                    });
                    (
                        StatusCode::OK,
                        serde_json::to_string(&body).unwrap_or_default(),
                    )
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

struct EventResult {
    event_id: String,
    created: bool,
    jobs_created: u32,
}

async fn process_event(
    state: &AppState,
    source: &str,
    event_type: &str,
    payload: &str,
    headers: &HeaderMap,
) -> Result<EventResult> {
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

    // Check if this is an outbound source — endpoints come from DB, not config
    let is_outbound = state
        .config
        .sources
        .get(source)
        .is_some_and(|s| s.source_type == "outbound");

    if !is_outbound && matching_handlers.is_empty() && matching_workflows.is_empty() {
        tracing::debug!(source, event_type, "No matching handlers or workflows");
        return Ok(EventResult {
            event_id,
            created: true,
            jobs_created: 0,
        });
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
        return Ok(EventResult {
            event_id,
            created: false,
            jobs_created: 0,
        });
    }

    let mut jobs_created: u32 = 0;

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

        jobs_created += 1;
        state.metrics.inc_jobs_created();
        tracing::info!(
            event_id,
            job_id,
            handler = *handler_name,
            event_type,
            "Job created"
        );
    }

    // Create jobs for outbound endpoints (dynamically registered via API)
    if is_outbound {
        match state.db.find_subscribed_endpoints(source, event_type).await {
            Ok(endpoints) => {
                for ep in &endpoints {
                    let job_id = ulid::Ulid::new().to_string();
                    let handler_name = format!("outbound/{}", ep.id);
                    let max_attempts = state.config.delivery.default_retry.max;

                    if let Err(e) = state
                        .db
                        .insert_job(&job_id, &event_id, &handler_name, &ep.url, max_attempts)
                        .await
                    {
                        tracing::error!(error = %e, endpoint_id = ep.id, "Failed to create outbound job");
                        continue;
                    }

                    jobs_created += 1;
                    state.metrics.inc_jobs_created();
                    tracing::info!(
                        event_id,
                        job_id,
                        handler = handler_name,
                        endpoint_url = ep.url,
                        event_type,
                        "Outbound job created"
                    );
                }
            }
            Err(e) => {
                tracing::error!(error = %e, source, "Failed to find subscribed endpoints");
            }
        }
    }

    // Start matching workflows
    for (workflow_name, workflow) in &matching_workflows {
        start_workflow(state, workflow_name, workflow, &event_id, payload).await?;
    }

    Ok(EventResult {
        event_id,
        created,
        jobs_created,
    })
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

/// Check Bearer auth for management API endpoints.
fn check_api_auth(state: &AppState, headers: &HeaderMap) -> Option<axum::response::Response> {
    if let Some(expected_token) = &state.config.api.auth_token {
        let provided = headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));

        match provided {
            Some(token) => {
                use subtle::ConstantTimeEq;
                if !bool::from(token.as_bytes().ct_eq(expected_token.as_bytes())) {
                    return Some(
                        (StatusCode::UNAUTHORIZED, "Invalid token".to_string()).into_response(),
                    );
                }
            }
            _ => {
                return Some(
                    (StatusCode::UNAUTHORIZED, "Invalid token".to_string()).into_response(),
                );
            }
        }
    }
    None
}

async fn handle_list_events(
    State(state): State<SharedState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Some(resp) = check_api_auth(&state, &headers) {
        return resp;
    }

    let source = params.get("source").map(|s| s.as_str());
    let event_type = params.get("event_type").map(|s| s.as_str());
    let since = params.get("since").map(|s| s.as_str());
    let until = params.get("until").map(|s| s.as_str());
    let after = params.get("after").map(|s| s.as_str());
    let limit: i32 = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50)
        .min(1000);

    // Fetch limit+1 to determine has_more
    let events = match state
        .db
        .list_events_for_api(source, event_type, since, until, limit + 1, after)
        .await
    {
        Ok(e) => e,
        Err(e) => {
            tracing::error!(error = %e, "Failed to list events");
            let body = serde_json::json!({"error": "internal error"});
            return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response();
        }
    };

    let has_more = events.len() > limit as usize;
    let events: Vec<_> = events.into_iter().take(limit as usize).collect();

    let body = serde_json::json!({
        "events": events.iter().map(|e| serde_json::json!({
            "id": e.id,
            "source": e.source,
            "event_type": e.event_type,
            "unique_key": e.unique_key,
            "created_at": e.created_at,
        })).collect::<Vec<_>>(),
        "has_more": has_more,
    });
    (StatusCode::OK, axum::Json(body)).into_response()
}

async fn handle_list_event_jobs(
    State(state): State<SharedState>,
    Path(event_id): Path<String>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Some(resp) = check_api_auth(&state, &headers) {
        return resp;
    }

    let jobs = match state.db.list_jobs_by_event(&event_id).await {
        Ok(j) => j,
        Err(e) => {
            tracing::error!(error = %e, "Failed to list jobs for event");
            let body = serde_json::json!({"error": "internal error"});
            return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response();
        }
    };

    let body = serde_json::json!({
        "jobs": jobs.iter().map(|j| serde_json::json!({
            "id": j.id,
            "handler": j.handler,
            "status": j.status,
            "attempt": j.attempt,
            "max_attempts": j.max_attempts,
            "scheduled_at": j.scheduled_at,
            "last_error": j.last_error,
            "created_at": j.created_at,
        })).collect::<Vec<_>>(),
    });
    (StatusCode::OK, axum::Json(body)).into_response()
}

async fn handle_list_jobs(
    State(state): State<SharedState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Some(resp) = check_api_auth(&state, &headers) {
        return resp;
    }

    let status = params.get("status").map(|s| s.as_str());
    let handler = params.get("handler").map(|s| s.as_str());
    let after = params.get("after").map(|s| s.as_str());
    let limit: i32 = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50)
        .min(1000);

    let jobs = match state
        .db
        .list_jobs_filtered(status, handler, limit + 1, after)
        .await
    {
        Ok(j) => j,
        Err(e) => {
            tracing::error!(error = %e, "Failed to list jobs");
            let body = serde_json::json!({"error": "internal error"});
            return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response();
        }
    };

    let has_more = jobs.len() > limit as usize;
    let jobs: Vec<_> = jobs.into_iter().take(limit as usize).collect();

    let body = serde_json::json!({
        "jobs": jobs.iter().map(|j| serde_json::json!({
            "id": j.id,
            "event_id": j.event_id,
            "handler": j.handler,
            "status": j.status,
            "attempt": j.attempt,
            "max_attempts": j.max_attempts,
            "scheduled_at": j.scheduled_at,
            "last_error": j.last_error,
            "created_at": j.created_at,
        })).collect::<Vec<_>>(),
        "has_more": has_more,
    });
    (StatusCode::OK, axum::Json(body)).into_response()
}

async fn handle_list_job_attempts(
    State(state): State<SharedState>,
    Path(job_id): Path<String>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Some(resp) = check_api_auth(&state, &headers) {
        return resp;
    }

    let attempts = match state.db.list_job_attempts(&job_id).await {
        Ok(a) => a,
        Err(e) => {
            tracing::error!(error = %e, "Failed to list job attempts");
            let body = serde_json::json!({"error": "internal error"});
            return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response();
        }
    };

    let body = serde_json::json!({
        "attempts": attempts.iter().map(|a| serde_json::json!({
            "attempt": a.attempt,
            "status_code": a.status_code,
            "error": a.error,
            "duration_ms": a.duration_ms,
            "created_at": a.created_at,
        })).collect::<Vec<_>>(),
    });
    (StatusCode::OK, axum::Json(body)).into_response()
}

async fn handle_get_event(
    State(state): State<SharedState>,
    Path(event_id): Path<String>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Some(resp) = check_api_auth(&state, &headers) {
        return resp;
    }

    let event = match state.db.get_event_by_id(&event_id).await {
        Ok(Some(e)) => e,
        Ok(None) => {
            let body = serde_json::json!({"error": "event not found"});
            return (StatusCode::NOT_FOUND, axum::Json(body)).into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to get event");
            let body = serde_json::json!({"error": "internal error"});
            return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response();
        }
    };

    let jobs = state
        .db
        .list_jobs_by_event(&event_id)
        .await
        .unwrap_or_default();
    let workflows = state
        .db
        .list_workflow_runs_by_event(&event_id)
        .await
        .unwrap_or_default();

    let body = serde_json::json!({
        "id": event.id,
        "source": event.source,
        "event_type": event.event_type,
        "payload": serde_json::from_str::<Value>(&event.payload).unwrap_or(Value::String(event.payload.clone())),
        "headers": event.headers.as_deref().and_then(|h| serde_json::from_str::<Value>(h).ok()),
        "unique_key": event.unique_key,
        "created_at": event.created_at,
        "jobs": jobs.iter().map(|j| serde_json::json!({
            "id": j.id,
            "handler": j.handler,
            "url": j.url,
            "status": j.status,
            "attempt": j.attempt,
            "max_attempts": j.max_attempts,
            "last_error": j.last_error,
        })).collect::<Vec<_>>(),
        "workflow_runs": workflows.iter().map(|w| serde_json::json!({
            "id": w.id,
            "workflow": w.workflow,
            "status": w.status,
            "current_step": w.current_step,
            "created_at": w.created_at,
            "completed_at": w.completed_at,
        })).collect::<Vec<_>>(),
    });
    (StatusCode::OK, axum::Json(body)).into_response()
}

#[derive(serde::Deserialize)]
struct GetJobQuery {
    #[serde(default)]
    include_attempts: bool,
}

async fn handle_get_job(
    State(state): State<SharedState>,
    Path(job_id): Path<String>,
    Query(query): Query<GetJobQuery>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Some(resp) = check_api_auth(&state, &headers) {
        return resp;
    }

    let job_row = match state.db.get_job_by_id(&job_id).await {
        Ok(Some(j)) => j,
        Ok(None) => {
            let body = serde_json::json!({"error": "job not found"});
            return (StatusCode::NOT_FOUND, axum::Json(body)).into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to get job");
            let body = serde_json::json!({"error": "internal error"});
            return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response();
        }
    };

    let mut body = serde_json::json!({
        "id": job_row.id,
        "event_id": job_row.event_id,
        "handler": job_row.handler,
        "url": job_row.url,
        "status": job_row.status,
        "attempt": job_row.attempt,
        "max_attempts": job_row.max_attempts,
        "scheduled_at": job_row.scheduled_at,
        "last_error": job_row.last_error,
    });

    if query.include_attempts {
        let attempts = state
            .db
            .list_job_attempts(&job_id)
            .await
            .unwrap_or_default();
        body["attempts"] = serde_json::json!(
            attempts
                .iter()
                .map(|a| serde_json::json!({
                    "attempt": a.attempt,
                    "status_code": a.status_code,
                    "error": a.error,
                    "duration_ms": a.duration_ms,
                }))
                .collect::<Vec<_>>()
        );
    }

    // Include workflow data if present
    if let Ok(Some(wf_data)) = state.db.get_workflow_job_data(&job_id).await {
        body["workflow_run_id"] = serde_json::json!(wf_data.workflow_run_id);
        body["step_name"] = serde_json::json!(wf_data.step_name);
        body["step_index"] = serde_json::json!(wf_data.step_index);
    }

    (StatusCode::OK, axum::Json(body)).into_response()
}

async fn handle_echo(
    headers: HeaderMap,
    body: Bytes,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    String,
) {
    let body_value = match serde_json::from_slice::<Value>(&body) {
        Ok(v) => v,
        Err(_) => Value::String(String::from_utf8_lossy(&body).into_owned()),
    };

    let mut header_map = serde_json::Map::new();
    for (name, value) in &headers {
        if let Ok(v) = value.to_str() {
            header_map.insert(name.to_string(), Value::String(v.to_string()));
        }
    }

    let response = serde_json::json!({
        "headers": header_map,
        "body": body_value,
    });

    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        response.to_string(),
    )
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

/// Log differences between current and new config on SIGHUP.
fn log_config_diff(current: &Config, new: &Config) {
    // Sources
    for name in new.sources.keys() {
        if !current.sources.contains_key(name) {
            tracing::info!(source = name.as_str(), "SIGHUP: source added");
        }
    }
    for name in current.sources.keys() {
        if !new.sources.contains_key(name) {
            tracing::info!(source = name.as_str(), "SIGHUP: source removed");
        }
    }

    // Handlers
    for (name, h) in &new.handlers {
        if let Some(old_h) = current.handlers.get(name) {
            if old_h.url != h.url {
                tracing::info!(
                    handler = name.as_str(),
                    old = old_h.url.as_str(),
                    new = h.url.as_str(),
                    "SIGHUP: handler URL changed"
                );
            }
        } else {
            tracing::info!(handler = name.as_str(), "SIGHUP: handler added");
        }
    }
    for name in current.handlers.keys() {
        if !new.handlers.contains_key(name) {
            tracing::info!(handler = name.as_str(), "SIGHUP: handler removed");
        }
    }

    // Workflows
    for name in new.workflows.keys() {
        if !current.workflows.contains_key(name) {
            tracing::info!(workflow = name.as_str(), "SIGHUP: workflow added");
        }
    }
    for name in current.workflows.keys() {
        if !new.workflows.contains_key(name) {
            tracing::info!(workflow = name.as_str(), "SIGHUP: workflow removed");
        }
    }

    // Server port change
    if current.server.port != new.server.port {
        tracing::warn!(
            old = current.server.port,
            new = new.server.port,
            "SIGHUP: server.port changed (requires restart)"
        );
    }

    // Database change
    if current.database.driver != new.database.driver {
        tracing::warn!(
            old = current.database.driver.as_str(),
            new = new.database.driver.as_str(),
            "SIGHUP: database.driver changed (requires restart)"
        );
    }
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

    // "not <expr>" — logical negation (must check before comparison operators)
    if let Some(inner) = filter.strip_prefix("not ") {
        return !evaluate_filter(payload, inner.trim());
    }

    // "$.path exists" — field existence check (must check before comparison operators)
    if let Some(path) = filter.strip_suffix(" exists") {
        let path = path.trim();
        return extract_json_path_value(payload, path).is_some();
    }

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

    // "$.path contains value" — substring or array membership
    if let Some((path, value)) = filter.split_once(" contains ") {
        let path = path.trim();
        let needle = value.trim().trim_matches('"');
        return match extract_json_path_value(payload, path) {
            Some(Value::String(s)) => s.contains(needle),
            Some(Value::Array(arr)) => arr.iter().any(|v| match v {
                Value::String(s) => s == needle,
                Value::Number(n) => n.to_string() == needle,
                _ => false,
            }),
            _ => false,
        };
    }

    // "$.path starts_with value"
    if let Some((path, value)) = filter.split_once(" starts_with ") {
        let path = path.trim();
        let prefix = value.trim().trim_matches('"');
        return match extract_json_path_value(payload, path) {
            Some(Value::String(s)) => s.starts_with(prefix),
            _ => false,
        };
    }

    // "$.path ends_with value"
    if let Some((path, value)) = filter.split_once(" ends_with ") {
        let path = path.trim();
        let suffix = value.trim().trim_matches('"');
        return match extract_json_path_value(payload, path) {
            Some(Value::String(s)) => s.ends_with(suffix),
            _ => false,
        };
    }

    // "$.path matches <regex>"
    if let Some((path, pattern)) = filter.split_once(" matches ") {
        let path = path.trim();
        let pattern = pattern.trim();
        return match extract_json_path_value(payload, path) {
            Some(Value::String(s)) => {
                regex_lite::Regex::new(pattern).is_ok_and(|re| re.is_match(&s))
            }
            _ => false,
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

/// Validate event payload against a JSON Schema (subset).
/// Supports: type, required, properties (with nested type checks).
/// Returns Ok(()) if valid or schema is empty, Err with details if invalid.
pub fn validate_event_schema(payload: &str, schema: &str) -> Result<()> {
    if schema.is_empty() {
        return Ok(());
    }

    let schema_val: Value =
        serde_json::from_str(schema).map_err(|e| anyhow::anyhow!("invalid schema JSON: {e}"))?;
    let payload_val: Value =
        serde_json::from_str(payload).map_err(|e| anyhow::anyhow!("invalid payload JSON: {e}"))?;

    let validator = jsonschema::validator_for(&schema_val)
        .map_err(|e| anyhow::anyhow!("invalid JSON Schema: {e}"))?;

    let errors: Vec<String> = validator
        .iter_errors(&payload_val)
        .map(|e| {
            let path = e.instance_path().to_string();
            if path.is_empty() {
                format!("schema validation failed: {e}")
            } else {
                format!("schema validation failed at {path}: {e}")
            }
        })
        .collect();

    if errors.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("{}", errors.join("; "))
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

// --- Outbound webhook management API ---

async fn handle_create_endpoint(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> axum::response::Response {
    if let Some(resp) = check_api_auth(&state, &headers) {
        return resp;
    }

    let source = body["source"].as_str().unwrap_or("");
    let url = body["url"].as_str().unwrap_or("");
    let description = body["description"].as_str();

    if source.is_empty() || url.is_empty() {
        let body = serde_json::json!({"error": "source and url are required"});
        return (StatusCode::BAD_REQUEST, axum::Json(body)).into_response();
    }

    // Verify source exists and is outbound type
    match state.config.sources.get(source) {
        Some(s) if s.source_type == "outbound" => {}
        Some(_) => {
            let body = serde_json::json!({"error": format!("source '{}' is not an outbound source", source)});
            return (StatusCode::BAD_REQUEST, axum::Json(body)).into_response();
        }
        None => {
            let body = serde_json::json!({"error": format!("source '{}' not found", source)});
            return (StatusCode::NOT_FOUND, axum::Json(body)).into_response();
        }
    }

    // Validate URL (reuse SSRF protection)
    if !url.starts_with("http://") && !url.starts_with("https://") {
        let body = serde_json::json!({"error": "url must start with http:// or https://"});
        return (StatusCode::BAD_REQUEST, axum::Json(body)).into_response();
    }

    let id = ulid::Ulid::new().to_string();
    let signing_secret = crate::verify::generate_signing_secret();

    match state
        .db
        .insert_endpoint(&id, source, url, description, &signing_secret)
        .await
    {
        Ok(()) => {
            let body = serde_json::json!({
                "id": id,
                "source": source,
                "url": url,
                "description": description,
                "signing_secret": signing_secret,
                "status": "active",
            });
            (StatusCode::CREATED, axum::Json(body)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to create endpoint");
            let body = serde_json::json!({"error": "internal error"});
            (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response()
        }
    }
}

async fn handle_list_endpoints(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> axum::response::Response {
    if let Some(resp) = check_api_auth(&state, &headers) {
        return resp;
    }

    let source = params.get("source").map(|s| s.as_str());
    match state.db.list_endpoints(source).await {
        Ok(endpoints) => {
            // Redact signing secrets in list response
            let items: Vec<Value> = endpoints
                .iter()
                .map(|ep| {
                    serde_json::json!({
                        "id": ep.id,
                        "source": ep.source,
                        "url": ep.url,
                        "description": ep.description,
                        "status": ep.status,
                        "created_at": ep.created_at,
                        "updated_at": ep.updated_at,
                    })
                })
                .collect();
            axum::Json(serde_json::json!({"endpoints": items})).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to list endpoints");
            let body = serde_json::json!({"error": "internal error"});
            (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response()
        }
    }
}

async fn handle_get_endpoint(
    State(state): State<SharedState>,
    Path(endpoint_id): Path<String>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Some(resp) = check_api_auth(&state, &headers) {
        return resp;
    }

    match state.db.get_endpoint(&endpoint_id).await {
        Ok(Some(ep)) => {
            let subs = state
                .db
                .list_subscriptions(&endpoint_id)
                .await
                .unwrap_or_default();
            let body = serde_json::json!({
                "id": ep.id,
                "source": ep.source,
                "url": ep.url,
                "description": ep.description,
                "signing_secret": ep.signing_secret,
                "status": ep.status,
                "created_at": ep.created_at,
                "updated_at": ep.updated_at,
                "subscriptions": subs,
            });
            axum::Json(body).into_response()
        }
        Ok(None) => {
            let body = serde_json::json!({"error": "endpoint not found"});
            (StatusCode::NOT_FOUND, axum::Json(body)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to get endpoint");
            let body = serde_json::json!({"error": "internal error"});
            (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response()
        }
    }
}

async fn handle_update_endpoint(
    State(state): State<SharedState>,
    Path(endpoint_id): Path<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> axum::response::Response {
    if let Some(resp) = check_api_auth(&state, &headers) {
        return resp;
    }

    let url = body["url"].as_str();
    let description = body["description"].as_str();
    let status = body["status"].as_str();

    // Validate status if provided
    if let Some(s) = status {
        if s != "active" && s != "disabled" {
            let body = serde_json::json!({"error": "status must be 'active' or 'disabled'"});
            return (StatusCode::BAD_REQUEST, axum::Json(body)).into_response();
        }
    }

    // Validate URL if provided
    if let Some(u) = url {
        if !u.starts_with("http://") && !u.starts_with("https://") {
            let body = serde_json::json!({"error": "url must start with http:// or https://"});
            return (StatusCode::BAD_REQUEST, axum::Json(body)).into_response();
        }
    }

    match state
        .db
        .update_endpoint(&endpoint_id, url, description, status)
        .await
    {
        Ok(true) => {
            let ep = state.db.get_endpoint(&endpoint_id).await.ok().flatten();
            if let Some(ep) = ep {
                let body = serde_json::json!({
                    "id": ep.id,
                    "source": ep.source,
                    "url": ep.url,
                    "description": ep.description,
                    "status": ep.status,
                    "updated_at": ep.updated_at,
                });
                axum::Json(body).into_response()
            } else {
                StatusCode::OK.into_response()
            }
        }
        Ok(false) => {
            let body = serde_json::json!({"error": "endpoint not found"});
            (StatusCode::NOT_FOUND, axum::Json(body)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to update endpoint");
            let body = serde_json::json!({"error": "internal error"});
            (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response()
        }
    }
}

async fn handle_delete_endpoint(
    State(state): State<SharedState>,
    Path(endpoint_id): Path<String>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Some(resp) = check_api_auth(&state, &headers) {
        return resp;
    }

    match state.db.delete_endpoint(&endpoint_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => {
            let body = serde_json::json!({"error": "endpoint not found"});
            (StatusCode::NOT_FOUND, axum::Json(body)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to delete endpoint");
            let body = serde_json::json!({"error": "internal error"});
            (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response()
        }
    }
}

async fn handle_rotate_secret(
    State(state): State<SharedState>,
    Path(endpoint_id): Path<String>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Some(resp) = check_api_auth(&state, &headers) {
        return resp;
    }

    let new_secret = crate::verify::generate_signing_secret();
    match state
        .db
        .rotate_endpoint_secret(&endpoint_id, &new_secret)
        .await
    {
        Ok(true) => {
            let body = serde_json::json!({"signing_secret": new_secret});
            axum::Json(body).into_response()
        }
        Ok(false) => {
            let body = serde_json::json!({"error": "endpoint not found"});
            (StatusCode::NOT_FOUND, axum::Json(body)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to rotate secret");
            let body = serde_json::json!({"error": "internal error"});
            (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response()
        }
    }
}

async fn handle_create_subscriptions(
    State(state): State<SharedState>,
    Path(endpoint_id): Path<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> axum::response::Response {
    if let Some(resp) = check_api_auth(&state, &headers) {
        return resp;
    }

    // Verify endpoint exists
    match state.db.get_endpoint(&endpoint_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            let body = serde_json::json!({"error": "endpoint not found"});
            return (StatusCode::NOT_FOUND, axum::Json(body)).into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to get endpoint");
            let body = serde_json::json!({"error": "internal error"});
            return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response();
        }
    }

    let event_types: Vec<String> = body["event_types"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    if event_types.is_empty() {
        let body = serde_json::json!({"error": "event_types array is required"});
        return (StatusCode::BAD_REQUEST, axum::Json(body)).into_response();
    }

    match state
        .db
        .insert_subscriptions(&endpoint_id, &event_types)
        .await
    {
        Ok(subs) => {
            let body = serde_json::json!({"subscriptions": subs});
            (StatusCode::CREATED, axum::Json(body)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to create subscriptions");
            let body = serde_json::json!({"error": "internal error"});
            (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response()
        }
    }
}

async fn handle_list_subscriptions(
    State(state): State<SharedState>,
    Path(endpoint_id): Path<String>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Some(resp) = check_api_auth(&state, &headers) {
        return resp;
    }

    match state.db.list_subscriptions(&endpoint_id).await {
        Ok(subs) => {
            let body = serde_json::json!({"subscriptions": subs});
            axum::Json(body).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to list subscriptions");
            let body = serde_json::json!({"error": "internal error"});
            (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response()
        }
    }
}

async fn handle_delete_subscription(
    State(state): State<SharedState>,
    Path((_, subscription_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Some(resp) = check_api_auth(&state, &headers) {
        return resp;
    }

    match state.db.delete_subscription(&subscription_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => {
            let body = serde_json::json!({"error": "subscription not found"});
            (StatusCode::NOT_FOUND, axum::Json(body)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to delete subscription");
            let body = serde_json::json!({"error": "internal error"});
            (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response()
        }
    }
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

    // --- Advanced filter operators ---

    #[test]
    fn test_filter_contains() {
        assert!(evaluate_filter(
            r#"{"msg": "hello world"}"#,
            "$.msg contains world"
        ));
        assert!(!evaluate_filter(
            r#"{"msg": "hello world"}"#,
            "$.msg contains xyz"
        ));
    }

    #[test]
    fn test_filter_contains_array() {
        assert!(evaluate_filter(
            r#"{"tags": ["rust", "web"]}"#,
            "$.tags contains rust"
        ));
        assert!(!evaluate_filter(
            r#"{"tags": ["rust", "web"]}"#,
            "$.tags contains python"
        ));
    }

    #[test]
    fn test_filter_starts_with() {
        assert!(evaluate_filter(
            r#"{"ref": "refs/heads/main"}"#,
            "$.ref starts_with refs/heads/"
        ));
        assert!(!evaluate_filter(
            r#"{"ref": "refs/tags/v1"}"#,
            "$.ref starts_with refs/heads/"
        ));
    }

    #[test]
    fn test_filter_ends_with() {
        assert!(evaluate_filter(
            r#"{"file": "image.png"}"#,
            "$.file ends_with .png"
        ));
        assert!(!evaluate_filter(
            r#"{"file": "image.png"}"#,
            "$.file ends_with .jpg"
        ));
    }

    #[test]
    fn test_filter_matches() {
        assert!(evaluate_filter(
            r#"{"email": "user@example.com"}"#,
            "$.email matches ^[^@]+@[^@]+$"
        ));
        assert!(!evaluate_filter(
            r#"{"email": "invalid"}"#,
            "$.email matches ^[^@]+@[^@]+$"
        ));
    }

    #[test]
    fn test_filter_exists() {
        assert!(evaluate_filter(r#"{"a": 1}"#, "$.a exists"));
        assert!(!evaluate_filter(r#"{"a": 1}"#, "$.b exists"));
        // null field exists in the JSON but is null — still "exists"
        assert!(evaluate_filter(r#"{"a": null}"#, "$.a exists"));
    }

    #[test]
    fn test_filter_not() {
        assert!(evaluate_filter(
            r#"{"status": "pending"}"#,
            "not $.status == completed"
        ));
        assert!(!evaluate_filter(
            r#"{"status": "completed"}"#,
            "not $.status == completed"
        ));
        // not with truthy
        assert!(evaluate_filter(r#"{"a": false}"#, "not $.a"));
        assert!(!evaluate_filter(r#"{"a": true}"#, "not $.a"));
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

    // --- Schema validation ---

    #[test]
    fn test_validate_schema_valid() {
        let schema = r#"{
            "type": "object",
            "required": ["id", "amount"],
            "properties": {
                "id": {"type": "string"},
                "amount": {"type": "number"}
            }
        }"#;
        let payload = r#"{"id": "abc", "amount": 100}"#;
        assert!(validate_event_schema(payload, schema).is_ok());
    }

    #[test]
    fn test_validate_schema_missing_required() {
        let schema = r#"{
            "type": "object",
            "required": ["id", "amount"],
            "properties": {
                "id": {"type": "string"},
                "amount": {"type": "number"}
            }
        }"#;
        let payload = r#"{"id": "abc"}"#;
        assert!(validate_event_schema(payload, schema).is_err());
    }

    #[test]
    fn test_validate_schema_wrong_type() {
        let schema = r#"{
            "type": "object",
            "properties": {
                "count": {"type": "integer"}
            }
        }"#;
        let payload = r#"{"count": "not a number"}"#;
        assert!(validate_event_schema(payload, schema).is_err());
    }

    #[test]
    fn test_validate_schema_no_schema() {
        // When no schema is configured, any payload is valid
        assert!(validate_event_schema(r#"{"anything": true}"#, "").is_ok());
    }

    #[test]
    fn test_validate_schema_enum() {
        let schema = r#"{
            "type": "object",
            "properties": {
                "status": {"type": "string", "enum": ["active", "disabled"]}
            }
        }"#;
        assert!(validate_event_schema(r#"{"status": "active"}"#, schema).is_ok());
        assert!(validate_event_schema(r#"{"status": "unknown"}"#, schema).is_err());
    }

    #[test]
    fn test_validate_schema_min_max_length() {
        let schema = r#"{
            "type": "object",
            "properties": {
                "name": {"type": "string", "minLength": 2, "maxLength": 10}
            }
        }"#;
        assert!(validate_event_schema(r#"{"name": "ok"}"#, schema).is_ok());
        assert!(validate_event_schema(r#"{"name": "x"}"#, schema).is_err());
        assert!(validate_event_schema(r#"{"name": "this is way too long"}"#, schema).is_err());
    }

    #[test]
    fn test_validate_schema_pattern() {
        let schema = r#"{
            "type": "object",
            "properties": {
                "email": {"type": "string", "pattern": "^.+@.+\\..+$"}
            }
        }"#;
        assert!(validate_event_schema(r#"{"email": "user@example.com"}"#, schema).is_ok());
        assert!(validate_event_schema(r#"{"email": "not-an-email"}"#, schema).is_err());
    }

    #[tokio::test]
    async fn test_echo_returns_json_with_body_and_headers() {
        let body = Bytes::from(r#"{"order_id": "123", "amount": 500}"#);
        let headers = HeaderMap::new();
        let resp = handle_echo(headers, body).await;
        assert_eq!(resp.0, StatusCode::OK);
        assert_eq!(
            resp.1,
            [(axum::http::header::CONTENT_TYPE, "application/json")]
        );
        let parsed: Value = serde_json::from_str(&resp.2).unwrap();
        assert_eq!(parsed["body"]["order_id"], "123");
        assert_eq!(parsed["body"]["amount"], 500);
        assert!(parsed["headers"].is_object());
    }

    #[tokio::test]
    async fn test_echo_includes_request_headers() {
        let body = Bytes::from(r#"{}"#);
        let mut headers = HeaderMap::new();
        headers.insert("x-custom", "test-value".parse().unwrap());
        let resp = handle_echo(headers, body).await;
        assert_eq!(resp.0, StatusCode::OK);
        let parsed: Value = serde_json::from_str(&resp.2).unwrap();
        assert_eq!(parsed["body"], serde_json::json!({}));
        assert_eq!(parsed["headers"]["x-custom"], "test-value");
    }

    #[tokio::test]
    async fn test_echo_non_json_body() {
        let body = Bytes::from("plain text body");
        let headers = HeaderMap::new();
        let resp = handle_echo(headers, body).await;
        assert_eq!(resp.0, StatusCode::OK);
        let parsed: Value = serde_json::from_str(&resp.2).unwrap();
        assert_eq!(parsed["body"], "plain text body");
    }
}
