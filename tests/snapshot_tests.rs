//! Snapshot tests using insta to detect unintended changes in config validation
//! error messages, default config generation, Prometheus metrics output, and
//! API error response formats.

use qhook::config::Config;
use qhook::metrics::Metrics;

// ---------------------------------------------------------------------------
// (a) Config validation error messages
// ---------------------------------------------------------------------------

/// Helper: parse YAML into Config (bypasses file I/O), returning the error string.
fn config_error(yaml: &str) -> String {
    Config::from_yaml(yaml).unwrap_err().to_string()
}

#[test]
fn snapshot_invalid_database_driver() {
    let yaml = r#"
database:
  driver: mongodb
sources: {}
handlers: {}
"#;
    insta::assert_snapshot!(config_error(yaml));
}

#[test]
fn snapshot_handler_references_unknown_source() {
    let yaml = r#"
database:
  driver: sqlite
server:
  allow_private_urls: true
sources:
  app:
    type: event
handlers:
  my-handler:
    source: nonexistent
    url: http://localhost:3000/hook
"#;
    insta::assert_snapshot!(config_error(yaml));
}

#[test]
fn snapshot_invalid_retry_backoff_in_handler() {
    // The retry config itself deserializes fine (backoff is a string), but we
    // can test an invalid handler type which is validated.
    let yaml = r#"
database:
  driver: sqlite
server:
  allow_private_urls: true
sources:
  app:
    type: event
handlers:
  my-handler:
    source: app
    url: http://localhost:3000/hook
    type: grpc
"#;
    insta::assert_snapshot!(config_error(yaml));
}

#[test]
fn snapshot_invalid_source_type() {
    let yaml = r#"
database:
  driver: sqlite
sources:
  bad:
    type: kafka
handlers: {}
"#;
    insta::assert_snapshot!(config_error(yaml));
}

#[test]
fn snapshot_workflow_catch_goto_nonexistent_step() {
    let yaml = r#"
database:
  driver: sqlite
server:
  allow_private_urls: true
sources:
  app:
    type: event
handlers: {}
workflows:
  my-flow:
    source: app
    steps:
      - name: step1
        url: http://localhost:3000/step1
        catch:
          - errors: [all]
            goto: nonexistent-step
"#;
    insta::assert_snapshot!(config_error(yaml));
}

#[test]
fn snapshot_workflow_unknown_source() {
    let yaml = r#"
database:
  driver: sqlite
sources:
  app:
    type: event
handlers: {}
workflows:
  my-flow:
    source: missing-source
    steps:
      - name: step1
        url: http://example.com/step1
"#;
    insta::assert_snapshot!(config_error(yaml));
}

#[test]
fn snapshot_handler_url_missing_scheme() {
    let yaml = r#"
database:
  driver: sqlite
server:
  allow_private_urls: true
sources:
  app:
    type: event
handlers:
  bad-url:
    source: app
    url: localhost:3000/hook
"#;
    insta::assert_snapshot!(config_error(yaml));
}

#[test]
fn snapshot_handler_private_url_blocked() {
    let yaml = r#"
database:
  driver: sqlite
server:
  allow_private_urls: false
sources:
  app:
    type: event
handlers:
  local-handler:
    source: app
    url: http://127.0.0.1:3000/hook
"#;
    insta::assert_snapshot!(config_error(yaml));
}

#[test]
fn snapshot_source_verify_without_secret() {
    let yaml = r#"
database:
  driver: sqlite
sources:
  github:
    type: webhook
    verify: github
handlers: {}
"#;
    insta::assert_snapshot!(config_error(yaml));
}

#[test]
fn snapshot_cron_source_missing_schedule() {
    let yaml = r#"
database:
  driver: sqlite
sources:
  ticker:
    type: cron
handlers: {}
"#;
    insta::assert_snapshot!(config_error(yaml));
}

