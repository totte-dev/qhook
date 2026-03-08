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
    pub http: reqwest::Client,
}

impl AppState {
    pub fn new(config: Config, db: Database) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to build HTTP client");
        Self {
            config,
            db: Arc::new(db),
            http,
        }
    }
}

type SharedState = Arc<AppState>;

pub async fn serve(state: AppState) -> Result<()> {
    let port = state.config.server.port;
    let shared = Arc::new(state);

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

    // Start queue worker
    let worker = Worker::new(shared.db.clone());
    tokio::spawn(async move {
        worker.run().await;
    });

    let app = Router::new()
        .route("/webhooks/{source}", post(handle_webhook))
        .route("/events/{event_type}", post(handle_event))
        .route("/sns/{source}", post(handle_sns))
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

    // CloudEvents ce-type header overrides URL path event type
    let event_type = if let Some(ce_type) = headers.get("ce-type").and_then(|v| v.to_str().ok()) {
        ce_type.to_string()
    } else {
        event_type
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
                    tracing::error!(error = %e, "Failed to process SNS event");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal error".to_string(),
                    )
                }
            }
        }
        "UnsubscribeConfirmation" => {
            tracing::info!(source = source_name, "SNS unsubscribe confirmation received");
            (StatusCode::OK, "Unsubscribe acknowledged".to_string())
        }
        other => {
            tracing::warn!(source = source_name, message_type = other, "Unknown SNS message type");
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

fn extract_event_type(source: &str, payload: &str, headers: &HeaderMap) -> String {
    // CloudEvents binary mode: ce-type header takes precedence
    if let Some(ce_type) = headers.get("ce-type").and_then(|v| v.to_str().ok()) {
        return ce_type.to_string();
    }

    // CloudEvents structured mode: application/cloudevents+json
    if let Some(ct) = headers.get("content-type").and_then(|v| v.to_str().ok()) {
        if ct.contains("application/cloudevents+json") {
            let json: Value = serde_json::from_str(payload).unwrap_or(Value::Null);
            if let Some(ce_type) = json.get("type").and_then(|v| v.as_str()) {
                return ce_type.to_string();
            }
        }
    }

    // Provider-specific extraction
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

fn extract_sns_event_type(message: &str, subject: Option<&str>) -> String {
    if let Ok(json) = serde_json::from_str::<Value>(message) {
        // CloudEvents structured mode
        if json.get("specversion").is_some() {
            if let Some(t) = json.get("type").and_then(|v| v.as_str()) {
                return t.to_string();
            }
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
    if let Some(s) = subject {
        if !s.is_empty() {
            return s.to_string();
        }
    }

    "sns.notification".to_string()
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
        headers.insert("content-type", "application/cloudevents+json".parse().unwrap());

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
        headers.insert("content-type", "application/cloudevents+json".parse().unwrap());

        let payload = r#"{"type": "from.body"}"#;
        let result = extract_event_type("source", payload, &headers);
        assert_eq!(result, "from.header");
    }

    // --- Provider-specific event type extraction ---

    #[test]
    fn test_stripe_event_type() {
        let headers = HeaderMap::new();
        let payload = r#"{"type": "invoice.paid", "id": "evt_123"}"#;
        assert_eq!(extract_event_type("stripe", payload, &headers), "invoice.paid");
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
        assert_eq!(extract_event_type("shopify", payload, &headers), "orders/create");
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
        let map: std::collections::HashMap<String, String> =
            serde_json::from_str(&json).unwrap();

        assert_eq!(map.get("ce-type").unwrap(), "test.event");
        assert_eq!(map.get("ce-source").unwrap(), "/myapp");
        assert_eq!(map.get("content-type").unwrap(), "application/json");
    }
}
