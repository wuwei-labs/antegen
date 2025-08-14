use async_trait::async_trait;
use anyhow::Result;
use crate::types::BuiltTransaction;

/// Trait for receiving transactions from various sources
#[async_trait]
pub trait TransactionSource: Send + Sync {
    /// Receive next transaction from source
    async fn receive(&mut self) -> Result<Option<BuiltTransaction>>;
    
    /// Acknowledge successful processing
    async fn ack(&mut self, tx_id: &str) -> Result<()>;
    
    /// Report failure (for retry logic)
    async fn nack(&mut self, tx_id: &str) -> Result<()>;
    
    /// Get source name for logging
    fn name(&self) -> &str;
}