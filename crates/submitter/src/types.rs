use serde::{Deserialize, Serialize};
use crate::{TpuConfig, SubmissionMode};

/// Configuration for the submitter service
#[derive(Debug, Clone)]
pub struct SubmitterConfig {
    /// Enable replay functionality (consume from NATS)
    pub enable_replay: bool,
    /// NATS server connection URL
    pub nats_url: Option<String>,
    /// Delay in milliseconds before replaying a transaction
    pub replay_delay_ms: u64,
    /// Maximum age of transactions to replay (in milliseconds)
    pub replay_max_age_ms: u64,
    /// Maximum number of replay attempts per transaction
    pub replay_max_attempts: u32,
    /// TPU client configuration
    pub tpu_config: Option<TpuConfig>,
    /// Submission mode preference
    pub submission_mode: SubmissionMode,
}

impl Default for SubmitterConfig {
    fn default() -> Self {
        Self {
            enable_replay: false,
            nats_url: None,
            replay_delay_ms: 30_000,        // 30 seconds
            replay_max_age_ms: 300_000,     // 5 minutes
            replay_max_attempts: 3,
            tpu_config: Some(TpuConfig::default()),
            submission_mode: SubmissionMode::default(), // TpuWithFallback
        }
    }
}

/// Message format for durable transactions published to NATS
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableTransactionMessage {
    /// Base64 encoded signed transaction
    pub base64_transaction: String,
    /// Thread pubkey that this transaction executes
    pub thread_pubkey: String,
    /// Original signature from the executor
    pub original_signature: String,
    /// Unix timestamp when transaction was originally submitted
    pub submitted_at: u64,
    /// Pubkey of executor that originally submitted
    pub executor_pubkey: String,
    /// Current retry count
    pub retry_count: u32,
}

impl DurableTransactionMessage {
    pub fn new(
        base64_transaction: String,
        thread_pubkey: String,
        original_signature: String,
        executor_pubkey: String,
    ) -> Self {
        Self {
            base64_transaction,
            thread_pubkey,
            original_signature,
            submitted_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            executor_pubkey,
            retry_count: 0,
        }
    }
    
    /// Check if transaction is too old to replay
    pub fn is_expired(&self, max_age_ms: u64) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        (now - self.submitted_at) * 1000 > max_age_ms
    }
    
    /// Get age in milliseconds
    pub fn age_ms(&self) -> u64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        (now - self.submitted_at) * 1000
    }
}