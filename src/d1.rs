//! Cloudflare D1 database client.
//!
//! D1 is accessed via HTTP API — either through the Cloudflare REST API
//! or through an Outbound Worker proxy endpoint (for Cloudflare Containers).
//!
//! Since D1 is SQLite-based, the SQL dialect is identical to SQLite.
//! The main difference is the transport: HTTP JSON instead of a native connection.

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Configuration for D1 access.
#[derive(Debug, Clone)]
pub struct D1Config {
    /// Access mode: Cloudflare REST API or local proxy endpoint.
    pub mode: D1Mode,
    /// HTTP client (shared).
    client: Client,
}

/// D1 access modes.
#[derive(Debug, Clone)]
pub enum D1Mode {
    /// Cloudflare REST API: `POST /client/v4/accounts/{account_id}/d1/database/{database_id}/query`
    Api {
        account_id: String,
        database_id: String,
        api_token: String,
    },
    /// Local proxy endpoint (Outbound Workers in Cloudflare Containers).
    /// The endpoint receives `{"sql": "...", "params": [...]}` and forwards to D1.
    Proxy {
        endpoint: String,
        /// Optional auth header value for the proxy.
        auth_token: Option<String>,
    },
}

/// D1 query request body.
#[derive(Debug, Serialize)]
struct D1QueryRequest {
    sql: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    params: Vec<serde_json::Value>,
}

/// D1 API response envelope.
#[derive(Debug, Deserialize)]
struct D1ApiResponse {
    #[serde(default)]
    result: Vec<D1QueryResult>,
    success: bool,
    #[serde(default)]
    errors: Vec<D1Error>,
}

/// Individual query result.
#[derive(Debug, Deserialize)]
struct D1QueryResult {
    #[serde(default)]
    results: Vec<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    meta: D1QueryMeta,
}

/// Query metadata from D1.
#[derive(Debug, Deserialize, Default)]
pub struct D1QueryMeta {
    #[serde(default)]
    pub changes: u64,
    #[serde(default)]
    pub rows_read: u64,
    #[serde(default)]
    pub rows_written: u64,
}

/// D1 error in response.
#[derive(Debug, Deserialize)]
struct D1Error {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    message: String,
}

/// Result of a D1 query execution.
pub struct D1ExecResult {
    pub rows_affected: u64,
}

/// A row returned from a D1 query, represented as a JSON object.
pub type D1Row = serde_json::Map<String, serde_json::Value>;

impl D1Config {
    /// Create a new D1 client using the Cloudflare REST API.
    pub fn new_api(account_id: String, database_id: String, api_token: String) -> Self {
        Self {
            mode: D1Mode::Api {
                account_id,
                database_id,
                api_token,
            },
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("Failed to build D1 HTTP client"),
        }
    }

    /// Create a new D1 client using a local proxy endpoint.
    pub fn new_proxy(endpoint: String, auth_token: Option<String>) -> Self {
        Self {
            mode: D1Mode::Proxy {
                endpoint,
                auth_token,
            },
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("Failed to build D1 HTTP client"),
        }
    }

    /// Execute a SQL statement (INSERT/UPDATE/DELETE/CREATE).
    /// Returns the number of rows affected.
    pub async fn execute(&self, sql: &str, params: Vec<serde_json::Value>) -> Result<D1ExecResult> {
        let result = self.send_query(sql, params).await?;
        Ok(D1ExecResult {
            rows_affected: result.meta.changes,
        })
    }

    /// Execute a SQL query and return rows.
    pub async fn query(&self, sql: &str, params: Vec<serde_json::Value>) -> Result<Vec<D1Row>> {
        let result = self.send_query(sql, params).await?;
        Ok(result.results)
    }

    /// Execute a SQL query and return exactly one row (or error).
    pub async fn query_one(&self, sql: &str, params: Vec<serde_json::Value>) -> Result<D1Row> {
        let rows = self.query(sql, params).await?;
        rows.into_iter()
            .next()
            .context("Expected one row but got none")
    }

    /// Execute a SQL query and return at most one row.
    pub async fn query_optional(
        &self,
        sql: &str,
        params: Vec<serde_json::Value>,
    ) -> Result<Option<D1Row>> {
        let rows = self.query(sql, params).await?;
        Ok(rows.into_iter().next())
    }

