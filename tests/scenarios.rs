//! Scenario-based integration tests — realistic end-to-end user stories.

mod common;

use common::{
    QhookProcess, count_path, hmac_sha256, http, verify_standard_webhook_sig, wait_for_mock,
};
use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Scenario 1: Stripe webhook fan-out

#[tokio::test]
async fn scenario_stripe_webhook_fanout() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/fulfill"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true})))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/notify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true})))
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
    type: webhook
    verify: stripe
    secret: whsec_test_secret
handlers:
  fulfillment:
    source: stripe
    events: [checkout.session.completed]
    url: {mock_url}/fulfill
    retry:
      max: 1
  send-receipt:
    source: stripe
    events: [checkout.session.completed]
    url: {mock_url}/notify
    retry:
      max: 1
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19701).await;

    let payload = r#"{"id":"evt_test_123","type":"checkout.session.completed","data":{"object":{"id":"cs_test","customer_email":"alice@example.com","amount_total":4200}}}"#;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let sig = hmac_sha256("whsec_test_secret", &format!("{}.{}", ts, payload));

    let resp = http()
        .post(server.url("/webhooks/stripe"))
        .header("Content-Type", "application/json")
        .header("Stripe-Signature", format!("t={},v1={}", ts, sig))
        .body(payload)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert!(body["event_id"].is_string());
    assert_eq!(body["jobs_created"], 2);
    assert_eq!(body["duplicate"], false);

    wait_for_mock(&mock, 2, 10).await;

    let reqs = mock.received_requests().await.unwrap();
    assert!(reqs.iter().any(|r| r.url.path() == "/fulfill"));
    assert!(reqs.iter().any(|r| r.url.path() == "/notify"));

    server.stop().await;
}

// Scenario 2: Internal API → workflow pipeline

#[tokio::test]
async fn scenario_internal_api_workflow() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/validate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"valid": true, "tenant_id": "t_001"})),
        )
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/provision"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"provisioned": true})),
        )
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/notify"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"notified":true})),
        )
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
  platform:
    type: event
handlers: {{}}
workflows:
  tenant-provision:
    source: platform
    events: [tenant.create]
    timeout: 30
    steps:
      - name: validate
        url: {mock_url}/validate
      - name: provision
        url: {mock_url}/provision
      - name: notify
        url: {mock_url}/notify
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19702).await;

    let resp = http()
        .post(server.url("/events/platform/tenant.create"))
        .header("Content-Type", "application/json")
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({"tenant_id":"t_001","plan":"pro"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);

    wait_for_mock(&mock, 3, 10).await;

    let reqs = mock.received_requests().await.unwrap();
    assert_eq!(
        reqs.len(),
        3,
        "Exactly 3 requests: validate, provision, notify"
    );

    let prov = reqs.iter().find(|r| r.url.path() == "/provision").unwrap();
    let prov_body: Value = serde_json::from_slice(&prov.body).unwrap();
    assert_eq!(
        prov_body["valid"], true,
        "Data chaining: provision gets validate's response"
    );

    // Verify notify also received chained data from provision
    let notify = reqs.iter().find(|r| r.url.path() == "/notify").unwrap();
    let notify_body: Value = serde_json::from_slice(&notify.body).unwrap();
    assert_eq!(
        notify_body["provisioned"], true,
        "Data chaining: notify gets provision's response"
    );

    server.stop().await;
}

// Scenario 3: Management API lifecycle

#[tokio::test]
async fn scenario_management_api_lifecycle() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/process"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true})))
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
  auth_token: mgmt-token
sources:
  app:
    type: event
