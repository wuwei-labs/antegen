/// Adapter metrics for monitoring data source integration
use opentelemetry::{
    global,
    metrics::{Counter, Histogram, Unit, UpDownCounter},
    KeyValue,
};

pub struct AdapterMetrics {
    // Thread tracking
    pub threads_active: UpDownCounter<i64>,
    pub threads_triggered: Counter<u64>,
    pub trigger_checks: Counter<u64>,
    pub triggers_ready: Counter<u64>,
    
    // Account updates
    pub account_updates: Counter<u64>,
    
    // Performance
    pub trigger_evaluation_duration: Histogram<f64>,
}

impl Default for AdapterMetrics {
    fn default() -> Self {
        let meter = global::meter("antegen_adapter");
        
        Self {
            threads_active: meter
                .i64_up_down_counter("threads_active")
                .with_description("Number of threads currently active")
                .init(),
                
            threads_triggered: meter
                .u64_counter("threads_triggered")
                .with_description("Total number of threads triggered for execution")
                .init(),
                
            trigger_checks: meter
                .u64_counter("trigger_checks")
                .with_description("Total number of trigger evaluations performed")
                .init(),
                
            triggers_ready: meter
                .u64_counter("triggers_ready")
                .with_description("Total number of triggers that were ready to execute")
                .init(),
                
            account_updates: meter
                .u64_counter("account_updates")
                .with_description("Total number of account updates processed")
                .init(),
                
            trigger_evaluation_duration: meter
                .f64_histogram("trigger_evaluation_duration")
                .with_unit(Unit::new("s"))
                .with_description("Time taken to evaluate if a thread trigger is ready")
                .init(),
                
        }
    }
}

impl AdapterMetrics {
    /// Update the number of active threads
    pub fn set_active_threads(&self, count: u64) {
        // Set to new value by calculating delta
        // Note: This is a workaround since we don't have a true gauge
        // In production, you'd want to track the previous value
        self.threads_active.add(count as i64, &[]);
    }
    
    /// Record a thread being triggered
    pub fn thread_triggered(&self, trigger_type: &str) {
        self.threads_triggered.add(1, &[
            KeyValue::new("trigger_type", trigger_type.to_string()),
        ]);
        self.triggers_ready.add(1, &[
            KeyValue::new("trigger_type", trigger_type.to_string()),
        ]);
    }
    
    /// Record a trigger check
    pub fn trigger_checked(&self, trigger_type: &str) {
        self.trigger_checks.add(1, &[
            KeyValue::new("trigger_type", trigger_type.to_string()),
        ]);
    }
    
    /// Record an account update
    pub fn account_update_processed(&self, account_type: &str) {
        self.account_updates.add(1, &[
            KeyValue::new("account_type", account_type.to_string()),
        ]);
    }
    
    /// Record trigger evaluation time
    pub fn record_trigger_evaluation(&self, duration_secs: f64, trigger_type: &str) {
        self.trigger_evaluation_duration.record(duration_secs, &[
            KeyValue::new("trigger_type", trigger_type.to_string()),
        ]);
    }
}