    /// Send a query to D1 and parse the response.
    async fn send_query(&self, sql: &str, params: Vec<serde_json::Value>) -> Result<D1QueryResult> {
        let body = D1QueryRequest {
            sql: sql.to_string(),
            params,
        };

        let response = match &self.mode {
            D1Mode::Api {
                account_id,
                database_id,
                api_token,
            } => {
                let url = format!(
                    "https://api.cloudflare.com/client/v4/accounts/{}/d1/database/{}/query",
                    account_id, database_id
                );
                self.client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", api_token))
                    .header("Content-Type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .context("Failed to send D1 API request")?
            }
            D1Mode::Proxy {
                endpoint,
                auth_token,
            } => {
                let mut req = self
                    .client
                    .post(endpoint)
                    .header("Content-Type", "application/json");
                if let Some(token) = auth_token {
                    req = req.header("Authorization", format!("Bearer {}", token));
                }
                req.json(&body)
                    .send()
                    .await
                    .context("Failed to send D1 proxy request")?
            }
        };

        let status = response.status();
        let response_text = response
            .text()
            .await
            .context("Failed to read D1 response body")?;

        if !status.is_success() {
            anyhow::bail!(
                "D1 HTTP error (status {}): {}",
                status,
                truncate(&response_text, 500)
            );
        }

        let api_response: D1ApiResponse =
            serde_json::from_str(&response_text).with_context(|| {
                format!(
                    "Failed to parse D1 response: {}",
                    truncate(&response_text, 200)
                )
            })?;

        if !api_response.success {
            let err_msg = api_response
                .errors
                .iter()
                .map(|e| format!("[{}] {}", e.code, e.message))
                .collect::<Vec<_>>()
                .join("; ");
            anyhow::bail!("D1 query failed: {}", err_msg);
        }

        api_response
            .result
            .into_iter()
            .next()
            .context("D1 returned empty result array")
    }
}

/// Helper: extract a String from a D1Row.
pub fn row_get_string(row: &D1Row, key: &str) -> Result<String> {
    match row.get(key) {
        Some(serde_json::Value::String(s)) => Ok(s.clone()),
        Some(serde_json::Value::Null) | None => {
            anyhow::bail!("Column '{}' is null or missing", key)
        }
        Some(v) => Ok(v.to_string().trim_matches('"').to_string()),
    }
}

/// Helper: extract an optional String from a D1Row.
pub fn row_get_opt_string(row: &D1Row, key: &str) -> Option<String> {
    match row.get(key) {
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(serde_json::Value::Null) | None => None,
        Some(v) => Some(v.to_string().trim_matches('"').to_string()),
    }
}

/// Helper: extract an i32 from a D1Row.
pub fn row_get_i32(row: &D1Row, key: &str) -> Result<i32> {
    match row.get(key) {
        Some(serde_json::Value::Number(n)) => n
            .as_i64()
            .and_then(|v| i32::try_from(v).ok())
            .context(format!(
                "Column '{}' is not an integer or out of i32 range",
                key
            )),
        Some(serde_json::Value::Null) | None => {
            anyhow::bail!("Column '{}' is null or missing", key)
        }
        Some(v) => anyhow::bail!("Column '{}' has unexpected type: {}", key, v),
    }
}

/// Helper: extract an optional i32 from a D1Row.
pub fn row_get_opt_i32(row: &D1Row, key: &str) -> Option<i32> {
    match row.get(key) {
        Some(serde_json::Value::Number(n)) => n.as_i64().and_then(|v| i32::try_from(v).ok()),
        _ => None,
    }
}

/// Helper: extract an i64 from a D1Row.
pub fn row_get_i64(row: &D1Row, key: &str) -> Result<i64> {
    match row.get(key) {
        Some(serde_json::Value::Number(n)) => n
            .as_i64()
            .context(format!("Column '{}' is not an i64", key)),
        Some(serde_json::Value::Null) | None => {
            anyhow::bail!("Column '{}' is null or missing", key)
        }
        Some(v) => anyhow::bail!("Column '{}' has unexpected type: {}", key, v),
    }
}

/// Truncate a string for error messages.
fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { s } else { &s[..max] }
}

/// Convert a D1Row to a JobRow.
pub fn row_to_job(row: &D1Row) -> Result<super::db::JobRow> {
    Ok(super::db::JobRow {
        id: row_get_string(row, "id")?,
        event_id: row_get_string(row, "event_id")?,
        handler: row_get_string(row, "handler")?,
        url: row_get_string(row, "url")?,
        status: row_get_string(row, "status")?,
        attempt: row_get_i32(row, "attempt")?,
        max_attempts: row_get_i32(row, "max_attempts")?,
        scheduled_at: row_get_string(row, "scheduled_at")?,
        last_error: row_get_opt_string(row, "last_error"),
        created_at: row_get_string(row, "created_at")?,
    })
}

/// Convert a D1Row to an EventRow.
pub fn row_to_event(row: &D1Row) -> Result<super::db::EventRow> {
    Ok(super::db::EventRow {
        id: row_get_string(row, "id")?,
        source: row_get_string(row, "source")?,
        event_type: row_get_string(row, "event_type")?,
        unique_key: row_get_opt_string(row, "unique_key"),
        created_at: row_get_string(row, "created_at")?,
    })
}