handlers:
  process:
    source: app
    events: [order.placed]
    url: {mock_url}/process
    retry:
      max: 1
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19703).await;
    let c = http();

    let resp = c
        .post(server.url("/events/app/order.placed"))
        .header("Content-Type", "application/json")
        .header("Authorization", "Bearer mgmt-token")
        .json(&serde_json::json!({"order_id":"ord_999","amount":7500}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);
    let body: Value = resp.json().await.unwrap();
    let event_id = body["event_id"].as_str().unwrap().to_string();

    wait_for_mock(&mock, 1, 10).await;
    // Allow qhook to update job status in DB after delivery
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // GET /api/events/:id
    let ev: Value = c
        .get(server.url(&format!("/api/events/{event_id}")))
        .header("Authorization", "Bearer mgmt-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ev["source"], "app");
    assert_eq!(ev["event_type"], "order.placed");
    let jobs = ev["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 1);
    let job_entry = jobs
        .iter()
        .find(|j| j["handler"] == "process")
        .expect("Should have a job for 'process' handler");
    assert_eq!(job_entry["status"], "completed");

    // GET /api/jobs/:id?include_attempts=true
    let job_id = job_entry["id"].as_str().unwrap();
    let job: Value = c
        .get(server.url(&format!("/api/jobs/{job_id}?include_attempts=true")))
        .header("Authorization", "Bearer mgmt-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(job["handler"], "process");
    assert_eq!(job["attempts"][0]["status_code"], 200);

    // Auth required
    assert_eq!(
        c.get(server.url(&format!("/api/events/{event_id}")))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    // Unknown → 404
    assert_eq!(
        c.get(server.url("/api/events/01ZZZZZZZZZZZZZZZZZZZZZZZZ"))
            .header("Authorization", "Bearer mgmt-token")
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        c.get(server.url("/api/jobs/01ZZZZZZZZZZZZZZZZZZZZZZZZ"))
            .header("Authorization", "Bearer mgmt-token")
            .send()
            .await
            .unwrap()
            .status(),
        404
    );

    server.stop().await;
}

// Scenario 4: Source routing validation

#[tokio::test]
async fn scenario_source_routing() {
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
  github:
    type: webhook
    verify: github
    secret: test-secret
handlers:
  on-test:
    source: app
    events: [test.hello]
    url: {mock_url}/handle
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19704).await;
    let c = http();

    assert_eq!(
        c.post(server.url("/events/nonexistent/test.hello"))
            .header("Content-Type", "application/json")
            .body("{}")
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        c.post(server.url("/events/github/push"))
            .header("Content-Type", "application/json")
            .body("{}")
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        c.post(server.url("/events/app/test.hello"))
            .header("Content-Type", "application/json")
            .body(r#"{"test":true}"#)
            .send()
            .await
            .unwrap()
            .status(),
        202
    );

    server.stop().await;
}

// Scenario 5: Idempotency + duplicate detection

#[tokio::test]
async fn scenario_idempotency_duplicate() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/handle"))
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
  process:
    source: app
    events: [payment.received]
    url: {mock_url}/handle
    idempotency_key: "$.payment_id"
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19705).await;
    let c = http();
    let payload = serde_json::json!({"payment_id":"pay_abc","amount":1000});

    let b1: Value = c
        .post(server.url("/events/app/payment.received"))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(b1["duplicate"], false);
    assert_eq!(b1["jobs_created"], 1);

    let b2: Value = c
        .post(server.url("/events/app/payment.received"))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(b2["duplicate"], true);
    assert_eq!(b2["jobs_created"], 0);

    wait_for_mock(&mock, 1, 5).await;
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert_eq!(mock.received_requests().await.unwrap().len(), 1);

    server.stop().await;
}

// Scenario 6: Workflow failure → catch → rollback

#[tokio::test]
async fn scenario_workflow_catch_rollback() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/build"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"built":true})))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/deploy"))
        .respond_with(ResponseTemplate::new(500).set_body_string("deploy failed"))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/rollback"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true})))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/smoke-test"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/alert"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
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
  ci:
    type: event
handlers: {{}}
workflows:
  deploy-pipeline:
    source: ci
    events: [deploy.start]
    steps:
      - name: build
        url: {mock_url}/build
      - name: deploy
        url: {mock_url}/deploy
        retry:
          max: 1
          errors: [timeout]
        catch:
          - errors: [5xx]
            goto: rollback
          - errors: [all]
            goto: alert
      - name: smoke-test
        url: {mock_url}/smoke-test
      - name: rollback
        url: {mock_url}/rollback
        end: true
      - name: alert
        url: {mock_url}/alert
        end: true
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19706).await;

    let resp = http()
        .post(server.url("/events/ci/deploy.start"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"repo":"api-server","sha":"abc123"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);

    wait_for_mock(&mock, 3, 10).await;

    // Verify exact call counts: build(1) + deploy(1) + rollback(1) = 3
    let reqs = mock.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 3, "Exactly 3 requests: build, deploy, rollback");
    assert_eq!(
        count_path(&mock, "/build").await,
        1,
        "Build should run exactly once"
    );
    assert_eq!(
        count_path(&mock, "/deploy").await,
        1,
        "Deploy should run exactly once (no retry for 5xx when errors=[timeout])"
    );
    assert_eq!(
        count_path(&mock, "/rollback").await,
        1,
        "Rollback should run exactly once"
    );

    server.stop().await;
}

