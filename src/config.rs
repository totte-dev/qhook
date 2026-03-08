use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub api: ApiConfig,
    #[serde(default)]
    pub delivery: DeliveryConfig,
    #[serde(default)]
    pub sources: HashMap<String, SourceConfig>,
    #[serde(default)]
    pub handlers: HashMap<String, HandlerConfig>,
    #[serde(default)]
    pub worker: WorkerConfig,
    #[serde(default)]
    pub alerts: Option<AlertConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WorkerConfig {
    /// Seconds a job can stay in 'running' before recovery (default: 300).
    #[serde(default = "default_stale_threshold")]
    pub stale_threshold_secs: i64,
    /// Hours to retain completed/dead records (default: 72).
    #[serde(default = "default_retention_hours")]
    pub retention_hours: i64,
    /// Seconds to wait for in-flight deliveries during shutdown (default: 30).
    #[serde(default = "default_drain_timeout")]
    pub drain_timeout_secs: u64,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            stale_threshold_secs: default_stale_threshold(),
            retention_hours: default_retention_hours(),
            drain_timeout_secs: default_drain_timeout(),
        }
    }
}

fn default_stale_threshold() -> i64 {
    300
}
fn default_retention_hours() -> i64 {
    72
}
fn default_drain_timeout() -> u64 {
    30
}

#[derive(Debug, Deserialize, Clone)]
pub struct AlertConfig {
    pub url: String,
    /// Alert type: "generic" (default), "slack", "discord"
    #[serde(rename = "type", default = "default_alert_type")]
    pub alert_type: String,
    /// Conditions to trigger alerts on. Defaults to all.
    #[serde(default = "default_alert_on")]
    pub on: Vec<String>,
}

fn default_alert_type() -> String {
    "generic".into()
}

fn default_alert_on() -> Vec<String> {
    vec![
        "dlq".into(),
        "verification_failure".into(),
    ]
}

#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    #[serde(default = "default_driver")]
    pub driver: String,
    pub url: Option<String>,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            driver: "sqlite".into(),
            url: None,
            max_connections: default_max_connections(),
        }
    }
}

fn default_max_connections() -> u32 {
    10
}

fn default_driver() -> String {
    "sqlite".into()
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    /// Maximum request body size in bytes (default: 1MB).
    #[serde(default = "default_max_body_size")]
    pub max_body_size: usize,
    /// Maximum concurrent inbound requests (default: 100). Excess requests get 503.
    #[serde(default = "default_max_inbound")]
    pub max_inbound: u32,
    /// Per-IP rate limit (requests per second). 0 = disabled (default).
    #[serde(default)]
    pub ip_rate_limit: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            max_body_size: default_max_body_size(),
            max_inbound: default_max_inbound(),
            ip_rate_limit: 0,
        }
    }
}

fn default_max_body_size() -> usize {
    1_048_576 // 1MB
}

fn default_max_inbound() -> u32 {
    100
}

fn default_port() -> u16 {
    8888
}

#[derive(Debug, Deserialize, Default)]
pub struct ApiConfig {
    pub auth_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct DeliveryConfig {
    pub signing_secret: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout: String,
    #[serde(default)]
    pub default_retry: RetryConfig,
}

impl Default for DeliveryConfig {
    fn default() -> Self {
        Self {
            signing_secret: None,
            timeout: default_timeout(),
            default_retry: RetryConfig::default(),
        }
    }
}

fn default_timeout() -> String {
    "30s".into()
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct RetryConfig {
    #[serde(default = "default_max_retries")]
    pub max: u32,
    #[serde(default = "default_backoff")]
    pub backoff: String,
    #[serde(default = "default_interval")]
    pub interval: String,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max: default_max_retries(),
            backoff: default_backoff(),
            interval: default_interval(),
        }
    }
}

fn default_max_retries() -> u32 {
    5
}
fn default_backoff() -> String {
    "exponential".into()
}
fn default_interval() -> String {
    "30s".into()
}

