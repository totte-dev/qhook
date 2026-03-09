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
    pub workflows: HashMap<String, WorkflowConfig>,
    #[serde(default)]
    pub worker: WorkerConfig,
    #[serde(default)]
    pub alerts: Option<AlertConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WorkflowConfig {
    pub source: String,
    #[serde(default)]
    pub events: Vec<String>,
    pub steps: Vec<StepConfig>,
    /// Overall workflow timeout in seconds. If set, the workflow fails if it runs longer.
    pub timeout: Option<u64>,
    /// Input parameters for the workflow. Each param has a name and type constraint.
    /// When defined, the event payload is validated against these parameters.
    #[serde(default)]
    pub params: Vec<ParamConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ParamConfig {
    /// Parameter name (must exist as a top-level key in the event payload).
    pub name: String,
    /// Parameter type: "string", "number", "boolean", "object", "array".
    #[serde(rename = "type", default = "default_param_type")]
    pub param_type: String,
    /// Whether this parameter is required (default: true).
    #[serde(default = "default_required")]
    pub required: bool,
}

fn default_param_type() -> String {
    "string".into()
}
fn default_required() -> bool {
    true
}

#[derive(Debug, Deserialize, Clone)]
pub struct StepConfig {
    pub name: String,
    /// HTTP/gRPC endpoint to call (required for task steps).
    #[serde(default)]
    pub url: Option<String>,
    /// Handler type: "http" (default), "grpc", "choice", "parallel", "map", "wait", "callback".
    #[serde(rename = "type", default = "default_handler_type")]
    pub handler_type: String,
    /// Custom HTTP headers to send with this step's request. Supports env var expansion.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Transform input before sending (replaces request body).
    pub input: Option<String>,
    /// Where to merge the response into the payload.
    /// "$" = replace (default when None), "$.key" = add as field, null = discard.
    pub result_path: Option<String>,
    /// Transform output before passing to the next step.
    pub output: Option<String>,
    /// Retry configuration with error type matching.
    pub retry: Option<StepRetryConfig>,
    /// Catch blocks for error routing after retries exhausted.
    pub catch: Option<Vec<CatchConfig>>,
    /// Timeout in seconds.
    pub timeout: Option<u64>,
    /// What to do on failure: stop (default) or continue to next step.
    #[serde(default)]
    pub on_failure: OnFailure,
    /// If true, the workflow ends after this step.
    #[serde(default)]
    pub end: bool,
    // --- Choice step fields ---
    /// Condition rules for choice steps.
    pub choices: Option<Vec<ChoiceRule>>,
    /// Default goto target if no choice matches.
    pub default: Option<String>,
    // --- Parallel step fields ---
    /// Branches to execute in parallel.
    pub branches: Option<Vec<BranchConfig>>,
    // --- Map step fields ---
    /// JSONPath to array in payload to iterate over.
    pub items_path: Option<String>,
    /// Max concurrent map executions (default: 10).
    pub max_concurrency: Option<u32>,
    // --- Wait step fields ---
    /// Fixed wait duration in seconds (for type: wait).
    pub seconds: Option<u64>,
    /// JSONPath to a timestamp in the payload (for type: wait, alternative to seconds).
    pub timestamp_path: Option<String>,
    // --- Callback step fields ---
    /// Timeout in seconds for callback step (how long to wait for external callback).
    pub callback_timeout: Option<u64>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ChoiceRule {
    /// Filter condition (same syntax as handler.filter).
    pub when: String,
    /// Step name to jump to if condition matches.
    pub goto: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BranchConfig {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct StepRetryConfig {
    #[serde(default = "default_max_retries")]
    pub max: u32,
    #[serde(default = "default_backoff")]
    pub backoff: String,
    #[serde(default = "default_interval")]
    pub interval: String,
    /// Error types to retry on. Default: [all].
    #[serde(default = "default_error_types")]
    pub errors: Vec<ErrorType>,
}

fn default_error_types() -> Vec<ErrorType> {
    vec![ErrorType::All]
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ErrorType {
    Timeout,
    #[serde(rename = "5xx")]
    Http5xx,
    #[serde(rename = "4xx")]
    Http4xx,
    Network,
    All,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CatchConfig {
    pub errors: Vec<ErrorType>,
    pub goto: String,
}

#[derive(Debug, Deserialize, Clone, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OnFailure {
    #[default]
    Stop,
    Continue,
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
    vec!["dlq".into(), "verification_failure".into()]
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
    /// Allow handler/workflow URLs pointing to private/loopback IPs (default: false).
    /// Enable for development or when handlers run on the same host.
    #[serde(default)]
    pub allow_private_urls: bool,
    /// Trust proxy headers (X-Forwarded-For, X-Real-IP) for IP rate limiting (default: false).
    /// Enable when running behind a reverse proxy (nginx, cloud load balancer).
    #[serde(default)]
    pub trust_proxy: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            max_body_size: default_max_body_size(),
            max_inbound: default_max_inbound(),
            ip_rate_limit: 0,
            allow_private_urls: false,
            trust_proxy: false,
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
    /// Bearer token for the /metrics endpoint. If not set, the endpoint is open.
    pub metrics_auth_token: Option<String>,
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
    /// Custom HTTP headers to send with delivery requests. Supports env var expansion.
    #[serde(default)]
    pub headers: HashMap<String, String>,
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
            validate_handler_url(name, &handler.url, self.server.allow_private_urls)?;

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
            if source.verify.is_some() && source.secret.as_ref().is_none_or(|s| s.is_empty()) {
                anyhow::bail!(
                    "source '{}' has verify enabled but no secret configured",
                    name
                );
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

        // Validate workflows
        for (name, workflow) in &self.workflows {
            // Source must exist
            if !self.sources.contains_key(&workflow.source) {
                anyhow::bail!(
                    "workflow '{}' references unknown source '{}'",
                    name,
                    workflow.source
                );
            }

            // Must have at least one step
            if workflow.steps.is_empty() {
                anyhow::bail!("workflow '{}' has no steps", name);
            }

            // Collect step names for uniqueness and goto validation
            let mut step_names = std::collections::HashSet::new();
            for step in &workflow.steps {
                if !step_names.insert(step.name.as_str()) {
                    anyhow::bail!(
                        "workflow '{}' has duplicate step name '{}'",
                        name,
                        step.name
                    );
                }
            }

            // Validate step URLs, catch goto targets, and step type-specific fields
            for step in &workflow.steps {
                if let Some(ref url) = step.url {
                    validate_handler_url(
                        &format!("{}/{}", name, step.name),
                        url,
                        self.server.allow_private_urls,
                    )?;
                }
                if let Some(ref catches) = step.catch {
                    for c in catches {
                        if !step_names.contains(c.goto.as_str()) {
                            anyhow::bail!(
                                "workflow '{}' step '{}' catch goto '{}' references nonexistent step",
                                name,
                                step.name,
                                c.goto
                            );
                        }
                    }
                }

                // Validate choice steps
                if step.handler_type == "choice" {
                    if step.choices.as_ref().is_none_or(|c| c.is_empty()) {
                        anyhow::bail!(
                            "workflow '{}' choice step '{}' has no choices",
                            name,
                            step.name
                        );
                    }
                    // Validate goto targets
                    if let Some(ref choices) = step.choices {
                        for c in choices {
                            if !step_names.contains(c.goto.as_str()) {
                                anyhow::bail!(
                                    "workflow '{}' choice step '{}' goto '{}' references nonexistent step",
                                    name,
                                    step.name,
                                    c.goto
                                );
                            }
                        }
                    }
                    if let Some(ref default) = step.default {
                        if !step_names.contains(default.as_str()) {
                            anyhow::bail!(
                                "workflow '{}' choice step '{}' default '{}' references nonexistent step",
                                name,
                                step.name,
                                default
                            );
                        }
                    }
                }

                // Validate parallel steps
                if step.handler_type == "parallel" {
                    if step.branches.as_ref().is_none_or(|b| b.is_empty()) {
                        anyhow::bail!(
                            "workflow '{}' parallel step '{}' has no branches",
                            name,
                            step.name
                        );
                    }
                    if let Some(ref branches) = step.branches {
                        let mut branch_names = std::collections::HashSet::new();
                        for b in branches {
                            if !branch_names.insert(b.name.as_str()) {
                                anyhow::bail!(
                                    "workflow '{}' parallel step '{}' has duplicate branch name '{}'",
                                    name,
                                    step.name,
                                    b.name
                                );
                            }
                            validate_handler_url(
                                &format!("{}/{}:{}", name, step.name, b.name),
                                &b.url,
                                self.server.allow_private_urls,
                            )?;
                        }
                    }
                }

                // Validate wait steps
                if step.handler_type == "wait"
                    && step.seconds.is_none()
                    && step.timestamp_path.is_none()
                {
                    anyhow::bail!(
                        "workflow '{}' wait step '{}' needs either 'seconds' or 'timestamp_path'",
                        name,
                        step.name
                    );
                }

                // Validate callback steps
                if step.handler_type == "callback" {
                    // callback steps are valid with no extra fields (callback_timeout is optional)
                }

                // Validate map steps
                if step.handler_type == "map" {
                    if step.items_path.is_none() {
                        anyhow::bail!(
                            "workflow '{}' map step '{}' has no items_path",
                            name,
                            step.name
                        );
                    }
                    if step.url.is_none() {
                        anyhow::bail!("workflow '{}' map step '{}' has no url", name, step.name);
                    }
                }
            }

            // Validate params
            for param in &workflow.params {
                match param.param_type.as_str() {
                    "string" | "number" | "boolean" | "object" | "array" => {}
                    other => anyhow::bail!(
                        "workflow '{}' param '{}' has invalid type '{}'",
                        name,
                        param.name,
                        other
                    ),
                }
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
  # allow_private_urls: true   # Allow localhost/private IPs in handler URLs (dev only)
  # trust_proxy: true          # Trust X-Forwarded-For for rate limiting (behind reverse proxy)
  # ip_rate_limit: 100         # Per-IP requests/sec (0 = disabled)

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

# workflows:
#   order-pipeline:
#     source: app
#     events: [order.created]
#     steps:
#       - name: validate
#         url: http://localhost:3000/validate
#       - name: fulfill
#         url: http://localhost:3000/fulfill
#       - name: notify
#         url: http://localhost:3000/notify
#         end: true

# alerts:
#   url: ${SLACK_WEBHOOK_URL}
#   type: slack  # slack / discord / generic
#   on: [dlq, verification_failure]
"#
    }
}

/// Validate that a handler URL is safe (HTTP(S) scheme, no private IPs).
/// When `allow_private` is false (production default), private/loopback URLs are rejected.
fn validate_handler_url(handler_name: &str, url: &str, allow_private: bool) -> Result<()> {
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

    // Check for private/loopback addresses
    let blocked = [
        "localhost",
        "127.0.0.1",
        "0.0.0.0",
        "[::1]",
        "[::0]",
        "[::]",
    ];
    let is_blocked = blocked.contains(&host);

    let is_private = if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        ip.is_loopback() || ip.is_unspecified() || is_private_ipv4(ip)
    } else {
        false
    };

    if is_blocked || is_private {
        if allow_private {
            tracing::warn!(
                handler = handler_name,
                url,
                "Handler URL points to private/loopback address (allowed by allow_private_urls)"
            );
        } else {
            anyhow::bail!(
                "handler '{}' URL '{}' points to a private/loopback address. \
                 Set server.allow_private_urls: true to allow this (dev only)",
                handler_name,
                url
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

    fn make_config(
        handlers: HashMap<String, HandlerConfig>,
        sources: HashMap<String, SourceConfig>,
    ) -> Config {
        Config {
            database: DatabaseConfig::default(),
            server: ServerConfig::default(),
            api: ApiConfig::default(),
            delivery: DeliveryConfig::default(),
            sources,
            handlers,
            workflows: HashMap::new(),
            worker: WorkerConfig::default(),
            alerts: None,
        }
    }

    #[test]
    fn test_validate_valid_config() {
        let mut sources = HashMap::new();
        sources.insert(
            "stripe".into(),
            SourceConfig {
                source_type: "webhook".into(),
                verify: None,
                secret: None,
                skip_verify: false,
            },
        );
        let mut handlers = HashMap::new();
        handlers.insert(
            "payment".into(),
            HandlerConfig {
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
                headers: HashMap::new(),
            },
        );
        let config = make_config(handlers, sources);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_invalid_url_scheme() {
        let mut sources = HashMap::new();
        sources.insert(
            "app".into(),
            SourceConfig {
                source_type: "event".into(),
                verify: None,
                secret: None,
                skip_verify: false,
            },
        );
        let mut handlers = HashMap::new();
        handlers.insert(
            "bad".into(),
            HandlerConfig {
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
                headers: HashMap::new(),
            },
        );
        let config = make_config(handlers, sources);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_unknown_source_ref() {
        let sources = HashMap::new();
        let mut handlers = HashMap::new();
        handlers.insert(
            "orphan".into(),
            HandlerConfig {
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
                headers: HashMap::new(),
            },
        );
        let config = make_config(handlers, sources);
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("unknown source"));
    }

    #[test]
    fn test_validate_invalid_source_type() {
        let mut sources = HashMap::new();
        sources.insert(
            "bad".into(),
            SourceConfig {
                source_type: "grpc".into(),
                verify: None,
                secret: None,
                skip_verify: false,
            },
        );
        let config = make_config(HashMap::new(), sources);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_private_ip_rejected_by_default() {
        let mut sources = HashMap::new();
        sources.insert(
            "app".into(),
            SourceConfig {
                source_type: "event".into(),
                verify: None,
                secret: None,
                skip_verify: false,
            },
        );
        let mut handlers = HashMap::new();
        handlers.insert(
            "internal".into(),
            HandlerConfig {
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
                headers: HashMap::new(),
            },
        );
        // Private IPs are rejected by default (SSRF protection)
        let config = make_config(handlers, sources);
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("private/loopback"));
    }

    #[test]
    fn test_validate_private_ip_allowed_when_configured() {
        let mut sources = HashMap::new();
        sources.insert(
            "app".into(),
            SourceConfig {
                source_type: "event".into(),
                verify: None,
                secret: None,
                skip_verify: false,
            },
        );
        let mut handlers = HashMap::new();
        handlers.insert(
            "internal".into(),
            HandlerConfig {
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
                headers: HashMap::new(),
            },
        );
        let mut config = make_config(handlers, sources);
        config.server.allow_private_urls = true;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_localhost_rejected_by_default() {
        let mut sources = HashMap::new();
        sources.insert(
            "app".into(),
            SourceConfig {
                source_type: "event".into(),
                verify: None,
                secret: None,
                skip_verify: false,
            },
        );
        let mut handlers = HashMap::new();
        handlers.insert(
            "local".into(),
            HandlerConfig {
                source: "app".into(),
                events: vec![],
                url: "http://localhost:3000/hook".into(),
                retry: None,
                timeout: None,
                idempotency_key: None,
                rate_limit: None,
                filter: None,
                transform: None,
                handler_type: "http".into(),
                headers: HashMap::new(),
            },
        );
        let config = make_config(handlers, sources);
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("private/loopback"));
    }

    #[test]
    fn test_validate_metadata_ip_rejected() {
        let mut sources = HashMap::new();
        sources.insert(
            "app".into(),
            SourceConfig {
                source_type: "event".into(),
                verify: None,
                secret: None,
                skip_verify: false,
            },
        );
        let mut handlers = HashMap::new();
        handlers.insert(
            "ssrf".into(),
            HandlerConfig {
                source: "app".into(),
                events: vec![],
                url: "http://169.254.169.254/latest/meta-data".into(),
                retry: None,
                timeout: None,
                idempotency_key: None,
                rate_limit: None,
                filter: None,
                transform: None,
                handler_type: "http".into(),
                headers: HashMap::new(),
            },
        );
        let config = make_config(handlers, sources);
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("private/loopback"));
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
        sources.insert(
            "github".into(),
            SourceConfig {
                source_type: "webhook".into(),
                verify: Some("github".into()),
                secret: None, // missing!
                skip_verify: false,
            },
        );
        let config = make_config(HashMap::new(), sources);
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("no secret configured"));
    }

    #[test]
    fn test_validate_verify_with_empty_secret() {
        let mut sources = HashMap::new();
        sources.insert(
            "stripe".into(),
            SourceConfig {
                source_type: "webhook".into(),
                verify: Some("stripe".into()),
                secret: Some("".into()), // empty!
                skip_verify: false,
            },
        );
        let config = make_config(HashMap::new(), sources);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_invalid_handler_type() {
        let mut sources = HashMap::new();
        sources.insert(
            "app".into(),
            SourceConfig {
                source_type: "event".into(),
                verify: None,
                secret: None,
                skip_verify: false,
            },
        );
        let mut handlers = HashMap::new();
        handlers.insert(
            "bad".into(),
            HandlerConfig {
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
                headers: HashMap::new(),
            },
        );
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

    // --- v0.2 Workflow Config Tests ---

    #[test]
    fn test_parse_workflow_config() {
        let yaml = r#"
sources:
  app:
    type: event

handlers: {}

workflows:
  payment-flow:
    source: app
    events: [checkout.completed]
    steps:
      - name: validate
        url: https://api.example.com/validate
      - name: fulfill
        url: https://api.example.com/fulfill
      - name: notify
        url: https://hooks.slack.com/xxx
        on_failure: continue
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        assert_eq!(config.workflows.len(), 1);
        let wf = &config.workflows["payment-flow"];
        assert_eq!(wf.source, "app");
        assert_eq!(wf.events, vec!["checkout.completed"]);
        assert_eq!(wf.steps.len(), 3);
        assert_eq!(wf.steps[0].name, "validate");
        assert_eq!(wf.steps[2].on_failure, OnFailure::Continue);
    }

    #[test]
    fn test_parse_step_with_retry_errors_and_catch() {
        let yaml = r#"
sources:
  app:
    type: event

handlers: {}

workflows:
  test-flow:
    source: app
    steps:
      - name: call-api
        url: https://api.example.com/do
        retry:
          max: 3
          errors: [timeout, 5xx]
        catch:
          - errors: [4xx]
            goto: handle-bad-request
          - errors: [all]
            goto: alert
      - name: handle-bad-request
        url: https://api.example.com/bad-request
        end: true
      - name: alert
        url: https://hooks.slack.com/alert
        end: true
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        let wf = &config.workflows["test-flow"];
        let step = &wf.steps[0];
        let retry = step.retry.as_ref().unwrap();
        assert_eq!(retry.max, 3);
        assert_eq!(retry.errors, vec![ErrorType::Timeout, ErrorType::Http5xx]);
        let catch = step.catch.as_ref().unwrap();
        assert_eq!(catch.len(), 2);
        assert_eq!(catch[0].errors, vec![ErrorType::Http4xx]);
        assert_eq!(catch[0].goto, "handle-bad-request");
        assert_eq!(catch[1].errors, vec![ErrorType::All]);
        assert!(wf.steps[1].end);
    }

    #[test]
    fn test_parse_step_with_data_flow() {
        let yaml = r#"
sources:
  app:
    type: event

handlers: {}

workflows:
  data-flow:
    source: app
    steps:
      - name: enrich
        url: https://api.example.com/enrich
        input: '{"id": "{{$.id}}"}'
        result_path: "$.enrichment"
        output: '{"id": "{{$.id}}", "score": {{$.enrichment.score}}}'
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let step = &config.workflows["data-flow"].steps[0];
        assert_eq!(step.input.as_deref(), Some(r#"{"id": "{{$.id}}"}"#));
        assert_eq!(step.result_path.as_deref(), Some("$.enrichment"));
        assert_eq!(
            step.output.as_deref(),
            Some(r#"{"id": "{{$.id}}", "score": {{$.enrichment.score}}}"#)
        );
    }

    #[test]
    fn test_validate_workflow_unknown_source() {
        let yaml = r#"
sources:
  app:
    type: event

handlers: {}

workflows:
  bad:
    source: nonexistent
    steps:
      - name: step1
        url: https://example.com
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("unknown source"));
    }

    #[test]
    fn test_validate_workflow_duplicate_step_names() {
        let yaml = r#"
sources:
  app:
    type: event

handlers: {}

workflows:
  bad:
    source: app
    steps:
      - name: step1
        url: https://example.com
      - name: step1
        url: https://example.com/2
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("duplicate step name"));
    }

    #[test]
    fn test_validate_workflow_catch_goto_nonexistent() {
        let yaml = r#"
sources:
  app:
    type: event

handlers: {}

workflows:
  bad:
    source: app
    steps:
      - name: step1
        url: https://example.com
        catch:
          - errors: [all]
            goto: nonexistent
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
    }

    #[test]
    fn test_validate_workflow_empty_steps() {
        let yaml = r#"
sources:
  app:
    type: event

handlers: {}

workflows:
  bad:
    source: app
    steps: []
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("no steps"));
    }

    #[test]
    fn test_default_on_failure_is_stop() {
        let yaml = r#"
sources:
  app:
    type: event

handlers: {}

workflows:
  test:
    source: app
    steps:
      - name: step1
        url: https://example.com
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(
            config.workflows["test"].steps[0].on_failure,
            OnFailure::Stop
        );
    }

    // --- v0.2 Phase 2: Choice, Parallel, Map ---

    #[test]
    fn test_parse_choice_step() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  routing:
    source: app
    events: [order.created]
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
        url: https://api.example.com/high-value
        end: true
      - name: premium
        url: https://api.example.com/premium
        end: true
      - name: standard
        url: https://api.example.com/standard
        end: true
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        let wf = &config.workflows["routing"];
        let step = &wf.steps[0];
        assert_eq!(step.handler_type, "choice");
        let choices = step.choices.as_ref().unwrap();
        assert_eq!(choices.len(), 2);
        assert_eq!(choices[0].when, "$.amount >= 10000");
        assert_eq!(choices[0].goto, "high-value");
        assert_eq!(step.default.as_deref(), Some("standard"));
    }

    #[test]
    fn test_validate_choice_no_choices() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  bad:
    source: app
    steps:
      - name: route
        type: choice
      - name: target
        url: https://example.com
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("no choices"));
    }

    #[test]
    fn test_validate_choice_goto_nonexistent() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  bad:
    source: app
    steps:
      - name: route
        type: choice
        choices:
          - when: "$.x == 1"
            goto: nonexistent
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
    }

    #[test]
    fn test_parse_parallel_step() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  checks:
    source: app
    steps:
      - name: verify
        type: parallel
        branches:
          - name: credit
            url: https://credit-service/check
          - name: fraud
            url: https://fraud-service/check
        result_path: "$.checks"
      - name: process
        url: https://api.example.com/process
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        let step = &config.workflows["checks"].steps[0];
        assert_eq!(step.handler_type, "parallel");
        let branches = step.branches.as_ref().unwrap();
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[0].name, "credit");
        assert_eq!(branches[1].name, "fraud");
    }

    #[test]
    fn test_validate_parallel_no_branches() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  bad:
    source: app
    steps:
      - name: verify
        type: parallel
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("no branches"));
    }

    #[test]
    fn test_validate_parallel_duplicate_branch_names() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  bad:
    source: app
    steps:
      - name: verify
        type: parallel
        branches:
          - name: check
            url: https://a.example.com
          - name: check
            url: https://b.example.com
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("duplicate branch name"));
    }

    #[test]
    fn test_parse_map_step() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  batch:
    source: app
    steps:
      - name: process-items
        type: map
        items_path: "$.items"
        url: https://api.example.com/process
        max_concurrency: 5
      - name: summarize
        url: https://api.example.com/summarize
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        let step = &config.workflows["batch"].steps[0];
        assert_eq!(step.handler_type, "map");
        assert_eq!(step.items_path.as_deref(), Some("$.items"));
        assert_eq!(step.max_concurrency, Some(5));
    }

    #[test]
    fn test_validate_map_no_items_path() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  bad:
    source: app
    steps:
      - name: process
        type: map
        url: https://example.com
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("no items_path"));
    }

    #[test]
    fn test_validate_map_no_url() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  bad:
    source: app
    steps:
      - name: process
        type: map
        items_path: "$.items"
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("no url"));
    }

    #[test]
    fn test_default_result_path_is_none() {
        let yaml = r#"
sources:
  app:
    type: event

handlers: {}

workflows:
  test:
    source: app
    steps:
      - name: step1
        url: https://example.com
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        // Default: no result_path means replace entirely ($)
        assert!(config.workflows["test"].steps[0].result_path.is_none());
    }

    // --- v0.2 Phase 3: Wait, Callback, Timeout ---

    #[test]
    fn test_parse_wait_step_seconds() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  delayed:
    source: app
    steps:
      - name: delay
        type: wait
        seconds: 60
      - name: process
        url: https://api.example.com/process
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        let step = &config.workflows["delayed"].steps[0];
        assert_eq!(step.handler_type, "wait");
        assert_eq!(step.seconds, Some(60));
    }

    #[test]
    fn test_parse_wait_step_timestamp_path() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  scheduled:
    source: app
    steps:
      - name: wait-until
        type: wait
        timestamp_path: "$.scheduled_at"
      - name: execute
        url: https://api.example.com/execute
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        let step = &config.workflows["scheduled"].steps[0];
        assert_eq!(step.handler_type, "wait");
        assert_eq!(step.timestamp_path.as_deref(), Some("$.scheduled_at"));
    }

    #[test]
    fn test_validate_wait_no_seconds_or_timestamp() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  bad:
    source: app
    steps:
      - name: delay
        type: wait
      - name: process
        url: https://example.com
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("seconds"));
    }

    #[test]
    fn test_parse_callback_step() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  approval:
    source: app
    steps:
      - name: request-approval
        url: https://api.example.com/request
      - name: wait-approval
        type: callback
        callback_timeout: 3600
      - name: process
        url: https://api.example.com/process
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        let step = &config.workflows["approval"].steps[1];
        assert_eq!(step.handler_type, "callback");
        assert_eq!(step.callback_timeout, Some(3600));
    }

    #[test]
    fn test_parse_workflow_timeout() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  timed:
    source: app
    timeout: 300
    steps:
      - name: step1
        url: https://api.example.com/do
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        assert_eq!(config.workflows["timed"].timeout, Some(300));
    }

    #[test]
    fn test_parse_handler_with_custom_headers() {
        let yaml = r#"
sources:
  app:
    type: event
handlers:
  deploy:
    source: app
    url: https://api.example.com/deploy
    headers:
      Authorization: "Bearer my-token"
      X-Custom: "value"
workflows: {}
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        let handler = &config.handlers["deploy"];
        assert_eq!(handler.headers.len(), 2);
        assert_eq!(handler.headers["Authorization"], "Bearer my-token");
        assert_eq!(handler.headers["X-Custom"], "value");
    }

    #[test]
    fn test_parse_handler_without_headers() {
        let yaml = r#"
sources:
  app:
    type: event
handlers:
  simple:
    source: app
    url: https://api.example.com/hook
workflows: {}
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        assert!(config.handlers["simple"].headers.is_empty());
    }

    #[test]
    fn test_parse_step_with_custom_headers() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  provision:
    source: app
    steps:
      - name: create-infra
        url: https://api.terraform.io/v2/runs
        headers:
          Authorization: "Bearer tf-token"
          Content-Type: "application/vnd.api+json"
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        let step = &config.workflows["provision"].steps[0];
        assert_eq!(step.headers.len(), 2);
        assert_eq!(step.headers["Authorization"], "Bearer tf-token");
    }

    #[test]
    fn test_parse_new_verify_providers() {
        let yaml = r#"
sources:
  pagerduty:
    type: webhook
    verify: pagerduty
    secret: pd-secret
  grafana:
    type: webhook
    verify: grafana
    secret: gf-secret
  terraform:
    type: webhook
    verify: terraform
    secret: tf-secret
  gitlab:
    type: webhook
    verify: gitlab
    secret: gl-token
handlers: {}
workflows: {}
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        assert_eq!(
            config.sources["pagerduty"].verify.as_deref(),
            Some("pagerduty")
        );
        assert_eq!(config.sources["gitlab"].verify.as_deref(), Some("gitlab"));
    }

    #[test]
    fn test_parse_workflow_with_params() {
        let yaml = r#"
sources:
  platform:
    type: event
handlers: {}
workflows:
  provision:
    source: platform
    events: [tenant.create]
    params:
      - name: tenant_id
        type: string
      - name: region
        type: string
        required: false
      - name: config
        type: object
    steps:
      - name: create-infra
        url: https://infra.example.com/provision
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        let wf = &config.workflows["provision"];
        assert_eq!(wf.params.len(), 3);
        assert_eq!(wf.params[0].name, "tenant_id");
        assert_eq!(wf.params[0].param_type, "string");
        assert!(wf.params[0].required);
        assert!(!wf.params[1].required);
        assert_eq!(wf.params[2].param_type, "object");
    }

    #[test]
    fn test_parse_metrics_auth_token() {
        let yaml = r#"
sources: {}
handlers: {}
api:
  auth_token: my-token
  metrics_auth_token: metrics-secret
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(
            config.api.metrics_auth_token.as_deref(),
            Some("metrics-secret")
        );
    }

    #[test]
    fn test_parse_trust_proxy() {
        let yaml = r#"
sources: {}
handlers: {}
server:
  trust_proxy: true
  allow_private_urls: true
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.server.trust_proxy);
        assert!(config.server.allow_private_urls);
    }

    #[test]
    fn test_validate_workflow_invalid_param_type() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  bad:
    source: app
    params:
      - name: field
        type: integer
    steps:
      - name: step1
        url: https://example.com
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("invalid type 'integer'"));
    }
}