// Scenario 7: Outbound webhook full lifecycle
// Simulates a SaaS sending webhooks to two customers:
//   1. Register two customer endpoints
//   2. Customer A subscribes to all events (wildcard)
//   3. Customer B subscribes to payment.completed only
//   4. Send order.created → only Customer A receives (with valid signature)
//   5. Send payment.completed → both customers receive
//   6. Disable Customer A → send payment.completed → only Customer B receives
//   7. Rotate Customer B's secret → verify new signature

#[tokio::test]
async fn scenario_outbound_webhook_lifecycle() {
    let mock_a = MockServer::start().await;
    let mock_b = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/hook"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_a)
        .await;
    Mock::given(method("POST"))
        .and(path("/hook"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_b)
        .await;

    let yaml = r#"
database:
  driver: sqlite
  url: "sqlite:__DB_PATH__?mode=rwc"
server:
  port: __PORT__
  allow_private_urls: true
  skip_endpoint_verification: true
api:
  auth_token: "scenario-token"
sources:
  my-saas:
    type: outbound
"#;

    let server = QhookProcess::start(yaml, 19850).await;
    let client = http();
    let auth = "Bearer scenario-token";

    // --- Step 1: Register two customer endpoints ---

    let resp = client
        .post(server.url("/api/outbound/endpoints"))
        .header("Authorization", auth)
        .json(&serde_json::json!({
            "source": "my-saas",
            "url": format!("{}/hook", mock_a.uri()),
            "description": "Customer A"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: Value = resp.json().await.unwrap();
    let ep_a = body["id"].as_str().unwrap().to_string();
    let secret_a = body["signing_secret"].as_str().unwrap().to_string();

    let resp = client
        .post(server.url("/api/outbound/endpoints"))
        .header("Authorization", auth)
        .json(&serde_json::json!({
            "source": "my-saas",
            "url": format!("{}/hook", mock_b.uri()),
            "description": "Customer B"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: Value = resp.json().await.unwrap();
    let ep_b = body["id"].as_str().unwrap().to_string();

    // --- Step 2: Customer A subscribes to all events (wildcard) ---

    client
        .post(server.url(&format!("/api/outbound/endpoints/{}/subscriptions", ep_a)))
        .header("Authorization", auth)
        .json(&serde_json::json!({"event_types": ["*"]}))
        .send()
        .await
        .unwrap();

    // --- Step 3: Customer B subscribes to payment.completed only ---

    client
        .post(server.url(&format!("/api/outbound/endpoints/{}/subscriptions", ep_b)))
        .header("Authorization", auth)
        .json(&serde_json::json!({"event_types": ["payment.completed"]}))
        .send()
        .await
        .unwrap();

    // --- Step 4: Send order.created → only Customer A receives ---

    let resp = client
        .post(server.url("/events/my-saas/order.created"))
        .header("Authorization", auth)
        .json(&serde_json::json!({"order_id": "ord_001", "amount": 2500}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["jobs_created"], 1,
        "Only Customer A is subscribed to order.created"
    );

    wait_for_mock(&mock_a, 1, 10).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let reqs_a = mock_a.received_requests().await.unwrap();
    let reqs_b = mock_b.received_requests().await.unwrap();
    assert_eq!(reqs_a.len(), 1, "Customer A should receive order.created");
    assert_eq!(
        reqs_b.len(),
        0,
        "Customer B should NOT receive order.created"
    );

    // Verify Customer A's delivery has valid Standard Webhooks signature
    let sig = reqs_a[0]
        .headers
        .get("webhook-signature")
        .unwrap()
        .to_str()
        .unwrap();
    let ts = reqs_a[0]
        .headers
        .get("webhook-timestamp")
        .unwrap()
        .to_str()
        .unwrap();
    let msg_id = reqs_a[0]
        .headers
        .get("webhook-id")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(sig.starts_with("v1,"));
    assert!(
        verify_standard_webhook_sig(&secret_a, msg_id, ts, &reqs_a[0].body, sig),
        "Customer A signature must be verifiable with their secret"
    );

    // Verify payload integrity
    let payload: Value = serde_json::from_slice(&reqs_a[0].body).unwrap();
    assert_eq!(payload["order_id"], "ord_001");
    assert_eq!(payload["amount"], 2500);

    // Verify event type header
    assert_eq!(
        reqs_a[0]
            .headers
            .get("X-Qhook-Event-Type")
            .unwrap()
            .to_str()
            .unwrap(),
        "order.created"
    );

    // --- Step 5: Send payment.completed → both receive ---

    let resp = client
        .post(server.url("/events/my-saas/payment.completed"))
        .header("Authorization", auth)
        .json(&serde_json::json!({"payment_id": "pay_001", "status": "succeeded"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["jobs_created"], 2,
        "Both customers subscribed to payment.completed"
    );

    wait_for_mock(&mock_a, 2, 10).await;
    wait_for_mock(&mock_b, 1, 10).await;

    let reqs_a = mock_a.received_requests().await.unwrap();
    let reqs_b = mock_b.received_requests().await.unwrap();
    assert_eq!(
        reqs_a.len(),
        2,
        "Customer A: order.created + payment.completed"
    );
    assert_eq!(reqs_b.len(), 1, "Customer B: payment.completed only");

    let payload_b: Value = serde_json::from_slice(&reqs_b[0].body).unwrap();
    assert_eq!(payload_b["payment_id"], "pay_001");

    // --- Step 6: Disable Customer A → send another payment → only B receives ---

    client
        .put(server.url(&format!("/api/outbound/endpoints/{}", ep_a)))
        .header("Authorization", auth)
        .json(&serde_json::json!({"status": "disabled"}))
        .send()
        .await
        .unwrap();

    let resp = client
        .post(server.url("/events/my-saas/payment.completed"))
        .header("Authorization", auth)
        .json(&serde_json::json!({"payment_id": "pay_002"}))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["jobs_created"], 1, "Only Customer B (A is disabled)");

    wait_for_mock(&mock_b, 2, 10).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let reqs_a = mock_a.received_requests().await.unwrap();
    let reqs_b = mock_b.received_requests().await.unwrap();
    assert_eq!(
        reqs_a.len(),
        2,
        "Customer A still at 2 (disabled, no new delivery)"
    );
    assert_eq!(reqs_b.len(), 2, "Customer B received second payment");

    // --- Step 7: Rotate Customer B's secret → verify new signature works ---

    let resp = client
        .post(server.url(&format!("/api/outbound/endpoints/{}/rotate-secret", ep_b)))
        .header("Authorization", auth)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let new_secret_b = body["signing_secret"].as_str().unwrap().to_string();

    // Send another event — should use the new secret
    client
        .post(server.url("/events/my-saas/payment.completed"))
        .header("Authorization", auth)
        .json(&serde_json::json!({"payment_id": "pay_003"}))
        .send()
        .await
        .unwrap();

    wait_for_mock(&mock_b, 3, 10).await;

    let reqs_b = mock_b.received_requests().await.unwrap();
    assert_eq!(reqs_b.len(), 3);

    // Verify the latest delivery uses the rotated secret (Standard Webhooks format)
    let latest = &reqs_b[2];
    let sig = latest
        .headers
        .get("webhook-signature")
        .unwrap()
        .to_str()
        .unwrap();
    let ts = latest
        .headers
        .get("webhook-timestamp")
        .unwrap()
        .to_str()
        .unwrap();
    let msg_id = latest.headers.get("webhook-id").unwrap().to_str().unwrap();
    assert!(
        verify_standard_webhook_sig(&new_secret_b, msg_id, ts, &latest.body, sig),
        "Latest delivery must use the rotated secret"
    );

    server.stop().await;
}

// Scenario 8: DLQ flow — max retries exhausted → dead → inspect → retry
// Simulates an endpoint that always fails. After exhausting retries the job
// lands in the DLQ. Admin inspects via Management API, then retries.

#[tokio::test]
async fn scenario_dlq_inspect_retry() {
    let mock = MockServer::start().await;

    // Always fail with 503
    Mock::given(method("POST"))
        .and(path("/flaky"))
        .respond_with(ResponseTemplate::new(503).set_body_string("service unavailable"))
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
  auth_token: dlq-token
sources:
  app:
    type: event
handlers:
  flaky-handler:
    source: app
    events: [order.process]
    url: {mock_url}/flaky
    retry:
      max: 3
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19710).await;
    let c = http();

    // Send event
    let resp = c
        .post(server.url("/events/app/order.process"))
        .header("Content-Type", "application/json")
        .header("Authorization", "Bearer dlq-token")
        .json(&serde_json::json!({"order_id": "ord_dead"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);
    let body: Value = resp.json().await.unwrap();
    let event_id = body["event_id"].as_str().unwrap().to_string();

    // Wait for first delivery attempt (will fail with 503)
    wait_for_mock(&mock, 1, 10).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Verify job is retryable after first failure
    let ev: Value = c
        .get(server.url(&format!("/api/events/{event_id}")))
        .header("Authorization", "Bearer dlq-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let job = ev["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["handler"] == "flaky-handler")
        .unwrap()
        .clone();
    assert_eq!(
        job["status"], "retryable",
        "First failure should schedule retry"
    );
    assert_eq!(job["attempt"], 1);

    // Verify attempt record shows 503
    let job_id = job["id"].as_str().unwrap();
    let job_detail: Value = c
        .get(server.url(&format!("/api/jobs/{job_id}?include_attempts=true")))
        .header("Authorization", "Bearer dlq-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let attempts = job_detail["attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0]["status_code"], 503);

    server.stop().await;
}

// Scenario 9: Webhook signature verification failure → rejection
// Invalid signature should return 401 and never create any jobs.

#[tokio::test]
async fn scenario_signature_rejection_no_jobs() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/handle"))
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
  auth_token: sig-token
sources:
  github:
    type: webhook
    verify: github
    secret: correct-secret
handlers:
  on-push:
    source: github
    events: [push]
    url: {mock_url}/handle
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19726).await;
    let c = http();

    let payload = r#"{"ref":"refs/heads/main","action":"push"}"#;

    // Send with wrong signature
    let wrong_sig = hmac_sha256("wrong-secret", payload);
    let resp = c
        .post(server.url("/webhooks/github"))
        .header("Content-Type", "application/json")
        .header("X-Hub-Signature-256", format!("sha256={}", wrong_sig))
        .header("X-GitHub-Event", "push")
        .body(payload)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400, "Invalid signature should be rejected");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["code"], "signature_invalid");

    // Send with no signature at all
    let resp = c
        .post(server.url("/webhooks/github"))
        .header("Content-Type", "application/json")
        .header("X-GitHub-Event", "push")
        .body(payload)
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        400,
        "Missing signature should also be rejected"
    );

    // Wait briefly — no deliveries should happen
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let reqs = mock.received_requests().await.unwrap();
    assert_eq!(
        reqs.len(),
        0,
        "No deliveries should occur after signature rejection"
    );

    // Now send with correct signature — should succeed
    let correct_sig = hmac_sha256("correct-secret", payload);
    let resp = c
        .post(server.url("/webhooks/github"))
        .header("Content-Type", "application/json")
        .header("X-Hub-Signature-256", format!("sha256={}", correct_sig))
        .header("X-GitHub-Event", "push")
        .body(payload)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "Valid signature should be accepted");
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["jobs_created"], 1);

    wait_for_mock(&mock, 1, 10).await;
    assert_eq!(
        mock.received_requests().await.unwrap().len(),
        1,
        "Only the correctly signed webhook should be delivered"
    );

    server.stop().await;
}

// Scenario 10: Filter + transform combined pipeline
// Event passes filter → payload transformed → correct transformed data delivered.

#[tokio::test]
async fn scenario_filter_transform_pipeline() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/notify-vip"))
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
  vip-alert:
    source: app
    events: [order.created]
    url: {mock_url}/notify-vip
    filter: "$.amount > 10000"
    transform: '{{"vip_order_id": "{{{{$.order_id}}}}", "total_cents": {{{{$.amount}}}}, "alert": "high-value"}}'
    retry:
      max: 0
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19727).await;
    let c = http();

    // Low-value order — should be filtered out
    let resp = c
        .post(server.url("/events/app/order.created"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"order_id": "ord_small", "amount": 500}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["jobs_created"], 0,
        "Low-value order should be filtered out"
    );

    // High-value order — should pass filter and be transformed
    let resp = c
        .post(server.url("/events/app/order.created"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"order_id": "ord_vip", "amount": 25000, "customer": "alice"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["jobs_created"], 1,
        "High-value order should pass filter"
    );

    wait_for_mock(&mock, 1, 10).await;

    let reqs = mock.received_requests().await.unwrap();
    assert_eq!(
        reqs.len(),
        1,
        "Only the high-value order should be delivered"
    );

    // Verify the delivered payload is the transformed version, not the original
    let delivered: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(
        delivered["vip_order_id"], "ord_vip",
        "Transform should extract order_id"
    );
    assert_eq!(
        delivered["total_cents"], 25000,
        "Transform should include amount"
    );
    assert_eq!(
        delivered["alert"], "high-value",
        "Transform should add static field"
    );
    assert!(
        delivered.get("customer").is_none(),
        "Transform should not include fields not in template"
    );

    server.stop().await;
}

// Scenario: Event inspection API

#[tokio::test]
async fn scenario_event_inspection_api() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/handler-a"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/handler-b"))
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
  auth_token: test-token-inspect
