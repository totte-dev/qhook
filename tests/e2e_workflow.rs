//! Workflow E2E tests — converted from e2e_workflow.sh.

mod common;

use common::{QhookProcess, count_path, http, wait_for_mock};
use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Standard workflow mock responses (matching mock_workflow_server.py).
async fn mount_workflow_mocks(mock: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/validate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"valid": true, "risk_score": 0.05})),
        )
        .mount(mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/fulfill"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"fulfilled": true, "tracking": "TRK-001"})),
        )
        .mount(mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/notify"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"notified": true})),
        )
        .mount(mock)
        .await;
}

// WF 1: 3-step sequential pipeline with data chaining

#[tokio::test]
async fn wf_sequential_pipeline() {
    let mock = MockServer::start().await;
    mount_workflow_mocks(&mock).await;

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
handlers: {{}}
workflows:
  order-flow:
    source: app
    events: [order.created]
    steps:
      - name: validate
        url: {mock_url}/validate
      - name: fulfill
        url: {mock_url}/fulfill
      - name: notify
        url: {mock_url}/notify
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19731).await;

    let resp = http()
        .post(server.url("/events/app/order.created"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"id": "ord_001", "amount": 5000}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);

    wait_for_mock(&mock, 3, 10).await;

    assert_eq!(count_path(&mock, "/validate").await, 1);
    assert_eq!(count_path(&mock, "/fulfill").await, 1);
    assert_eq!(count_path(&mock, "/notify").await, 1);

    // Data chaining: fulfill received validate's response
    let reqs = mock.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 3, "Exactly 3 requests total");
    let fulfill = reqs.iter().find(|r| r.url.path() == "/fulfill").unwrap();
    let body: Value = serde_json::from_slice(&fulfill.body).unwrap();
    assert_eq!(body["valid"], true);

    // Data chaining: notify received fulfill's response
    let notify = reqs.iter().find(|r| r.url.path() == "/notify").unwrap();
    let notify_body: Value = serde_json::from_slice(&notify.body).unwrap();
    assert_eq!(notify_body["fulfilled"], true);

    server.stop().await;
}

// WF 2: input transform + result_path

#[tokio::test]
async fn wf_input_transform_result_path() {
    let mock = MockServer::start().await;
    mount_workflow_mocks(&mock).await;

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
handlers: {{}}
workflows:
  enrich-flow:
    source: app
    events: [user.signup]
    steps:
      - name: enrich
        url: {mock_url}/validate
        input: '{{"user_id": "{{{{$.id}}}}"}}'
        result_path: "$.enrichment"
      - name: complete
        url: {mock_url}/fulfill
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19732).await;

    http()
        .post(server.url("/events/app/user.signup"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"id": "usr_001", "name": "Alice"}))
        .send()
        .await
        .unwrap();

    wait_for_mock(&mock, 2, 10).await;

    let reqs = mock.received_requests().await.unwrap();

    // Step 1 should receive transformed input
    let enrich = reqs.iter().find(|r| r.url.path() == "/validate").unwrap();
    let enrich_body: Value = serde_json::from_slice(&enrich.body).unwrap();
    assert_eq!(enrich_body["user_id"], "usr_001");
    assert!(
        enrich_body.get("name").is_none(),
        "name should not be in transformed input"
    );

    // Step 2 should have enrichment merged
    let complete = reqs.iter().find(|r| r.url.path() == "/fulfill").unwrap();
    let complete_body: Value = serde_json::from_slice(&complete.body).unwrap();
    assert_eq!(complete_body["enrichment"]["valid"], true);

    server.stop().await;
}

// WF 3: on_failure=continue

#[tokio::test]
async fn wf_on_failure_continue() {
    let mock = MockServer::start().await;
    // validate fails
    Mock::given(method("POST"))
        .and(path("/validate"))
        .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({"error":"fail"})))
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
delivery:
  default_retry:
    max: 1
sources:
  app:
    type: event