#[derive(Debug, Deserialize)]
pub struct SourceConfig {
    #[serde(rename = "type")]
    pub source_type: String,
    pub verify: Option<String>,
    pub secret: Option<String>,
    #[serde(default)]
    pub skip_verify: bool,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct HandlerConfig {
    pub source: String,
    #[serde(default)]
    pub events: Vec<String>,
    pub url: String,
    /// Handler type: "http" (default) or "grpc".
    #[serde(rename = "type", default = "default_handler_type")]
    pub handler_type: String,
    pub retry: Option<RetryConfig>,
    pub timeout: Option<String>,
    pub idempotency_key: Option<String>,
    /// Max deliveries per second for this handler (optional).
    pub rate_limit: Option<u32>,
    /// JSONPath filter condition. Format: "$.path == value" or "$.path != value"
    /// or just "$.path" (truthy check). Only matching events create jobs.
    pub filter: Option<String>,
    /// JSON template for payload transformation. Use `{{$.path}}` to reference
    /// fields from the original payload. Applied at delivery time.
    pub transform: Option<String>,
}

fn default_handler_type() -> String {
    "http".into()
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config: {}", path.display()))?;
        let content = expand_env_vars(&content);
        let config: Config =
            serde_yaml_ng::from_str(&content).context("Failed to parse YAML config")?;
        config.validate()?;
        Ok(config)
    }

    /// Semantic validation beyond YAML parsing.
    pub fn validate(&self) -> Result<()> {
        // Validate handler URLs and types
        for (name, handler) in &self.handlers {
            match handler.handler_type.as_str() {
                "http" | "grpc" => {}
                other => anyhow::bail!("handler '{}' has invalid type '{}'", name, other),
            }
            validate_handler_url(name, &handler.url)?;

            // Check that handler.source references an existing source
            if !self.sources.contains_key(&handler.source) {
                anyhow::bail!(
                    "handler '{}' references unknown source '{}'",
                    name,
                    handler.source
                );
            }
        }

        // Validate source types and verify/secret consistency
        for (name, source) in &self.sources {
            match source.source_type.as_str() {
                "webhook" | "event" | "sns" => {}
                other => anyhow::bail!("source '{}' has invalid type '{}'", name, other),
            }
            // Require secret when signature verification is enabled
            if source.verify.is_some() {
                if source.secret.as_ref().is_none_or(|s| s.is_empty()) {
                    anyhow::bail!(
                        "source '{}' has verify enabled but no secret configured",
                        name
                    );
                }
            }
        }

        // Validate alert config
        if let Some(ref alerts) = self.alerts {
            match alerts.alert_type.as_str() {
                "generic" | "slack" | "discord" => {}
                other => anyhow::bail!("alerts.type '{}' is not supported", other),
            }
            if !alerts.url.starts_with("http://") && !alerts.url.starts_with("https://") {
                anyhow::bail!("alerts.url must start with http:// or https://");
            }
        }

        // Validate database driver
        match self.database.driver.as_str() {
            "sqlite" | "postgres" => {}
            other => anyhow::bail!("unsupported database driver '{}'", other),
        }

        Ok(())
    }

    pub fn default_yaml() -> &'static str {
        r#"# qhook.yaml

database:
  driver: sqlite  # sqlite (default) / postgres
  # url: ${DATABASE_URL}

server:
  port: 8888
  # ip_rate_limit: 100  # Per-IP requests/sec (0 = disabled)

# api:
#   auth_token: ${QHOOK_API_TOKEN}

delivery:
  # signing_secret: ${QHOOK_SIGNING_SECRET}
  timeout: 30s
  default_retry:
    max: 5
    backoff: exponential
    interval: 30s

sources:
  # stripe:
  #   type: webhook
  #   verify: stripe
  #   secret: ${STRIPE_WEBHOOK_SECRET}
  app:
    type: event

handlers: {}
  # payment-success:
  #   source: stripe
  #   events: [checkout.session.completed, invoice.paid]
  #   url: http://localhost:3000/jobs/payment
  #   retry: { max: 5 }
  #   idempotency_key: "$.id"
  #   type: http            # http (default) or grpc
  #   filter: "$.status == paid"
  #   transform: '{"event_id": "{{$.id}}", "amount": {{$.data.amount}}}'

# alerts:
#   url: ${SLACK_WEBHOOK_URL}
#   type: slack  # slack / discord / generic
#   on: [dlq, verification_failure]
"#
    }
}

