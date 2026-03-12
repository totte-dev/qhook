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

    // Verify specific X-Qhook delivery headers
    let headers = &reqs[0].headers;
    assert!(
        headers.get("X-Qhook-Job-ID").is_some(),
        "X-Qhook-Job-ID header should be present"
    );
    assert!(
        headers.get("X-Qhook-Event-ID").is_some(),
        "X-Qhook-Event-ID header should be present"
    );
    assert_eq!(
        headers.get("X-Qhook-Handler").unwrap().to_str().unwrap(),
        "on-test",
        "X-Qhook-Handler should match handler name"
    );
    assert_eq!(
        headers.get("X-Qhook-Attempt").unwrap().to_str().unwrap(),
        "1",
        "X-Qhook-Attempt should be 1 for first delivery"
    );

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
    // ip_rate_limit=3 with sliding window counter: at most 3 per window.
    // Window boundary timing may cause 2-3 accepted out of 5 rapid requests.
    assert!(
        accepted <= 3,
        "At most 3 requests should be accepted per window (ip_rate_limit: 3), got: {:?}",
        codes
    );
    assert!(
        rate_limited >= 2,
        "At least 2 requests should be rate-limited (5 - 3 = 2), got: {:?}",
        codes
    );
    assert_eq!(
        accepted + rate_limited,
        5,
        "All responses should be either 202 or 429, got: {:?}",
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

// Test 13: IP allowlist — localhost (127.0.0.1) is allowed

#[tokio::test]
async fn ip_allowlist_allowed() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/jobs/allowed"))
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
  test-source:
    type: webhook
    verify: hmac
    secret: "test-secret"
    allowed_ips:
      - "127.0.0.1/8"
handlers:
  handler:
    source: test-source
    url: "{}/jobs/allowed"
"#,
        mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19723).await;
    let client = http();

    let payload = r#"{"test": true}"#;
    let sig = hmac_sha256("test-secret", payload);

    let resp = client
        .post(server.url("/webhooks/test-source"))
        .header("Content-Type", "application/json")
        .header("X-Webhook-Signature", &sig)
        .body(payload)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    wait_for_mock(&mock, 1, 5).await;
    server.stop().await;
}

// Test 14: IP allowlist — request from non-allowed IP is rejected

#[tokio::test]
async fn ip_allowlist_rejected() {
    let yaml = r#"
database:
  driver: sqlite
  url: "sqlite:__DB_PATH__?mode=rwc"
server:
  port: __PORT__
  allow_private_urls: true
  trust_proxy: true
sources:
  restricted:
    type: webhook
    verify: hmac
    secret: "test-secret"
    allowed_ips:
      - "10.0.0.0/8"
handlers:
  handler:
    source: restricted
    url: "http://localhost:9999/unused"
"#;

    let server = QhookProcess::start(yaml, 19724).await;
    let client = http();

    let payload = r#"{"test": true}"#;
    let sig = hmac_sha256("test-secret", payload);

    // Request from a non-allowed IP via X-Forwarded-For
    let resp = client
        .post(server.url("/webhooks/restricted"))
        .header("Content-Type", "application/json")
        .header("X-Webhook-Signature", &sig)
        .header("X-Forwarded-For", "203.0.113.1")
        .body(payload)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 403);

    server.stop().await;
}

// Test 15: replay-local — replay JSONL events to a running server

#[tokio::test]
async fn replay_local() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/jobs/replay"))
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
api:
  auth_token: "test-token-replay"
sources:
  app:
    type: event