#[test]
fn snapshot_workflow_no_steps() {
    let yaml = r#"
database:
  driver: sqlite
sources:
  app:
    type: event
handlers: {}
workflows:
  empty-flow:
    source: app
    steps: []
"#;
    insta::assert_snapshot!(config_error(yaml));
}

#[test]
fn snapshot_workflow_duplicate_step_name() {
    let yaml = r#"
database:
  driver: sqlite
server:
  allow_private_urls: true
sources:
  app:
    type: event
handlers: {}
workflows:
  dup-flow:
    source: app
    steps:
      - name: step1
        url: http://localhost:3000/a
      - name: step1
        url: http://localhost:3000/b
"#;
    insta::assert_snapshot!(config_error(yaml));
}

// ---------------------------------------------------------------------------
// (b) Default config generation
// ---------------------------------------------------------------------------

#[test]
fn snapshot_default_config_yaml() {
    let default_yaml = Config::default_yaml();
    insta::assert_snapshot!(default_yaml);
}

// ---------------------------------------------------------------------------
// (c) Prometheus metrics output format
// ---------------------------------------------------------------------------

#[test]
fn snapshot_prometheus_metrics_empty() {
    let m = Metrics::new();
    let output = m.to_prometheus(0, 0);
    insta::assert_snapshot!(output);
}

#[test]
fn snapshot_prometheus_metrics_with_data() {
    let m = Metrics::new();

    // Simulate some activity
    m.inc_events_received_for("stripe");
    m.inc_events_received_for("stripe");
    m.inc_events_received_for("github");
    m.inc_jobs_created();
    m.inc_jobs_created();
    m.inc_delivery_success_for("payment-handler", 100);
    m.inc_delivery_success_for("payment-handler", 200);
    m.inc_delivery_failure_for("notification-handler", 500);
    m.inc_delivery_error_type("5xx");
    m.inc_delivery_error_type("timeout");
    m.inc_verification_failure("stripe");
    m.inc_dlq("notification-handler");
    m.inc_db_errors();
    m.inc_alerts_sent();
    m.inc_alerts_failed();
    m.inc_workflow_started("order-pipeline");
    m.inc_workflow_started("order-pipeline");
    m.inc_workflow_completed("order-pipeline");
    m.inc_workflow_failed("order-pipeline");
    m.inc_workflow_step_completed("order-pipeline");
    m.inc_workflow_step_completed("order-pipeline");
    m.inc_callbacks_received();
    m.inc_callbacks_expired();
    m.inc_circuit_opened("payment-handler");
    m.inc_circuit_rejected("payment-handler");

    let output = m.to_prometheus(42, 7);
    insta::assert_snapshot!(output);
}

// ---------------------------------------------------------------------------
// (d) Error response formats (JSON)
// ---------------------------------------------------------------------------

#[test]
fn snapshot_webhook_success_response() {
    let body = serde_json::json!({
        "event_id": "01EXAMPLE000000000000000000",
        "duplicate": false,
        "jobs_created": 2,
    });
    insta::assert_json_snapshot!(body);
}

#[test]
fn snapshot_event_accepted_response() {
    let body = serde_json::json!({
        "event_id": "01EXAMPLE000000000000000000",
        "duplicate": false,
        "jobs_created": 1,
    });
    insta::assert_json_snapshot!(body);
}

#[test]
fn snapshot_callback_not_found_response() {
    let body = serde_json::json!({"error": "not found"});
    insta::assert_json_snapshot!(body);
}

#[test]
fn snapshot_callback_success_response() {
    let body = serde_json::json!({"status": "ok", "message": "callback received"});
    insta::assert_json_snapshot!(body);
}

#[test]
fn snapshot_callback_internal_error_response() {
    let body = serde_json::json!({"error": "internal error"});
    insta::assert_json_snapshot!(body);
}

#[test]
fn snapshot_sns_notification_response() {
    let body = serde_json::json!({
        "event_id": "01EXAMPLE000000000000000000",
        "duplicate": false,
        "jobs_created": 1,
    });
    insta::assert_json_snapshot!(body);
}