handlers: {{}}
workflows:
  continue-flow:
    source: app
    events: [test.continue]
    steps:
      - name: might-fail
        url: {mock_url}/validate
        on_failure: continue
        retry:
          max: 1
      - name: always-runs
        url: {mock_url}/notify
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19733).await;

    http()
        .post(server.url("/events/app/test.continue"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"test": true}))
        .send()
        .await
        .unwrap();

    // Wait for retry + continue + next step
    wait_for_mock(&mock, 2, 10).await;

    // Step 2 should run despite step 1 failure
    assert_eq!(
        count_path(&mock, "/notify").await,
        1,
        "Notify step should run exactly once"
    );

    // Step 2 should receive error info with correct values
    let reqs = mock.received_requests().await.unwrap();
    let notify = reqs.iter().find(|r| r.url.path() == "/notify").unwrap();
    let body: Value = serde_json::from_slice(&notify.body).unwrap();
    assert_eq!(
        body["failed_step"], "might-fail",
        "Should identify the failed step"
    );
    assert!(
        body["error"].is_string(),
        "Should have error message string"
    );

    server.stop().await;
}

// WF 4: catch error routing

#[tokio::test]
async fn wf_catch_error_routing() {
    let mock = MockServer::start().await;
    // validate returns 400 (4xx)
    Mock::given(method("POST"))
        .and(path("/validate"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/fulfill"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/bad-request"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"handled":true})))
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
  app:
    type: event
handlers: {{}}
workflows:
  catch-flow:
    source: app
    events: [test.catch]
    steps:
      - name: validate
        url: {mock_url}/validate
        retry:
          max: 1
          errors: [5xx, timeout]
        catch:
          - errors: [4xx]
            goto: handle-error
          - errors: [all]
            goto: alert
      - name: should-not-run
        url: {mock_url}/fulfill
      - name: handle-error
        url: {mock_url}/bad-request
        end: true
      - name: alert
        url: {mock_url}/alert
        end: true
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19734).await;

    http()
        .post(server.url("/events/app/test.catch"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"test": true}))
        .send()
        .await
        .unwrap();

    wait_for_mock(&mock, 2, 10).await;

    assert_eq!(
        count_path(&mock, "/bad-request").await,
        1,
        "4xx routed to handle-error exactly once"
    );
    assert_eq!(
        count_path(&mock, "/validate").await,
        1,
        "validate called exactly once"
    );
    // fulfill and alert expect(0) verified by wiremock on drop

    server.stop().await;
}

// WF 5: end step terminates workflow

#[tokio::test]
async fn wf_end_step() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true})))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/fulfill"))
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
  app:
    type: event
handlers: {{}}
workflows:
  end-flow:
    source: app
    events: [test.end]
    steps:
      - name: first
        url: {mock_url}/validate
        end: true
      - name: should-not-run
        url: {mock_url}/fulfill
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19735).await;

    http()
        .post(server.url("/events/app/test.end"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"test": true}))
        .send()
        .await
        .unwrap();

    wait_for_mock(&mock, 1, 5).await;
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    assert_eq!(
        count_path(&mock, "/validate").await,
        1,
        "First step should run exactly once"
    );
    // fulfill expect(0) verified by wiremock on drop

    server.stop().await;
}

// WF 6: handlers + workflows coexist

#[tokio::test]
async fn wf_handlers_workflows_coexist() {
    let mock = MockServer::start().await;
    mount_workflow_mocks(&mock).await;

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
  simple:
    source: app
    events: [dual.test]
    url: {mock_url}/notify
workflows:
  dual-flow:
    source: app
    events: [dual.test]
    steps:
      - name: process
        url: {mock_url}/validate
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19736).await;

    http()
        .post(server.url("/events/app/dual.test"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"dual": true}))
        .send()
        .await
        .unwrap();

    wait_for_mock(&mock, 2, 10).await;

    assert_eq!(
        count_path(&mock, "/notify").await,
        1,
        "handler should fire exactly once"
    );
    assert_eq!(
        count_path(&mock, "/validate").await,
        1,
        "workflow should fire exactly once"
    );

    server.stop().await;
}

// WF 7: choice step routing