handlers:
  on-replay:
    source: app
    events: [order.created]
    url: {mock_url}/jobs/replay
    retry:
      max: 0
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19725).await;

    // Create a JSONL file (same format as `qhook export events`)
    let id = ulid::Ulid::new();
    let jsonl_path = format!("/tmp/qhook_test_replay_{}.jsonl", id);
    let config_path = format!("/tmp/qhook_test_replay_{}.yaml", id);
    let jsonl_content = r#"{"id":"evt_1","source":"app","event_type":"order.created","payload":{"item":"widget","qty":1},"created_at":"2026-01-01T00:00:00.000"}
{"id":"evt_2","source":"app","event_type":"order.created","payload":{"item":"gadget","qty":2},"created_at":"2026-01-01T00:01:00.000"}"#;
    std::fs::write(&jsonl_path, jsonl_content).unwrap();

    // Write a minimal config for replay-local (only needs port for default target)
    std::fs::write(
        &config_path,
        "database:\n  driver: sqlite\n  url: \"sqlite:/tmp/unused.db?mode=rwc\"\nserver:\n  port: 19725\nsources:\n  app:\n    type: event\nhandlers: {}\n",
    )
    .unwrap();

    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_qhook"))
        .args([
            "replay-local",
            &jsonl_path,
            "--target",
            &server.base_url,
            "--token",
            "test-token-replay",
            "-y",
            "--config",
            &config_path,
        ])
        .output()
        .await
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("2 ok"),
        "Expected 2 ok events, got stdout: {}, stderr: {}",
        stdout,
        String::from_utf8_lossy(&output.stderr),
    );

    // Wait for delivery
    wait_for_mock(&mock, 2, 10).await;

    let reqs = mock.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 2);

    let items: Vec<String> = reqs
        .iter()
        .map(|r| {
            let b: Value = serde_json::from_slice(&r.body).unwrap();
            b["item"].as_str().unwrap().to_string()
        })
        .collect();
    assert!(items.contains(&"widget".to_string()), "missing widget");
    assert!(items.contains(&"gadget".to_string()), "missing gadget");

    // Cleanup
    let _ = std::fs::remove_file(&jsonl_path);
    let _ = std::fs::remove_file(&config_path);
    server.stop().await;
}

// Test 16: replay-local with filters — only replay matching events

#[tokio::test]
async fn replay_local_filters() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/jobs/replay-filter"))
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
api:
  auth_token: "test-token-filter"
sources:
  stripe:
    type: event
  github:
    type: event
