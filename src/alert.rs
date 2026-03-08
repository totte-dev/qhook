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
}

impl AlertEvent {
    fn kind(&self) -> &'static str {
        match self {
            AlertEvent::Dlq { .. } => "dlq",
            AlertEvent::VerificationFailure { .. } => "verification_failure",
        }
    }
}

/// Sends alerts via webhook (generic JSON, Slack, or Discord format).
pub struct Alerter {
    tx: mpsc::UnboundedSender<AlertEvent>,
}

impl Alerter {
    /// Create a new alerter that sends to the configured webhook.
    /// Spawns a background task to process alert events.
    pub fn new(config: AlertConfig, metrics: Arc<Metrics>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(alert_worker(config, metrics, rx));
        Self { tx }
    }

    /// Send an alert event. Non-blocking, drops if channel is full/closed.
    pub fn send(&self, event: AlertEvent) {
        let _ = self.tx.send(event);
    }
}

async fn alert_worker(
    config: AlertConfig,
    metrics: Arc<Metrics>,
    mut rx: mpsc::UnboundedReceiver<AlertEvent>,
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
    fn test_event_kind() {
        let dlq = AlertEvent::Dlq {
            job_id: "j".into(),
            handler: "h".into(),
            attempts: 1,
        };
        assert_eq!(dlq.kind(), "dlq");

        let verify = AlertEvent::VerificationFailure {
            source: "s".into(),
        };
        assert_eq!(verify.kind(), "verification_failure");
    }
}
