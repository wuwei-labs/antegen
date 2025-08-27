use serde::{Deserialize, Serialize};

/// Configuration for TPU client
#[derive(Debug, Clone)]
pub struct TpuConfig {
    /// Number of leaders to send to in parallel
    pub fanout_slots: u64,
    /// Connection pool size
    pub connection_pool_size: usize,
    /// Submission mode
    pub mode: SubmissionMode,
}

impl Default for TpuConfig {
    fn default() -> Self {
        Self {
            fanout_slots: 12, // Send to 12 leader slots
            connection_pool_size: 4,
            mode: SubmissionMode::TpuWithFallback,
        }
    }
}

/// Submission mode for transactions
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SubmissionMode {
    /// Use TPU client for submission
    Tpu,
    /// Use RPC for submission  
    Rpc,
    /// Try TPU first, fallback to RPC
    TpuWithFallback,
}

impl Default for SubmissionMode {
    fn default() -> Self {
        SubmissionMode::TpuWithFallback
    }
}

/// Configuration for the cached RPC client
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// TTL for account cache in seconds  
    pub account_ttl_secs: u64,
    /// Maximum number of cached accounts
    pub max_cached_accounts: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            account_ttl_secs: 48 * 3600,  // 48 hours - safe with observer updates
            max_cached_accounts: 10000,   // Increased limit with longer TTL
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
        blockchain_timestamp: i64,
    ) -> Self {
        Self {
            base64_transaction,
            thread_pubkey,
            original_signature,
            submitted_at: blockchain_timestamp as u64,
            executor_pubkey,
            retry_count: 0,
        }
    }
    
    /// Check if transaction is too old to replay (based on blockchain time)
    pub fn is_expired(&self, max_age_ms: u64, current_blockchain_time: i64) -> bool {
        let now = current_blockchain_time as u64;
        (now - self.submitted_at) * 1000 > max_age_ms
    }
    
    /// Get age in milliseconds (based on blockchain time)
    pub fn age_ms(&self, current_blockchain_time: i64) -> u64 {
        let now = current_blockchain_time as u64;
        (now - self.submitted_at) * 1000
    }
    
    /// Check if transaction is too old to replay (using system time for backward compat)
    /// This is used by the replay consumer which doesn't have access to blockchain time
    pub fn is_expired_system_time(&self, max_age_ms: u64) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        (now - self.submitted_at) * 1000 > max_age_ms
    }
    
    /// Get age in milliseconds (using system time for backward compat)
    /// This is used by the replay consumer which doesn't have access to blockchain time
    pub fn age_ms_system_time(&self) -> u64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        (now - self.submitted_at) * 1000
    }
}

/// Configuration for replay functionality
#[derive(Debug, Clone)]
pub struct ReplayConfig {
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
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            enable_replay: false,
            nats_url: None,
            replay_delay_ms: 30_000,        // 30 seconds
            replay_max_age_ms: 300_000,     // 5 minutes
            replay_max_attempts: 3,
        }
    }
}