//! Scenario-based integration tests — realistic end-to-end user stories.

mod common;

use common::{QhookProcess, count_path, hmac_sha256, http, wait_for_mock};
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
    assert_eq!(jobs[0]["status"], "completed");

    // GET /api/jobs/:id?include_attempts=true
    let job_id = jobs[0]["id"].as_str().unwrap();
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

    // Verify Customer A's delivery has valid HMAC-SHA256 signature
    let sig = reqs_a[0]
        .headers
        .get("X-Qhook-Signature")
        .unwrap()
        .to_str()
        .unwrap();
    let ts = reqs_a[0]
        .headers
        .get("X-Qhook-Timestamp")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(sig.starts_with("v1="));
    let expected_sig = hmac_sha256(
        &secret_a,
        &format!("{}.{}", ts, std::str::from_utf8(&reqs_a[0].body).unwrap()),
    );
    assert_eq!(
        &sig[3..],
        expected_sig,
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

    // Verify the latest delivery uses the rotated secret
    let latest = &reqs_b[2];
    let sig = latest
        .headers
        .get("X-Qhook-Signature")
        .unwrap()
        .to_str()
        .unwrap();
    let ts = latest
        .headers
        .get("X-Qhook-Timestamp")
        .unwrap()
        .to_str()
        .unwrap();
    let expected_sig = hmac_sha256(
        &new_secret_b,
        &format!("{}.{}", ts, std::str::from_utf8(&latest.body).unwrap()),
    );
    assert_eq!(
        &sig[3..],
        expected_sig,
        "Latest delivery must use the rotated secret"
    );

    server.stop().await;
}