/// Convert a D1Row to an EventRowFull.
pub fn row_to_event_full(row: &D1Row) -> Result<super::db::EventRowFull> {
    Ok(super::db::EventRowFull {
        id: row_get_string(row, "id")?,
        source: row_get_string(row, "source")?,
        event_type: row_get_string(row, "event_type")?,
        payload: row_get_string(row, "payload")?,
        headers: row_get_opt_string(row, "headers"),
        unique_key: row_get_opt_string(row, "unique_key"),
        created_at: row_get_string(row, "created_at")?,
    })
}

/// Convert a D1Row to a WorkflowRunRow.
pub fn row_to_workflow_run(row: &D1Row) -> Result<super::db::WorkflowRunRow> {
    Ok(super::db::WorkflowRunRow {
        id: row_get_string(row, "id")?,
        workflow: row_get_string(row, "workflow")?,
        event_id: row_get_string(row, "event_id")?,
        status: row_get_string(row, "status")?,
        current_step: row_get_opt_string(row, "current_step"),
        created_at: row_get_string(row, "created_at")?,
        completed_at: row_get_opt_string(row, "completed_at"),
    })
}

/// Convert a D1Row to a WorkflowJobRow.
pub fn row_to_workflow_job(row: &D1Row) -> Result<super::db::WorkflowJobRow> {
    Ok(super::db::WorkflowJobRow {
        workflow_run_id: row_get_opt_string(row, "workflow_run_id"),
        step_name: row_get_opt_string(row, "step_name"),
        step_index: row_get_opt_i32(row, "step_index"),
        step_input: row_get_opt_string(row, "step_input"),
        step_output: row_get_opt_string(row, "step_output"),
        branch_name: row_get_opt_string(row, "branch_name"),
    })
}

/// Convert a D1Row to a QueueMessageRow.
pub fn row_to_queue_message(row: &D1Row) -> Result<super::db::QueueMessageRow> {
    Ok(super::db::QueueMessageRow {
        id: row_get_string(row, "id")?,
        event_id: row_get_string(row, "event_id")?,
        event_type: row_get_string(row, "event_type")?,
        payload: row_get_string(row, "payload")?,
        headers: row_get_opt_string(row, "headers"),
        attempt: row_get_i32(row, "attempt")?,
        created_at: row_get_string(row, "created_at")?,
    })
}

/// Convert a D1Row to a JobAttemptRow.
pub fn row_to_job_attempt(row: &D1Row) -> Result<super::db::JobAttemptRow> {
    Ok(super::db::JobAttemptRow {
        attempt: row_get_i32(row, "attempt")?,
        status_code: row_get_opt_i32(row, "status_code"),
        error: row_get_opt_string(row, "error"),
        duration_ms: row_get_opt_i32(row, "duration_ms"),
        created_at: row_get_string(row, "created_at")?,
    })
}

/// Convert a D1Row to an EndpointRow.
pub fn row_to_endpoint(row: &D1Row) -> Result<super::db::EndpointRow> {
    Ok(super::db::EndpointRow {
        id: row_get_string(row, "id")?,
        source: row_get_string(row, "source")?,
        url: row_get_string(row, "url")?,
        description: row_get_opt_string(row, "description"),
        signing_secret: row_get_string(row, "signing_secret")?,
        status: row_get_string(row, "status")?,
        created_at: row_get_string(row, "created_at")?,
        updated_at: row_get_string(row, "updated_at")?,
    })
}

/// Convert a D1Row to a SubscriptionRow.
pub fn row_to_subscription(row: &D1Row) -> Result<super::db::SubscriptionRow> {
    Ok(super::db::SubscriptionRow {
        id: row_get_string(row, "id")?,
        endpoint_id: row_get_string(row, "endpoint_id")?,
        event_type: row_get_string(row, "event_type")?,
        created_at: row_get_string(row, "created_at")?,
    })
}

