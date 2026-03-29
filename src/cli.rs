use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::config::Config;

#[derive(Parser)]
#[command(
    name = "qhook",
    about = "Lightweight event gateway with queue and retry"
)]
pub struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize a new qhook.yaml config file
    Init {
        /// Use a template: github, stripe, sns, cron
        #[arg(short = 't', long)]
        template: Option<String>,
    },
    /// Start the qhook server
    Start {
        /// Path or URL to config file (local path, s3://bucket/key, or https://...)
        #[arg(short, long, default_value = "qhook.yaml")]
        config: String,
        /// Environment name (loads qhook.{env}.yaml overlay and .env.{env})
        #[arg(short, long)]
        env: Option<String>,
    },
    /// Validate the config file
    Validate {
        /// Path to config file
        #[arg(short, long, default_value = "qhook.yaml")]
        config: PathBuf,
    },
    /// Manage jobs
    Jobs {
        #[command(subcommand)]
        action: JobsAction,
    },
    /// Manage events
    Events {
        #[command(subcommand)]
        action: EventsAction,
    },
    /// Send a test event to a running qhook server
    Send {
        /// Source name (must match a configured source)
        #[arg(short, long)]
        source: String,
        /// Event type (e.g. order.created)
        #[arg(short = 't', long = "type")]
        event_type: String,
        /// JSON payload (inline string)
        #[arg()]
        payload: Option<String>,
        /// Read payload from file
        #[arg(short, long)]
        file: Option<PathBuf>,
        /// Show which handlers/workflows would match without sending
        #[arg(long)]
        dry_run: bool,
        /// Path to config file
        #[arg(short, long, default_value = "qhook.yaml")]
        config: PathBuf,
    },
    /// Inspect an event's full lifecycle (jobs, attempts, workflow runs)
    Inspect {
        /// Event ID to inspect
        #[arg()]
        event_id: String,
        /// Path to config file
        #[arg(short, long, default_value = "qhook.yaml")]
        config: PathBuf,
    },
    /// Check server readiness and endpoint reachability
    Doctor {
        /// Path to config file
        #[arg(short, long, default_value = "qhook.yaml")]
        config: PathBuf,
    },
    /// Stream events and job results in real time
    Tail {
        /// Filter by source name
        #[arg(short, long)]
        source: Option<String>,
        /// Filter by job status (completed, dead, retryable)
        #[arg(long)]
        status: Option<String>,
        /// Path to config file
        #[arg(short, long, default_value = "qhook.yaml")]
        config: PathBuf,
    },
    /// Export events as JSONL
    Export {
        #[command(subcommand)]
        action: ExportAction,
    },
    /// Replay events from a JSONL file to a running qhook server
    #[command(name = "replay-local")]
    ReplayLocal {
        /// Path to JSONL file (output of `qhook export events`), or `-` for stdin
        #[arg()]
        file: String,
        /// Target server URL (default: http://localhost:{config port})
        #[arg(short, long)]
        target: Option<String>,
        /// API auth token (overrides config)
        #[arg(long)]
        token: Option<String>,
        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
        /// Path to config file (used for default port and auth token)
        #[arg(short, long, default_value = "qhook.yaml")]
        config: PathBuf,
        /// Filter by source name
        #[arg(long)]
        source: Option<String>,
        /// Filter by event type (exact match or prefix with *)
        #[arg(long)]
        event_type: Option<String>,
        /// Filter events created since this timestamp (ISO 8601, e.g. 2026-03-01T00:00:00)
        #[arg(long)]
        since: Option<String>,
        /// Filter events created until this timestamp (ISO 8601)
        #[arg(long)]
        until: Option<String>,
        /// Only replay events with matching status (for replaying failed deliveries)
        #[arg(long)]
        status: Option<String>,
    },
    /// Manage workflow runs
    #[command(name = "workflow-runs")]
    WorkflowRuns {
        #[command(subcommand)]
        action: WorkflowRunsAction,
    },
    /// Manage pull-mode queues
    Queues {
        #[command(subcommand)]
        action: QueuesAction,
    },
}

#[derive(Subcommand)]
enum JobsAction {
    /// List jobs
    List {
        /// Filter by status (available, running, completed, retryable, dead)
        #[arg(short, long)]
        status: Option<String>,
        /// Max number of jobs to show
        #[arg(short, long, default_value = "20")]
        limit: i32,
        /// Path to config file
        #[arg(short, long, default_value = "qhook.yaml")]
        config: PathBuf,
    },
    /// Retry failed jobs
    Retry {
        /// Job ID to retry (omit to retry all dead jobs)
        #[arg()]
        job_id: Option<String>,
        /// Path to config file
        #[arg(short, long, default_value = "qhook.yaml")]
        config: PathBuf,
    },
}

#[derive(Subcommand)]
enum EventsAction {
    /// List events
    List {
        /// Max number of events to show
        #[arg(short, long, default_value = "20")]
        limit: i32,
        /// Path to config file
        #[arg(short, long, default_value = "qhook.yaml")]
        config: PathBuf,
    },
    /// Replay events by re-creating jobs for matching handlers
    Replay {
        /// Filter by source name
        #[arg(short, long)]
        source: Option<String>,
        /// Filter by event type
        #[arg(short = 't', long)]
        event_type: Option<String>,
        /// Only events created after this timestamp (e.g. 2024-01-01T00:00:00)
        #[arg(long)]
        since: Option<String>,
        /// Only events created before this timestamp
        #[arg(long)]
        until: Option<String>,
        /// Max number of events to replay
        #[arg(short, long, default_value = "100")]
        limit: i32,
        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
        /// Path to config file
        #[arg(short, long, default_value = "qhook.yaml")]
        config: PathBuf,
    },
}

