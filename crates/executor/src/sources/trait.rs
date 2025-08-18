use async_trait::async_trait;
use anyhow::Result;
use solana_program::pubkey::Pubkey;
use antegen_thread_program::state::Thread;

/// Notification when a thread should be claimed (from observer)
#[derive(Clone, Debug)]
pub struct ClaimableThread {
    pub thread_pubkey: Pubkey,
    pub thread: Thread,
    pub slot: u64,
}

/// Events that can be sent to the executor
#[derive(Clone, Debug)]
pub enum ExecutorEvent {
    /// A thread that should be claimed
    ClaimableThread(ClaimableThread),
    /// Clock update from the cluster
    ClockUpdate {
        slot: u64,
        epoch: u64,
        unix_timestamp: i64,
    },
}

/// Notification when a thread is claimed
#[derive(Clone, Debug)]
pub struct ClaimedThread {
    pub thread_pubkey: Pubkey,
    pub thread: Thread,
    pub claimed_at: i64,
    pub builder_id: u32,
}

/// Trait for receiving claimed threads from various sources
#[async_trait]
pub trait ClaimedThreadSource: Send + Sync {
    /// Receive next claimed thread from source
    async fn receive(&mut self) -> Result<Option<ClaimedThread>>;
    
    /// Acknowledge successful execution
    async fn ack(&mut self, thread_pubkey: &Pubkey) -> Result<()>;
    
    /// Report failure (for retry logic)
    async fn nack(&mut self, thread_pubkey: &Pubkey) -> Result<()>;
    
    /// Get source name for logging
    fn name(&self) -> &str;
}