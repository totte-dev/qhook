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