#[derive(Subcommand)]
enum ExportAction {
    /// Export events as JSONL (one JSON object per line)
    Events {
        /// Filter by source name
        #[arg(short, long)]
        source: Option<String>,
        /// Filter by event type
        #[arg(short = 't', long)]
        event_type: Option<String>,
        /// Only events created after this timestamp
        #[arg(long)]
        since: Option<String>,
        /// Only events created before this timestamp
        #[arg(long)]
        until: Option<String>,
        /// Max number of events to export
        #[arg(short, long, default_value = "1000")]
        limit: i32,
        /// Path to config file
        #[arg(short, long, default_value = "qhook.yaml")]
        config: PathBuf,
    },
}

#[derive(Subcommand)]
enum WorkflowRunsAction {
    /// List workflow runs
    List {
        /// Filter by status (running, completed, failed)
        #[arg(short, long)]
        status: Option<String>,
        /// Max number of runs to show
        #[arg(short, long, default_value = "20")]
        limit: i32,
        /// Path to config file
        #[arg(short, long, default_value = "qhook.yaml")]
        config: PathBuf,
    },
    /// Redrive a failed workflow run (restart from failed step)
    Redrive {
        /// Workflow run ID
        #[arg()]
        run_id: String,
        /// Path to config file
        #[arg(short, long, default_value = "qhook.yaml")]
        config: PathBuf,
    },
}

#[derive(Subcommand)]
enum QueuesAction {
    /// List all configured queues with job counts
    List {
        /// Path to config file
        #[arg(short, long, default_value = "qhook.yaml")]
        config: PathBuf,
    },
    /// Show detailed stats for a queue
    Inspect {
        /// Queue name
        #[arg()]
        name: String,
        /// Max number of recent messages to show
        #[arg(short, long, default_value = "10")]
        limit: i32,
        /// Path to config file
        #[arg(short, long, default_value = "qhook.yaml")]
        config: PathBuf,
    },
    /// Delete all jobs for a queue
    Drain {
        /// Queue name
        #[arg()]
        name: String,
        /// Skip confirmation prompt
        #[arg(short, long)]
        force: bool,
        /// Path to config file
        #[arg(short, long, default_value = "qhook.yaml")]
        config: PathBuf,
    },
    /// List dead-letter messages for a queue
    Dlq {
        /// Queue name
        #[arg()]
        name: String,
        /// Max number of messages to show
        #[arg(short, long, default_value = "20")]
        limit: i32,
        /// Path to config file
        #[arg(short, long, default_value = "qhook.yaml")]
        config: PathBuf,
    },
    /// Retry dead jobs for a queue
    Retry {
        /// Queue name
        #[arg()]
        name: String,
        /// Specific job ID to retry (omit to retry all dead jobs)
        #[arg(long)]
        id: Option<String>,
        /// Path to config file
        #[arg(short, long, default_value = "qhook.yaml")]
        config: PathBuf,
    },
    /// Show the next available message without consuming it
    Peek {
        /// Queue name
        #[arg()]
        name: String,
        /// Path to config file
        #[arg(short, long, default_value = "qhook.yaml")]
        config: PathBuf,
    },
}