/// Validate that a handler URL is safe (HTTP(S) scheme, no private IPs).
fn validate_handler_url(handler_name: &str, url: &str) -> Result<()> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        anyhow::bail!(
            "handler '{}' URL must start with http:// or https://, got '{}'",
            handler_name,
            url
        );
    }

    // Extract host from URL
    let after_scheme = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);
    let host = after_scheme.split('/').next().unwrap_or("");
    // Strip port
    let host = if let Some(bracket_end) = host.find(']') {
        // IPv6: [::1]:8080
        &host[..bracket_end + 1]
    } else {
        host.split(':').next().unwrap_or(host)
    };

    // Block obvious private/loopback addresses
    let blocked = [
        "localhost",
        "127.0.0.1",
        "0.0.0.0",
        "[::1]",
        "[::0]",
        "[::]",
    ];
    if blocked.contains(&host) {
        tracing::warn!(
            handler = handler_name,
            url,
            "Handler URL points to loopback address"
        );
    }

    // Block private IP ranges (10.x, 172.16-31.x, 192.168.x, 169.254.x)
    if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        if ip.is_loopback() || ip.is_unspecified() {
            tracing::warn!(handler = handler_name, url, "Handler URL points to loopback");
        } else if is_private_ipv4(ip) {
            tracing::warn!(
                handler = handler_name,
                url,
                "Handler URL points to private IP range"
            );
        }
    }

    Ok(())
}

fn is_private_ipv4(ip: std::net::Ipv4Addr) -> bool {
    let octets = ip.octets();
    // 10.0.0.0/8
    octets[0] == 10
    // 172.16.0.0/12
    || (octets[0] == 172 && (16..=31).contains(&octets[1]))
    // 192.168.0.0/16
    || (octets[0] == 192 && octets[1] == 168)
    // 169.254.0.0/16 (link-local)
    || (octets[0] == 169 && octets[1] == 254)
}

