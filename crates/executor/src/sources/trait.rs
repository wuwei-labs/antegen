use async_trait::async_trait;
use anyhow::Result;
use solana_program::pubkey::Pubkey;
use antegen_thread_program::state::Thread;

/// Notification when a thread is ready to execute (from observer)
#[derive(Clone, Debug)]
pub struct ExecutableThread {
    pub thread_pubkey: Pubkey,
    pub thread: Thread,
    pub slot: u64,
}

/// Events that can be sent to the executor
#[derive(Clone, Debug)]
pub enum ExecutorEvent {
    /// A thread that is ready to execute
    ExecutableThread(ExecutableThread),
    /// Clock update from the cluster
    ClockUpdate {
        slot: u64,
        epoch: u64,
        unix_timestamp: i64,
    },
}

/// Thread ready for execution from RPC scanning
#[derive(Clone, Debug)]
pub struct ScannedThread {
    pub thread_pubkey: Pubkey,
    pub thread: Thread,
    pub discovered_at: i64,
}

/// Trait for receiving executable threads from various sources
#[async_trait]
pub trait ThreadSource: Send + Sync {
    /// Receive next executable thread from source
    async fn receive(&mut self) -> Result<Option<ScannedThread>>;
    
    /// Acknowledge successful execution
    async fn ack(&mut self, thread_pubkey: &Pubkey) -> Result<()>;
    
    /// Report failure (for retry logic)
    async fn nack(&mut self, thread_pubkey: &Pubkey) -> Result<()>;
    
    /// Get source name for logging
    fn name(&self) -> &str;
}