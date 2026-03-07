use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use tokio::time::interval;

use crate::db::Database;

pub struct Worker {
    db: Arc<Database>,
    http: reqwest::Client,
    poll_interval: Duration,
}

impl Worker {
    pub fn new(db: Arc<Database>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            db,
            http,
            poll_interval: Duration::from_secs(1),
        }
    }

    pub async fn run(&self) {
        tracing::info!("Queue worker started");
        let mut ticker = interval(self.poll_interval);

        loop {
            ticker.tick().await;

            match self.poll_and_deliver().await {
                Ok(processed) => {
                    if processed > 0 {
                        tracing::info!(count = processed, "Processed jobs");
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Worker error");
                }
            }
        }
    }

    async fn poll_and_deliver(&self) -> Result<usize> {
        let jobs = self.db.fetch_available_jobs(10).await?;
        let mut processed = 0;

        for job in jobs {
            if !self.db.mark_job_running(&job.id).await? {
                continue; // Another worker grabbed it
            }

            let start = std::time::Instant::now();
            let result = self.deliver(&job).await;
            let duration_ms = start.elapsed().as_millis() as i64;

            match result {
                Ok(status_code) => {
                    let attempt_id = ulid::Ulid::new().to_string();
                    self.db
                        .insert_attempt(
                            &attempt_id,
                            &job.id,
                            job.attempt + 1,
                            Some(status_code as i32),
                            None,
                            None,
                            duration_ms,
                        )
                        .await?;

                    if (200..300).contains(&status_code) {
                        self.db.mark_job_completed(&job.id).await?;
                        tracing::info!(
                            job_id = job.id,
                            handler = job.handler,
                            status = status_code,
                            "Job completed"
                        );
                    } else {
                        let error = format!("HTTP {status_code}");
                        self.handle_failure(&job, &error).await?;
                    }
                }
                Err(e) => {
                    let error = e.to_string();
                    let attempt_id = ulid::Ulid::new().to_string();
                    self.db
                        .insert_attempt(
                            &attempt_id,
                            &job.id,
                            job.attempt + 1,
                            None,
                            None,
                            Some(&error),
                            duration_ms,
                        )
                        .await?;
                    self.handle_failure(&job, &error).await?;
                }
            }

            processed += 1;
        }

        Ok(processed)
    }

    async fn deliver(&self, job: &crate::db::JobRow) -> Result<u16> {
        let payload = self.db.get_event_payload(&job.event_id).await?;

        let response = self
            .http
            .post(&job.url)
            .header("Content-Type", "application/json")
            .header("X-Qhook-Job-ID", &job.id)
            .header("X-Qhook-Event-ID", &job.event_id)
            .header("X-Qhook-Handler", &job.handler)
            .header(
                "X-Qhook-Attempt",
                (job.attempt + 1).to_string(),
            )
            .body(payload)
            .send()
            .await?;

        Ok(response.status().as_u16())
    }

    async fn handle_failure(&self, job: &crate::db::JobRow, error: &str) -> Result<()> {
        let current_attempt = job.attempt + 1;

        if current_attempt >= job.max_attempts {
            self.db.mark_job_dead(&job.id, error).await?;
            tracing::warn!(
                job_id = job.id,
                handler = job.handler,
                attempts = current_attempt,
                "Job moved to DLQ"
            );
        } else {
            // Exponential backoff: 30s * 2^attempt
            let backoff_secs = 30i64 * (1i64 << current_attempt.min(10));
            let next_at = Utc::now().naive_utc() + chrono::Duration::seconds(backoff_secs);
            self.db
                .mark_job_retryable(&job.id, next_at, error)
                .await?;
            tracing::info!(
                job_id = job.id,
                handler = job.handler,
                attempt = current_attempt,
                next_retry_secs = backoff_secs,
                error,
                "Job scheduled for retry"
            );
        }

        Ok(())
    }
}
