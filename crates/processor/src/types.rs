use serde::{Deserialize, Serialize};
use solana_sdk::{account::Account, pubkey::Pubkey};

/// Configuration for the processor service
#[derive(Debug, Clone)]
pub struct ProcessorConfig {
    /// RPC URL for blockchain interaction
    pub rpc_url: String,
    
    /// Path to executor keypair file
    pub executor_keypair_path: String,
    
    /// Whether to forgo executor commission
    pub forgo_executor_commission: bool,
    
    
    /// Simulation configuration
    pub simulate_before_submit: bool,
    pub compute_unit_multiplier: f64,
    pub max_compute_units: u32,
    
    /// Task management configuration
    pub max_concurrent_threads: usize,
    
}

impl Default for ProcessorConfig {
    fn default() -> Self {
        Self {
            rpc_url: "http://localhost:8899".to_string(),
            executor_keypair_path: String::new(),
            forgo_executor_commission: false,
            simulate_before_submit: true,
            compute_unit_multiplier: 1.2,
            max_compute_units: 1_400_000,
            max_concurrent_threads: 50,
        }
    }
}

impl ProcessorConfig {
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
        
        if let Ok(val) = std::env::var("ANTEGEN_RPC_URL") {
            config.rpc_url = val;
        }
        
        if let Ok(val) = std::env::var("ANTEGEN_EXECUTOR_KEYPAIR_PATH") {
            config.executor_keypair_path = val;
        }
        
        if let Ok(val) = std::env::var("ANTEGEN_FORGO_COMMISSION") {
            config.forgo_executor_commission = val.parse().unwrap_or(false);
        }
        
        config
    }
}

/// Minimal thread data needed for execution (after fiber is fetched)
#[derive(Debug, Clone)]
pub struct ThreadExecutionData {
    /// Thread authority (needed for authorization checks)
    pub authority: Pubkey,
    /// Nonce account if using durable transactions
    pub nonce_account: Pubkey,
    /// Whether the thread has a nonce account
    pub has_nonce: bool,
    /// Current execution index (for fiber lookup)
    pub exec_index: u8,
}

/// Executable thread ready for processing
#[derive(Debug, Clone)]
pub struct ExecutableThread {
    pub thread_pubkey: Pubkey,
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

/// Account update event from observer
#[derive(Debug, Clone)]
pub struct AccountUpdate {
    pub pubkey: Pubkey,
    pub account: Account,
    pub slot: u64,
}

/// Serializable version of ExecutableThread for queue storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredThread {
    pub thread_pubkey: Pubkey,
    pub slot: u64,
}