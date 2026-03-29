//! Integration tests for the D1 database backend.
//!
//! Uses wiremock to mock the D1 HTTP API and test the full Database interface
//! through the D1 backend.

use qhook::config::DatabaseConfig;
use qhook::db::Database;
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Helper: Create a D1 Database connected to a mock server (proxy mode).
async fn make_d1_db(endpoint: &str) -> Database {
    let config = DatabaseConfig {
        driver: "d1".into(),
        url: None,
        max_connections: 1,
        account_id: None,
        database_id: None,
        api_token: None,
        d1_endpoint: Some(endpoint.into()),
    };
    Database::connect(&config).await.unwrap()
}

/// Helper: Build a D1 success response with results.
fn d1_success(results: Vec<serde_json::Value>, changes: u64) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "result": [{
            "results": results,
            "meta": {
                "changes": changes,
                "rows_read": results.len(),
                "rows_written": changes
            }
        }],
        "success": true,
        "errors": []
    }))
}

/// Helper: Build a D1 success response with changes only (no result rows).
fn d1_exec_success(changes: u64) -> ResponseTemplate {
    d1_success(vec![], changes)
}

/// Helper: Build a D1 error response.
fn d1_error(code: i64, message: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "result": [],
        "success": false,
        "errors": [{"code": code, "message": message}]
    }))
}

#[tokio::test]
async fn test_d1_connect_proxy_mode() {
    let config = DatabaseConfig {
        driver: "d1".into(),
        url: None,
        max_connections: 1,
        account_id: None,
        database_id: None,
        api_token: None,
        d1_endpoint: Some("http://localhost:9999".into()),
    };

    let db = Database::connect(&config).await.unwrap();
    assert!(db.is_d1());
    assert_eq!(db.driver, "d1");
    assert!(db.pool.is_none());
}

#[tokio::test]
async fn test_d1_connect_api_mode() {
    let config = DatabaseConfig {
        driver: "d1".into(),
        url: None,
        max_connections: 1,
        account_id: Some("acc-123".into()),
        database_id: Some("db-456".into()),
        api_token: Some("token-789".into()),
        d1_endpoint: None,
    };

    let db = Database::connect(&config).await.unwrap();
    assert!(db.is_d1());
    assert_eq!(db.driver, "d1");
}

#[tokio::test]
async fn test_d1_connect_api_mode_missing_account_id() {
    let config = DatabaseConfig {
        driver: "d1".into(),
        url: None,
        max_connections: 1,
        account_id: None,
        database_id: Some("db-456".into()),
        api_token: Some("token-789".into()),
        d1_endpoint: None,
    };

    let result = Database::connect(&config).await;
    assert!(result.is_err());
    let err = format!("{}", result.err().unwrap());
    assert!(err.contains("account_id"), "Error should mention account_id: {err}");
}

#[tokio::test]
async fn test_d1_connect_api_mode_missing_database_id() {
    let config = DatabaseConfig {
        driver: "d1".into(),
        url: None,
        max_connections: 1,
        account_id: Some("acc-123".into()),
        database_id: None,
        api_token: Some("token-789".into()),
        d1_endpoint: None,
    };

    let result = Database::connect(&config).await;
    assert!(result.is_err());
    let err = format!("{}", result.err().unwrap());
    assert!(err.contains("database_id"), "Error should mention database_id: {err}");
}

#[tokio::test]
async fn test_d1_connect_api_mode_missing_api_token() {
    let config = DatabaseConfig {
        driver: "d1".into(),
        url: None,
        max_connections: 1,
        account_id: Some("acc-123".into()),
        database_id: Some("db-456".into()),
        api_token: None,
        d1_endpoint: None,
    };

    let result = Database::connect(&config).await;
    assert!(result.is_err());
    let err = format!("{}", result.err().unwrap());
    assert!(err.contains("api_token"), "Error should mention api_token: {err}");
}