impl Args {
    pub async fn run(self) -> Result<()> {
        match self.command {
            Command::Init { template } => {
                let path = PathBuf::from("qhook.yaml");
                if path.exists() {
                    anyhow::bail!("qhook.yaml already exists");
                }
                let content = match template.as_deref() {
                    Some("github") => include_str!("templates/github.yaml"),
                    Some("stripe") => include_str!("templates/stripe.yaml"),
                    Some("sns") => include_str!("templates/sns.yaml"),
                    Some("cron") => include_str!("templates/cron.yaml"),
                    Some(t) => anyhow::bail!(
                        "Unknown template '{}'. Available: github, stripe, sns, cron",
                        t
                    ),
                    None => Config::default_yaml(),
                };
                std::fs::write(&path, content)?;
                // Also generate qhook.local.yaml if it doesn't exist
                let local_path = PathBuf::from("qhook.local.yaml");
                if !local_path.exists() {
                    let local_content = include_str!("templates/local.yaml");
                    std::fs::write(&local_path, local_content)?;
                }
                if let Some(t) = template.as_deref() {
                    println!("Created qhook.yaml (template: {})", t);
                } else {
                    println!("Created qhook.yaml");
                }
                println!("Created qhook.local.yaml (use: qhook start --env local)");
                Ok(())
            }
            Command::Start { config, env } => {
                let cfg = Config::load_from(&config, env.as_deref()).await?;
                tracing::info!(
                    driver = cfg.database.driver,
                    port = cfg.server.port,
                    "Starting qhook"
                );

                let db = crate::db::Database::connect(&cfg.database).await?;
                db.migrate().await?;

                let state = crate::api::AppState::new(cfg, db);
                crate::api::serve(state, config).await
            }
            Command::Validate { config } => {
                let cfg = Config::load(&config).context("Validation failed")?;
                println!("Sources: {}", cfg.sources.len());
                println!("Handlers: {}", cfg.handlers.len());
                for (name, handler) in &cfg.handlers {
                    println!("  {} -> {} (source: {})", name, handler.url, handler.source);
                }
                println!("Workflows: {}", cfg.workflows.len());
                for (name, workflow) in &cfg.workflows {
                    println!(
                        "  {} -> {} steps (source: {})",
                        name,
                        workflow.steps.len(),
                        workflow.source
                    );
                }
                if cfg.alerts.is_some() {
                    println!("Alerts: configured");
                }
                // Production warnings
                if cfg.api.auth_token.is_none() {
                    eprintln!("[warn] Warning: api.auth_token not configured. The management API is open to anyone.");
                }
                if cfg.server.allow_private_urls {
                    eprintln!("[warn] Warning: allow_private_urls is enabled. Disable in production to prevent SSRF.");
                }
                for (name, queue) in &cfg.queues {
                    if queue.api_key.is_none() {
                        eprintln!("[warn] Warning: queue '{}' has no api_key. Anyone can consume messages.", name);
                    }
                }
                tracing::info!("Config is valid");
                Ok(())
            }
            Command::Send {
                source,
                event_type,
                payload,
                file,
                dry_run,
                config,
            } => {
                let cfg = Config::load(&config)?;

                // Validate source exists
                if !cfg.sources.contains_key(&source) {
                    let available: Vec<_> = cfg.sources.keys().collect();
                    anyhow::bail!(
                        "Unknown source '{}'. Available: {}",
                        source,
                        available
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }

                let body = match (payload, file) {
                    (Some(p), _) => p,
                    (None, Some(f)) => std::fs::read_to_string(&f)
                        .with_context(|| format!("Failed to read {}", f.display()))?,
                    (None, None) => "{}".to_string(),
                };

                // Validate JSON
                let payload_value: serde_json::Value =
                    serde_json::from_str(&body).context("Payload is not valid JSON")?;

                if dry_run {
                    let payload_str = payload_value.to_string();
                    // Show matching handlers
                    let matching_handlers: Vec<_> = cfg
                        .handlers
                        .iter()
                        .filter(|(_, h)| {
                            h.source == source
                                && (h.events.is_empty()
                                    || h.events
                                        .iter()
                                        .any(|e| crate::api::event_matches(e, &event_type)))
                        })
                        .filter(|(_, h)| {
                            h.filter
                                .as_ref()
                                .is_none_or(|f| crate::api::evaluate_filter(&payload_str, f))
                        })
                        .collect();

                    if matching_handlers.is_empty() {
                        println!("No matching handlers.");
                    } else {
                        println!("Matched handlers:");
                        for (name, h) in &matching_handlers {
                            let filter_info = h
                                .filter
                                .as_ref()
                                .map(|f| format!(" (filter: {})", f))
                                .unwrap_or_default();
                            println!("  {} -> {}{}", name, h.url, filter_info);
                        }
                    }

                    // Show matching workflows
                    let matching_workflows: Vec<_> = cfg
                        .workflows
                        .iter()
                        .filter(|(_, w)| {
                            w.source == source
                                && (w.events.is_empty()
                                    || w.events
                                        .iter()
                                        .any(|e| crate::api::event_matches(e, &event_type)))
                        })
                        .collect();

                    if !matching_workflows.is_empty() {
                        println!("Matched workflows:");
                        for (name, w) in &matching_workflows {
                            println!("  {} ({} steps)", name, w.steps.len());
                        }
                    }

                    if matching_handlers.is_empty() && matching_workflows.is_empty() {
                        println!("No handlers or workflows match this event.");
                    } else {
                        println!(
                            "\n{} handler(s), {} workflow(s) would be triggered.",
                            matching_handlers.len(),
                            matching_workflows.len()
                        );
                    }
                    println!("No jobs created (dry-run).");
                    return Ok(());
                }

                let port = cfg.server.port;
                let source_cfg = &cfg.sources[&source];
                let source_type = source_cfg.source_type.as_str();

                let (url, auth_header) = match source_type {
                    "event" => {
                        let url =
                            format!("http://localhost:{}/events/{}/{}", port, source, event_type);
                        let token = cfg.api.auth_token.as_deref().unwrap_or("");
                        (url, Some(format!("Bearer {}", token)))
                    }
                    "sns" => {
                        let url = format!("http://localhost:{}/sns/{}", port, source);
                        (url, None)
                    }
                    _ => {
                        let url = format!("http://localhost:{}/webhooks/{}", port, source);
                        (url, None)
                    }
                };

                let client = reqwest::Client::new();
                let mut req = client
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .header("ce-type", &event_type)
                    .body(body.clone());

                if let Some(auth) = &auth_header {
                    req = req.header("Authorization", auth);
                }

                println!("Sending {} event to {} source...", event_type, source);

                match req.send().await {
                    Ok(resp) => {
                        let status = resp.status();
                        let resp_body = resp.text().await.unwrap_or_default();
                        if status.is_success() {
                            println!(
                                "  {} {}",
                                status.as_u16(),
                                status.canonical_reason().unwrap_or("OK")
                            );
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&resp_body)
                            {
                                if let Some(id) = json.get("event_id") {
                                    println!("  Event ID: {}", id.as_str().unwrap_or("-"));
                                }
                                if let Some(jobs) = json.get("jobs_created") {
                                    println!("  Jobs created: {}", jobs);
                                }
                            }
                        } else {
                            println!(
                                "  {} {}",
                                status.as_u16(),
                                status.canonical_reason().unwrap_or("Error")
                            );
                            if !resp_body.is_empty() {
                                println!("  {}", resp_body);
                            }
                        }
                    }
                    Err(e) => {
                        anyhow::bail!(
                            "Failed to connect to qhook at localhost:{}. Is the server running?\n  {}",
                            port,
                            e
                        );
                    }
                }
                Ok(())
            }
            Command::Inspect { event_id, config } => {
                let cfg = Config::load(&config)?;
                let db = crate::db::Database::connect(&cfg.database).await?;
                db.migrate().await?;

                let event = db.get_event_by_id(&event_id).await?;
                let Some(event) = event else {
                    println!("Event {} not found.", event_id);
                    return Ok(());
                };

                println!(
                    "Event: {} ({}, source: {})",
                    event.id, event.event_type, event.source
                );
                println!("  Created: {}", event.created_at);
                if let Some(ref key) = event.unique_key {
                    println!("  Dedup key: {}", key);
                }

                // Truncate payload for display
                let payload_display = if event.payload.len() > 200 {
                    format!("{}...", &event.payload[..200])
                } else {
                    event.payload.clone()
                };
                println!("  Payload: {}", payload_display);

                // Jobs
                let jobs = db.list_jobs_by_event(&event_id).await?;
                if jobs.is_empty() {
                    println!("\nNo jobs.");
                } else {
                    println!("\nJobs:");
                    for job in &jobs {
                        println!(
                            "  {} -> {:<16} {:<10} ({}/{} attempts)",
                            &job.id[..job.id.len().min(12)],
                            job.handler,
                            job.status,
                            job.attempt,
                            job.max_attempts
                        );
                        if let Some(ref err) = job.last_error {
                            println!("    Last error: {}", err);
                        }
                        // Show attempts for non-completed jobs or dead jobs
                        let attempts = db.list_job_attempts(&job.id).await?;
                        for att in &attempts {
                            let status = att
                                .status_code
                                .map(|c| c.to_string())
                                .unwrap_or_else(|| "err".into());
                            let dur = att
                                .duration_ms
                                .map(|d| format!("{}ms", d))
                                .unwrap_or_else(|| "-".into());
                            let err = att.error.as_deref().unwrap_or("");
                            println!("    Attempt {}: {} ({}) {}", att.attempt, status, dur, err);
                        }
                    }
                }

                // Workflow runs
                let runs = db.list_workflow_runs_by_event(&event_id).await?;
                if !runs.is_empty() {
                    println!("\nWorkflows:");
                    for run in &runs {
                        let step = run.current_step.as_deref().unwrap_or("-");
                        println!(
                            "  {} -> {:<16} {:<10} (step: {})",
                            &run.id[..run.id.len().min(12)],
                            run.workflow,
                            run.status,
                            step
                        );
                    }
                }

                Ok(())
            }
            Command::Doctor { config } => {
                let cfg = Config::load(&config);
                let cfg = match cfg {
                    Ok(c) => {
                        println!("  Config valid ({})", config.display());
                        c
                    }
                    Err(e) => {
                        println!("  Config invalid: {}", e);
                        return Ok(());
                    }
                };

                // Check database connection
                match crate::db::Database::connect(&cfg.database).await {
                    Ok(_) => println!("  Database connection OK ({})", cfg.database.driver),
                    Err(e) => println!("  Database connection failed: {}", e),
                }

                // Check server is running
                let port = cfg.server.port;
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(3))
                    .build()
                    .unwrap();

                match client
                    .get(format!("http://localhost:{}/health", port))
                    .send()
                    .await
                {
                    Ok(resp) if resp.status().is_success() => {
                        println!("  Server reachable at localhost:{}", port);
                    }
                    Ok(resp) => {
                        println!(
                            "  Server responded with {} at localhost:{}",
                            resp.status(),
                            port
                        );
                    }
                    Err(_) => {
                        println!(
                            "  Server not reachable at localhost:{} (not running?)",
                            port
                        );
                    }
                }

                // Check handler endpoints
                let mut ok_count = 0;
                let mut fail_count = 0;
                for (name, handler) in &cfg.handlers {
                    match client.get(&handler.url).send().await {
                        Ok(resp) if resp.status().is_server_error() => {
                            println!(
                                "  Handler '{}' endpoint error: {} {}",
                                name,
                                handler.url,
                                resp.status()
                            );
                            fail_count += 1;
                        }
                        Ok(_) => ok_count += 1,
                        Err(_) => {
                            println!("  Handler '{}' endpoint unreachable: {}", name, handler.url);
                            fail_count += 1;
                        }
                    }
                }
                if ok_count > 0 {
                    println!("  {} handler endpoint(s) reachable", ok_count);
                }

                // Check workflow step endpoints
                let mut wf_ok = 0;
                for (wf_name, workflow) in &cfg.workflows {
                    for step in &workflow.steps {
                        let Some(url) = &step.url else { continue };
                        if url.is_empty()
                            || matches!(
                                step.handler_type.as_str(),
                                "choice" | "wait" | "callback" | "workflow"
                            )
                        {
                            continue;
                        }
                        match client.get(url).send().await {
                            Ok(resp) if resp.status().is_server_error() => {
                                println!(
                                    "  Workflow '{}' step '{}' error: {} {}",
                                    wf_name,
                                    step.name,
                                    url,
                                    resp.status()
                                );
                                fail_count += 1;
                            }
                            Ok(_) => wf_ok += 1,
                            Err(_) => {
                                println!(
                                    "  Workflow '{}' step '{}' unreachable: {}",
                                    wf_name, step.name, url
                                );
                                fail_count += 1;
                            }
                        }
                    }
                }
                if wf_ok > 0 {
                    println!("  {} workflow step endpoint(s) reachable", wf_ok);
                }

                // Security checks
                if cfg.server.allow_private_urls {
                    println!("  SSRF protection disabled (allow_private_urls: true)");
                }
                if cfg.api.auth_token.is_none() {
                    println!("  No API auth token configured");
                }

                println!();
                if fail_count == 0 {
                    println!("All checks passed.");
                } else {
                    println!("{} issue(s) found.", fail_count);
                }
                Ok(())
            }
            Command::Tail {
                source,
                status,
                config,
            } => {
                let cfg = Config::load(&config)?;
                let db = crate::db::Database::connect(&cfg.database).await?;
                db.migrate().await?;

                println!("Tailing events (Ctrl+C to stop)...\n");

                let mut last_event_id: Option<String> = None;
                let mut last_job_id: Option<String> = None;

                loop {
                    // Poll for new events
                    let events = db
                        .list_events_after(last_event_id.as_deref(), source.as_deref(), 20)
                        .await?;
                    for event in &events {
                        println!(
                            "\x1b[36m{}\x1b[0m {} \x1b[33m{}\x1b[0m (source: {})",
                            &event.created_at[..event.created_at.len().min(19)],
                            event.event_type,
                            &event.id[..event.id.len().min(12)],
                            event.source,
                        );
                        last_event_id = Some(event.id.clone());
                    }

                    // Poll for new job completions
                    let jobs = db
                        .list_jobs_after(last_job_id.as_deref(), status.as_deref(), 20)
                        .await?;
                    for job in &jobs {
                        let status_color = match job.status.as_str() {
                            "completed" => "\x1b[32m", // green
                            "dead" => "\x1b[31m",      // red
                            "retryable" => "\x1b[33m", // yellow
                            _ => "\x1b[37m",           // white
                        };
                        let error_info = job
                            .last_error
                            .as_ref()
                            .map(|e| format!(" — {}", e))
                            .unwrap_or_default();
                        println!(
                            "  {}{:<10}\x1b[0m {} → {} ({}/{}){error_info}",
                            status_color,
                            job.status,
                            &job.id[..job.id.len().min(12)],
                            &job.handler[..job.handler.len().min(20)],
                            job.attempt,
                            job.max_attempts,
                        );
                        last_job_id = Some(job.id.clone());
                    }

                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
            Command::Export { action } => match action {
                ExportAction::Events {
                    source,
                    event_type,
                    since,
                    until,
                    limit,
                    config,
                } => {
                    let cfg = Config::load(&config)?;
                    let db = crate::db::Database::connect(&cfg.database).await?;

                    let events = db
                        .list_events_filtered(
                            source.as_deref(),
                            event_type.as_deref(),
                            since.as_deref(),
                            until.as_deref(),
                            limit,
                        )
                        .await?;

                    for event in &events {
                        let payload: serde_json::Value =
                            serde_json::from_str(&event.payload).unwrap_or_default();
                        let line = serde_json::json!({
                            "id": event.id,
                            "source": event.source,
                            "event_type": event.event_type,
                            "payload": payload,
                            "created_at": event.created_at,
                            "unique_key": event.unique_key,
                        });
                        println!("{}", line);
                    }

                    eprintln!("Exported {} event(s).", events.len());
                    Ok(())
                }
            },
            Command::Jobs { action } => match action {
                JobsAction::List {
                    status,
                    limit,
                    config,
                } => {
                    let cfg = Config::load(&config)?;
                    let db = crate::db::Database::connect(&cfg.database).await?;
                    let jobs = db.list_jobs(status.as_deref(), limit).await?;

                    if jobs.is_empty() {
                        println!("No jobs found.");
                        return Ok(());
                    }

                    println!(
                        "{:<28} {:<12} {:<16} {:<10} {:<8}",
                        "ID", "STATUS", "HANDLER", "ATTEMPT", "SCHEDULED"
                    );
                    println!("{}", "-".repeat(74));
                    for job in &jobs {
                        println!(
                            "{:<28} {:<12} {:<16} {}/{:<5} {}",
                            &job.id[..job.id.len().min(26)],
                            job.status,
                            &job.handler[..job.handler.len().min(14)],
                            job.attempt,
                            job.max_attempts,
                            &job.scheduled_at[..job.scheduled_at.len().min(19)],
                        );
                    }
                    if jobs.iter().any(|j| j.last_error.is_some()) {
                        println!();
                        for job in &jobs {
                            if let Some(err) = &job.last_error {
                                println!("  {} error: {}", &job.id[..job.id.len().min(12)], err);
                            }
                        }
                    }
                    Ok(())
                }
                JobsAction::Retry { job_id, config } => {
                    let cfg = Config::load(&config)?;
                    let db = crate::db::Database::connect(&cfg.database).await?;

                    if let Some(id) = job_id {
                        if db.retry_job(&id).await? {
                            println!("Job {} queued for retry.", id);
                        } else {
                            println!("Job {} not found or not in retryable/dead state.", id);
                        }
                    } else {
                        let count = db.retry_dead_jobs().await?;
                        println!("{} dead job(s) queued for retry.", count);
                    }
                    Ok(())
                }
            },
            Command::WorkflowRuns { action } => match action {
                WorkflowRunsAction::List {
                    status,
                    limit,
                    config,
                } => {
                    let cfg = Config::load(&config)?;
                    let db = crate::db::Database::connect(&cfg.database).await?;
                    db.migrate().await?;
                    let runs = db.list_workflow_runs(status.as_deref(), limit).await?;

                    if runs.is_empty() {
                        println!("No workflow runs found.");
                        return Ok(());
                    }

                    println!(
                        "{:<28} {:<12} {:<16} {:<16} {:<20}",
                        "ID", "STATUS", "WORKFLOW", "STEP", "CREATED"
                    );
                    println!("{}", "-".repeat(92));
                    for run in &runs {
                        let step = run.current_step.as_deref().unwrap_or("-");
                        println!(
                            "{:<28} {:<12} {:<16} {:<16} {}",
                            &run.id[..run.id.len().min(26)],
                            run.status,
                            &run.workflow[..run.workflow.len().min(14)],
                            &step[..step.len().min(14)],
                            &run.created_at[..run.created_at.len().min(19)],
                        );
                    }
                    Ok(())
                }
                WorkflowRunsAction::Redrive { run_id, config } => {
                    let cfg = Config::load(&config)?;
                    let db = crate::db::Database::connect(&cfg.database).await?;
                    db.migrate().await?;

                    if db.redrive_workflow_run(&run_id).await? {
                        // Find the failed job for this run and retry it
                        let jobs = db.list_jobs(Some("dead"), 100).await?;
                        let mut redriven = 0;
                        for job in &jobs {
                            // Check if this job belongs to the workflow run
                            if let Ok(Some(wf_data)) = db.get_workflow_job_data(&job.id).await {
                                if wf_data.workflow_run_id.as_deref() == Some(&run_id) {
                                    db.retry_job(&job.id).await?;
                                    redriven += 1;
                                }
                            }
                        }
                        println!(
                            "Workflow run {} redriven ({} job(s) retried).",
                            run_id, redriven
                        );
                    } else {
                        println!("Workflow run {} not found or not in failed state.", run_id);
                    }
                    Ok(())
                }
            },
            Command::ReplayLocal {
                file,
                target,
                token,
                yes,
                config,
                source: source_filter,
                event_type: event_type_filter,
                since,
                until,
                status: status_filter,
            } => {
                let cfg = Config::load(&config)?;
                let base_url =
                    target.unwrap_or_else(|| format!("http://localhost:{}", cfg.server.port));
                let auth_token = token.or_else(|| cfg.api.auth_token.clone());

                // Read JSONL lines
                let lines: Vec<String> = if file == "-" {
                    use std::io::BufRead;
                    std::io::stdin().lock().lines().collect::<Result<_, _>>()?
                } else {
                    let content = std::fs::read_to_string(&file)
                        .with_context(|| format!("Failed to read {}", file))?;
                    content
                        .lines()
                        .filter(|l| !l.trim().is_empty())
                        .map(String::from)
                        .collect()
                };

                if lines.is_empty() {
                    println!("No events found in input.");
                    return Ok(());
                }

                // Parse and validate all lines first
                let mut all_events: Vec<serde_json::Value> = Vec::with_capacity(lines.len());
                for (i, line) in lines.iter().enumerate() {
                    let val: serde_json::Value = serde_json::from_str(line)
                        .with_context(|| format!("Invalid JSON on line {}", i + 1))?;
                    if val.get("source").and_then(|v| v.as_str()).is_none() {
                        anyhow::bail!("Line {} missing 'source' field", i + 1);
                    }
                    if val.get("event_type").and_then(|v| v.as_str()).is_none() {
                        anyhow::bail!("Line {} missing 'event_type' field", i + 1);
                    }
                    all_events.push(val);
                }

                let total_count = all_events.len();

                // Apply filters
                let has_filters = source_filter.is_some()
                    || event_type_filter.is_some()
                    || since.is_some()
                    || until.is_some()
                    || status_filter.is_some();

                let events: Vec<serde_json::Value> = all_events
                    .into_iter()
                    .filter(|event| {
                        if let Some(ref src) = source_filter {
                            if event["source"].as_str() != Some(src.as_str()) {
                                return false;
                            }
                        }
                        if let Some(ref et) = event_type_filter {
                            let event_et = event["event_type"].as_str().unwrap_or("");
                            if et.ends_with('*') {
                                let prefix = &et[..et.len() - 1];
                                if !event_et.starts_with(prefix) {
                                    return false;
                                }
                            } else if event_et != et {
                                return false;
                            }
                        }
                        if let Some(ref s) = since {
                            if let Some(created) = event["created_at"].as_str() {
                                if created < s.as_str() {
                                    return false;
                                }
                            }
                        }
                        if let Some(ref u) = until {
                            if let Some(created) = event["created_at"].as_str() {
                                if created > u.as_str() {
                                    return false;
                                }
                            }
                        }
                        if let Some(ref st) = status_filter {
                            if let Some(event_status) = event["status"].as_str() {
                                if event_status != st.as_str() {
                                    return false;
                                }
                            }
                            // If event has no status field, skip the filter (include it)
                        }
                        true
                    })
                    .collect();

                if has_filters {
                    // Build filter description
                    let mut filters = Vec::new();
                    if let Some(ref src) = source_filter {
                        filters.push(format!("source={}", src));
                    }
                    if let Some(ref et) = event_type_filter {
                        filters.push(format!("event_type={}", et));
                    }
                    if let Some(ref s) = since {
                        filters.push(format!("since={}", s));
                    }
                    if let Some(ref u) = until {
                        filters.push(format!("until={}", u));
                    }
                    if let Some(ref st) = status_filter {
                        filters.push(format!("status={}", st));
                    }
                    println!(
                        "Replaying {} of {} events (filtered by {})",
                        events.len(),
                        total_count,
                        filters.join(", ")
                    );
                } else {
                    println!(
                        "Loaded {} event(s) from {}.",
                        events.len(),
                        if file == "-" { "stdin" } else { &file }
                    );
                }
                println!("Target: {}", base_url);

                if events.is_empty() {
                    println!("No events match the given filters.");
                    return Ok(());
                }

                if !yes {
                    print!("Proceed? [y/N] ");
                    use std::io::Write;
                    std::io::stdout().flush()?;
                    let mut input = String::new();
                    std::io::stdin().read_line(&mut input)?;
                    if !input.trim().eq_ignore_ascii_case("y") {
                        println!("Aborted.");
                        return Ok(());
                    }
                }

                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(10))
                    .build()?;

                let mut ok = 0u32;
                let mut fail = 0u32;
                for event in &events {
                    let source = event["source"].as_str().unwrap();
                    let event_type = event["event_type"].as_str().unwrap();
                    let payload = event
                        .get("payload")
                        .cloned()
                        .unwrap_or(serde_json::Value::Object(Default::default()));

                    let url = format!("{}/events/{}/{}", base_url, source, event_type);
                    let mut req = client
                        .post(&url)
                        .header("Content-Type", "application/json")
                        .json(&payload);

                    if let Some(ref token) = auth_token {
                        req = req.header("Authorization", format!("Bearer {}", token));
                    }

                    match req.send().await {
                        Ok(resp) if resp.status().is_success() => ok += 1,
                        Ok(resp) => {
                            let status = resp.status();
                            let body = resp.text().await.unwrap_or_default();
                            eprintln!("  FAIL {}/{}: {} {}", source, event_type, status, body);
                            fail += 1;
                        }
                        Err(e) => {
                            eprintln!("  ERROR {}/{}: {}", source, event_type, e);
                            fail += 1;
                        }
                    }
                }

                println!(
                    "Replayed {} event(s): {} ok, {} failed.",
                    events.len(),
                    ok,
                    fail
                );
                Ok(())
            }
            Command::Queues { action } => match action {
                QueuesAction::List { config } => {
                    let cfg = Config::load(&config)?;
                    let db = crate::db::Database::connect(&cfg.database).await?;
                    db.migrate().await?;

                    if cfg.queues.is_empty() {
                        println!("No queues configured.");
                        return Ok(());
                    }

                    println!(
                        "{:<20} {:<10} {:<10} {:<12} {:<10}",
                        "NAME", "SOURCE", "PENDING", "PROCESSING", "DEAD"
                    );
                    println!("{}", "-".repeat(62));

                    for (name, queue_cfg) in &cfg.queues {
                        let handler = format!("queue/{}", name);
                        let counts = db.count_jobs_by_handler_status(&handler).await?;
                        let mut pending: i64 = 0;
                        let mut processing: i64 = 0;
                        let mut dead: i64 = 0;
                        for (status, cnt) in &counts {
                            match status.as_str() {
                                "available" | "retryable" => pending += cnt,
                                "running" => processing += cnt,
                                "dead" => dead += cnt,
                                _ => {}
                            }
                        }
                        println!(
                            "{:<20} {:<10} {:<10} {:<12} {:<10}",
                            name,
                            &queue_cfg.source[..queue_cfg.source.len().min(8)],
                            pending,
                            processing,
                            dead,
                        );
                    }
                    Ok(())
                }
                QueuesAction::Inspect {
                    name,
                    limit,
                    config,
                } => {
                    let cfg = Config::load(&config)?;
                    if !cfg.queues.contains_key(&name) {
                        let available: Vec<_> = cfg.queues.keys().collect();
                        anyhow::bail!(
                            "Unknown queue '{}'. Available: {}",
                            name,
                            available
                                .iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                    }

                    let db = crate::db::Database::connect(&cfg.database).await?;
                    db.migrate().await?;

                    let handler = format!("queue/{}", name);
                    let queue_cfg = &cfg.queues[&name];

                    println!("Queue: {}", name);
                    println!("  Source: {}", queue_cfg.source);
                    if !queue_cfg.events.is_empty() {
                        println!("  Events: {}", queue_cfg.events.join(", "));
                    }
                    println!("  Visibility timeout: {}", queue_cfg.visibility_timeout);
                    if let Some(max) = queue_cfg.max_attempts {
                        println!("  Max attempts: {}", max);
                    }

                    let counts = db.count_jobs_by_handler_status(&handler).await?;
                    let mut pending: i64 = 0;
                    let mut processing: i64 = 0;
                    let mut dead: i64 = 0;
                    let mut completed: i64 = 0;
                    for (status, cnt) in &counts {
                        match status.as_str() {
                            "available" | "retryable" => pending += cnt,
                            "running" => processing += cnt,
                            "dead" => dead += cnt,
                            "completed" => completed += cnt,
                            _ => {}
                        }
                    }
                    println!("\n  Pending: {}", pending);
                    println!("  Processing: {}", processing);
                    println!("  Dead: {}", dead);
                    println!("  Completed: {}", completed);

                    // Show recent messages
                    let jobs = db.list_jobs_by_handler(&handler, None, limit).await?;
                    if !jobs.is_empty() {
                        println!("\nRecent messages:");
                        println!(
                            "  {:<28} {:<12} {:<8} {:<20}",
                            "ID", "STATUS", "ATTEMPT", "SCHEDULED"
                        );
                        println!("  {}", "-".repeat(68));
                        for job in &jobs {
                            println!(
                                "  {:<28} {:<12} {}/{:<5} {}",
                                &job.id[..job.id.len().min(26)],
                                job.status,
                                job.attempt,
                                job.max_attempts,
                                &job.scheduled_at[..job.scheduled_at.len().min(19)],
                            );
                        }
                    }
                    Ok(())
                }
                QueuesAction::Drain {
                    name,
                    force,
                    config,
                } => {
                    let cfg = Config::load(&config)?;
                    if !cfg.queues.contains_key(&name) {
                        anyhow::bail!("Unknown queue '{}'.", name);
                    }

                    let db = crate::db::Database::connect(&cfg.database).await?;
                    db.migrate().await?;

                    let handler = format!("queue/{}", name);

                    if !force {
                        let counts = db.count_jobs_by_handler_status(&handler).await?;
                        let total: i64 = counts.iter().map(|(_, c)| c).sum();
                        if total == 0 {
                            println!("Queue '{}' is already empty.", name);
                            return Ok(());
                        }
                        print!("Delete all {} job(s) from queue '{}'? [y/N] ", total, name);
                        use std::io::Write;
                        std::io::stdout().flush()?;
                        let mut input = String::new();
                        std::io::stdin().read_line(&mut input)?;
                        if !input.trim().eq_ignore_ascii_case("y") {
                            println!("Aborted.");
                            return Ok(());
                        }
                    }

                    let deleted = db.delete_jobs_by_handler(&handler).await?;
                    println!("Deleted {} job(s) from queue '{}'.", deleted, name);
                    Ok(())
                }
                QueuesAction::Dlq { name, limit, config } => {
                    let cfg = Config::load(&config)?;
                    if !cfg.queues.contains_key(&name) {
                        anyhow::bail!("Unknown queue '{}'.", name);
                    }

                    let db = crate::db::Database::connect(&cfg.database).await?;
                    db.migrate().await?;

                    let handler = format!("queue/{}", name);
                    let jobs = db.list_jobs_by_handler(&handler, Some("dead"), limit).await?;

                    if jobs.is_empty() {
                        println!("No dead-letter messages in queue '{}'.", name);
                        return Ok(());
                    }

                    println!(
                        "{:<28} {:<8} {:<20} {}",
                        "ID", "ATTEMPT", "SCHEDULED", "ERROR"
                    );
                    println!("{}", "-".repeat(80));
                    for job in &jobs {
                        let err = job.last_error.as_deref().unwrap_or("-");
                        println!(
                            "{:<28} {}/{:<5} {:<20} {}",
                            &job.id[..job.id.len().min(26)],
                            job.attempt,
                            job.max_attempts,
                            &job.scheduled_at[..job.scheduled_at.len().min(19)],
                            err,
                        );
                    }
                    println!("\n{} dead-letter message(s).", jobs.len());
                    Ok(())
                }
                QueuesAction::Retry { name, id, config } => {
                    let cfg = Config::load(&config)?;
                    if !cfg.queues.contains_key(&name) {
                        anyhow::bail!("Unknown queue '{}'.", name);
                    }

                    let db = crate::db::Database::connect(&cfg.database).await?;
                    db.migrate().await?;

                    let handler = format!("queue/{}", name);

                    if let Some(job_id) = id {
                        // Verify the job belongs to this queue by checking handler directly
                        let row: Option<(String,)> = sqlx::query_as(
                            "SELECT handler FROM jobs WHERE id = $1 AND handler = $2 AND status = 'dead'",
                        )
                        .bind(&job_id)
                        .bind(&handler)
                        .fetch_optional(db.sqlx_pool())
                        .await?;
                        if row.is_none() {
                            anyhow::bail!(
                                "Job '{}' not found in queue '{}' dead-letter.",
                                job_id,
                                name
                            );
                        }
                        if db.retry_job(&job_id).await? {
                            println!("Job {} queued for retry.", job_id);
                        } else {
                            println!("Job {} not in retryable/dead state.", job_id);
                        }
                    } else {
                        let count = db.retry_dead_jobs_by_handler(&handler).await?;
                        println!(
                            "{} dead job(s) in queue '{}' queued for retry.",
                            count, name
                        );
                    }
                    Ok(())
                }
                QueuesAction::Peek { name, config } => {
                    let cfg = Config::load(&config)?;
                    if !cfg.queues.contains_key(&name) {
                        anyhow::bail!("Unknown queue '{}'.", name);
                    }

                    let db = crate::db::Database::connect(&cfg.database).await?;
                    db.migrate().await?;

                    let handler = format!("queue/{}", name);

                    match db.peek_queue_job(&handler).await? {
                        Some(job) => {
                            println!("Next message in queue '{}':", name);
                            println!("  ID: {}", job.id);
                            println!("  Event: {}", job.event_id);
                            println!("  Status: {}", job.status);
                            println!("  Attempt: {}/{}", job.attempt, job.max_attempts);
                            println!("  Scheduled: {}", job.scheduled_at);

                            // Fetch event payload for display
                            if let Ok(Some(event)) = db.get_event_by_id(&job.event_id).await {
                                let payload_display = if event.payload.len() > 200 {
                                    format!("{}...", &event.payload[..200])
                                } else {
                                    event.payload.clone()
                                };
                                println!("  Type: {}", event.event_type);
                                println!("  Payload: {}", payload_display);
                            }
                        }
                        None => {
                            println!("Queue '{}' is empty.", name);
                        }
                    }
                    Ok(())
                }
            },
            Command::Events { action } => match action {
                EventsAction::List { limit, config } => {
                    let cfg = Config::load(&config)?;
                    let db = crate::db::Database::connect(&cfg.database).await?;
                    let events = db.list_events(limit).await?;

                    if events.is_empty() {
                        println!("No events found.");
                        return Ok(());
                    }

                    println!(
                        "{:<28} {:<10} {:<20} {:<16} {:<20}",
                        "ID", "SOURCE", "TYPE", "KEY", "CREATED"
                    );
                    println!("{}", "-".repeat(94));
                    for event in &events {
                        let key = event.unique_key.as_deref().unwrap_or("-");
                        println!(
                            "{:<28} {:<10} {:<20} {:<16} {}",
                            &event.id[..event.id.len().min(26)],
                            &event.source[..event.source.len().min(8)],
                            &event.event_type[..event.event_type.len().min(18)],
                            &key[..key.len().min(14)],
                            &event.created_at[..event.created_at.len().min(19)],
                        );
                    }
                    Ok(())
                }
                EventsAction::Replay {
                    source,
                    event_type,
                    since,
                    until,
                    limit,
                    yes,
                    config,
                } => {
                    let cfg = Config::load(&config)?;
                    let db = crate::db::Database::connect(&cfg.database).await?;
                    db.migrate().await?;

                    let events = db
                        .list_events_filtered(
                            source.as_deref(),
                            event_type.as_deref(),
                            since.as_deref(),
                            until.as_deref(),
                            limit,
                        )
                        .await?;

                    if events.is_empty() {
                        println!("No matching events found.");
                        return Ok(());
                    }

                    // Count how many jobs would be created
                    let mut total_jobs = 0;
                    for event in &events {
                        let handler_count = cfg
                            .handlers
                            .iter()
                            .filter(|(_, h)| {
                                h.source == event.source
                                    && (h.events.is_empty()
                                        || h.events.iter().any(|e| {
                                            crate::api::event_matches(e, &event.event_type)
                                        }))
                            })
                            .filter(|(_, h)| {
                                h.filter
                                    .as_ref()
                                    .is_none_or(|f| crate::api::evaluate_filter(&event.payload, f))
                            })
                            .count();
                        total_jobs += handler_count;
                    }

                    println!(
                        "Found {} event(s), will create {} job(s).",
                        events.len(),
                        total_jobs
                    );

                    if !yes {
                        print!("Proceed? [y/N] ");
                        use std::io::Write;
                        std::io::stdout().flush()?;
                        let mut input = String::new();
                        std::io::stdin().read_line(&mut input)?;
                        if !input.trim().eq_ignore_ascii_case("y") {
                            println!("Aborted.");
                            return Ok(());
                        }
                    }

                    let mut created = 0;
                    for event in &events {
                        let matching_handlers: Vec<_> = cfg
                            .handlers
                            .iter()
                            .filter(|(_, h)| {
                                h.source == event.source
                                    && (h.events.is_empty()
                                        || h.events.iter().any(|e| {
                                            crate::api::event_matches(e, &event.event_type)
                                        }))
                            })
                            .filter(|(_, h)| {
                                h.filter
                                    .as_ref()
                                    .is_none_or(|f| crate::api::evaluate_filter(&event.payload, f))
                            })
                            .collect();

                        for (handler_name, handler) in &matching_handlers {
                            let job_id = ulid::Ulid::new().to_string();
                            let max_attempts = handler
                                .retry
                                .as_ref()
                                .map(|r| r.max)
                                .unwrap_or(cfg.delivery.default_retry.max);

                            db.insert_job(
                                &job_id,
                                &event.id,
                                handler_name,
                                &handler.url,
                                max_attempts,
                            )
                            .await?;
                            created += 1;
                        }
                    }

                    println!(
                        "Replayed {} event(s), created {} job(s).",
                        events.len(),
                        created
                    );
                    Ok(())
                }
            },
        }
    }
}
