//! E2E integration tests for pull-mode queue delivery.

mod common;

use common::{QhookProcess, http};
use serde_json::Value;

fn queue_yaml() -> String {
    r#"
database:
  driver: sqlite
  url: "sqlite:__DB_PATH__?mode=rwc"
server:
  port: __PORT__
  allow_private_urls: true
sources:
  stripe:
    type: event
handlers: {}
queues:
  billing:
    source: stripe
    events: [checkout.session.completed, charge.failed]
    visibility_timeout: 2s
    max_attempts: 3
"#
    .to_string()
}

fn queue_yaml_with_auth() -> String {
    r#"
database:
  driver: sqlite
  url: "sqlite:__DB_PATH__?mode=rwc"
server:
  port: __PORT__
  allow_private_urls: true
sources:
  stripe:
    type: event
handlers: {}
queues:
  billing:
    source: stripe
    events: [checkout.session.completed]
    visibility_timeout: 2s
    max_attempts: 3
    api_key: test_queue_key_123
"#
    .to_string()
}

async fn send_event(base_url: &str, event_type: &str) -> Value {
    let client = http();
    let resp = client
        .post(format!("{}/events/stripe/{}", base_url, event_type))
        .header("Content-Type", "application/json")
        .body(format!(
            r#"{{"id":"evt_{}","type":"{}","amount":1900}}"#,
            ulid::Ulid::new(),
            event_type
        ))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "send_event failed: {}",
        resp.status()
    );
    resp.json().await.unwrap()
}

// ── Basic receive/ack flow ───────────────────────────────────────────

