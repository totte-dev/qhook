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
}

#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    #[serde(default = "default_driver")]
    pub driver: String,
    pub url: Option<String>,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            driver: "sqlite".into(),
            url: None,
        }
    }
}

fn default_driver() -> String {
    "sqlite".into()
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_port")]
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
        }
    }
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
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct HandlerConfig {
    pub source: String,
    #[serde(default)]
    pub events: Vec<String>,
    pub url: String,
    pub retry: Option<RetryConfig>,
    pub timeout: Option<String>,
    pub idempotency_key: Option<String>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config: {}", path.display()))?;
        let content = expand_env_vars(&content);
        let config: Config =
            serde_yaml::from_str(&content).context("Failed to parse YAML config")?;
        Ok(config)
    }

    pub fn default_yaml() -> &'static str {
        r#"# qhook.yaml

database:
  driver: sqlite  # sqlite (default) / postgres
  # url: ${DATABASE_URL}

server:
  port: 8888

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
"#
    }
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
            result = format!("{}{}{}", &result[..start], value, &result[start + end + 1..]);
        } else {
            break;
        }
    }
    result
}
