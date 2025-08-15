use anchor_lang::{AnchorSerialize, AnchorDeserialize};
use solana_program::pubkey::Pubkey;
use antegen_thread_program::state::Trigger;
use antegen_utils::thread::SerializableAccountMeta;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct BuiltTransaction {
    /// Unique transaction ID
    pub id: String,
    
    /// Serialized unsigned transaction
    pub partial_tx: Vec<u8>,
    
    /// Thread being executed
    pub thread_pubkey: Pubkey,
    
    /// Builder who created this
    pub builder_id: u32,
    
    /// Slot when built
    pub slot: u64,
    
    /// Unix timestamp
    pub timestamp: i64,
    
    /// Builder's signature for verification
    pub builder_signature: Vec<u8>,
    
    /// Estimated compute units
    pub compute_units: u32,
    
    /// Remaining accounts needed for execution
    pub remaining_accounts: Vec<SerializableAccountMeta>,
    
    /// Thread trigger type for scheduling
    pub trigger: Trigger,
    
    /// Last time the thread was started (from trigger_context)
    pub last_started_at: i64,
}

impl BuiltTransaction {
    pub fn new(
        thread_pubkey: Pubkey,
        builder_id: u32,
        partial_tx: Vec<u8>,
        remaining_accounts: Vec<SerializableAccountMeta>,
    ) -> Self {
        let timestamp = chrono::Utc::now().timestamp();
        let id = format!("{}_{}_{}", thread_pubkey, builder_id, timestamp);
        
        Self {
            id,
            partial_tx,
            thread_pubkey,
            builder_id,
            slot: 0, // To be set by builder
            timestamp,
            builder_signature: Vec::new(),
            compute_units: 200_000, // Default
            remaining_accounts,
            trigger: Trigger::Now, // Default, should be overridden by builder
            last_started_at: 0, // Default, should be overridden by builder
        }
    }
}