/// Common metrics utilities and constants for Antegen components
use opentelemetry::KeyValue;

/// Common metric name prefixes
pub const METRIC_PREFIX_OBSERVER: &str = "antegen.observer";
pub const METRIC_PREFIX_SUBMITTER: &str = "antegen.submitter";
pub const METRIC_PREFIX_THREAD: &str = "antegen.thread";

/// Common metric attribute keys
pub mod attributes {
    /// Trigger type (interval, cron, slot, epoch, etc.)
    pub const TRIGGER_TYPE: &str = "trigger.type";
    
    /// Transaction status (success, failed, timeout)
    pub const STATUS: &str = "status";
    
    /// Thread ID
    pub const THREAD_ID: &str = "thread.id";
    
    /// Queue type (time, slot, epoch)
    pub const QUEUE_TYPE: &str = "queue.type";
    
    /// Result of an operation
    pub const RESULT: &str = "result";
    
    /// Priority level
    pub const PRIORITY: &str = "priority";
    
    /// Error type/code
    pub const ERROR_TYPE: &str = "error.type";
    
    /// Recipient of payment (executor, team, authority)
    pub const RECIPIENT: &str = "recipient";
}

/// Common metric names
pub mod metrics {
    /// Observer metrics
    pub const THREADS_MONITORED: &str = "threads.monitored";
    pub const THREADS_DISCOVERED: &str = "threads.discovered.total";
    pub const THREADS_TRIGGERED: &str = "threads.triggered.total";
    pub const ACCOUNT_UPDATES: &str = "account.updates.total";
    pub const TRIGGER_EVALUATION_DURATION: &str = "trigger.evaluation.duration";
    pub const QUEUE_DEPTH: &str = "queue.depth";
    
    /// Submitter metrics
    pub const TRANSACTIONS_SUBMITTED: &str = "transactions.submitted.total";
    pub const TRANSACTIONS_REPLAYED: &str = "transactions.replayed.total";
    pub const SIMULATIONS: &str = "simulations.total";
    pub const SIMULATION_DURATION: &str = "simulation.duration";
    pub const COMPUTE_UNITS_USED: &str = "compute_units.used";
    pub const SUBMISSION_DURATION: &str = "submission.duration";
    pub const CONFIRMATION_DURATION: &str = "confirmation.duration";
    pub const QUEUE_SIZE: &str = "queue.size";
    pub const EXECUTOR_BALANCE_CHANGE: &str = "executor.balance.change";
    
    /// Economic metrics
    pub const LAMPORTS_PAID: &str = "lamports.paid.total";
    pub const COMMISSION_FORGONE: &str = "commission.forgone.total";
    pub const COMMISSION_MULTIPLIER: &str = "commission.multiplier";
    pub const EXECUTION_DELAY: &str = "execution.delay";
}

/// Helper function to create status attribute
pub fn status_attr(success: bool) -> KeyValue {
    KeyValue::new(attributes::STATUS, if success { "success" } else { "failed" })
}

/// Helper function to create trigger type attribute
pub fn trigger_type_attr(trigger_type: &str) -> KeyValue {
    KeyValue::new(attributes::TRIGGER_TYPE, trigger_type.to_string())
}

/// Helper function to create result attribute
pub fn result_attr(result: &str) -> KeyValue {
    KeyValue::new(attributes::RESULT, result.to_string())
}