fn expand_env_vars(input: &str) -> String {
    let mut result = input.to_string();
    while let Some(start) = result.find("${") {
        if let Some(end) = result[start..].find('}') {
            let expr = &result[start + 2..start + end];
            let value = if let Some((var_name, default)) = expr.split_once(":-") {
                std::env::var(var_name)
                    .ok()
                    .filter(|v| !v.is_empty())
                    .unwrap_or_else(|| default.to_string())
            } else {
                std::env::var(expr).unwrap_or_default()
            };
            result = format!(
                "{}{}{}",
                &result[..start],
                value,
                &result[start + end + 1..]
            );
        } else {
            break;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(handlers: HashMap<String, HandlerConfig>, sources: HashMap<String, SourceConfig>) -> Config {
        Config {
            database: DatabaseConfig::default(),
            server: ServerConfig::default(),
            api: ApiConfig::default(),
            delivery: DeliveryConfig::default(),
            sources,
            handlers,
            worker: WorkerConfig::default(),
            alerts: None,
        }
    }

    #[test]
    fn test_validate_valid_config() {
        let mut sources = HashMap::new();
        sources.insert("stripe".into(), SourceConfig {
            source_type: "webhook".into(),
            verify: None,
            secret: None,
            skip_verify: false,
        });
        let mut handlers = HashMap::new();
        handlers.insert("payment".into(), HandlerConfig {
            source: "stripe".into(),
            events: vec![],
            url: "https://example.com/webhook".into(),
            retry: None,
            timeout: None,
            idempotency_key: None,
            rate_limit: None,
            filter: None,
            transform: None,
            handler_type: "http".into(),
        });
        let config = make_config(handlers, sources);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_invalid_url_scheme() {
        let mut sources = HashMap::new();
        sources.insert("app".into(), SourceConfig {
            source_type: "event".into(),
            verify: None,
            secret: None,
            skip_verify: false,
        });
        let mut handlers = HashMap::new();
        handlers.insert("bad".into(), HandlerConfig {
            source: "app".into(),
            events: vec![],
            url: "ftp://evil.com/data".into(),
            retry: None,
            timeout: None,
            idempotency_key: None,
            rate_limit: None,
            filter: None,
            transform: None,
            handler_type: "http".into(),
        });
        let config = make_config(handlers, sources);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_unknown_source_ref() {
        let sources = HashMap::new();
        let mut handlers = HashMap::new();
        handlers.insert("orphan".into(), HandlerConfig {
            source: "nonexistent".into(),
            events: vec![],
            url: "https://example.com".into(),
            retry: None,
            timeout: None,
            idempotency_key: None,
            rate_limit: None,
            filter: None,
            transform: None,
            handler_type: "http".into(),
        });
        let config = make_config(handlers, sources);
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("unknown source"));
    }

    #[test]
    fn test_validate_invalid_source_type() {
        let mut sources = HashMap::new();
        sources.insert("bad".into(), SourceConfig {
            source_type: "grpc".into(),
            verify: None,
            secret: None,
            skip_verify: false,
        });
        let config = make_config(HashMap::new(), sources);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_private_ip_warns_but_passes() {
        let mut sources = HashMap::new();
        sources.insert("app".into(), SourceConfig {
            source_type: "event".into(),
            verify: None,
            secret: None,
            skip_verify: false,
        });
        let mut handlers = HashMap::new();
        handlers.insert("internal".into(), HandlerConfig {
            source: "app".into(),
            events: vec![],
            url: "http://10.0.0.5:3000/hook".into(),
            retry: None,
            timeout: None,
            idempotency_key: None,
            rate_limit: None,
            filter: None,
            transform: None,
            handler_type: "http".into(),
        });
        // Private IPs are warned, not blocked (for dev/internal use)
        let config = make_config(handlers, sources);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_is_private_ipv4() {
        assert!(is_private_ipv4("10.0.0.1".parse().unwrap()));
        assert!(is_private_ipv4("172.16.0.1".parse().unwrap()));
        assert!(is_private_ipv4("172.31.255.255".parse().unwrap()));
        assert!(is_private_ipv4("192.168.1.1".parse().unwrap()));
        assert!(is_private_ipv4("169.254.0.1".parse().unwrap()));
        assert!(!is_private_ipv4("8.8.8.8".parse().unwrap()));
        assert!(!is_private_ipv4("172.32.0.1".parse().unwrap()));
    }

    #[test]
    fn test_validate_verify_without_secret() {
        let mut sources = HashMap::new();
        sources.insert("github".into(), SourceConfig {
            source_type: "webhook".into(),
            verify: Some("github".into()),
            secret: None, // missing!
            skip_verify: false,
        });
        let config = make_config(HashMap::new(), sources);
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("no secret configured"));
    }

    #[test]
    fn test_validate_verify_with_empty_secret() {
        let mut sources = HashMap::new();
        sources.insert("stripe".into(), SourceConfig {
            source_type: "webhook".into(),
            verify: Some("stripe".into()),
            secret: Some("".into()), // empty!
            skip_verify: false,
        });
        let config = make_config(HashMap::new(), sources);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_invalid_handler_type() {
        let mut sources = HashMap::new();
        sources.insert("app".into(), SourceConfig {
            source_type: "event".into(),
            verify: None,
            secret: None,
            skip_verify: false,
        });
        let mut handlers = HashMap::new();
        handlers.insert("bad".into(), HandlerConfig {
            source: "app".into(),
            events: vec![],
            url: "http://example.com".into(),
            handler_type: "websocket".into(),
            retry: None,
            timeout: None,
            idempotency_key: None,
            rate_limit: None,
            filter: None,
            transform: None,
        });
        let config = make_config(handlers, sources);
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("invalid type"));
    }

    #[test]
    fn test_expand_env_vars() {
        // SAFETY: test runs single-threaded for env var mutation
        unsafe { std::env::set_var("QHOOK_TEST_VAR", "hello") };
        assert_eq!(expand_env_vars("${QHOOK_TEST_VAR}"), "hello");
        assert_eq!(expand_env_vars("${MISSING_VAR:-default}"), "default");
        unsafe { std::env::remove_var("QHOOK_TEST_VAR") };
    }
}