#[tokio::test]
async fn wf_choice_routing() {
    let mock = MockServer::start().await;
    mount_workflow_mocks(&mock).await;

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
handlers: {{}}
workflows:
  routing-flow:
    source: app
    events: [order.route]
    steps:
      - name: route
        type: choice
        choices:
          - when: "$.amount >= 10000"
            goto: high-value
          - when: "$.category == premium"
            goto: premium
        default: standard
      - name: high-value
        url: {mock_url}/fulfill
        end: true
      - name: premium
        url: {mock_url}/notify
        end: true
      - name: standard
        url: {mock_url}/validate
        end: true
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19737).await;
    let c = http();

    // High value (amount >= 10000) → fulfill only
    c.post(server.url("/events/app/order.route"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"amount": 15000, "category": "regular"}))
        .send()
        .await
        .unwrap();

    wait_for_mock(&mock, 1, 5).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(
        count_path(&mock, "/fulfill").await,
        1,
        "High-value → fulfill"
    );
    assert_eq!(
        count_path(&mock, "/notify").await,
        0,
        "High-value should NOT go to notify"
    );
    assert_eq!(
        count_path(&mock, "/validate").await,
        0,
        "High-value should NOT go to validate"
    );

    // Premium category → notify only
    c.post(server.url("/events/app/order.route"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"amount": 500, "category": "premium"}))
        .send()
        .await
        .unwrap();

    wait_for_mock(&mock, 2, 5).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(
        count_path(&mock, "/fulfill").await,
        1,
        "Still only 1 fulfill from high-value"
    );
    assert_eq!(count_path(&mock, "/notify").await, 1, "Premium → notify");
    assert_eq!(
        count_path(&mock, "/validate").await,
        0,
        "Premium should NOT go to validate"
    );

    // Default → validate only
    c.post(server.url("/events/app/order.route"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"amount": 500, "category": "regular"}))
        .send()
        .await
        .unwrap();

    wait_for_mock(&mock, 3, 5).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(
        count_path(&mock, "/fulfill").await,
        1,
        "Still only 1 fulfill"
    );
    assert_eq!(count_path(&mock, "/notify").await, 1, "Still only 1 notify");
    assert_eq!(
        count_path(&mock, "/validate").await,
        1,
        "Default → validate"
    );

    server.stop().await;
}

// WF 8: parallel step execution

#[tokio::test]
async fn wf_parallel_execution() {
    let mock = MockServer::start().await;
    mount_workflow_mocks(&mock).await;

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
handlers: {{}}
workflows:
  parallel-flow:
    source: app
    events: [check.all]
    steps:
      - name: checks
        type: parallel
        branches:
          - name: credit
            url: {mock_url}/validate
          - name: fraud
            url: {mock_url}/fulfill
        result_path: "$.checks"
      - name: finalize
        url: {mock_url}/notify
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19738).await;

    http()
        .post(server.url("/events/app/check.all"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"user_id": "usr_001"}))
        .send()
        .await
        .unwrap();

    wait_for_mock(&mock, 3, 10).await;

    assert_eq!(
        count_path(&mock, "/validate").await,
        1,
        "credit branch exactly once"
    );
    assert_eq!(
        count_path(&mock, "/fulfill").await,
        1,
        "fraud branch exactly once"
    );
    assert_eq!(
        count_path(&mock, "/notify").await,
        1,
        "finalize step exactly once"
    );

    // Finalize should receive merged results under $.checks
    let reqs = mock.received_requests().await.unwrap();
    let finalize = reqs.iter().find(|r| r.url.path() == "/notify").unwrap();
    let body: Value = serde_json::from_slice(&finalize.body).unwrap();
    assert!(
        body["checks"]["credit"].is_object(),
        "checks.credit should exist"
    );
    assert!(
        body["checks"]["fraud"].is_object(),
        "checks.fraud should exist"
    );

    server.stop().await;
}

// WF 9: map step execution

#[tokio::test]
async fn wf_map_execution() {
    let mock = MockServer::start().await;
    mount_workflow_mocks(&mock).await;

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
handlers: {{}}
workflows:
  map-flow:
    source: app
    events: [batch.process]
    steps:
      - name: process-items
        type: map
        items_path: "$.items"
        url: {mock_url}/validate
        result_path: "$.results"
      - name: summarize
        url: {mock_url}/notify
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19739).await;

    http()
        .post(server.url("/events/app/batch.process"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"items": [{"id": 1}, {"id": 2}, {"id": 3}]}))
        .send()
        .await
        .unwrap();

    // 3 items + 1 summarize = 4
    wait_for_mock(&mock, 4, 10).await;

    assert_eq!(
        count_path(&mock, "/validate").await,
        3,
        "Exactly 3 items processed"
    );

    let reqs = mock.received_requests().await.unwrap();
    let summarize = reqs.iter().find(|r| r.url.path() == "/notify").unwrap();
    let body: Value = serde_json::from_slice(&summarize.body).unwrap();
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 3, "results array should have 3 items");

    server.stop().await;
}