#[tokio::test]
async fn test_d1_insert_event() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(d1_exec_success(1))
        .expect(1)
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    let result = db
        .insert_event("evt-1", "github", "push", r#"{"ref":"main"}"#, None, None)
        .await
        .unwrap();
    assert!(result);
}

#[tokio::test]
async fn test_d1_insert_event_duplicate() {
    let server = MockServer::start().await;

    // D1 returns 0 changes for ON CONFLICT DO NOTHING
    Mock::given(method("POST"))
        .respond_with(d1_exec_success(0))
        .expect(1)
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    let result = db
        .insert_event(
            "evt-1",
            "github",
            "push",
            r#"{"ref":"main"}"#,
            None,
            Some("unique-key-1"),
        )
        .await
        .unwrap();
    assert!(!result, "Duplicate event should return false");
}

#[tokio::test]
async fn test_d1_insert_job() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(d1_exec_success(1))
        .expect(1)
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    db.insert_job("job-1", "evt-1", "test", "http://example.com/webhook", 5)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_d1_fetch_available_jobs() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(d1_success(
            vec![json!({
                "id": "job-1",
                "event_id": "evt-1",
                "handler": "test",
                "url": "http://example.com/webhook",
                "status": "available",
                "attempt": 0,
                "max_attempts": 5,
                "scheduled_at": "2024-01-01T00:00:00.000",
                "last_error": null,
                "created_at": "2024-01-01T00:00:00.000"
            })],
            0,
        ))
        .expect(1)
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    let jobs = db.fetch_available_jobs(10).await.unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, "job-1");
    assert_eq!(jobs[0].handler, "test");
    assert_eq!(jobs[0].status, "available");
}

#[tokio::test]
async fn test_d1_mark_job_lifecycle() {
    let server = MockServer::start().await;

    // mark_job_running returns 1 row affected
    Mock::given(method("POST"))
        .respond_with(d1_exec_success(1))
        .expect(1)
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    let result = db.mark_job_running("job-1").await.unwrap();
    assert!(result);
}

#[tokio::test]
async fn test_d1_mark_job_running_already_taken() {
    let server = MockServer::start().await;

    // Another worker already claimed the job: 0 rows affected
    Mock::given(method("POST"))
        .respond_with(d1_exec_success(0))
        .expect(1)
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    let result = db.mark_job_running("job-1").await.unwrap();
    assert!(!result, "Should return false when job is already running");
}

#[tokio::test]
async fn test_d1_queue_depth() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(d1_success(vec![json!({"cnt": 42})], 0))
        .expect(1)
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    let depth = db.queue_depth().await.unwrap();
    assert_eq!(depth, 42);
}

#[tokio::test]
async fn test_d1_dead_job_count() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(d1_success(vec![json!({"cnt": 3})], 0))
        .expect(1)
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    let count = db.dead_job_count().await.unwrap();
    assert_eq!(count, 3);
}

#[tokio::test]
async fn test_d1_api_error_handling() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(d1_error(7500, "no such table: events"))
        .expect(1)
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    let result = db.list_events(10).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("no such table"),
        "Error should contain D1 error message: {err}"
    );
}

#[tokio::test]
async fn test_d1_http_error_handling() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .expect(1)
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    let result = db.list_events(10).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("500"),
        "Error should contain HTTP status: {err}"
    );
}

#[tokio::test]
async fn test_d1_list_events() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(d1_success(
            vec![
                json!({
                    "id": "evt-2",
                    "source": "stripe",
                    "event_type": "payment.succeeded",
                    "unique_key": null,
                    "created_at": "2024-01-02T00:00:00.000"
                }),
                json!({
                    "id": "evt-1",
                    "source": "github",
                    "event_type": "push",
                    "unique_key": "abc123",
                    "created_at": "2024-01-01T00:00:00.000"
                }),
            ],
            0,
        ))
        .expect(1)
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    let events = db.list_events(10).await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].id, "evt-2");
    assert_eq!(events[0].source, "stripe");
    assert_eq!(events[1].id, "evt-1");
    assert_eq!(events[1].unique_key, Some("abc123".into()));
}

