use std::collections::HashMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// Thread-safe labeled counter. Each unique label key gets its own AtomicU64.
struct LabeledCounter {
    counters: RwLock<HashMap<String, AtomicU64>>,
}

impl LabeledCounter {
    fn new() -> Self {
        Self {
            counters: RwLock::new(HashMap::new()),
        }
    }

    fn inc(&self, label: &str) {
        // Fast path: read lock
        {
            let map = self.counters.read().unwrap_or_else(|e| e.into_inner());
            if let Some(counter) = map.get(label) {
                counter.fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
        // Slow path: write lock to insert
        let mut map = self.counters.write().unwrap_or_else(|e| e.into_inner());
        map.entry(label.to_string())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> Vec<(String, u64)> {
        let map = self.counters.read().unwrap_or_else(|e| e.into_inner());
        let mut entries: Vec<_> = map
            .iter()
            .map(|(k, v)| (k.clone(), v.load(Ordering::Relaxed)))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }

    fn total(&self) -> u64 {
        let map = self.counters.read().unwrap_or_else(|e| e.into_inner());
        map.values().map(|v| v.load(Ordering::Relaxed)).sum()
    }

    fn label_count(&self) -> usize {
        let map = self.counters.read().unwrap_or_else(|e| e.into_inner());
        map.len()
    }
}

/// Lightweight application metrics using atomic counters.
/// No external dependencies — formats Prometheus text exposition directly.
pub struct Metrics {
    // Global counters (kept for backward compat)
    pub events_received: AtomicU64,
    pub events_duplicated: AtomicU64,
    pub jobs_created: AtomicU64,
    pub deliveries_success: AtomicU64,
    pub deliveries_failure: AtomicU64,
    pub delivery_duration_ms_sum: AtomicU64,

    // Labeled counters
    events_by_source: LabeledCounter,
    verification_failures: LabeledCounter,
    deliveries_by_handler_ok: LabeledCounter,
    deliveries_by_handler_fail: LabeledCounter,
    dlq_by_handler: LabeledCounter,
    delivery_errors_by_type: LabeledCounter,
    alerts_sent: AtomicU64,
    alerts_failed: AtomicU64,
    db_errors: AtomicU64,
    /// Track max delivery duration (ms) for lightweight latency insight.
    delivery_duration_ms_max: AtomicU64,
    // Workflow metrics
    workflow_runs_started: LabeledCounter,
    workflow_runs_completed: LabeledCounter,
    workflow_runs_failed: LabeledCounter,
    workflow_steps_completed: LabeledCounter,
    callbacks_received: AtomicU64,
    callbacks_expired: AtomicU64,
    // Circuit breaker metrics
    circuit_opened: LabeledCounter,
    circuit_rejected: LabeledCounter,
    // Queue (pull-mode) metrics
    queue_messages_delivered: LabeledCounter,
    queue_messages_acked: LabeledCounter,
    queue_messages_nacked: LabeledCounter,
    queue_messages_expired: LabeledCounter,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            events_received: AtomicU64::new(0),
            events_duplicated: AtomicU64::new(0),
            jobs_created: AtomicU64::new(0),
            deliveries_success: AtomicU64::new(0),
            deliveries_failure: AtomicU64::new(0),
            delivery_duration_ms_sum: AtomicU64::new(0),
            events_by_source: LabeledCounter::new(),
            verification_failures: LabeledCounter::new(),
            deliveries_by_handler_ok: LabeledCounter::new(),
            deliveries_by_handler_fail: LabeledCounter::new(),
            dlq_by_handler: LabeledCounter::new(),
            delivery_errors_by_type: LabeledCounter::new(),
            alerts_sent: AtomicU64::new(0),
            alerts_failed: AtomicU64::new(0),
            db_errors: AtomicU64::new(0),
            delivery_duration_ms_max: AtomicU64::new(0),
            workflow_runs_started: LabeledCounter::new(),
            workflow_runs_completed: LabeledCounter::new(),
            workflow_runs_failed: LabeledCounter::new(),
            workflow_steps_completed: LabeledCounter::new(),
            callbacks_received: AtomicU64::new(0),
            callbacks_expired: AtomicU64::new(0),
            circuit_opened: LabeledCounter::new(),
            circuit_rejected: LabeledCounter::new(),
            queue_messages_delivered: LabeledCounter::new(),
            queue_messages_acked: LabeledCounter::new(),
            queue_messages_nacked: LabeledCounter::new(),
            queue_messages_expired: LabeledCounter::new(),
        }
    }

    pub fn inc_events_received(&self) {
        self.events_received.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_events_received_for(&self, source: &str) {
        self.events_received.fetch_add(1, Ordering::Relaxed);
        self.events_by_source.inc(source);
    }

    pub fn inc_events_duplicated(&self) {
        self.events_duplicated.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_jobs_created(&self) {
        self.jobs_created.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_delivery_success(&self, duration_ms: u64) {
        self.deliveries_success.fetch_add(1, Ordering::Relaxed);
        self.delivery_duration_ms_sum
            .fetch_add(duration_ms, Ordering::Relaxed);
    }

    pub fn inc_delivery_success_for(&self, handler: &str, duration_ms: u64) {
        self.deliveries_success.fetch_add(1, Ordering::Relaxed);
        self.delivery_duration_ms_sum
            .fetch_add(duration_ms, Ordering::Relaxed);
        self.update_max_duration(duration_ms);
        self.deliveries_by_handler_ok.inc(handler);
    }

    pub fn inc_delivery_failure(&self, duration_ms: u64) {
        self.deliveries_failure.fetch_add(1, Ordering::Relaxed);
        self.delivery_duration_ms_sum
            .fetch_add(duration_ms, Ordering::Relaxed);
        self.update_max_duration(duration_ms);
    }

    pub fn inc_delivery_failure_for(&self, handler: &str, duration_ms: u64) {
        self.deliveries_failure.fetch_add(1, Ordering::Relaxed);
        self.delivery_duration_ms_sum
            .fetch_add(duration_ms, Ordering::Relaxed);
        self.update_max_duration(duration_ms);
        self.deliveries_by_handler_fail.inc(handler);
    }

    /// Classify and count delivery error type (4xx, 5xx, timeout).
    pub fn inc_delivery_error_type(&self, error_type: &str) {
        self.delivery_errors_by_type.inc(error_type);
    }

    pub fn inc_alerts_sent(&self) {
        self.alerts_sent.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_alerts_failed(&self) {
        self.alerts_failed.fetch_add(1, Ordering::Relaxed);
    }

    fn update_max_duration(&self, duration_ms: u64) {
        self.delivery_duration_ms_max
            .fetch_max(duration_ms, Ordering::Relaxed);
    }

    pub fn inc_verification_failure(&self, source: &str) {
        self.verification_failures.inc(source);
    }

    pub fn inc_dlq(&self, handler: &str) {
        self.dlq_by_handler.inc(handler);
    }

    pub fn inc_db_errors(&self) {
        self.db_errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_workflow_started(&self, workflow: &str) {
        self.workflow_runs_started.inc(workflow);
    }

    pub fn inc_workflow_completed(&self, workflow: &str) {
        self.workflow_runs_completed.inc(workflow);
    }

    pub fn inc_workflow_failed(&self, workflow: &str) {
        self.workflow_runs_failed.inc(workflow);
    }

    pub fn inc_workflow_step_completed(&self, workflow: &str) {
        self.workflow_steps_completed.inc(workflow);
    }

    pub fn inc_callbacks_received(&self) {
        self.callbacks_received.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_callbacks_expired(&self) {
        self.callbacks_expired.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_circuit_opened(&self, handler: &str) {
        self.circuit_opened.inc(handler);
    }

    pub fn inc_circuit_rejected(&self, handler: &str) {
        self.circuit_rejected.inc(handler);
    }

    // Queue (pull-mode) metrics
    pub fn inc_queue_messages_delivered(&self, queue: &str, count: u64) {
        for _ in 0..count {
            self.queue_messages_delivered.inc(queue);
        }
    }

    pub fn inc_queue_messages_acked(&self, queue: &str, count: u64) {
        for _ in 0..count {
            self.queue_messages_acked.inc(queue);
        }
    }

    pub fn inc_queue_messages_nacked(&self, queue: &str, count: u64) {
        for _ in 0..count {
            self.queue_messages_nacked.inc(queue);
        }
    }

    pub fn inc_queue_messages_expired(&self, queue: &str, count: u64) {
        for _ in 0..count {
            self.queue_messages_expired.inc(queue);
        }
    }

    pub fn dlq_total(&self) -> u64 {
        self.dlq_by_handler.total()
    }

    pub fn verification_failure_total(&self) -> u64 {
        self.verification_failures.total()
    }

    /// Format as Prometheus text exposition.
    /// `queue_depth` and `dead_jobs` are passed in since they come from DB queries.
    pub fn to_prometheus(&self, queue_depth: i64, dead_jobs: i64) -> String {
        let events = self.events_received.load(Ordering::Relaxed);
        let dupes = self.events_duplicated.load(Ordering::Relaxed);
        let jobs = self.jobs_created.load(Ordering::Relaxed);
        let ok = self.deliveries_success.load(Ordering::Relaxed);
        let fail = self.deliveries_failure.load(Ordering::Relaxed);
        let dur_sum = self.delivery_duration_ms_sum.load(Ordering::Relaxed);
        let total_deliveries = ok + fail;
        let dur_sum_secs = dur_sum as f64 / 1000.0;
        let db_errs = self.db_errors.load(Ordering::Relaxed);

        let mut out = String::with_capacity(4096);

        // Global counters
        out.push_str("# HELP qhook_events_received_total Total events received\n");
        out.push_str("# TYPE qhook_events_received_total counter\n");
        push_fmt(&mut out, &format!("qhook_events_received_total {events}\n"));

        // Events by source
        let by_source = self.events_by_source.snapshot();
        if !by_source.is_empty() {
            out.push_str("# HELP qhook_events_by_source_total Events received per source\n");
            out.push_str("# TYPE qhook_events_by_source_total counter\n");
            for (source, count) in &by_source {
                push_fmt(
                    &mut out,
                    &format!("qhook_events_by_source_total{{source=\"{source}\"}} {count}\n"),
                );
            }
        }

        out.push_str("# HELP qhook_events_duplicated_total Duplicate events ignored\n");
        out.push_str("# TYPE qhook_events_duplicated_total counter\n");
        push_fmt(
            &mut out,
            &format!("qhook_events_duplicated_total {dupes}\n"),
        );

        out.push_str("# HELP qhook_jobs_created_total Total jobs created\n");
        out.push_str("# TYPE qhook_jobs_created_total counter\n");
        push_fmt(&mut out, &format!("qhook_jobs_created_total {jobs}\n"));

        // Deliveries (global)
        out.push_str("# HELP qhook_deliveries_total Total delivery attempts\n");
        out.push_str("# TYPE qhook_deliveries_total counter\n");
        push_fmt(
            &mut out,
            &format!("qhook_deliveries_total{{result=\"success\"}} {ok}\n"),
        );
        push_fmt(
            &mut out,
            &format!("qhook_deliveries_total{{result=\"failure\"}} {fail}\n"),
        );

        // Deliveries by handler
        let handler_ok = self.deliveries_by_handler_ok.snapshot();
        let handler_fail = self.deliveries_by_handler_fail.snapshot();
        if !handler_ok.is_empty() || !handler_fail.is_empty() {
            out.push_str(
                "# HELP qhook_deliveries_by_handler_total Delivery attempts per handler\n",
            );
            out.push_str("# TYPE qhook_deliveries_by_handler_total counter\n");
            for (handler, count) in &handler_ok {
                push_fmt(
                    &mut out,
                    &format!(
                        "qhook_deliveries_by_handler_total{{handler=\"{handler}\",result=\"success\"}} {count}\n"
                    ),
                );
            }
            for (handler, count) in &handler_fail {
                push_fmt(
                    &mut out,
                    &format!(
                        "qhook_deliveries_by_handler_total{{handler=\"{handler}\",result=\"failure\"}} {count}\n"
                    ),
                );
            }
        }

        // Duration
        out.push_str("# HELP qhook_delivery_duration_seconds_sum Total delivery duration\n");
        out.push_str("# TYPE qhook_delivery_duration_seconds_sum counter\n");
        push_fmt(
            &mut out,
            &format!("qhook_delivery_duration_seconds_sum {dur_sum_secs}\n"),
        );
        out.push_str("# HELP qhook_delivery_duration_seconds_count Total delivery attempts\n");
        out.push_str("# TYPE qhook_delivery_duration_seconds_count counter\n");
        push_fmt(
            &mut out,
            &format!("qhook_delivery_duration_seconds_count {total_deliveries}\n"),
        );

        // Duration max
        let dur_max = self.delivery_duration_ms_max.load(Ordering::Relaxed);
        let dur_max_secs = dur_max as f64 / 1000.0;
        out.push_str("# HELP qhook_delivery_duration_seconds_max Max single delivery duration\n");
        out.push_str("# TYPE qhook_delivery_duration_seconds_max gauge\n");
        push_fmt(
            &mut out,
            &format!("qhook_delivery_duration_seconds_max {dur_max_secs}\n"),
        );

        // Delivery errors by type
        let error_types = self.delivery_errors_by_type.snapshot();
        if !error_types.is_empty() {
            out.push_str("# HELP qhook_delivery_errors_by_type_total Delivery errors by type\n");
            out.push_str("# TYPE qhook_delivery_errors_by_type_total counter\n");
            for (error_type, count) in &error_types {
                push_fmt(
                    &mut out,
                    &format!(
                        "qhook_delivery_errors_by_type_total{{type=\"{error_type}\"}} {count}\n"
                    ),
                );
            }
        }

        // Verification failures
        let verify_fails = self.verification_failures.snapshot();
        if !verify_fails.is_empty() {
            out.push_str("# HELP qhook_verification_failures_total Signature verification failures per source\n");
            out.push_str("# TYPE qhook_verification_failures_total counter\n");
            for (source, count) in &verify_fails {
                push_fmt(
                    &mut out,
                    &format!("qhook_verification_failures_total{{source=\"{source}\"}} {count}\n"),
                );
            }
        }

        // DLQ by handler
        let dlq = self.dlq_by_handler.snapshot();
        if !dlq.is_empty() {
            out.push_str("# HELP qhook_dlq_total Jobs moved to dead letter queue per handler\n");
            out.push_str("# TYPE qhook_dlq_total counter\n");
            for (handler, count) in &dlq {
                push_fmt(
                    &mut out,
                    &format!("qhook_dlq_total{{handler=\"{handler}\"}} {count}\n"),
                );
            }
        }

        // DB errors
        out.push_str("# HELP qhook_db_errors_total Database errors\n");
        out.push_str("# TYPE qhook_db_errors_total counter\n");
        push_fmt(&mut out, &format!("qhook_db_errors_total {db_errs}\n"));

        // Alerts
        let alerts_ok = self.alerts_sent.load(Ordering::Relaxed);
        let alerts_err = self.alerts_failed.load(Ordering::Relaxed);
        if alerts_ok > 0 || alerts_err > 0 {
            out.push_str("# HELP qhook_alerts_sent_total Alert webhooks sent successfully\n");
            out.push_str("# TYPE qhook_alerts_sent_total counter\n");
            push_fmt(&mut out, &format!("qhook_alerts_sent_total {alerts_ok}\n"));
            out.push_str("# HELP qhook_alerts_failed_total Alert webhooks that failed to send\n");
            out.push_str("# TYPE qhook_alerts_failed_total counter\n");
            push_fmt(
                &mut out,
                &format!("qhook_alerts_failed_total {alerts_err}\n"),
            );
        }

        // Gauges
        out.push_str("# HELP qhook_queue_depth Jobs waiting to be delivered\n");
        out.push_str("# TYPE qhook_queue_depth gauge\n");
        push_fmt(&mut out, &format!("qhook_queue_depth {queue_depth}\n"));
        out.push_str("# HELP qhook_dead_jobs Jobs in dead letter queue\n");
        out.push_str("# TYPE qhook_dead_jobs gauge\n");
        push_fmt(&mut out, &format!("qhook_dead_jobs {dead_jobs}\n"));

        // Workflow metrics
        let wf_started = self.workflow_runs_started.snapshot();
        let wf_completed = self.workflow_runs_completed.snapshot();
        let wf_failed = self.workflow_runs_failed.snapshot();
        if !wf_started.is_empty() || !wf_completed.is_empty() || !wf_failed.is_empty() {
            out.push_str("# HELP qhook_workflow_runs_total Workflow runs by workflow and status\n");
            out.push_str("# TYPE qhook_workflow_runs_total counter\n");
            for (wf, count) in &wf_started {
                push_fmt(
                    &mut out,
                    &format!(
                        "qhook_workflow_runs_total{{workflow=\"{wf}\",status=\"started\"}} {count}\n"
                    ),
                );
            }
            for (wf, count) in &wf_completed {
                push_fmt(
                    &mut out,
                    &format!(
                        "qhook_workflow_runs_total{{workflow=\"{wf}\",status=\"completed\"}} {count}\n"
                    ),
                );
            }
            for (wf, count) in &wf_failed {
                push_fmt(
                    &mut out,
                    &format!(
                        "qhook_workflow_runs_total{{workflow=\"{wf}\",status=\"failed\"}} {count}\n"
                    ),
                );
            }
        }

        let wf_steps = self.workflow_steps_completed.snapshot();
        if !wf_steps.is_empty() {
            out.push_str(
                "# HELP qhook_workflow_steps_completed_total Workflow steps completed per workflow\n",
            );
            out.push_str("# TYPE qhook_workflow_steps_completed_total counter\n");
            for (wf, count) in &wf_steps {
                push_fmt(
                    &mut out,
                    &format!("qhook_workflow_steps_completed_total{{workflow=\"{wf}\"}} {count}\n"),
                );
            }
        }

        let cb_recv = self.callbacks_received.load(Ordering::Relaxed);
        let cb_exp = self.callbacks_expired.load(Ordering::Relaxed);
        if cb_recv > 0 || cb_exp > 0 {
            out.push_str("# HELP qhook_callbacks_received_total Callback webhook invocations\n");
            out.push_str("# TYPE qhook_callbacks_received_total counter\n");
            push_fmt(
                &mut out,
                &format!("qhook_callbacks_received_total {cb_recv}\n"),
            );
            out.push_str("# HELP qhook_callbacks_expired_total Callback jobs expired by timeout\n");
            out.push_str("# TYPE qhook_callbacks_expired_total counter\n");
            push_fmt(
                &mut out,
                &format!("qhook_callbacks_expired_total {cb_exp}\n"),
            );
        }

        // Circuit breaker metrics
        let cb_opened = self.circuit_opened.snapshot();
        if !cb_opened.is_empty() {
            out.push_str(
                "# HELP qhook_circuit_breaker_opened_total Times circuit breaker opened by handler\n",
            );
            out.push_str("# TYPE qhook_circuit_breaker_opened_total counter\n");
            for (handler, count) in &cb_opened {
                push_fmt(
                    &mut out,
                    &format!(
                        "qhook_circuit_breaker_opened_total{{handler=\"{handler}\"}} {count}\n"
                    ),
                );
            }
        }
        let cb_rejected = self.circuit_rejected.snapshot();
        if !cb_rejected.is_empty() {
            out.push_str(
                "# HELP qhook_circuit_breaker_rejected_total Deliveries skipped by circuit breaker\n",
            );
            out.push_str("# TYPE qhook_circuit_breaker_rejected_total counter\n");
            for (handler, count) in &cb_rejected {
                push_fmt(
                    &mut out,
                    &format!(
                        "qhook_circuit_breaker_rejected_total{{handler=\"{handler}\"}} {count}\n"
                    ),
                );
            }
        }

        // Queue (pull-mode) metrics
        let q_delivered = self.queue_messages_delivered.snapshot();
        let q_acked = self.queue_messages_acked.snapshot();
        let q_nacked = self.queue_messages_nacked.snapshot();
        let q_expired = self.queue_messages_expired.snapshot();

        if !q_delivered.is_empty() {
            out.push_str(
                "# HELP qhook_queue_messages_delivered_total Messages delivered to consumers\n",
            );
            out.push_str("# TYPE qhook_queue_messages_delivered_total counter\n");
            for (queue, count) in &q_delivered {
                push_fmt(
                    &mut out,
                    &format!("qhook_queue_messages_delivered_total{{queue=\"{queue}\"}} {count}\n"),
                );
            }
        }
        if !q_acked.is_empty() {
            out.push_str("# HELP qhook_queue_messages_acked_total Messages acknowledged\n");
            out.push_str("# TYPE qhook_queue_messages_acked_total counter\n");
            for (queue, count) in &q_acked {
                push_fmt(
                    &mut out,
                    &format!("qhook_queue_messages_acked_total{{queue=\"{queue}\"}} {count}\n"),
                );
            }
        }
        if !q_nacked.is_empty() {
            out.push_str(
                "# HELP qhook_queue_messages_nacked_total Messages negatively acknowledged\n",
            );
            out.push_str("# TYPE qhook_queue_messages_nacked_total counter\n");
            for (queue, count) in &q_nacked {
                push_fmt(
                    &mut out,
                    &format!("qhook_queue_messages_nacked_total{{queue=\"{queue}\"}} {count}\n"),
                );
            }
        }
        if !q_expired.is_empty() {
            out.push_str("# HELP qhook_queue_messages_expired_total Messages recovered after visibility timeout\n");
            out.push_str("# TYPE qhook_queue_messages_expired_total counter\n");
            for (queue, count) in &q_expired {
                push_fmt(
                    &mut out,
                    &format!("qhook_queue_messages_expired_total{{queue=\"{queue}\"}} {count}\n"),
                );
            }
        }

        // Label cardinality (monitor for unbounded growth)
        let label_count = self.events_by_source.label_count()
            + self.deliveries_by_handler_ok.label_count()
            + self.deliveries_by_handler_fail.label_count()
            + self.dlq_by_handler.label_count()
            + self.verification_failures.label_count()
            + self.delivery_errors_by_type.label_count()
            + self.circuit_opened.label_count()
            + self.circuit_rejected.label_count()
            + self.workflow_runs_started.label_count()
            + self.workflow_runs_completed.label_count()
            + self.workflow_runs_failed.label_count()
            + self.workflow_steps_completed.label_count()
            + self.queue_messages_delivered.label_count()
            + self.queue_messages_acked.label_count()
            + self.queue_messages_nacked.label_count()
            + self.queue_messages_expired.label_count();
        out.push_str(
            "# HELP qhook_metric_label_count Total unique label values across all labeled counters\n",
        );
        out.push_str("# TYPE qhook_metric_label_count gauge\n");
        push_fmt(
            &mut out,
            &format!("qhook_metric_label_count {label_count}\n"),
        );

        out
    }
}

fn push_fmt(out: &mut String, s: &str) {
    out.push_str(s);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_counters() {
        let m = Metrics::new();
        m.inc_events_received();
        m.inc_events_received();
        m.inc_jobs_created();
        m.inc_delivery_success(150);
        m.inc_delivery_failure(200);

        assert_eq!(m.events_received.load(Ordering::Relaxed), 2);
        assert_eq!(m.jobs_created.load(Ordering::Relaxed), 1);
        assert_eq!(m.deliveries_success.load(Ordering::Relaxed), 1);
        assert_eq!(m.deliveries_failure.load(Ordering::Relaxed), 1);
        assert_eq!(m.delivery_duration_ms_sum.load(Ordering::Relaxed), 350);
    }

    #[test]
    fn test_labeled_counters() {
        let m = Metrics::new();
        m.inc_events_received_for("stripe");
        m.inc_events_received_for("stripe");
        m.inc_events_received_for("github");
        m.inc_verification_failure("stripe");
        m.inc_delivery_success_for("payment", 100);
        m.inc_delivery_failure_for("payment", 200);
        m.inc_dlq("payment");
        m.inc_db_errors();

        assert_eq!(m.events_received.load(Ordering::Relaxed), 3);

        let output = m.to_prometheus(5, 2);
        assert!(output.contains("qhook_events_by_source_total{source=\"stripe\"} 2"));
        assert!(output.contains("qhook_events_by_source_total{source=\"github\"} 1"));
        assert!(output.contains("qhook_verification_failures_total{source=\"stripe\"} 1"));
        assert!(output.contains(
            "qhook_deliveries_by_handler_total{handler=\"payment\",result=\"success\"} 1"
        ));
        assert!(output.contains(
            "qhook_deliveries_by_handler_total{handler=\"payment\",result=\"failure\"} 1"
        ));
        assert!(output.contains("qhook_dlq_total{handler=\"payment\"} 1"));
        assert!(output.contains("qhook_db_errors_total 1"));
    }

    #[test]
    fn test_error_classification_and_alerts() {
        let m = Metrics::new();
        m.inc_delivery_error_type("4xx");
        m.inc_delivery_error_type("4xx");
        m.inc_delivery_error_type("5xx");
        m.inc_delivery_error_type("timeout");
        m.inc_alerts_sent();
        m.inc_alerts_sent();
        m.inc_alerts_failed();

        let output = m.to_prometheus(0, 0);
        assert!(output.contains("qhook_delivery_errors_by_type_total{type=\"4xx\"} 2"));
        assert!(output.contains("qhook_delivery_errors_by_type_total{type=\"5xx\"} 1"));
        assert!(output.contains("qhook_delivery_errors_by_type_total{type=\"timeout\"} 1"));
        assert!(output.contains("qhook_alerts_sent_total 2"));
        assert!(output.contains("qhook_alerts_failed_total 1"));
    }

    #[test]
    fn test_max_duration_and_label_count() {
        let m = Metrics::new();
        m.inc_delivery_success_for("handler-a", 100);
        m.inc_delivery_success_for("handler-a", 500);
        m.inc_delivery_failure_for("handler-b", 200);

        let output = m.to_prometheus(0, 0);
        // Max should be 500ms = 0.5s
        assert!(output.contains("qhook_delivery_duration_seconds_max 0.5"));
        // Label count: handler-a(ok) + handler-b(fail) = 2
        assert!(output.contains("qhook_metric_label_count 2"));
    }

    #[test]
    fn test_workflow_metrics() {
        let m = Metrics::new();
        m.inc_workflow_started("order-pipeline");
        m.inc_workflow_started("order-pipeline");
        m.inc_workflow_completed("order-pipeline");
        m.inc_workflow_failed("order-pipeline");
        m.inc_workflow_step_completed("order-pipeline");
        m.inc_workflow_step_completed("order-pipeline");
        m.inc_workflow_step_completed("order-pipeline");
        m.inc_callbacks_received();
        m.inc_callbacks_expired();

        let output = m.to_prometheus(0, 0);
        assert!(output.contains(
            "qhook_workflow_runs_total{workflow=\"order-pipeline\",status=\"started\"} 2"
        ));
        assert!(output.contains(
            "qhook_workflow_runs_total{workflow=\"order-pipeline\",status=\"completed\"} 1"
        ));
        assert!(output.contains(
            "qhook_workflow_runs_total{workflow=\"order-pipeline\",status=\"failed\"} 1"
        ));
        assert!(
            output.contains("qhook_workflow_steps_completed_total{workflow=\"order-pipeline\"} 3")
        );
        assert!(output.contains("qhook_callbacks_received_total 1"));
        assert!(output.contains("qhook_callbacks_expired_total 1"));
    }

    #[test]
    fn test_prometheus_format() {
        let m = Metrics::new();
        m.inc_events_received();
        m.inc_delivery_success(1000);

        let output = m.to_prometheus(5, 2);
        assert!(output.contains("qhook_events_received_total 1"));
        assert!(output.contains("qhook_deliveries_total{result=\"success\"} 1"));
        assert!(output.contains("qhook_queue_depth 5"));
        assert!(output.contains("qhook_dead_jobs 2"));
        assert!(output.contains("qhook_delivery_duration_seconds_sum 1"));
    }
}
