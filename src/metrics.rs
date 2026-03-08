use std::sync::atomic::{AtomicU64, Ordering};

/// Lightweight application metrics using atomic counters.
/// No external dependencies — formats Prometheus text exposition directly.
pub struct Metrics {
    pub events_received: AtomicU64,
    pub events_duplicated: AtomicU64,
    pub jobs_created: AtomicU64,
    pub deliveries_success: AtomicU64,
    pub deliveries_failure: AtomicU64,
    pub delivery_duration_ms_sum: AtomicU64,
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
        }
    }

    pub fn inc_events_received(&self) {
        self.events_received.fetch_add(1, Ordering::Relaxed);
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

    pub fn inc_delivery_failure(&self, duration_ms: u64) {
        self.deliveries_failure.fetch_add(1, Ordering::Relaxed);
        self.delivery_duration_ms_sum
            .fetch_add(duration_ms, Ordering::Relaxed);
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

        format!(
            "# HELP qhook_events_received_total Total events received\n\
             # TYPE qhook_events_received_total counter\n\
             qhook_events_received_total {events}\n\
             # HELP qhook_events_duplicated_total Duplicate events ignored\n\
             # TYPE qhook_events_duplicated_total counter\n\
             qhook_events_duplicated_total {dupes}\n\
             # HELP qhook_jobs_created_total Total jobs created\n\
             # TYPE qhook_jobs_created_total counter\n\
             qhook_jobs_created_total {jobs}\n\
             # HELP qhook_deliveries_total Total delivery attempts\n\
             # TYPE qhook_deliveries_total counter\n\
             qhook_deliveries_total{{result=\"success\"}} {ok}\n\
             qhook_deliveries_total{{result=\"failure\"}} {fail}\n\
             # HELP qhook_delivery_duration_seconds_sum Total delivery duration\n\
             # TYPE qhook_delivery_duration_seconds_sum counter\n\
             qhook_delivery_duration_seconds_sum {dur_sum_secs}\n\
             # HELP qhook_delivery_duration_seconds_count Total delivery attempts\n\
             # TYPE qhook_delivery_duration_seconds_count counter\n\
             qhook_delivery_duration_seconds_count {total_deliveries}\n\
             # HELP qhook_queue_depth Jobs waiting to be delivered\n\
             # TYPE qhook_queue_depth gauge\n\
             qhook_queue_depth {queue_depth}\n\
             # HELP qhook_dead_jobs Jobs in dead letter queue\n\
             # TYPE qhook_dead_jobs gauge\n\
             qhook_dead_jobs {dead_jobs}\n"
        )
    }
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
