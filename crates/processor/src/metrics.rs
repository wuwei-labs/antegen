/// Processor metrics for monitoring thread processing and execution
use opentelemetry::{
    global,
    metrics::{Counter, Histogram, Unit, UpDownCounter},
    KeyValue,
};

pub struct ProcessorMetrics {
    // Transaction tracking
    pub transactions: Counter<u64>,
    pub transaction_retries: Counter<u64>,
    pub simulations: Counter<u64>,

    // Performance metrics
    pub simulation_duration: Histogram<f64>,
    pub compute_units_used: Histogram<u64>,
    pub submission_duration: Histogram<f64>,
    pub thread_execution_time: Histogram<f64>,

    // Queue metrics
    pub queue_size: UpDownCounter<i64>,

    // Network metrics
    pub rpc_requests: Counter<u64>,
    pub tpu_submissions: Counter<u64>,
    
    // Cache metrics
    pub cache_hits: Counter<u64>,
    pub cache_misses: Counter<u64>,
}

impl Default for ProcessorMetrics {
    fn default() -> Self {
        let meter = global::meter("antegen_processor");

        Self {
            transactions: meter
                .u64_counter("transactions")
                .with_description("Total number of transactions by status")
                .init(),

            transaction_retries: meter
                .u64_counter("transaction_retries")
                .with_description("Total number of transaction retry attempts")
                .init(),

            simulations: meter
                .u64_counter("simulations")
                .with_description("Total number of transaction simulations by result")
                .init(),

            simulation_duration: meter
                .f64_histogram("simulation_duration")
                .with_unit(Unit::new("s"))
                .with_description("Time taken to simulate transactions")
                .init(),

            compute_units_used: meter
                .u64_histogram("compute_units_used")
                .with_description("Compute units consumed by transactions")
                .init(),

            submission_duration: meter
                .f64_histogram("submission_duration")
                .with_unit(Unit::new("s"))
                .with_description("Time taken to submit transactions")
                .init(),

            thread_execution_time: meter
                .f64_histogram("thread_execution")
                .with_unit(Unit::new("s"))
                .with_description("Time from thread trigger to transaction submission")
                .init(),

            queue_size: meter
                .i64_up_down_counter("queue_size")
                .with_description("Current number of threads in submission queue")
                .init(),

            rpc_requests: meter
                .u64_counter("rpc_requests")
                .with_description("Total RPC requests made")
                .init(),

            tpu_submissions: meter
                .u64_counter("tpu_submissions")
                .with_description("Total TPU submission attempts")
                .init(),
                
            cache_hits: meter
                .u64_counter("cache_hits")
                .with_description("Total cache hits by cache type")
                .init(),
                
            cache_misses: meter
                .u64_counter("cache_misses")
                .with_description("Total cache misses by cache type")
                .init(),
        }
    }
}

impl ProcessorMetrics {
    /// Create metrics with a specific meter
    pub fn new(meter: &opentelemetry::metrics::Meter) -> Self {
        Self {
            transactions: meter
                .u64_counter("processor.transactions")
                .with_description("Total number of transactions by status")
                .init(),

            transaction_retries: meter
                .u64_counter("processor.transaction_retries")
                .with_description("Total number of transaction retry attempts")
                .init(),

            simulations: meter
                .u64_counter("processor.simulations")
                .with_description("Total number of transaction simulations by result")
                .init(),

            simulation_duration: meter
                .f64_histogram("processor.simulation_duration")
                .with_unit(Unit::new("s"))
                .with_description("Duration of transaction simulations")
                .init(),

            compute_units_used: meter
                .u64_histogram("processor.compute_units_used")
                .with_description("Compute units used per transaction")
                .init(),

            submission_duration: meter
                .f64_histogram("processor.submission_duration")
                .with_unit(Unit::new("s"))
                .with_description("Duration of transaction submissions")
                .init(),

            thread_execution_time: meter
                .f64_histogram("processor.thread_execution_time")
                .with_unit(Unit::new("s"))
                .with_description("Time to execute a thread")
                .init(),

            queue_size: meter
                .i64_up_down_counter("processor.queue_size")
                .with_description("Current number of threads in queue")
                .init(),

            rpc_requests: meter
                .u64_counter("processor.rpc_requests")
                .with_description("Total RPC requests")
                .init(),

            tpu_submissions: meter
                .u64_counter("processor.tpu_submissions")
                .with_description("Total TPU submissions")
                .init(),
                
            cache_hits: meter
                .u64_counter("processor.cache_hits")
                .with_description("Total cache hits by cache type")
                .init(),
                
            cache_misses: meter
                .u64_counter("processor.cache_misses")
                .with_description("Total cache misses by cache type")
                .init(),
        }
    }
    
    /// Record a transaction submission
    pub fn transaction_submitted(&self, status: &str, trigger_type: Option<&str>) {
        let mut labels = vec![KeyValue::new("status", status.to_string())];
        if let Some(trigger) = trigger_type {
            labels.push(KeyValue::new("trigger_type", trigger.to_string()));
        }
        self.transactions.add(1, &labels);
    }

    /// Record a retry attempt
    pub fn transaction_retry(&self, attempt: u64, reason: &str) {
        self.transaction_retries.add(
            1,
            &[
                KeyValue::new("attempt", attempt as i64),
                KeyValue::new("reason", reason.to_string()),
            ],
        );
    }

    /// Record a simulation
    pub fn simulation_performed(&self, result: &str) {
        self.simulations
            .add(1, &[KeyValue::new("result", result.to_string())]);
    }

    /// Record simulation duration
    pub fn record_simulation_duration(&self, duration_secs: f64) {
        self.simulation_duration.record(duration_secs, &[]);
    }

    /// Record compute units used
    pub fn record_compute_units(&self, units: u64) {
        self.compute_units_used.record(units, &[]);
    }

    /// Record submission duration
    pub fn record_submission_duration(&self, duration_secs: f64) {
        self.submission_duration.record(duration_secs, &[]);
    }

    /// Record thread execution time
    pub fn record_thread_execution(&self, duration_secs: f64, trigger_type: &str) {
        self.thread_execution_time.record(
            duration_secs,
            &[KeyValue::new("trigger_type", trigger_type.to_string())],
        );
    }

    /// Update queue size
    pub fn set_queue_size(&self, size: u64, priority: Option<&str>) {
        let labels = if let Some(p) = priority {
            vec![KeyValue::new("priority", p.to_string())]
        } else {
            vec![]
        };
        // For up/down counter, we'd need to track the delta
        // This is a simplified approach - in production you'd track the previous value
        self.queue_size.add(size as i64, &labels);
    }

    /// Record RPC request
    pub fn rpc_request(&self, method: &str) {
        self.rpc_requests
            .add(1, &[KeyValue::new("method", method.to_string())]);
    }

    /// Record TPU submission
    pub fn tpu_submission(&self, result: &str) {
        self.tpu_submissions
            .add(1, &[KeyValue::new("result", result.to_string())]);
    }
    
    /// Record cache hit
    pub fn cache_hit(&self, cache_type: &str) {
        self.cache_hits
            .add(1, &[KeyValue::new("cache_type", cache_type.to_string())]);
    }
    
    /// Record cache miss
    pub fn cache_miss(&self, cache_type: &str) {
        self.cache_misses
            .add(1, &[KeyValue::new("cache_type", cache_type.to_string())]);
    }
}
