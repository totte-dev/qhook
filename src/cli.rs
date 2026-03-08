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
            },
        }
    }
}