#[tokio::test]
async fn queue_receive_and_ack() {
    let server = QhookProcess::start(&queue_yaml(), 19750).await;
    let client = http();

    // Send an event
    send_event(&server.base_url, "checkout.session.completed").await;

    // Small delay for job creation
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Receive message
    let resp = client
        .get(server.url("/api/queues/billing/messages?batch=10"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);

    let msg = &messages[0];
    assert_eq!(msg["event_type"], "checkout.session.completed");
    assert!(msg["payload"]["amount"].as_i64().unwrap() == 1900);
    assert!(msg["id"].as_str().is_some());
    assert!(msg["event_id"].as_str().is_some());
    assert_eq!(msg["attempt"], 0);

    let msg_id = msg["id"].as_str().unwrap().to_string();

    // Ack the message
    let resp = client
        .post(server.url("/api/queues/billing/ack"))
        .json(&serde_json::json!({"ids": [msg_id]}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["acked"], 1);

    // No more messages available
    let resp = client
        .get(server.url("/api/queues/billing/messages"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["messages"].as_array().unwrap().len(), 0);

    server.stop().await;
}

// ── Nack with DLQ ────────────────────────────────────────────────────

#[tokio::test]
async fn queue_nack_retries_and_dlq() {
    let server = QhookProcess::start(&queue_yaml(), 19751).await;
    let client = http();

    send_event(&server.base_url, "checkout.session.completed").await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Receive the message
    let resp = client
        .get(server.url("/api/queues/billing/messages"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    let msg_id = msgs[0]["id"].as_str().unwrap().to_string();

    // First nack — should retry (attempt 1 < max_attempts 3)
    let resp = client
        .post(server.url("/api/queues/billing/nack"))
        .json(&serde_json::json!({"ids": [msg_id]}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["retried"], 1);
    assert_eq!(body["dead"], 0);

    // To test DLQ: receive again after backoff expires.
    // Instead of waiting 30s+ for real backoff, we'll verify the job status via API.
    // The job should be in 'retryable' status, not 'dead'.
    let resp = client
        .get(server.url("/api/jobs?status=retryable"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    let jobs = body["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 1);
    assert!(jobs[0]["handler"].as_str().unwrap().starts_with("queue/"));

    server.stop().await;
}

/// Test that nacking a message at max_attempts sends it to DLQ.
#[tokio::test]
async fn queue_nack_to_dlq() {
    // Use max_attempts: 1 so first receive + nack goes straight to DLQ
    let yaml = r#"
database:
  driver: sqlite
  url: "sqlite:__DB_PATH__?mode=rwc"
server:
  port: __PORT__
  allow_private_urls: true
sources:
  stripe:
    type: event
handlers: {}
queues:
  billing:
    source: stripe
    events: [checkout.session.completed]
    visibility_timeout: 2s
    max_attempts: 1
"#;

    let server = QhookProcess::start(yaml, 19759).await;
    let client = http();

    send_event(&server.base_url, "checkout.session.completed").await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Receive
    let resp = client
        .get(server.url("/api/queues/billing/messages"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    let msg_id = msgs[0]["id"].as_str().unwrap().to_string();

    // Nack at max_attempts — should go to DLQ
    let resp = client
        .post(server.url("/api/queues/billing/nack"))
        .json(&serde_json::json!({"ids": [msg_id]}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["retried"], 0);
    assert_eq!(body["dead"], 1);

    // Verify via management API
    let resp = client
        .get(server.url("/api/jobs?status=dead"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    let jobs = body["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 1);

    server.stop().await;
}

// ── Long polling ─────────────────────────────────────────────────────

#[tokio::test]
async fn queue_long_polling() {
    let server = QhookProcess::start(&queue_yaml(), 19752).await;
    let base = server.base_url.clone();

    // Start long-polling in background
    let poll_handle = tokio::spawn(async move {
        let c = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap();
        let resp = c
            .get(format!("{}/api/queues/billing/messages?wait=5s", base))
            .send()
            .await
            .unwrap();
        let body: Value = resp.json().await.unwrap();
        body
    });

    // Wait a bit, then send an event
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    send_event(&server.base_url, "checkout.session.completed").await;

    // Long poll should return with the message
    let body = tokio::time::timeout(std::time::Duration::from_secs(8), poll_handle)
        .await
        .expect("poll timed out")
        .expect("poll task failed");

    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["event_type"], "checkout.session.completed");

    server.stop().await;
}

// ── Queue auth ───────────────────────────────────────────────────────

#[tokio::test]
async fn queue_auth_required() {
    let server = QhookProcess::start(&queue_yaml_with_auth(), 19753).await;
    let client = http();

    // No auth → 401
    let resp = client
        .get(server.url("/api/queues/billing/messages"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Wrong token → 401
    let resp = client
        .get(server.url("/api/queues/billing/messages"))
        .header("Authorization", "Bearer wrong_key")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Correct token → 200
    let resp = client
        .get(server.url("/api/queues/billing/messages"))
        .header("Authorization", "Bearer test_queue_key_123")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    server.stop().await;
}

// ── Queue not found ──────────────────────────────────────────────────

#[tokio::test]
async fn queue_not_found() {
    let server = QhookProcess::start(&queue_yaml(), 19754).await;
    let client = http();

    let resp = client
        .get(server.url("/api/queues/nonexistent/messages"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    let resp = client
        .post(server.url("/api/queues/nonexistent/ack"))
        .json(&serde_json::json!({"ids": ["abc"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    server.stop().await;
}

// ── Event filtering ──────────────────────────────────────────────────

#[tokio::test]
async fn queue_event_type_filtering() {
    let server = QhookProcess::start(&queue_yaml(), 19755).await;
    let client = http();

    // Send matching event
    send_event(&server.base_url, "checkout.session.completed").await;
    // Send non-matching event
    send_event(&server.base_url, "customer.created").await;

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Only the matching event should be in the queue
    let resp = client
        .get(server.url("/api/queues/billing/messages?batch=10"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["event_type"], "checkout.session.completed");

    server.stop().await;
}

// ── List queues ──────────────────────────────────────────────────────

#[tokio::test]
async fn queue_list_queues() {
    let server = QhookProcess::start(&queue_yaml(), 19756).await;
    let client = http();

    // Send an event to populate queue
    send_event(&server.base_url, "checkout.session.completed").await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let resp = client.get(server.url("/api/queues")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let queues = body["queues"].as_array().unwrap();
    assert_eq!(queues.len(), 1);
    assert_eq!(queues[0]["name"], "billing");
    assert_eq!(queues[0]["source"], "stripe");
    assert_eq!(queues[0]["depth"], 1);

    server.stop().await;
}

// ── Visibility timeout recovery ──────────────────────────────────────

#[tokio::test]
async fn queue_visibility_timeout_recovery() {
    let server = QhookProcess::start(&queue_yaml(), 19757).await;
    let client = http();

    send_event(&server.base_url, "checkout.session.completed").await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Receive message (moves to running)
    let resp = client
        .get(server.url("/api/queues/billing/messages"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);

    // Don't ack — wait for visibility timeout (2s) + recovery interval (10s)
    // The message should become available again
    let mut recovered = false;
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let resp = client
            .get(server.url("/api/queues/billing/messages"))
            .send()
            .await
            .unwrap();
        let body: Value = resp.json().await.unwrap();
        if body["messages"].as_array().unwrap().len() == 1 {
            recovered = true;
            break;
        }
    }
    assert!(
        recovered,
        "Message should be recovered after visibility timeout"
    );

    server.stop().await;
}

// ── Push + Pull coexistence ──────────────────────────────────────────

#[tokio::test]
async fn queue_push_pull_coexist() {
    let mock = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/push-handler"))
        .respond_with(wiremock::ResponseTemplate::new(200))
        .mount(&mock)
        .await;

    let yaml = format!(
        r#"
database:
  driver: sqlite
  url: "sqlite:__DB_PATH__?mode=rwc"
server:
  port: __PORT__
  allow_private_urls: true
sources:
  stripe:
    type: event
handlers:
  push-handler:
    source: stripe
    events: [checkout.session.completed]
    url: {mock_url}/push-handler
    retry:
      max: 1
queues:
  billing:
    source: stripe
    events: [checkout.session.completed]
    visibility_timeout: 30s
    max_attempts: 3
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19758).await;
    let client = http();

    send_event(&server.base_url, "checkout.session.completed").await;

    // Wait for push delivery
    common::wait_for_mock(&mock, 1, 5).await;

    // Queue should also have the message
    let resp = client
        .get(server.url("/api/queues/billing/messages"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);

    // Verify push handler got exactly 1 request
    let push_count = common::count_path(&mock, "/push-handler").await;
    assert_eq!(push_count, 1);

    server.stop().await;
}