/// Convert a D1Row to a SubscribedEndpoint.
pub fn row_to_subscribed_endpoint(row: &D1Row) -> Result<super::db::SubscribedEndpoint> {
    Ok(super::db::SubscribedEndpoint {
        id: row_get_string(row, "id")?,
        url: row_get_string(row, "url")?,
        signing_secret: row_get_string(row, "signing_secret")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_row_get_string() {
        let mut row = D1Row::new();
        row.insert("name".into(), serde_json::Value::String("hello".into()));
        assert_eq!(row_get_string(&row, "name").unwrap(), "hello");
    }

    #[test]
    fn test_row_get_string_missing() {
        let row = D1Row::new();
        assert!(row_get_string(&row, "name").is_err());
    }

    #[test]
    fn test_row_get_string_null() {
        let mut row = D1Row::new();
        row.insert("name".into(), serde_json::Value::Null);
        assert!(row_get_string(&row, "name").is_err());
    }

    #[test]
    fn test_row_get_opt_string() {
        let mut row = D1Row::new();
        row.insert("name".into(), serde_json::Value::String("hello".into()));
        row.insert("empty".into(), serde_json::Value::Null);
        assert_eq!(row_get_opt_string(&row, "name"), Some("hello".into()));
        assert_eq!(row_get_opt_string(&row, "empty"), None);
        assert_eq!(row_get_opt_string(&row, "missing"), None);
    }

    #[test]
    fn test_row_get_i32() {
        let mut row = D1Row::new();
        row.insert("count".into(), serde_json::json!(42));
        assert_eq!(row_get_i32(&row, "count").unwrap(), 42);
    }

    #[test]
    fn test_row_get_i32_missing() {
        let row = D1Row::new();
        assert!(row_get_i32(&row, "count").is_err());
    }

    #[test]
    fn test_row_get_i64() {
        let mut row = D1Row::new();
        row.insert("big".into(), serde_json::json!(1000000i64));
        assert_eq!(row_get_i64(&row, "big").unwrap(), 1000000);
    }

    #[test]
    fn test_row_to_job() {
        let mut row = D1Row::new();
        row.insert("id".into(), serde_json::json!("job-1"));
        row.insert("event_id".into(), serde_json::json!("evt-1"));
        row.insert("handler".into(), serde_json::json!("test"));
        row.insert("url".into(), serde_json::json!("http://example.com"));
        row.insert("status".into(), serde_json::json!("available"));
        row.insert("attempt".into(), serde_json::json!(0));
        row.insert("max_attempts".into(), serde_json::json!(5));
        row.insert(
            "scheduled_at".into(),
            serde_json::json!("2024-01-01T00:00:00.000"),
        );
        row.insert("last_error".into(), serde_json::Value::Null);
        row.insert(
            "created_at".into(),
            serde_json::json!("2024-01-01T00:00:00.000"),
        );

        let job = row_to_job(&row).unwrap();
        assert_eq!(job.id, "job-1");
        assert_eq!(job.event_id, "evt-1");
        assert_eq!(job.handler, "test");
        assert_eq!(job.status, "available");
        assert_eq!(job.attempt, 0);
        assert_eq!(job.max_attempts, 5);
        assert!(job.last_error.is_none());
    }

    #[test]
    fn test_row_to_event() {
        let mut row = D1Row::new();
        row.insert("id".into(), serde_json::json!("evt-1"));
        row.insert("source".into(), serde_json::json!("github"));
        row.insert("event_type".into(), serde_json::json!("push"));
        row.insert("unique_key".into(), serde_json::Value::Null);
        row.insert(
            "created_at".into(),
            serde_json::json!("2024-01-01T00:00:00.000"),
        );

        let event = row_to_event(&row).unwrap();
        assert_eq!(event.id, "evt-1");
        assert_eq!(event.source, "github");
        assert_eq!(event.event_type, "push");
        assert!(event.unique_key.is_none());
    }

    #[test]
    fn test_d1_query_request_serialization() {
        let req = D1QueryRequest {
            sql: "SELECT * FROM events WHERE id = ?1".into(),
            params: vec![serde_json::json!("evt-1")],
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("SELECT * FROM events WHERE id = ?1"));
        assert!(json.contains("evt-1"));
    }

    #[test]
    fn test_d1_query_request_empty_params() {
        let req = D1QueryRequest {
            sql: "SELECT 1".into(),
            params: vec![],
        };
        let json = serde_json::to_string(&req).unwrap();
        // params should be omitted when empty
        assert!(!json.contains("params"));
    }

    #[test]
    fn test_d1_api_response_success() {
        let json = r#"{"result":[{"results":[{"id":"test"}],"meta":{"changes":0,"rows_read":1,"rows_written":0}}],"success":true,"errors":[]}"#;
        let resp: D1ApiResponse = serde_json::from_str(json).unwrap();
        assert!(resp.success);
        assert_eq!(resp.result.len(), 1);
        assert_eq!(resp.result[0].results.len(), 1);
    }

    #[test]
    fn test_d1_api_response_error() {
        let json = r#"{"result":[],"success":false,"errors":[{"code":7500,"message":"no such table: events"}]}"#;
        let resp: D1ApiResponse = serde_json::from_str(json).unwrap();
        assert!(!resp.success);
        assert_eq!(resp.errors[0].code, 7500);
        assert_eq!(resp.errors[0].message, "no such table: events");
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hello");
    }

    #[test]
    fn test_row_get_string_from_number() {
        let mut row = D1Row::new();
        row.insert("num".into(), serde_json::json!(42));
        // Numbers should be converted to string representation
        assert_eq!(row_get_string(&row, "num").unwrap(), "42");
    }
}
