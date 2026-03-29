use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::config::AlertConfig;
use crate::metrics::Metrics;

/// Alert event types that can trigger notifications.
#[derive(Debug, Clone)]
pub enum AlertEvent {
    /// Job moved to dead letter queue.
    Dlq {
        job_id: String,
        handler: String,
        attempts: i32,
    },
    /// Signature verification failed.
    VerificationFailure { source: String },
    /// Custom alert message (e.g., config validation failure).
    Custom(String),
    /// Circuit breaker opened for a handler.
    CircuitOpened { handler: String },
    /// Workflow run failed.
    WorkflowFailed { workflow: String, run_id: String },
    /// Database health check failed.
    DbUnhealthy,
    /// Events expired by TTL.
    EventTtlExpired { count: u64 },
}

impl AlertEvent {
    fn kind(&self) -> &'static str {
        match self {
            AlertEvent::Dlq { .. } => "dlq",
            AlertEvent::VerificationFailure { .. } => "verification_failure",
            AlertEvent::Custom(_) => "custom",
            AlertEvent::CircuitOpened { .. } => "circuit_opened",
            AlertEvent::WorkflowFailed { .. } => "workflow_failed",
            AlertEvent::DbUnhealthy => "db_unhealthy",
            AlertEvent::EventTtlExpired { .. } => "event_ttl_expired",
        }
    }
}

/// Sends alerts via webhook (generic JSON, Slack, or Discord format).
/// Channel capacity for alert events. Alerts are dropped when the channel is full.
const ALERT_CHANNEL_CAPACITY: usize = 1000;

pub struct Alerter {
    tx: mpsc::Sender<AlertEvent>,
}

impl Alerter {
    /// Create a new alerter that sends to the configured webhook.
    /// Spawns a background task to process alert events.
    pub fn new(config: AlertConfig, metrics: Arc<Metrics>) -> Self {
        let (tx, rx) = mpsc::channel(ALERT_CHANNEL_CAPACITY);
        tokio::spawn(alert_worker(config, metrics, rx));
        Self { tx }
    }

    /// Send an alert event. Non-blocking, drops if channel is full/closed.
    pub fn send(&self, event: AlertEvent) {
        match self.tx.try_send(event) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!("Alert channel full, dropping alert event");
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    }
}

async fn alert_worker(
    config: AlertConfig,
    metrics: Arc<Metrics>,
    mut rx: mpsc::Receiver<AlertEvent>,
) {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    let enabled: Vec<String> = config.on.clone();

    while let Some(event) = rx.recv().await {
        if !enabled.iter().any(|e| e == event.kind()) {
            continue;
        }

        let payload = format_payload(&config.alert_type, &event);

        match http
            .post(&config.url)
            .header("Content-Type", "application/json")
            .body(payload)
            .send()
            .await
        {
            Ok(_) => metrics.inc_alerts_sent(),
            Err(e) => {
                metrics.inc_alerts_failed();
                tracing::error!(error = %e, alert_type = event.kind(), "Failed to send alert");
            }
        }
    }
}

fn format_payload(alert_type: &str, event: &AlertEvent) -> String {
    match alert_type {
        "slack" => format_slack(event),
        "discord" => format_discord(event),
        _ => format_generic(event),
    }
}

fn format_generic(event: &AlertEvent) -> String {
    match event {
        AlertEvent::Dlq {
            job_id,
            handler,
            attempts,
        } => {
            serde_json::json!({
                "alert": "dlq",
                "job_id": job_id,
                "handler": handler,
                "attempts": attempts,
                "message": format!("Job {job_id} moved to DLQ after {attempts} attempts (handler: {handler})")
            })
            .to_string()
        }
        AlertEvent::VerificationFailure { source } => {
            serde_json::json!({
                "alert": "verification_failure",
                "source": source,
                "message": format!("Signature verification failed for source: {source}")
            })
            .to_string()
        }
        AlertEvent::Custom(msg) => {
            serde_json::json!({
                "alert": "custom",
                "message": msg
            })
            .to_string()
        }
        AlertEvent::CircuitOpened { handler } => {
            serde_json::json!({
                "alert": "circuit_opened",
                "handler": handler,
                "message": format!("Circuit breaker opened for handler: {handler}")
            })
            .to_string()
        }
        AlertEvent::WorkflowFailed { workflow, run_id } => {
            serde_json::json!({
                "alert": "workflow_failed",
                "workflow": workflow,
                "run_id": run_id,
                "message": format!("Workflow '{workflow}' failed (run: {run_id})")
            })
            .to_string()
        }
        AlertEvent::DbUnhealthy => {
            serde_json::json!({
                "alert": "db_unhealthy",
                "message": "Database health check failed"
            })
            .to_string()
        }
        AlertEvent::EventTtlExpired { count } => {
            serde_json::json!({
                "alert": "event_ttl_expired",
                "count": count,
                "message": format!("{count} events expired by TTL")
            })
            .to_string()
        }
    }
}

