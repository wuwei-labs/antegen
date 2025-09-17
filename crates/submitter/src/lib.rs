pub mod builder;
pub mod metrics;
pub mod service;
pub mod submitter;

// Re-export main public APIs
pub use metrics::SubmitterMetrics;
pub use service::{SubmissionService, SubmissionConfig};
pub use submitter::TransactionSubmitter;

// Re-export shared types from SDK
pub use antegen_sdk::types::{TransactionMessage, DurableTransactionMessage};

// Submitter-specific types
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum SubmissionMode {
    Rpc,
    Tpu,
    TpuWithFallback,
    Both,
}

impl Default for SubmissionMode {
    fn default() -> Self {
        Self::TpuWithFallback
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TpuConfig {
    pub mode: SubmissionMode,
    pub leader_forward_count: u64,
    pub connection_timeout_ms: u64,
    pub fanout_slots: u64,
}

impl Default for TpuConfig {
    fn default() -> Self {
        Self {
            mode: SubmissionMode::TpuWithFallback,
            leader_forward_count: 2,
            connection_timeout_ms: 5000,
            fanout_slots: 12,
        }
    }
}

// TODO: Implement replay service for durable transaction replay
//
// The replay service should:
// - Queue failed transactions for retry
// - Support configurable retry delays
// - Handle nonce account refresh for durable transactions
// - Integrate with external message queue (e.g., NATS, Kafka, Redis)
//
// Example configuration:
// ```
// pub struct ReplayConfig {
//     pub enable_replay: bool,
//     pub queue_url: Option<String>,
//     pub retry_delay_ms: u64,
//     pub max_attempts: u32,
// }
// ```