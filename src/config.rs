use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Deserialize, Clone)]
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
    /// HTTP endpoint to call (required for task steps).
    #[serde(default)]
    pub url: Option<String>,
    /// Step type: "http" (default), "choice", "parallel", "map", "wait", "callback", "workflow".
    #[serde(rename = "type", default = "default_handler_type")]
    pub handler_type: String,
    /// HTTP method: GET, POST (default), PUT, PATCH, DELETE.
    #[serde(default = "default_http_method")]
    pub method: String,
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
    // --- Sub-workflow step fields ---
    /// Name of the workflow to invoke (for type: workflow).
    pub workflow: Option<String>,
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
    /// HTTP method: GET, POST (default), PUT, PATCH, DELETE.
    #[serde(default = "default_http_method")]
    pub method: String,
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
    /// Max concurrent deliveries per worker (default: 10).
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: usize,
    /// Number of jobs to fetch per poll cycle (default: 10).
    #[serde(default = "default_batch_size")]
    pub batch_size: i32,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            stale_threshold_secs: default_stale_threshold(),
            retention_hours: default_retention_hours(),
            drain_timeout_secs: default_drain_timeout(),
            max_concurrency: default_max_concurrency(),
            batch_size: default_batch_size(),
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
fn default_max_concurrency() -> usize {
    10
}
fn default_batch_size() -> i32 {
    10
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

#[derive(Debug, Deserialize, Clone)]
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

#[derive(Debug, Deserialize, Clone)]
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

#[derive(Debug, Deserialize, Default, Clone)]
pub struct ApiConfig {
    pub auth_token: Option<String>,
    /// Bearer token for the /metrics endpoint. If not set, the endpoint is open.
    pub metrics_auth_token: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
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

#[derive(Debug, Deserialize, Clone)]
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

#[derive(Debug, Deserialize, Clone)]
pub struct SourceConfig {
    #[serde(rename = "type")]
    pub source_type: String,
    pub verify: Option<String>,
    pub secret: Option<String>,
    #[serde(default)]
    pub skip_verify: bool,
    /// Cron schedule expression (5-field standard cron). Required for `cron` sources.
    pub schedule: Option<String>,
    /// Timezone for cron evaluation (e.g., "America/New_York"). Defaults to UTC.
    pub timezone: Option<String>,
    /// JSON Schema for validating incoming event payloads.
    /// Events that fail validation are rejected with 400.
    pub schema: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct HandlerConfig {
    pub source: String,
    #[serde(default)]
    pub events: Vec<String>,
    pub url: String,
    /// Handler type: "http" (default).
    #[serde(rename = "type", default = "default_handler_type")]
    pub handler_type: String,
    /// HTTP method: GET, POST (default), PUT, PATCH, DELETE.
    #[serde(default = "default_http_method")]
    pub method: String,
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

fn default_http_method() -> String {
    "POST".into()
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config: {}", path.display()))?;
        Self::from_yaml(&content)
    }

    /// Load config with optional environment overlay and .env file.
    ///
    /// Resolution order:
    /// 1. Load `.env.{env}` (if exists) into process environment
    /// 2. Load base config (e.g. `qhook.yaml`)
    /// 3. Deep merge overlay `qhook.{env}.yaml` (if exists)
    /// 4. Expand `${VAR}` references and validate
    pub fn load_with_env(base_path: &Path, env: &str) -> Result<Self> {
        // Load .env file for this environment
        let env_file_name = format!(".env.{}", env);
        let env_file = Path::new(&env_file_name);
        if env_file.exists() {
            load_dotenv(env_file)?;
        }

        // Read base config
        let base_content = std::fs::read_to_string(base_path)
            .with_context(|| format!("Failed to read config: {}", base_path.display()))?;
        let base_value: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(&base_content).context("Failed to parse base YAML config")?;

        // Read overlay if exists
        let stem = base_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("qhook");
        let ext = base_path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("yaml");
        let overlay_path = base_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(format!("{}.{}.{}", stem, env, ext));

        let merged = if overlay_path.exists() {
            let overlay_content = std::fs::read_to_string(&overlay_path).with_context(|| {
                format!("Failed to read overlay config: {}", overlay_path.display())
            })?;
            let overlay_value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&overlay_content)
                .context("Failed to parse overlay YAML config")?;
            deep_merge_yaml(base_value, overlay_value)
        } else {
            base_value
        };

        // Serialize back to string, expand env vars, parse into Config
        let merged_str =
            serde_yaml_ng::to_string(&merged).context("Failed to serialize merged config")?;
        Self::from_yaml(&merged_str)
    }

    /// Parse and validate config from a YAML string.
    pub fn from_yaml(raw: &str) -> Result<Self> {
        let content = expand_env_vars(raw);
        let config: Config =
            serde_yaml_ng::from_str(&content).context("Failed to parse YAML config")?;
        config.validate()?;
        Ok(config)
    }

    /// Check if a config path refers to a remote source.
    pub fn is_remote(path: &str) -> bool {
        path.starts_with("s3://")
            || path.starts_with("gs://")
            || path.starts_with("az://")
            || path.starts_with("http://")
            || path.starts_with("https://")
    }

    /// Resolve a remote config path to an HTTPS URL.
    /// Currently supports public (unauthenticated) access only.
    ///
    /// - `s3://bucket/key` → `https://bucket.s3.region.amazonaws.com/key`
    /// - `gs://bucket/key` → `https://storage.googleapis.com/bucket/key`
    /// - `az://account/container/key` → `https://account.blob.core.windows.net/container/key`
    /// - `http(s)://...` → passed through as-is
    pub fn resolve_remote_url(path: &str) -> Result<String> {
        if let Some(stripped) = path.strip_prefix("s3://") {
            let (bucket, key) = stripped
                .split_once('/')
                .with_context(|| format!("Invalid S3 path: {}. Expected s3://bucket/key", path))?;
            let region = std::env::var("AWS_REGION")
                .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
                .unwrap_or_else(|_| "us-east-1".to_string());
            if let Ok(endpoint) = std::env::var("AWS_ENDPOINT_URL") {
                Ok(format!(
                    "{}/{}/{}",
                    endpoint.trim_end_matches('/'),
                    bucket,
                    key
                ))
            } else {
                Ok(format!(
                    "https://{}.s3.{}.amazonaws.com/{}",
                    bucket, region, key
                ))
            }
        } else if let Some(stripped) = path.strip_prefix("gs://") {
            let (bucket, key) = stripped
                .split_once('/')
                .with_context(|| format!("Invalid GCS path: {}. Expected gs://bucket/key", path))?;
            Ok(format!("https://storage.googleapis.com/{}/{}", bucket, key))
        } else if let Some(stripped) = path.strip_prefix("az://") {
            // az://account/container/key
            let (account, rest) = stripped.split_once('/').with_context(|| {
                format!(
                    "Invalid Azure Blob path: {}. Expected az://account/container/key",
                    path
                )
            })?;
            if rest.is_empty() {
                anyhow::bail!(
                    "Invalid Azure Blob path: {}. Expected az://account/container/key",
                    path
                );
            }
            Ok(format!(
                "https://{}.blob.core.windows.net/{}",
                account, rest
            ))
        } else {
            // http:// or https:// — pass through
            Ok(path.to_string())
        }
    }

    /// Fetch config content from a remote source. Returns (content, etag).
    pub async fn fetch_remote(path: &str) -> Result<(String, Option<String>)> {
        let url = Self::resolve_remote_url(path)?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()?;
        let resp = client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Failed to fetch config from {}", path))?;
        if !resp.status().is_success() {
            anyhow::bail!(
                "Failed to fetch config from {}: HTTP {}",
                path,
                resp.status()
            );
        }
        let etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let body = resp.text().await?;
        Ok((body, etag))
    }

    /// Load config from a local path or remote URL, with optional environment overlay.
    pub async fn load_from(path: &str, env: Option<&str>) -> Result<Self> {
        if Self::is_remote(path) {
            let (content, _) = Self::fetch_remote(path).await?;
            Self::from_yaml(&content)
        } else if let Some(env) = env {
            Self::load_with_env(Path::new(path), env)
        } else {
            // Check QHOOK_ENV environment variable
            if let Ok(env) = std::env::var("QHOOK_ENV") {
                Self::load_with_env(Path::new(path), &env)
            } else {
                Self::load(Path::new(path))
            }
        }
    }

    /// Semantic validation beyond YAML parsing.
    pub fn validate(&self) -> Result<()> {
        // Validate handler URLs and types
        for (name, handler) in &self.handlers {
            match handler.handler_type.as_str() {
                "http" => {}
                other => anyhow::bail!("handler '{}' has invalid type '{}'", name, other),
            }
            validate_handler_url(name, &handler.url, self.server.allow_private_urls)?;
            validate_http_method(&format!("handler '{}'", name), &handler.method)?;

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
                "webhook" | "event" | "sns" | "outbound" => {}
                "cron" => {
                    // Cron sources require a schedule
                    let schedule = source.schedule.as_deref().unwrap_or("");
                    if schedule.is_empty() {
                        anyhow::bail!("source '{}' is type 'cron' but has no schedule", name);
                    }
                    // Validate the cron expression
                    if schedule.parse::<croner::Cron>().is_err() {
                        anyhow::bail!("source '{}' has invalid cron schedule '{}'", name, schedule);
                    }
                    // Validate timezone if specified
                    if let Some(ref tz) = source.timezone {
                        if !parse_timezone(tz) {
                            anyhow::bail!(
                                "source '{}' has invalid timezone '{}'. Use UTC (default) or a fixed offset like +09:00",
                                name,
                                tz
                            );
                        }
                    }
                }
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
                validate_http_method(
                    &format!("workflow '{}' step '{}'", name, step.name),
                    &step.method,
                )?;
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
                            validate_http_method(
                                &format!(
                                    "workflow '{}' step '{}' branch '{}'",
                                    name, step.name, b.name
                                ),
                                &b.method,
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

                // Validate sub-workflow steps
                if step.handler_type == "workflow" {
                    let wf_name = step.workflow.as_deref().unwrap_or("");
                    if wf_name.is_empty() {
                        anyhow::bail!(
                            "workflow '{}' step '{}' is type 'workflow' but has no workflow name",
                            name,
                            step.name
                        );
                    }
                    if !self.workflows.contains_key(wf_name) {
                        anyhow::bail!(
                            "workflow '{}' step '{}' references unknown workflow '{}'",
                            name,
                            step.name,
                            wf_name
                        );
                    }
                    if wf_name == name {
                        anyhow::bail!(
                            "workflow '{}' step '{}' cannot reference itself (recursive)",
                            name,
                            step.name
                        );
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
        r#"# yaml-language-server: $schema=https://totte-dev.github.io/qhook/schema.json
# qhook.yaml

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
  #   type: http
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
/// Parse a timezone string. Returns true if valid.
/// Supports "UTC" and fixed offsets like "+09:00", "-05:00".
fn parse_timezone(tz: &str) -> bool {
    if tz == "UTC" {
        return true;
    }
    // Try parsing as fixed offset: +HH:MM or -HH:MM
    tz.parse::<chrono::FixedOffset>().is_ok()
}

fn validate_http_method(context: &str, method: &str) -> Result<()> {
    match method {
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" => Ok(()),
        other => anyhow::bail!("{} has invalid HTTP method '{}'", context, other),
    }
}

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

/// Deep merge two YAML values. Overlay values override base values.
/// Mappings are merged recursively; all other types are replaced.
fn deep_merge_yaml(
    base: serde_yaml_ng::Value,
    overlay: serde_yaml_ng::Value,
) -> serde_yaml_ng::Value {
    use serde_yaml_ng::Value;
    match (base, overlay) {
        (Value::Mapping(mut base_map), Value::Mapping(overlay_map)) => {
            for (key, overlay_val) in overlay_map {
                let merged = if let Some(base_val) = base_map.remove(&key) {
                    deep_merge_yaml(base_val, overlay_val)
                } else {
                    overlay_val
                };
                base_map.insert(key, merged);
            }
            Value::Mapping(base_map)
        }
        (_, overlay) => overlay,
    }
}

/// Load a .env file: each line `KEY=VALUE` is set as an environment variable.
/// Lines starting with `#` and empty lines are skipped.
fn load_dotenv(path: &Path) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read env file: {}", path.display()))?;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"').trim_matches('\'');
            // Only set if not already set (process env takes precedence)
            if std::env::var(key).is_err() {
                // SAFETY: We are single-threaded at config load time (before tokio runtime tasks).
                unsafe { std::env::set_var(key, value) };
            }
        }
    }
    Ok(())
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
                schedule: None,
                timezone: None,
                schema: None,
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
                method: "POST".into(),
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
                schedule: None,
                timezone: None,
                schema: None,
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
                method: "POST".into(),
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
                method: "POST".into(),
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
                source_type: "invalid".into(),
                verify: None,
                secret: None,
                skip_verify: false,
                schedule: None,
                timezone: None,
                schema: None,
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
                schedule: None,
                timezone: None,
                schema: None,
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
                method: "POST".into(),
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
                schedule: None,
                timezone: None,
                schema: None,
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
                method: "POST".into(),
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
                schedule: None,
                timezone: None,
                schema: None,
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
                method: "POST".into(),
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
                schedule: None,
                timezone: None,
                schema: None,
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
                method: "POST".into(),
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
                schedule: None,
                timezone: None,
                schema: None,
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
                schedule: None,
                timezone: None,
                schema: None,
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
                schedule: None,
                timezone: None,
                schema: None,
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
                method: "POST".into(),
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

    // --- HTTP Method Tests ---

    #[test]
    fn test_handler_method_defaults_to_post() {
        let yaml = r#"
sources:
  app:
    type: event
handlers:
  my-handler:
    source: app
    url: https://example.com/hook
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(config.handlers["my-handler"].method, "POST");
    }

    #[test]
    fn test_handler_method_parsed() {
        let yaml = r#"
sources:
  app:
    type: event
handlers:
  get-handler:
    source: app
    url: https://example.com/hook
    method: GET
  put-handler:
    source: app
    url: https://example.com/hook
    method: PUT
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        assert_eq!(config.handlers["get-handler"].method, "GET");
        assert_eq!(config.handlers["put-handler"].method, "PUT");
    }

    #[test]
    fn test_handler_invalid_method_rejected() {
        let yaml = r#"
sources:
  app:
    type: event
handlers:
  bad:
    source: app
    url: https://example.com/hook
    method: TRACE
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("invalid HTTP method 'TRACE'"));
    }

    #[test]
    fn test_step_method_defaults_to_post() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  flow:
    source: app
    steps:
      - name: step1
        url: https://example.com/api
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(config.workflows["flow"].steps[0].method, "POST");
    }

    #[test]
    fn test_step_method_parsed() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  flow:
    source: app
    steps:
      - name: check
        url: https://example.com/status
        method: GET
      - name: update
        url: https://example.com/resource
        method: PATCH
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        assert_eq!(config.workflows["flow"].steps[0].method, "GET");
        assert_eq!(config.workflows["flow"].steps[1].method, "PATCH");
    }

    #[test]
    fn test_step_invalid_method_rejected() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  flow:
    source: app
    steps:
      - name: bad
        url: https://example.com
        method: OPTIONS
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("invalid HTTP method 'OPTIONS'"));
    }

    #[test]
    fn test_branch_method_parsed() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  flow:
    source: app
    steps:
      - name: fan-out
        type: parallel
        branches:
          - name: notify
            url: https://example.com/notify
            method: PUT
          - name: log
            url: https://example.com/log
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        let step = &config.workflows["flow"].steps[0];
        let branches = step.branches.as_ref().unwrap();
        assert_eq!(branches[0].method, "PUT");
        assert_eq!(branches[1].method, "POST"); // default
    }

    // --- Cron Source Tests ---

    #[test]
    fn test_parse_cron_source() {
        let yaml = r#"
sources:
  heartbeat:
    type: cron
    schedule: "*/5 * * * *"
handlers:
  on-heartbeat:
    source: heartbeat
    url: https://example.com/check
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        let source = &config.sources["heartbeat"];
        assert_eq!(source.source_type, "cron");
        assert_eq!(source.schedule.as_deref(), Some("*/5 * * * *"));
        assert!(source.timezone.is_none());
    }

    #[test]
    fn test_cron_source_with_timezone() {
        let yaml = r#"
sources:
  daily:
    type: cron
    schedule: "0 9 * * MON-FRI"
    timezone: "+09:00"
handlers:
  morning-report:
    source: daily
    url: https://example.com/report
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        let source = &config.sources["daily"];
        assert_eq!(source.timezone.as_deref(), Some("+09:00"));
    }

    #[test]
    fn test_cron_source_utc_timezone() {
        let yaml = r#"
sources:
  tick:
    type: cron
    schedule: "0 * * * *"
    timezone: UTC
handlers:
  on-tick:
    source: tick
    url: https://example.com/tick
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_cron_source_missing_schedule() {
        let yaml = r#"
sources:
  bad:
    type: cron
handlers: {}
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("no schedule"));
    }

    #[test]
    fn test_cron_source_invalid_schedule() {
        let yaml = r#"
sources:
  bad:
    type: cron
    schedule: "not a cron"
handlers: {}
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("invalid cron schedule"));
    }

    #[test]
    fn test_validate_invalid_alert_type() {
        let yaml = r#"
sources: {}
handlers: {}
alerts:
  type: email
  url: https://hooks.slack.com/xxx
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("not supported"));
    }

    #[test]
    fn test_validate_invalid_alert_url() {
        let yaml = r#"
sources: {}
handlers: {}
alerts:
  type: slack
  url: ftp://example.com/hook
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("http://"));
    }

    #[test]
    fn test_validate_invalid_database_driver() {
        let yaml = r#"
database:
  driver: mysql
sources: {}
handlers: {}
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("unsupported database driver"));
    }

    #[test]
    fn test_validate_choice_default_nonexistent_step() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  flow:
    source: app
    steps:
      - name: route
        type: choice
        choices:
          - when: "$.status == active"
            goto: step-a
        default: nonexistent
      - name: step-a
        url: https://example.com
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
    }

    // --- Sub-workflow step tests ---

    #[test]
    fn test_parse_subworkflow_step() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  notify-all:
    source: app
    steps:
      - name: slack
        url: https://hooks.slack.com/xxx
      - name: email
        url: https://email.example.com/send

  order-pipeline:
    source: app
    events: [order.created]
    steps:
      - name: fulfill
        url: https://api.example.com/fulfill
      - name: notify
        type: workflow
        workflow: notify-all
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_ok());
        let step = &config.workflows["order-pipeline"].steps[1];
        assert_eq!(step.handler_type, "workflow");
        assert_eq!(step.workflow.as_deref(), Some("notify-all"));
    }

    #[test]
    fn test_validate_subworkflow_missing_name() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  bad:
    source: app
    steps:
      - name: sub
        type: workflow
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("no workflow name"));
    }

    #[test]
    fn test_validate_subworkflow_unknown_target() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  bad:
    source: app
    steps:
      - name: sub
        type: workflow
        workflow: nonexistent
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("unknown workflow"));
    }

    #[test]
    fn test_validate_subworkflow_recursive() {
        let yaml = r#"
sources:
  app:
    type: event
handlers: {}
workflows:
  bad:
    source: app
    steps:
      - name: loop
        type: workflow
        workflow: bad
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("recursive"));
    }

    #[test]
    fn test_is_remote() {
        assert!(Config::is_remote("s3://my-bucket/config.yaml"));
        assert!(Config::is_remote("gs://my-bucket/config.yaml"));
        assert!(Config::is_remote("az://account/container/config.yaml"));
        assert!(Config::is_remote("https://example.com/config.yaml"));
        assert!(Config::is_remote("http://localhost:8080/config.yaml"));
        assert!(!Config::is_remote("qhook.yaml"));
        assert!(!Config::is_remote("/etc/qhook/config.yaml"));
    }

    #[test]
    fn test_resolve_remote_url_s3() {
        // Test S3 URL resolution (may include endpoint override from env)
        let url = Config::resolve_remote_url("s3://my-bucket/path/config.yaml").unwrap();
        assert!(url.contains("my-bucket"));
        assert!(url.contains("path/config.yaml"));
    }

    #[test]
    fn test_resolve_remote_url_gcs() {
        let url = Config::resolve_remote_url("gs://my-bucket/path/config.yaml").unwrap();
        assert_eq!(
            url,
            "https://storage.googleapis.com/my-bucket/path/config.yaml"
        );
    }

    #[test]
    fn test_resolve_remote_url_azure() {
        let url = Config::resolve_remote_url("az://myaccount/mycontainer/config.yaml").unwrap();
        assert_eq!(
            url,
            "https://myaccount.blob.core.windows.net/mycontainer/config.yaml"
        );
    }

    #[test]
    fn test_resolve_remote_url_azure_invalid() {
        let err = Config::resolve_remote_url("az://account-only").unwrap_err();
        assert!(err.to_string().contains("Invalid Azure Blob path"));
    }

    #[test]
    fn test_resolve_remote_url_https_passthrough() {
        let url = Config::resolve_remote_url("https://example.com/config.yaml").unwrap();
        assert_eq!(url, "https://example.com/config.yaml");
    }

    #[test]
    fn test_from_yaml_valid() {
        let yaml = r#"
server:
  allow_private_urls: true
sources:
  app:
    type: event
handlers:
  test:
    source: app
    url: http://localhost:3000/test
"#;
        let config = Config::from_yaml(yaml).unwrap();
        assert_eq!(config.sources.len(), 1);
        assert_eq!(config.handlers.len(), 1);
    }

    #[test]
    fn test_from_yaml_invalid_keeps_error() {
        let yaml = "not: valid: yaml: [[[";
        assert!(Config::from_yaml(yaml).is_err());
    }

    #[test]
    fn test_from_yaml_validation_error() {
        let yaml = r#"
server:
  allow_private_urls: true
sources:
  app:
    type: event
handlers:
  test:
    source: nonexistent
    url: http://localhost:3000/test
"#;
        let err = Config::from_yaml(yaml).unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
    }

    #[test]
    fn test_fetch_s3_path_parsing() {
        // Invalid S3 path (no key)
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(Config::fetch_remote("s3://bucket-only"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid S3 path"));
    }

    #[test]
    fn test_cron_source_invalid_timezone() {
        let yaml = r#"
sources:
  bad:
    type: cron
    schedule: "0 * * * *"
    timezone: "Mars/Olympus"
handlers: {}
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("invalid timezone"));
    }

    #[test]
    fn test_deep_merge_yaml_scalars() {
        use serde_yaml_ng::{Mapping, Value};
        let mut base = Mapping::new();
        base.insert(Value::String("a".into()), Value::String("1".into()));
        base.insert(Value::String("b".into()), Value::String("2".into()));

        let mut overlay = Mapping::new();
        overlay.insert(Value::String("b".into()), Value::String("99".into()));
        overlay.insert(Value::String("c".into()), Value::String("3".into()));

        let result = deep_merge_yaml(Value::Mapping(base), Value::Mapping(overlay));
        let map = result.as_mapping().unwrap();
        assert_eq!(
            map.get(Value::String("a".into())),
            Some(&Value::String("1".into()))
        );
        assert_eq!(
            map.get(Value::String("b".into())),
            Some(&Value::String("99".into()))
        );
        assert_eq!(
            map.get(Value::String("c".into())),
            Some(&Value::String("3".into()))
        );
    }

    #[test]
    fn test_deep_merge_yaml_nested() {
        let base_yaml =
            "database:\n  driver: sqlite\nserver:\n  port: 8888\n  allow_private_urls: true\n";
        let overlay_yaml = "database:\n  driver: postgres\n  url: postgres://localhost/qhook\n";

        let base: serde_yaml_ng::Value = serde_yaml_ng::from_str(base_yaml).unwrap();
        let overlay: serde_yaml_ng::Value = serde_yaml_ng::from_str(overlay_yaml).unwrap();
        let merged = deep_merge_yaml(base, overlay);

        let merged_str = serde_yaml_ng::to_string(&merged).unwrap();
        // Database should be merged (driver overridden, url added)
        assert!(merged_str.contains("postgres"));
        assert!(merged_str.contains("url:"));
        // Server should be preserved from base
        assert!(merged_str.contains("port: 8888"));
        assert!(merged_str.contains("allow_private_urls: true"));
    }

    #[test]
    fn test_deep_merge_yaml_overlay_replaces_non_mapping() {
        use serde_yaml_ng::Value;
        let base = Value::String("base".into());
        let overlay = Value::String("overlay".into());
        let result = deep_merge_yaml(base, overlay);
        assert_eq!(result, Value::String("overlay".into()));
    }

    #[test]
    fn test_load_dotenv() {
        let dir = std::env::temp_dir().join("qhook_test_dotenv");
        let _ = std::fs::create_dir_all(&dir);
        let env_file = dir.join(".env.test");
        std::fs::write(
            &env_file,
            "# comment\nTEST_QHOOK_DOT_A=hello\nTEST_QHOOK_DOT_B=\"world\"\n\nTEST_QHOOK_DOT_C='quoted'\n",
        )
        .unwrap();

        load_dotenv(&env_file).unwrap();

        assert_eq!(std::env::var("TEST_QHOOK_DOT_A").unwrap(), "hello");
        assert_eq!(std::env::var("TEST_QHOOK_DOT_B").unwrap(), "world");
        assert_eq!(std::env::var("TEST_QHOOK_DOT_C").unwrap(), "quoted");

        // Cleanup
        let _ = std::fs::remove_file(&env_file);
        let _ = std::fs::remove_dir(&dir);
    }
}