fn format_slack(event: &AlertEvent) -> String {
    let text = match event {
        AlertEvent::Dlq {
            job_id,
            handler,
            attempts,
        } => format!(
            ":rotating_light: *Job moved to DLQ*\n• Job: `{job_id}`\n• Handler: `{handler}`\n• Attempts: {attempts}"
        ),
        AlertEvent::VerificationFailure { source } => {
            format!(":warning: *Signature verification failed*\n• Source: `{source}`")
        }
        AlertEvent::Custom(msg) => {
            format!(":gear: *qhook*\n{msg}")
        }
        AlertEvent::CircuitOpened { handler } => {
            format!(":zap: *Circuit breaker opened*\n• Handler: `{handler}`")
        }
        AlertEvent::WorkflowFailed { workflow, run_id } => {
            format!(":x: *Workflow failed*\n• Workflow: `{workflow}`\n• Run: `{run_id}`")
        }
        AlertEvent::DbUnhealthy => {
            ":fire: *Database unhealthy*\nDatabase health check failed — webhooks may be rejected with 503".to_string()
        }
        AlertEvent::EventTtlExpired { count } => {
            format!(":hourglass: *Events expired*\n• Count: {count}")
        }
    };

    serde_json::json!({ "text": text }).to_string()
}

fn format_discord(event: &AlertEvent) -> String {
    let (title, description, color) = match event {
        AlertEvent::Dlq {
            job_id,
            handler,
            attempts,
        } => (
            "Job moved to DLQ".to_string(),
            format!("**Job:** `{job_id}`\n**Handler:** `{handler}`\n**Attempts:** {attempts}"),
            0xFF0000, // red
        ),
        AlertEvent::VerificationFailure { source } => (
            "Signature verification failed".to_string(),
            format!("**Source:** `{source}`"),
            0xFFA500, // orange
        ),
        AlertEvent::Custom(msg) => (
            "qhook".to_string(),
            msg.clone(),
            0x3498DB, // blue
        ),
        AlertEvent::CircuitOpened { handler } => (
            "Circuit breaker opened".to_string(),
            format!("**Handler:** `{handler}`"),
            0xFF4500, // orange-red
        ),
        AlertEvent::WorkflowFailed { workflow, run_id } => (
            "Workflow failed".to_string(),
            format!("**Workflow:** `{workflow}`\n**Run:** `{run_id}`"),
            0xFF0000, // red
        ),
        AlertEvent::DbUnhealthy => (
            "Database unhealthy".to_string(),
            "Database health check failed — webhooks may be rejected with 503".to_string(),
            0xFF0000, // red
        ),
        AlertEvent::EventTtlExpired { count } => (
            "Events expired by TTL".to_string(),
            format!("**Count:** {count}"),
            0xFFA500, // orange
        ),
    };

    serde_json::json!({
        "embeds": [{
            "title": title,
            "description": description,
            "color": color
        }]
    })
    .to_string()
}

/// Optional Arc wrapper for use in shared state.
pub type SharedAlerter = Option<Arc<Alerter>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generic_dlq_payload() {
        let event = AlertEvent::Dlq {
            job_id: "job-123".into(),
            handler: "payment".into(),
            attempts: 5,
        };
        let payload = format_generic(&event);
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["alert"], "dlq");
        assert_eq!(v["job_id"], "job-123");
        assert_eq!(v["handler"], "payment");
        assert_eq!(v["attempts"], 5);
    }

    #[test]
    fn test_generic_verification_payload() {
        let event = AlertEvent::VerificationFailure {
            source: "stripe".into(),
        };
        let payload = format_generic(&event);
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["alert"], "verification_failure");
        assert_eq!(v["source"], "stripe");
    }

    #[test]
    fn test_slack_format() {
        let event = AlertEvent::Dlq {
            job_id: "job-123".into(),
            handler: "payment".into(),
            attempts: 5,
        };
        let payload = format_slack(&event);
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(v["text"].as_str().unwrap().contains("DLQ"));
        assert!(v["text"].as_str().unwrap().contains("job-123"));
    }

    #[test]
    fn test_discord_format() {
        let event = AlertEvent::VerificationFailure {
            source: "github".into(),
        };
        let payload = format_discord(&event);
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        let embed = &v["embeds"][0];
        assert_eq!(embed["title"], "Signature verification failed");
        assert!(embed["description"].as_str().unwrap().contains("github"));
        assert_eq!(embed["color"], 0xFFA500);
    }

    #[test]
    fn test_format_payload_unknown_type_falls_back_to_generic() {
        let event = AlertEvent::Dlq {
            job_id: "j1".into(),
            handler: "h1".into(),
            attempts: 3,
        };
        let payload = format_payload("unknown_type", &event);
        // Should produce generic JSON (not Slack/Discord format)
        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(parsed["alert"], "dlq");
        assert_eq!(parsed["handler"], "h1");
    }

    #[test]
    fn test_event_kind() {
        let dlq = AlertEvent::Dlq {
            job_id: "j".into(),
            handler: "h".into(),
            attempts: 1,
        };
        assert_eq!(dlq.kind(), "dlq");

        let verify = AlertEvent::VerificationFailure { source: "s".into() };
        assert_eq!(verify.kind(), "verification_failure");
    }
}
