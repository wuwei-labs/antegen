use anyhow::Result;
use async_trait::async_trait;
use solana_program::pubkey::Pubkey;

use super::types::ObservedEvent;

/// Trait for different event sources (Geyser, Carbon, RPC polling, etc.)
#[async_trait]
pub trait EventSource: Send + Sync {
    /// Start receiving events from the event source
    async fn start(&mut self) -> Result<()>;

    /// Stop receiving events
    async fn stop(&mut self) -> Result<()>;

    /// Get next event (non-blocking)
    async fn next_event(&mut self) -> Result<Option<ObservedEvent>>;

    /// Subscribe to specific thread updates
    async fn subscribe_thread(&mut self, thread_pubkey: Pubkey) -> Result<()>;

    /// Unsubscribe from thread updates
    async fn unsubscribe_thread(&mut self, thread_pubkey: Pubkey) -> Result<()>;

    /// Get current slot
    async fn get_current_slot(&self) -> Result<u64>;

    /// Get event source name for logging
    fn name(&self) -> &str;
}