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
    Init,
    /// Start the qhook server
    Start {
        /// Path to config file
        #[arg(short, long, default_value = "qhook.yaml")]
        config: PathBuf,
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
    /// Manage workflow runs
    #[command(name = "workflow-runs")]
    WorkflowRuns {
        #[command(subcommand)]
        action: WorkflowRunsAction,
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

impl Args {
    pub async fn run(self) -> Result<()> {
        match self.command {
            Command::Init => {
                let path = PathBuf::from("qhook.yaml");
                if path.exists() {
                    anyhow::bail!("qhook.yaml already exists");
                }
                std::fs::write(&path, Config::default_yaml())?;
                tracing::info!("Created qhook.yaml");
                Ok(())
            }
            Command::Start { config } => {
                let cfg = Config::load(&config)?;
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
                tracing::info!("Config is valid");
                Ok(())
            }
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
                                h.filter.as_ref().is_none_or(|f| {
                                    crate::api::evaluate_filter(&event.payload, f)
                                })
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
                                h.filter.as_ref().is_none_or(|f| {
                                    crate::api::evaluate_filter(&event.payload, f)
                                })
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