#[tokio::test]
async fn test_d1_get_event_data() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(d1_success(
            vec![json!({
                "payload": r#"{"amount":100}"#,
                "headers": r#"{"content-type":"application/json"}"#
            })],
            0,
        ))
        .expect(1)
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    let (payload, headers) = db.get_event_data("evt-1").await.unwrap();
    assert_eq!(payload, r#"{"amount":100}"#);
    assert!(headers.is_some());
}

#[tokio::test]
async fn test_d1_workflow_operations() {
    let server = MockServer::start().await;

    // insert_workflow_run
    Mock::given(method("POST"))
        .respond_with(d1_exec_success(1))
        .expect(1)
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    db.insert_workflow_run("run-1", "deploy", "evt-1", "build")
        .await
        .unwrap();
}

#[tokio::test]
async fn test_d1_get_workflow_run() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(d1_success(
            vec![json!({
                "id": "run-1",
                "workflow": "deploy",
                "event_id": "evt-1",
                "status": "running",
                "current_step": "build",
                "created_at": "2024-01-01T00:00:00.000",
                "completed_at": null
            })],
            0,
        ))
        .expect(1)
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    let run = db.get_workflow_run("run-1").await.unwrap();
    assert!(run.is_some());
    let run = run.unwrap();
    assert_eq!(run.id, "run-1");
    assert_eq!(run.workflow, "deploy");
    assert_eq!(run.status, "running");
    assert_eq!(run.current_step, Some("build".into()));
}

#[tokio::test]
async fn test_d1_increment_parallel_completed() {
    let server = MockServer::start().await;

    // D1 increment_parallel_completed does UPDATE then SELECT.
    // wiremock matches in LIFO, so we mount the SELECT response first (matched last),
    // then the UPDATE response (matched first).
    // But both match same method("POST"), so we use a single mock that returns
    // the SELECT-style response for both calls — the UPDATE result is not checked
    // for data, only for success.
    Mock::given(method("POST"))
        .respond_with(d1_success(
            vec![json!({"parallel_completed": 2, "parallel_count": 3})],
            1,
        ))
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    let (completed, total) = db.increment_parallel_completed("run-1").await.unwrap();
    assert_eq!(completed, 2);
    assert_eq!(total, 3);
}

#[tokio::test]
async fn test_d1_retry_dead_jobs() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(d1_exec_success(5))
        .expect(1)
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    let count = db.retry_dead_jobs().await.unwrap();
    assert_eq!(count, 5);
}

#[tokio::test]
async fn test_d1_cleanup_old_records() {
    let server = MockServer::start().await;

    // Both DELETE queries will return same response.
    // We verify that the method runs without error.
    Mock::given(method("POST"))
        .respond_with(d1_exec_success(5))
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    let (jobs, attempts) = db.cleanup_old_records(24).await.unwrap();
    // Both return 5 since we use one mock for all requests
    assert_eq!(jobs, 5);
    assert_eq!(attempts, 5);
}

#[tokio::test]
async fn test_d1_outbound_endpoint_crud() {
    let server = MockServer::start().await;

    // insert_endpoint
    Mock::given(method("POST"))
        .respond_with(d1_exec_success(1))
        .expect(1)
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    db.insert_endpoint("ep-1", "stripe", "https://example.com/hook", Some("My endpoint"), "secret123")
        .await
        .unwrap();
}

#[tokio::test]
async fn test_d1_get_endpoint() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(d1_success(
            vec![json!({
                "id": "ep-1",
                "source": "stripe",
                "url": "https://example.com/hook",
                "description": "Test endpoint",
                "signing_secret": "secret123",
                "status": "active",
                "created_at": "2024-01-01T00:00:00.000",
                "updated_at": "2024-01-01T00:00:00.000"
            })],
            0,
        ))
        .expect(1)
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    let ep = db.get_endpoint("ep-1").await.unwrap();
    assert!(ep.is_some());
    let ep = ep.unwrap();
    assert_eq!(ep.id, "ep-1");
    assert_eq!(ep.source, "stripe");
    assert_eq!(ep.status, "active");
}

