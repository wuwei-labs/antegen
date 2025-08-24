use serde::{Deserialize, Serialize};
use solana_sdk::signature::Keypair;
use std::sync::Arc;
use tokio::sync::mpsc::Receiver;
use crate::{TpuConfig, SubmissionMode};

/// Operational mode for the submitter service
pub enum SubmitterMode {
    /// Full mode with integrated executor functionality
    Full {
        /// Keypair for signing execute transactions
        executor_keypair: Arc<Keypair>,
        /// Receiver for executable threads from Observer
        thread_receiver: Option<Receiver<ExecutableThread>>,
    },
    /// Replay-only mode for transaction propagation
    ReplayOnly,
}

/// Configuration for the submitter service
#[derive(Debug, Clone)]
pub struct SubmitterConfig {
    // Common fields
    /// RPC URL for blockchain interaction
    pub rpc_url: String,
    /// Enable replay functionality (consume from NATS)
    pub enable_replay: bool,
    /// NATS server connection URL
    pub nats_url: Option<String>,
    
    // Full mode fields (if present, enables full mode)
    /// Path to executor keypair file (enables full mode when present)
    pub executor_keypair_path: Option<String>,
    /// Whether to forgo executor commission
    pub forgo_executor_commission: bool,
    
    // Replay configuration
    /// Delay in milliseconds before replaying a transaction
    pub replay_delay_ms: u64,
    /// Maximum age of transactions to replay (in milliseconds)
    pub replay_max_age_ms: u64,
    /// Maximum number of replay attempts per transaction
    pub replay_max_attempts: u32,
    
    // Submission configuration
    /// TPU client configuration
    pub tpu_config: Option<TpuConfig>,
    /// Submission mode preference
    pub submission_mode: SubmissionMode,
    
    // Simulation configuration
    /// Whether to simulate transactions before submission
    pub simulate_before_submit: bool,
    /// Compute unit multiplier for simulation (e.g., 1.2 for 20% overhead)
    pub compute_unit_multiplier: f64,
    /// Maximum compute units allowed for transactions
    pub max_compute_units: u32,
    
    // Task management configuration
    /// Maximum number of concurrent thread processing tasks
    pub max_concurrent_threads: usize,
}

impl Default for SubmitterConfig {
    fn default() -> Self {
        Self {
            rpc_url: "http://localhost:8899".to_string(),
            enable_replay: false,
            nats_url: None,
            executor_keypair_path: None,
            forgo_executor_commission: false,
            replay_delay_ms: 30_000,        // 30 seconds
            replay_max_age_ms: 300_000,     // 5 minutes
            replay_max_attempts: 3,
            tpu_config: Some(TpuConfig::default()),
            submission_mode: SubmissionMode::default(), // TpuWithFallback
            simulate_before_submit: true,
            compute_unit_multiplier: 1.2,   // 20% overhead
            max_compute_units: 1_400_000,
            max_concurrent_threads: 50,     // Reasonable default
        }
    }
}

impl SubmitterConfig {
    /// Create a new config with environment variable overrides
    pub fn from_env() -> Self {
        let mut config = Self::default();
        
        // Override with environment variables if present
        if let Ok(val) = std::env::var("ANTEGEN_SIMULATE_BEFORE_SUBMIT") {
            config.simulate_before_submit = val.parse().unwrap_or(true);
        }
        
        if let Ok(val) = std::env::var("ANTEGEN_CU_MULTIPLIER") {
            if let Ok(multiplier) = val.parse::<f64>() {
                config.compute_unit_multiplier = multiplier;
            }
        }
        
        if let Ok(val) = std::env::var("ANTEGEN_MAX_COMPUTE_UNITS") {
            if let Ok(max_cu) = val.parse::<u32>() {
                config.max_compute_units = max_cu;
            }
        }
        
        if let Ok(val) = std::env::var("ANTEGEN_MAX_CONCURRENT_THREADS") {
            if let Ok(max_threads) = val.parse::<usize>() {
                config.max_concurrent_threads = max_threads;
            }
        }
        
        // Other common environment variables
        if let Ok(val) = std::env::var("ANTEGEN_RPC_URL") {
            config.rpc_url = val;
        }
        
        if let Ok(val) = std::env::var("ANTEGEN_ENABLE_REPLAY") {
            config.enable_replay = val.parse().unwrap_or(false);
        }
        
        if let Ok(val) = std::env::var("ANTEGEN_NATS_URL") {
            config.nats_url = Some(val);
        }
        
        if let Ok(val) = std::env::var("ANTEGEN_EXECUTOR_KEYPAIR_PATH") {
            config.executor_keypair_path = Some(val);
        }
        
        if let Ok(val) = std::env::var("ANTEGEN_FORGO_COMMISSION") {
            config.forgo_executor_commission = val.parse().unwrap_or(false);
        }
        
        config
    }
}

/// Executable thread ready for processing
#[derive(Debug, Clone)]
pub struct ExecutableThread {
    pub thread_pubkey: solana_sdk::pubkey::Pubkey,
    pub thread: antegen_thread_program::state::Thread,
    pub slot: u64,
}

/// Clock update event for triggering thread processing
#[derive(Debug, Clone)]
pub struct ClockUpdate {
    pub slot: u64,
    pub epoch: u64,
    pub unix_timestamp: i64,
}

/// Serializable version of ExecutableThread for queue storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredThread {
    pub thread_pubkey: solana_sdk::pubkey::Pubkey,
    pub slot: u64,
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