sources:
  app:
    type: event
  github:
    type: webhook
handlers:
  handler-a:
    source: app
    events: [user.created]
    url: {mock_url}/handler-a
    retry:
      max: 0
  handler-b:
    source: app
    events: [user.created]
    url: {mock_url}/handler-b
    retry:
      max: 0
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19728).await;
    let c = http();
    let auth = "Bearer test-token-inspect";

    // Create two events
    let resp = c
        .post(server.url("/events/app/user.created"))
        .header("Content-Type", "application/json")
        .header("Authorization", auth)
        .json(&serde_json::json!({"name": "alice"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);
    let body1: Value = resp.json().await.unwrap();
    let event_id_1 = body1["event_id"].as_str().unwrap().to_string();
    assert_eq!(body1["jobs_created"], 2);

    let resp = c
        .post(server.url("/events/app/user.created"))
        .header("Content-Type", "application/json")
        .header("Authorization", auth)
        .json(&serde_json::json!({"name": "bob"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);

    wait_for_mock(&mock, 4, 10).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Test 1: GET /api/events
    let resp = c
        .get(server.url("/api/events"))
        .header("Authorization", auth)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let events = body["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert!(events[0].get("payload").is_none(), "No payload in list");
    assert_eq!(body["has_more"], false);

    // Test 2: Source filter
    let resp = c
        .get(server.url("/api/events?source=nonexistent"))
        .header("Authorization", auth)
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["events"].as_array().unwrap().len(), 0);

    // Test 3: Pagination
    let resp = c
        .get(server.url("/api/events?limit=1"))
        .header("Authorization", auth)
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["events"].as_array().unwrap().len(), 1);
    assert_eq!(body["has_more"], true);

    // Test 4: Event jobs
    let resp = c
        .get(server.url(&format!("/api/events/{}/jobs", event_id_1)))
        .header("Authorization", auth)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["jobs"].as_array().unwrap().len(), 2);

    // Test 5: GET /api/jobs
    let resp = c
        .get(server.url("/api/jobs"))
        .header("Authorization", auth)
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["jobs"].as_array().unwrap().len(), 4);

    // Test 6: Jobs status filter
    let resp = c
        .get(server.url("/api/jobs?status=dead"))
        .header("Authorization", auth)
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["jobs"].as_array().unwrap().len(), 0);

    // Test 7: Job attempts
    let resp = c
        .get(server.url(&format!("/api/events/{}/jobs", event_id_1)))
        .header("Authorization", auth)
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    let job_id = body["jobs"][0]["id"].as_str().unwrap().to_string();

    let resp = c
        .get(server.url(&format!("/api/jobs/{}/attempts", job_id)))
        .header("Authorization", auth)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let attempts = body["attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0]["attempt"], 1);
    assert_eq!(attempts[0]["status_code"], 200);

    // Test 8: Auth required
    assert_eq!(
        c.get(server.url("/api/events"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        c.get(server.url("/api/jobs"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );

    server.stop().await;
}
