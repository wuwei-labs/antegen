use opentelemetry::metrics::{Counter, Histogram, Meter, Unit};
use std::sync::Arc;

/// Metrics collector for transaction submission
pub struct SubmitterMetrics {
    // Transaction metrics
    pub transactions_submitted: Counter<u64>,
    pub transactions_confirmed: Counter<u64>,
    pub transactions_failed: Counter<u64>,
    pub submission_latency: Histogram<f64>,

    // RPC metrics
    pub rpc_requests: Counter<u64>,
    pub rpc_errors: Counter<u64>,
    pub rpc_latency: Histogram<f64>,

    // TPU metrics
    pub tpu_submissions: Counter<u64>,
    pub tpu_errors: Counter<u64>,

    // Cache metrics
    pub cache_hits: Counter<u64>,
    pub cache_misses: Counter<u64>,

    // Replay metrics
    pub replay_attempts: Counter<u64>,
    pub replay_successes: Counter<u64>,
    pub replay_failures: Counter<u64>,
}

impl Default for SubmitterMetrics {
    fn default() -> Self {
        use opentelemetry::global;
        let meter = global::meter("antegen-submitter");

        // Create a basic metrics instance with noop metrics for default
        // This is used when no proper meter is provided
        Self {
            transactions_submitted: meter.u64_counter("noop").init(),
            transactions_confirmed: meter.u64_counter("noop").init(),
            transactions_failed: meter.u64_counter("noop").init(),
            submission_latency: meter.f64_histogram("noop").init(),
            rpc_requests: meter.u64_counter("noop").init(),
            rpc_errors: meter.u64_counter("noop").init(),
            rpc_latency: meter.f64_histogram("noop").init(),
            tpu_submissions: meter.u64_counter("noop").init(),
            tpu_errors: meter.u64_counter("noop").init(),
            cache_hits: meter.u64_counter("noop").init(),
            cache_misses: meter.u64_counter("noop").init(),
            replay_attempts: meter.u64_counter("noop").init(),
            replay_successes: meter.u64_counter("noop").init(),
            replay_failures: meter.u64_counter("noop").init(),
        }
    }
}

impl SubmitterMetrics {
    pub fn new(meter: &Meter) -> Arc<Self> {
        Arc::new(Self {
            transactions_submitted: meter
                .u64_counter("submitter.transactions.submitted")
                .with_description("Number of transactions submitted")
                .init(),

            transactions_confirmed: meter
                .u64_counter("submitter.transactions.confirmed")
                .with_description("Number of transactions confirmed")
                .init(),

            transactions_failed: meter
                .u64_counter("submitter.transactions.failed")
                .with_description("Number of transactions failed")
                .init(),

            submission_latency: meter
                .f64_histogram("submitter.submission.latency")
                .with_description("Transaction submission latency")
                .with_unit(Unit::new("milliseconds"))
                .init(),

            rpc_requests: meter
                .u64_counter("submitter.rpc.requests")
                .with_description("Number of RPC requests")
                .init(),

            rpc_errors: meter
                .u64_counter("submitter.rpc.errors")
                .with_description("Number of RPC errors")
                .init(),

            rpc_latency: meter
                .f64_histogram("submitter.rpc.latency")
                .with_description("RPC request latency")
                .with_unit(Unit::new("milliseconds"))
                .init(),

            tpu_submissions: meter
                .u64_counter("submitter.tpu.submissions")
                .with_description("Number of TPU submissions")
                .init(),

            tpu_errors: meter
                .u64_counter("submitter.tpu.errors")
                .with_description("Number of TPU errors")
                .init(),

            cache_hits: meter
                .u64_counter("submitter.cache.hits")
                .with_description("Number of cache hits")
                .init(),

            cache_misses: meter
                .u64_counter("submitter.cache.misses")
                .with_description("Number of cache misses")
                .init(),

            replay_attempts: meter
                .u64_counter("submitter.replay.attempts")
                .with_description("Number of replay attempts")
                .init(),

            replay_successes: meter
                .u64_counter("submitter.replay.successes")
                .with_description("Number of successful replays")
                .init(),

            replay_failures: meter
                .u64_counter("submitter.replay.failures")
                .with_description("Number of failed replays")
                .init(),
        })
    }

    // Helper methods for common operations
    pub fn transaction_submitted(&self, mode: &str) {
        self.transactions_submitted.add(1, &[]);
        match mode {
            "tpu" => self.tpu_submissions.add(1, &[]),
            _ => {}
        }
    }

    pub fn transaction_confirmed(&self) {
        self.transactions_confirmed.add(1, &[]);
    }

    pub fn transaction_failed(&self) {
        self.transactions_failed.add(1, &[]);
    }

    pub fn rpc_request(&self, _method: &str) {
        self.rpc_requests.add(1, &[]);
    }

    pub fn cache_hit(&self, _cache_type: &str) {
        self.cache_hits.add(1, &[]);
    }

    pub fn cache_miss(&self, _cache_type: &str) {
        self.cache_misses.add(1, &[]);
    }

    pub fn batch_submitted(&self, mode: &str, count: usize) {
        match mode {
            "tpu" => self.tpu_submissions.add(count as u64, &[]),
            "rpc" => self.rpc_requests.add(count as u64, &[]),
            _ => {}
        }
        self.transactions_submitted.add(count as u64, &[]);
    }

    pub fn durable_tx_published(&self) {
        self.transactions_submitted.add(1, &[]);
    }
}
