//! E2E tests — feature-level integration tests converted from e2e.sh.

mod common;

use common::{QhookProcess, hmac_sha256, http, wait_for_mock};
use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Test 1: Internal event → delivery (payload + X-Qhook headers)

#[tokio::test]
async fn event_delivery() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/jobs/test"))
        .respond_with(ResponseTemplate::new(200))
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
  app:
    type: event
handlers:
  on-test:
    source: app
    events: [test.hello]
    url: {mock_url}/jobs/test
    retry:
      max: 3
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19711).await;

    let resp = http()
        .post(server.url("/events/app/test.hello"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"message": "hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);

    wait_for_mock(&mock, 1, 10).await;

    let reqs = mock.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);

    // Payload preserved
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["message"], "hello");

    // X-Qhook headers present
    let has_qhook = reqs[0]
        .headers
        .iter()
        .any(|(name, _)| name.as_str().to_lowercase().contains("qhook"));
    assert!(has_qhook, "X-Qhook headers should be present");

    server.stop().await;
}

// Test 2: Unknown webhook → 404

#[tokio::test]
async fn unknown_webhook_404() {
    let yaml = r#"
database:
  driver: sqlite
  url: "sqlite:__DB_PATH__?mode=rwc"
server:
  port: __PORT__
  allow_private_urls: true
sources:
  app:
    type: event
handlers: {}
"#;

    let server = QhookProcess::start(yaml, 19712).await;

    let resp = http()
        .post(server.url("/webhooks/nonexistent"))
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    server.stop().await;
}

// Test 3: Idempotency

#[tokio::test]
async fn idempotency() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
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
  app:
    type: event
handlers:
  dedup:
    source: app
    events: [dedup.test]
    url: {mock_url}/jobs/dedup
    idempotency_key: "$.id"
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19713).await;
    let c = http();

    c.post(server.url("/events/app/dedup.test"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"id": "evt_123", "data": "first"}))
        .send()
        .await
        .unwrap();

    c.post(server.url("/events/app/dedup.test"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"id": "evt_123", "data": "duplicate"}))
        .send()
        .await
        .unwrap();

    wait_for_mock(&mock, 1, 5).await;
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert_eq!(mock.received_requests().await.unwrap().len(), 1);

    server.stop().await;
}

// Test 4: GitHub signature verification

#[tokio::test]
async fn github_signature_verification() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
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
  github:
    type: webhook
    verify: github
    secret: test-secret-123
handlers:
  on-push:
    source: github
    events: [push]
    url: {mock_url}/jobs/deploy
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19714).await;
    let c = http();

    let payload = r#"{"action":"push","ref":"refs/heads/main"}"#;
    let sig = hmac_sha256("test-secret-123", payload);

    // Valid signature → 200
    assert_eq!(
        c.post(server.url("/webhooks/github"))
            .header("Content-Type", "application/json")
            .header("X-Hub-Signature-256", format!("sha256={sig}"))
            .body(payload)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );

    // Invalid signature → 401
    assert_eq!(
        c.post(server.url("/webhooks/github"))
            .header("Content-Type", "application/json")
            .header("X-Hub-Signature-256", "sha256=invalid")
            .body(payload)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );

    // Missing signature → 401
    assert_eq!(
        c.post(server.url("/webhooks/github"))
            .header("Content-Type", "application/json")
            .body(payload)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );

    // Only valid one delivered
    wait_for_mock(&mock, 1, 5).await;
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert_eq!(mock.received_requests().await.unwrap().len(), 1);

    server.stop().await;
}

// Test 5: Retry on failure
// Note: qhook's retry backoff is hardcoded at 30s × 2^attempt in queue.rs,
// so we cannot wait for actual retries in a fast test. Instead, we verify:
// 1. The initial delivery attempt was made (and returned 500)
// 2. The job status is "retrying" (not "completed"), proving retry was scheduled
// 3. The attempt record shows status_code=500