// WF 10: wait step (fixed seconds)

#[tokio::test]
async fn wf_wait_step() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/validate"))
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
  app:
    type: event
handlers: {{}}
workflows:
  delayed-flow:
    source: app
    events: [test.wait]
    steps:
      - name: delay
        type: wait
        seconds: 2
      - name: process
        url: {mock_url}/validate
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19740).await;

    http()
        .post(server.url("/events/app/test.wait"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"test": "wait"}))
        .send()
        .await
        .unwrap();

    // Should NOT have run yet (2-second wait)
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert_eq!(
        count_path(&mock, "/validate").await,
        0,
        "Should not run before delay"
    );

    // Should run after the delay
    wait_for_mock(&mock, 1, 10).await;
    assert_eq!(
        count_path(&mock, "/validate").await,
        1,
        "Should run exactly once after delay"
    );

    server.stop().await;
}

// WF 11: callback step

#[tokio::test]
async fn wf_callback_step() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true})))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/callback-notify"))
        .respond_with(ResponseTemplate::new(200))
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
  app:
    type: event
handlers: {{}}
workflows:
  approval-flow:
    source: app
    events: [test.callback]
    steps:
      - name: request
        url: {mock_url}/validate
      - name: wait-approval
        type: callback
        url: {mock_url}/callback-notify
      - name: finalize
        url: {mock_url}/notify
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19741).await;
    let c = http();

    c.post(server.url("/events/app/test.callback"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"order": "ord_001"}))
        .send()
        .await
        .unwrap();

    // Wait for step 1 + callback notification
    wait_for_mock(&mock, 2, 10).await;

    assert_eq!(
        count_path(&mock, "/validate").await,
        1,
        "Step 1 executed exactly once"
    );
    assert_eq!(
        count_path(&mock, "/notify").await,
        0,
        "Finalize should not run yet"
    );

    // Extract callback token from the notification request
    let reqs = mock.received_requests().await.unwrap();
    let cb_req = reqs
        .iter()
        .find(|r| r.url.path() == "/callback-notify")
        .expect("callback notification should be sent");
    let cb_body: Value = serde_json::from_slice(&cb_req.body).unwrap();
    let token = cb_body["callback_token"].as_str().unwrap();

    // Send the callback
    let resp = c
        .post(server.url(&format!("/callback/{}", token)))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"approved": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Wait for finalize step
    wait_for_mock(&mock, 3, 10).await;
    assert_eq!(
        count_path(&mock, "/notify").await,
        1,
        "Finalize should run exactly once after callback"
    );

    server.stop().await;
}

// WF 12: workflow timeout

#[tokio::test]
async fn wf_workflow_timeout() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/validate"))
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
api:
  auth_token: test-token
sources:
  app:
    type: event
handlers: {{}}
workflows:
  timeout-flow:
    source: app
    events: [test.timeout]
    timeout: 3
    steps:
      - name: delay
        type: wait
        seconds: 5
      - name: should-not-run
        url: {mock_url}/validate
"#,
        mock_url = mock.uri()
    );

    let server = QhookProcess::start(&yaml, 19742).await;

    let resp = http()
        .post(server.url("/events/app/test.timeout"))
        .header("Content-Type", "application/json")
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({"test": "timeout"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);
    let body: Value = resp.json().await.unwrap();
    let event_id = body["event_id"].as_str().unwrap().to_string();

    // Wait for timeout to elapse
    tokio::time::sleep(std::time::Duration::from_secs(8)).await;

    // validate expect(0) verified by wiremock

    // Check workflow status via Management API
    let ev: Value = http()
        .get(server.url(&format!("/api/events/{}", event_id)))
        .header("Authorization", "Bearer test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let wf_runs = ev["workflow_runs"].as_array().unwrap();
    assert!(!wf_runs.is_empty());
    assert_eq!(
        wf_runs[0]["status"], "failed",
        "Workflow should be failed due to timeout"
    );

    server.stop().await;
}
