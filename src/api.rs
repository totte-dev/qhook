use std::sync::Arc;

use anyhow::Result;
use axum::{
    Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use serde_json::Value;

use crate::config::Config;
use crate::db::Database;
use crate::queue::Worker;

pub struct AppState {
    pub config: Config,
    pub db: Arc<Database>,
}

impl AppState {
    pub fn new(config: Config, db: Database) -> Self {
        Self {
            config,
            db: Arc::new(db),
        }
    }
}

type SharedState = Arc<AppState>;

pub async fn serve(state: AppState) -> Result<()> {
    let port = state.config.server.port;
    let shared = Arc::new(state);

    // Print registered webhook endpoints
    for (name, source) in &shared.config.sources {
        if source.source_type == "webhook" {
            tracing::info!(
                "  POST http://localhost:{port}/webhooks/{name}",
            );
        }
    }

    // Start queue worker
    let worker = Worker::new(shared.db.clone());
    tokio::spawn(async move {
        worker.run().await;
    });

    let app = Router::new()
        .route("/webhooks/{source}", post(handle_webhook))
        .route("/events/{event_type}", post(handle_event))
        .route("/health", axum::routing::get(|| async { "ok" }))
        .with_state(shared);

    let addr = format!("0.0.0.0:{port}");
    tracing::info!("qhook running on :{port}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
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
    let payload_str = match String::from_utf8(body.to_vec()) {
        Ok(s) => s,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid UTF-8".to_string()),
    };

    // Extract event type from payload
    let event_type = extract_event_type(&source_name, &payload_str);

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
    // Check auth token if configured
    if let Some(expected_token) = &state.config.api.auth_token {
        let provided = headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));

        match provided {
            Some(token) if token == expected_token => {}
            _ => return (StatusCode::UNAUTHORIZED, "Invalid token".to_string()),
        }
    }

    let payload_str = match String::from_utf8(body.to_vec()) {
        Ok(s) => s,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid UTF-8".to_string()),
    };

    match process_event(&state, "app", &event_type, &payload_str, &headers).await {
        Ok(_) => (StatusCode::ACCEPTED, "Event accepted".to_string()),
        Err(e) => {
            tracing::error!(error = %e, "Failed to process event");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal error".to_string(),
            )
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

    if matching_handlers.is_empty() {
        tracing::debug!(source, event_type, "No matching handlers");
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

    if !created {
        tracing::info!(source, event_type, "Duplicate event");
        return Ok(false);
    }

    // Create jobs for each matching handler
    for (handler_name, handler) in &matching_handlers {
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

        tracing::info!(
            event_id,
            job_id,
            handler = *handler_name,
            event_type,
            "Job created"
        );
    }

    Ok(true)
}

fn extract_event_type(source: &str, payload: &str) -> String {
    let json: Value = serde_json::from_str(payload).unwrap_or(Value::Null);

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

fn event_matches(pattern: &str, event_type: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix(".*") {
        return event_type.starts_with(prefix);
    }
    pattern == event_type
}

fn extract_json_path(payload: &str, path: &str) -> Option<String> {
    let json: Value = serde_json::from_str(payload).ok()?;

    // Simple JSON path: $.field or $.field.nested
    let path = path.strip_prefix("$.").unwrap_or(path);
    let mut current = &json;

    for part in path.split('.') {
        current = current.get(part)?;
    }

    current.as_str().map(|s| s.to_string())
}

fn serialize_headers(headers: &HeaderMap) -> String {
    let map: std::collections::HashMap<String, String> = headers
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|v| (k.as_str().to_string(), v.to_string()))
        })
        .collect();

    serde_json::to_string(&map).unwrap_or_else(|_| "{}".to_string())
}