#[tokio::test]
async fn retry_on_failure() {
    let mock = MockServer::start().await;

    // All requests return 500 — qhook will retry
    Mock::given(method("POST"))
        .and(path("/jobs/retry"))
        .respond_with(ResponseTemplate::new(500).set_body_string("error"))
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
api:
  auth_token: test-token
sources:
  app:
    type: event
handlers:
  retry-test:
    source: app
    events: [retry.test]
    url: {mock_url}/jobs/retry
    retry:
      max: 3
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19715).await;

    let resp = http()
        .post(server.url("/events/app/retry.test"))
        .header("Content-Type", "application/json")
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({"retry": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);
    let body: Value = resp.json().await.unwrap();
    let event_id = body["event_id"].as_str().unwrap().to_string();

    // Wait for first delivery attempt
    wait_for_mock(&mock, 1, 10).await;
    // Allow qhook to update job status in DB
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Verify exactly 1 attempt so far (retry is 30s+ away)
    assert_eq!(mock.received_requests().await.unwrap().len(), 1);

    // Verify job status via Management API — should be "retrying", not "completed"
    let ev: Value = http()
        .get(server.url(&format!("/api/events/{event_id}")))
        .header("Authorization", "Bearer test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let jobs = ev["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(
        jobs[0]["status"], "retryable",
        "Job should be scheduled for retry after 500 response"
    );

    // Verify attempt record shows the 500 failure
    let job_id = jobs[0]["id"].as_str().unwrap();
    let job: Value = http()
        .get(server.url(&format!("/api/jobs/{job_id}?include_attempts=true")))
        .header("Authorization", "Bearer test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let attempts = job["attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 1, "Should have exactly 1 attempt so far");
    assert_eq!(
        attempts[0]["status_code"], 500,
        "Attempt should record 500 status"
    );

    server.stop().await;
}

// Test 6: CloudEvents binary mode

#[tokio::test]
async fn cloudevents_binary_mode() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
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
  app:
    type: event
handlers:
  on-ce:
    source: app
    events: [com.example.order.created]
    url: {mock_url}/jobs/ce
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19716).await;

    // ce-type header overrides URL path event type
    let resp = http()
        .post(server.url("/events/app/ignored.by.header"))
        .header("Content-Type", "application/json")
        .header("ce-type", "com.example.order.created")
        .header("ce-source", "/myapp")
        .header("ce-id", "evt-ce-001")
        .header("ce-specversion", "1.0")
        .json(&serde_json::json!({"orderId": "ord_ce_1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);

    wait_for_mock(&mock, 1, 10).await;

    // Verify ce-* headers forwarded with correct values
    let reqs = mock.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);
    let ce_type = reqs[0]
        .headers
        .iter()
        .find(|(n, _)| n.as_str().eq_ignore_ascii_case("ce-type"))
        .map(|(_, v)| v.to_str().unwrap().to_string());
    let ce_source = reqs[0]
        .headers
        .iter()
        .find(|(n, _)| n.as_str().eq_ignore_ascii_case("ce-source"))
        .map(|(_, v)| v.to_str().unwrap().to_string());
    assert_eq!(
        ce_type.as_deref(),
        Some("com.example.order.created"),
        "ce-type value should be forwarded"
    );
    assert_eq!(
        ce_source.as_deref(),
        Some("/myapp"),
        "ce-source value should be forwarded"
    );

    server.stop().await;
}

// Test 7: Event filtering

#[tokio::test]
async fn event_filtering() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
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
  app:
    type: event
handlers:
  paid-only:
    source: app
    events: ["order.*"]
    url: {mock_url}/jobs/paid
    filter: "$.status == paid"
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19717).await;
    let c = http();

    // Should be filtered OUT
    c.post(server.url("/events/app/order.created"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"status": "pending", "id": "ord_1"}))
        .send()
        .await
        .unwrap();

    // Should pass filter
    c.post(server.url("/events/app/order.updated"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"status": "paid", "id": "ord_2"}))
        .send()
        .await
        .unwrap();

    wait_for_mock(&mock, 1, 5).await;
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let reqs = mock.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1, "Only the paid event should be delivered");

    // Verify it was the paid event (ord_2), not the pending one
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["status"], "paid");
    assert_eq!(body["id"], "ord_2");

    server.stop().await;
}

// Test 8: Payload transformation

#[tokio::test]
async fn payload_transformation() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
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
  app:
    type: event
handlers:
  transform-test:
    source: app
    events: [transform.test]
    url: {mock_url}/jobs/transform
    transform: '{{"event_id": "{{{{$.id}}}}", "amount": {{{{$.data.amount}}}}}}'
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19718).await;

    http()
        .post(server.url("/events/app/transform.test"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"id": "evt_t1", "data": {"amount": 42, "extra": "ignored"}}))
        .send()
        .await
        .unwrap();

    wait_for_mock(&mock, 1, 10).await;

    let reqs = mock.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["event_id"], "evt_t1");
    assert_eq!(body["amount"], 42);
    assert!(
        body.get("extra").is_none(),
        "extra field should not be present"
    );

    server.stop().await;
}

// Test 9: IP rate limiting

