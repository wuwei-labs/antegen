pub mod builder;
pub mod client;
pub mod metrics;
pub mod replay;
pub mod service;

// Re-export main public APIs
pub use client::TransactionSubmitter;
pub use metrics::SubmitterMetrics;
pub use replay::ReplayConsumer;
pub use service::{SubmissionService, SubmissionConfig};

// Re-export shared types from SDK
pub use antegen_sdk::rpc::{CachedRpcClient, CacheConfig};
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
        Self::Rpc
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayConfig {
    pub enable_replay: bool,
    pub nats_url: Option<String>,
    pub stream_name: String,
    pub consumer_name: String,
    pub batch_size: usize,
    pub replay_delay_ms: u64,
    pub replay_max_age_ms: u64,
    pub replay_max_attempts: u32,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            enable_replay: false,
            nats_url: None,
            stream_name: "antegen-replay".to_string(),
            consumer_name: "replay-consumer".to_string(),
            batch_size: 10,
            replay_delay_ms: 30_000,
            replay_max_age_ms: 3600_000,
            replay_max_attempts: 3,
        }
    }
}