handlers:
  on-filter:
    source: stripe
    events: ["*"]
    url: {mock_url}/jobs/replay-filter
    retry:
      max: 0
  on-filter-gh:
    source: github
    events: ["*"]
    url: {mock_url}/jobs/replay-filter
    retry:
      max: 0
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19729).await;

    let id = ulid::Ulid::new();
    let jsonl_path = format!("/tmp/qhook_test_replay_filter_{}.jsonl", id);
    let config_path = format!("/tmp/qhook_test_replay_filter_{}.yaml", id);

    // Create JSONL with mixed events: different sources, event types, timestamps, statuses
    let jsonl_content = [
        r#"{"id":"evt_f1","source":"stripe","event_type":"payment.created","payload":{"amount":100},"created_at":"2026-01-15T10:00:00.000","status":"completed"}"#,
        r#"{"id":"evt_f2","source":"stripe","event_type":"payment.refunded","payload":{"amount":50},"created_at":"2026-02-20T12:00:00.000","status":"failed"}"#,
        r#"{"id":"evt_f3","source":"github","event_type":"push","payload":{"ref":"main"},"created_at":"2026-03-01T08:00:00.000","status":"completed"}"#,
        r#"{"id":"evt_f4","source":"stripe","event_type":"payment.created","payload":{"amount":200},"created_at":"2026-03-10T14:00:00.000","status":"completed"}"#,
        r#"{"id":"evt_f5","source":"github","event_type":"pull_request.opened","payload":{"pr":1},"created_at":"2026-03-15T16:00:00.000","status":"completed"}"#,
    ]
    .join("\n");
    std::fs::write(&jsonl_path, &jsonl_content).unwrap();

    std::fs::write(
        &config_path,
        "database:\n  driver: sqlite\n  url: \"sqlite:/tmp/unused.db?mode=rwc\"\nserver:\n  port: 19729\nsources:\n  stripe:\n    type: event\n  github:\n    type: event\nhandlers: {}\n",
    )
    .unwrap();

    // Test A: filter by --source stripe (should get 3 events: evt_f1, evt_f2, evt_f4)
    let output_a = tokio::process::Command::new(env!("CARGO_BIN_EXE_qhook"))
        .args([
            "replay-local",
            &jsonl_path,
            "--target",
            &server.base_url,
            "--token",
            "test-token-filter",
            "-y",
            "--config",
            &config_path,
            "--source",
            "stripe",
        ])
        .output()
        .await
        .unwrap();

    let stdout_a = String::from_utf8_lossy(&output_a.stdout);
    assert!(
        stdout_a.contains("3 ok"),
        "Source filter: expected 3 ok, got stdout: {}, stderr: {}",
        stdout_a,
        String::from_utf8_lossy(&output_a.stderr),
    );
    assert!(
        stdout_a.contains("Replaying 3 of 5 events"),
        "Source filter: expected 'Replaying 3 of 5 events' in stdout: {}",
        stdout_a,
    );

    wait_for_mock(&mock, 3, 10).await;

    // Verify exactly 3 requests made for source filter
    let reqs_a = mock.received_requests().await.unwrap();
    assert_eq!(
        reqs_a.len(),
        3,
        "Source filter: expected 3 requests, got {}",
        reqs_a.len()
    );

    // Reset mock for next test (re-mount because reset() clears mounts too)
    mock.reset().await;
    Mock::given(method("POST"))
        .and(path("/jobs/replay-filter"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;

    // Test B: filter by --event-type with prefix wildcard "payment.*"
    let output_b = tokio::process::Command::new(env!("CARGO_BIN_EXE_qhook"))
        .args([
            "replay-local",
            &jsonl_path,
            "--target",
            &server.base_url,
            "--token",
            "test-token-filter",
            "-y",
            "--config",
            &config_path,
            "--event-type",
            "payment.*",
        ])
        .output()
        .await
        .unwrap();

    let stdout_b = String::from_utf8_lossy(&output_b.stdout);
    assert!(
        stdout_b.contains("3 ok"),
        "Event type prefix filter: expected 3 ok, got stdout: {}, stderr: {}",
        stdout_b,
        String::from_utf8_lossy(&output_b.stderr),
    );
    assert!(
        stdout_b.contains("Replaying 3 of 5 events"),
        "Event type prefix filter: expected 'Replaying 3 of 5 events' in stdout: {}",
        stdout_b,
    );

    wait_for_mock(&mock, 3, 10).await;
    let reqs_b = mock.received_requests().await.unwrap();
    assert_eq!(
        reqs_b.len(),
        3,
        "Event type prefix filter: expected 3 requests, got {}",
        reqs_b.len()
    );

    mock.reset().await;
    Mock::given(method("POST"))
        .and(path("/jobs/replay-filter"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;

    // Test C: filter by --since and --until (March 2026 only: evt_f3, evt_f4, evt_f5)
    let output_c = tokio::process::Command::new(env!("CARGO_BIN_EXE_qhook"))
        .args([
            "replay-local",
            &jsonl_path,
            "--target",
            &server.base_url,
            "--token",
            "test-token-filter",
            "-y",
            "--config",
            &config_path,
            "--since",
            "2026-03-01T00:00:00",
            "--until",
            "2026-03-31T23:59:59",
        ])
        .output()
        .await
        .unwrap();

    let stdout_c = String::from_utf8_lossy(&output_c.stdout);
    assert!(
        stdout_c.contains("3 ok"),
        "Time range filter: expected 3 ok, got stdout: {}, stderr: {}",
        stdout_c,
        String::from_utf8_lossy(&output_c.stderr),
    );
    assert!(
        stdout_c.contains("Replaying 3 of 5 events"),
        "Time range filter: expected 'Replaying 3 of 5 events' in stdout: {}",
        stdout_c,
    );

    wait_for_mock(&mock, 3, 10).await;
    let reqs_c = mock.received_requests().await.unwrap();
    assert_eq!(
        reqs_c.len(),
        3,
        "Time range filter: expected 3 requests, got {}",
        reqs_c.len()
    );

    mock.reset().await;
    Mock::given(method("POST"))
        .and(path("/jobs/replay-filter"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;

    // Test D: filter by --status failed (only evt_f2)
    let output_d = tokio::process::Command::new(env!("CARGO_BIN_EXE_qhook"))
        .args([
            "replay-local",
            &jsonl_path,
            "--target",
            &server.base_url,
            "--token",
            "test-token-filter",
            "-y",
            "--config",
            &config_path,
            "--status",
            "failed",
        ])
        .output()
        .await
        .unwrap();

    let stdout_d = String::from_utf8_lossy(&output_d.stdout);
    assert!(
        stdout_d.contains("1 ok"),
        "Status filter: expected 1 ok, got stdout: {}, stderr: {}",
        stdout_d,
        String::from_utf8_lossy(&output_d.stderr),
    );
    assert!(
        stdout_d.contains("Replaying 1 of 5 events"),
        "Status filter: expected 'Replaying 1 of 5 events' in stdout: {}",
        stdout_d,
    );

    wait_for_mock(&mock, 1, 10).await;
    let reqs_d = mock.received_requests().await.unwrap();
    assert_eq!(
        reqs_d.len(),
        1,
        "Status filter: expected 1 request, got {}",
        reqs_d.len()
    );
    // Verify the correct event was sent (the refunded one)
    let body_d: Value = serde_json::from_slice(&reqs_d[0].body).unwrap();
    assert_eq!(
        body_d["amount"].as_i64().unwrap(),
        50,
        "Status filter: expected amount=50 for the failed event"
    );

    mock.reset().await;
    Mock::given(method("POST"))
        .and(path("/jobs/replay-filter"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;

    // Test E: combined filters --source stripe --since 2026-03-01T00:00:00 (only evt_f4)
    let output_e = tokio::process::Command::new(env!("CARGO_BIN_EXE_qhook"))
        .args([
            "replay-local",
            &jsonl_path,
            "--target",
            &server.base_url,
            "--token",
            "test-token-filter",
            "-y",
            "--config",
            &config_path,
            "--source",
            "stripe",
            "--since",
            "2026-03-01T00:00:00",
        ])
        .output()
        .await
        .unwrap();

    let stdout_e = String::from_utf8_lossy(&output_e.stdout);
    assert!(
        stdout_e.contains("1 ok"),
        "Combined filter: expected 1 ok, got stdout: {}, stderr: {}",
        stdout_e,
        String::from_utf8_lossy(&output_e.stderr),
    );
    assert!(
        stdout_e.contains("Replaying 1 of 5 events"),
        "Combined filter: expected 'Replaying 1 of 5 events' in stdout: {}",
        stdout_e,
    );

    wait_for_mock(&mock, 1, 10).await;
    let reqs_e = mock.received_requests().await.unwrap();
    assert_eq!(
        reqs_e.len(),
        1,
        "Combined filter: expected 1 request, got {}",
        reqs_e.len()
    );
    let body_e: Value = serde_json::from_slice(&reqs_e[0].body).unwrap();
    assert_eq!(
        body_e["amount"].as_i64().unwrap(),
        200,
        "Combined filter: expected amount=200 for stripe+march event"
    );

    // Test F: no filters — should replay all 5 events (backwards compatible)
    mock.reset().await;
    Mock::given(method("POST"))
        .and(path("/jobs/replay-filter"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;

    let output_f = tokio::process::Command::new(env!("CARGO_BIN_EXE_qhook"))
        .args([
            "replay-local",
            &jsonl_path,
            "--target",
            &server.base_url,
            "--token",
            "test-token-filter",
            "-y",
            "--config",
            &config_path,
        ])
        .output()
        .await
        .unwrap();

    let stdout_f = String::from_utf8_lossy(&output_f.stdout);
    assert!(
        stdout_f.contains("5 ok"),
        "No filter: expected 5 ok, got stdout: {}, stderr: {}",
        stdout_f,
        String::from_utf8_lossy(&output_f.stderr),
    );
    // When no filters, should NOT show "Replaying X of Y" but just "Loaded 5 event(s)"
    assert!(
        !stdout_f.contains("Replaying"),
        "No filter: should not show filtered message, got stdout: {}",
        stdout_f,
    );

    wait_for_mock(&mock, 5, 10).await;
    let reqs_f = mock.received_requests().await.unwrap();
    assert_eq!(
        reqs_f.len(),
        5,
        "No filter: expected 5 requests, got {}",
        reqs_f.len()
    );

    // Cleanup
    let _ = std::fs::remove_file(&jsonl_path);
    let _ = std::fs::remove_file(&config_path);
    server.stop().await;
}