#[tokio::test]
async fn ip_rate_limiting() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;

    let yaml = format!(
        r#"
database:
  driver: sqlite
  url: "sqlite:__DB_PATH__?mode=rwc"
server:
  port: __PORT__
  ip_rate_limit: 3
  allow_private_urls: true
sources:
  app:
    type: event
handlers:
  rate-test:
    source: app
    events: [rate.test]
    url: {mock_url}/jobs/rate
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19719).await;
    let c = http();

    let mut codes = Vec::new();
    for i in 0..5 {
        let resp = c
            .post(server.url("/events/app/rate.test"))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({"i": i}))
            .send()
            .await
            .unwrap();
        codes.push(resp.status().as_u16());
    }

    let accepted = codes.iter().filter(|c| **c == 202).count();
    let rate_limited = codes.iter().filter(|c| **c == 429).count();
    assert!(
        accepted >= 1,
        "At least some requests should be accepted (202), got: {:?}",
        codes
    );
    assert!(
        rate_limited >= 1,
        "At least some requests should be rate-limited (429), got: {:?}",
        codes
    );
    assert_eq!(
        accepted + rate_limited,
        5,
        "All responses should be 202 or 429, got: {:?}",
        codes
    );

    server.stop().await;
}

// Test 10: auth_token protection

#[tokio::test]
async fn auth_token_protection() {
    let yaml = r#"
database:
  driver: sqlite
  url: "sqlite:__DB_PATH__?mode=rwc"
server:
  port: __PORT__
  allow_private_urls: true
api:
  auth_token: secret-token-123
sources:
  app:
    type: event
handlers: {}
"#;

    let server = QhookProcess::start(yaml, 19720).await;
    let c = http();

    // No token → 401
    assert_eq!(
        c.post(server.url("/events/app/auth.test"))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({"test": true}))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );

    // Wrong token → 401
    assert_eq!(
        c.post(server.url("/events/app/auth.test"))
            .header("Content-Type", "application/json")
            .header("Authorization", "Bearer wrong-token")
            .json(&serde_json::json!({"test": true}))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );

    // Correct token → 202
    assert_eq!(
        c.post(server.url("/events/app/auth.test"))
            .header("Content-Type", "application/json")
            .header("Authorization", "Bearer secret-token-123")
            .json(&serde_json::json!({"test": true}))
            .send()
            .await
            .unwrap()
            .status(),
        202
    );

    server.stop().await;
}

// Test 11: HTTP method specification

#[tokio::test]
async fn http_method_specification() {
    let mock = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/jobs/put"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/jobs/get"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/jobs/post"))
        .respond_with(ResponseTemplate::new(200))
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
  app:
    type: event
handlers:
  put-handler:
    source: app
    events: [resource.update]
    url: {mock_url}/jobs/put
    method: PUT
    retry:
      max: 0
  get-handler:
    source: app
    events: [resource.check]
    url: {mock_url}/jobs/get
    method: GET
    retry:
      max: 0
  default-handler:
    source: app
    events: [resource.create]
    url: {mock_url}/jobs/post
    retry:
      max: 0
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19721).await;
    let c = http();

    c.post(server.url("/events/app/resource.update"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"id": "res_1"}))
        .send()
        .await
        .unwrap();
    c.post(server.url("/events/app/resource.check"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"id": "res_2"}))
        .send()
        .await
        .unwrap();
    c.post(server.url("/events/app/resource.create"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"id": "res_3"}))
        .send()
        .await
        .unwrap();

    wait_for_mock(&mock, 3, 10).await;

    let reqs = mock.received_requests().await.unwrap();

    let put_req = reqs.iter().find(|r| r.url.path() == "/jobs/put").unwrap();
    assert_eq!(put_req.method.as_str(), "PUT");

    let get_req = reqs.iter().find(|r| r.url.path() == "/jobs/get").unwrap();
    assert_eq!(get_req.method.as_str(), "GET");
    assert!(get_req.body.is_empty(), "GET request should have no body");

    let post_req = reqs.iter().find(|r| r.url.path() == "/jobs/post").unwrap();
    assert_eq!(post_req.method.as_str(), "POST");

    server.stop().await;
}

// Test 12: Cron trigger

#[tokio::test]
async fn cron_trigger() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
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
  heartbeat:
    type: cron
    schedule: "*/3 * * * * *"
handlers:
  on-tick:
    source: heartbeat
    url: {mock_url}/jobs/cron
    retry:
      max: 0
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19722).await;

    // Wait for at least one cron fire (~3 seconds + margin)
    wait_for_mock(&mock, 1, 8).await;

    let reqs = mock.received_requests().await.unwrap();
    assert!(!reqs.is_empty());

    // Check cron payload has source and fired_at
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["source"], "heartbeat");
    assert!(body["fired_at"].is_string());

    server.stop().await;
}