#[tokio::test]
async fn test_d1_config_from_yaml() {
    // Test that D1 config can be deserialized from YAML
    let yaml = r#"
database:
  driver: d1
  account_id: "acc-123"
  database_id: "db-456"
  api_token: "token-789"
server:
  port: 8888
"#;
    let config: qhook::config::Config =
        serde_yaml_ng::from_str(yaml).expect("Failed to parse YAML config");
    assert_eq!(config.database.driver, "d1");
    assert_eq!(config.database.account_id, Some("acc-123".into()));
    assert_eq!(config.database.database_id, Some("db-456".into()));
    assert_eq!(config.database.api_token, Some("token-789".into()));
    assert!(config.database.d1_endpoint.is_none());
}

#[tokio::test]
async fn test_d1_config_proxy_mode_from_yaml() {
    let yaml = r#"
database:
  driver: d1
  d1_endpoint: "http://d1-binding.local"
  api_token: "optional-auth"
"#;
    let config: qhook::config::Config =
        serde_yaml_ng::from_str(yaml).expect("Failed to parse YAML config");
    assert_eq!(config.database.driver, "d1");
    assert_eq!(
        config.database.d1_endpoint,
        Some("http://d1-binding.local".into())
    );
    assert_eq!(config.database.api_token, Some("optional-auth".into()));
    // account_id/database_id not needed for proxy mode
    assert!(config.database.account_id.is_none());
}

#[tokio::test]
async fn test_d1_config_env_var_expansion() {
    // Verify that env var syntax in config fields works (this is handled by config loading, not D1)
    let yaml = r#"
database:
  driver: d1
  account_id: "${CF_ACCOUNT_ID}"
  database_id: "${CF_D1_DATABASE_ID}"
  api_token: "${CF_API_TOKEN}"
"#;
    let config: qhook::config::Config =
        serde_yaml_ng::from_str(yaml).expect("Failed to parse YAML config");
    // The raw strings are preserved (env var expansion happens at connect time)
    assert_eq!(config.database.account_id, Some("${CF_ACCOUNT_ID}".into()));
}

#[tokio::test]
async fn test_d1_migrate_success() {
    let server = MockServer::start().await;

    // Migration sends multiple queries. We just need all to succeed.
    // Using a catch-all mock that returns success for all POSTs.
    Mock::given(method("POST"))
        .respond_with(d1_success(vec![json!({"v": 0})], 0))
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    // Migration should complete without error
    let result = db.migrate().await;
    assert!(result.is_ok(), "Migration failed: {:?}", result.err());
}

#[tokio::test]
async fn test_d1_resume_callback_not_found() {
    let server = MockServer::start().await;

    // Resume returns 0 rows affected (token not found or already resumed)
    Mock::given(method("POST"))
        .respond_with(d1_exec_success(0))
        .expect(1)
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    let result = db
        .resume_callback_job("nonexistent-token", r#"{"result":"done"}"#)
        .await
        .unwrap();
    assert!(result.is_none(), "Should return None for non-existent token");
}

#[tokio::test]
async fn test_d1_get_job_by_id_not_found() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(d1_success(vec![], 0))
        .expect(1)
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    let result = db.get_job_by_id("nonexistent").await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_d1_delete_jobs_by_handler_no_transaction() {
    let server = MockServer::start().await;

    // Both DELETE queries share the same mock.
    Mock::given(method("POST"))
        .respond_with(d1_exec_success(3))
        .mount(&server)
        .await;

    let db = make_d1_db(&server.uri()).await;
    let count = db.delete_jobs_by_handler("queue/test").await.unwrap();
    // Returns the result of the second DELETE (jobs), which is 3 from our mock
    assert_eq!(count, 3);
}
