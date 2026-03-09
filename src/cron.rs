use std::sync::Arc;
use std::time::Duration;

use chrono::{Timelike, Utc};
use tokio::sync::watch;

use crate::api::AppState;

/// Run the cron scheduler. Evaluates all cron sources and fires events on schedule.
/// Exits when the shutdown signal is received.
pub async fn run(state: Arc<AppState>, mut shutdown: watch::Receiver<bool>) {
    // Collect cron sources
    let cron_sources: Vec<(String, croner::Cron)> = state
        .config
        .sources
        .iter()
        .filter(|(_, s)| s.source_type == "cron")
        .filter_map(|(name, s)| {
            let schedule = s.schedule.as_deref()?;
            match schedule.parse::<croner::Cron>() {
                Ok(cron) => Some((name.clone(), cron)),
                Err(e) => {
                    tracing::error!(source = name, error = %e, "Failed to parse cron schedule");
                    None
                }
            }
        })
        .collect();

    if cron_sources.is_empty() {
        return;
    }

    tracing::info!(count = cron_sources.len(), "Cron scheduler started");

    // Track last fire time per source to avoid double-fires
    let mut last_fired: std::collections::HashMap<String, chrono::DateTime<Utc>> =
        std::collections::HashMap::new();

    loop {
        // Calculate next fire time across all cron sources
        let now = Utc::now();
        let mut next_fire: Option<(String, chrono::DateTime<Utc>)> = None;

        for (name, cron) in &cron_sources {
            if let Ok(next) = cron.find_next_occurrence(&now, false) {
                if next_fire.is_none() || next < next_fire.as_ref().unwrap().1 {
                    next_fire = Some((name.clone(), next));
                }
            }
        }

        let Some((source_name, fire_at)) = next_fire else {
            tracing::warn!("No next cron occurrence found, stopping scheduler");
            break;
        };

        let wait_duration = (fire_at - now)
            .to_std()
            .unwrap_or(Duration::from_millis(100));

        tracing::debug!(
            source = source_name,
            fire_at = %fire_at,
            wait_secs = wait_duration.as_secs(),
            "Next cron fire"
        );

        // Wait until fire time or shutdown
        tokio::select! {
            _ = tokio::time::sleep(wait_duration) => {}
            _ = shutdown.changed() => {
                tracing::info!("Cron scheduler received shutdown signal");
                return;
            }
        }

        // Fire all sources whose schedule matches now
        let now = Utc::now();
        for (name, cron) in &cron_sources {
            // Check if this source should fire now (within 1-second tolerance)
            if let Ok(next) = cron.find_next_occurrence(
                &last_fired
                    .get(name)
                    .copied()
                    .unwrap_or(now - chrono::Duration::seconds(1)),
                false,
            ) {
                if next <= now {
                    // Avoid double-fire within the same minute
                    let now_minute = now.with_second(0).and_then(|t| t.with_nanosecond(0));
                    if let Some(now_min) = now_minute {
                        if last_fired.get(name).is_some_and(|&last| {
                            last.with_second(0)
                                .and_then(|t| t.with_nanosecond(0))
                                .is_some_and(|lm| lm == now_min)
                        }) {
                            continue;
                        }
                    }
                    last_fired.insert(name.clone(), now);

                    if let Err(e) = fire_cron_event(&state, name).await {
                        tracing::error!(source = name, error = %e, "Failed to fire cron event");
                    }
                }
            }
        }
    }
}

/// Create an event for a cron source trigger.
async fn fire_cron_event(state: &AppState, source_name: &str) -> anyhow::Result<()> {
    let event_id = ulid::Ulid::new().to_string();
    let event_type = "cron.tick";
    let now = Utc::now();
    let payload = serde_json::json!({
        "source": source_name,
        "fired_at": now.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
    })
    .to_string();

    // Insert event
    let created = state
        .db
        .insert_event(&event_id, source_name, event_type, &payload, None, None)
        .await?;

    if !created {
        return Ok(());
    }

    state.metrics.inc_events_received_for(source_name);
    tracing::info!(source = source_name, event_id, "Cron event fired");

    // Find matching handlers
    for (handler_name, handler) in &state.config.handlers {
        if handler.source != source_name {
            continue;
        }
        if !handler.events.is_empty()
            && !handler
                .events
                .iter()
                .any(|e| crate::api::event_matches(e, event_type))
        {
            continue;
        }

        // Apply filter
        if let Some(ref filter) = handler.filter {
            if !crate::api::evaluate_filter(&payload, filter) {
                continue;
            }
        }

        let job_id = ulid::Ulid::new().to_string();
        let max_attempts = handler
            .retry
            .as_ref()
            .map(|r| r.max)
            .unwrap_or(state.config.delivery.default_retry.max);

        state
            .db
            .insert_job(&job_id, &event_id, handler_name, &handler.url, max_attempts)
            .await?;

        state.metrics.inc_jobs_created();
        tracing::info!(
            event_id,
            job_id,
            handler = handler_name,
            event_type,
            "Cron job created"
        );
    }

    // Find matching workflows
    for (wf_name, workflow) in &state.config.workflows {
        if workflow.source != source_name {
            continue;
        }
        if !workflow.events.is_empty()
            && !workflow
                .events
                .iter()
                .any(|e| crate::api::event_matches(e, event_type))
        {
            continue;
        }

        crate::api::start_workflow(state, wf_name, workflow, &event_id, &payload).await?;
    }

    Ok(())
}
