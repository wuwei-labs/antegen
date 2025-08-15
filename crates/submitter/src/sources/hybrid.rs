use async_trait::async_trait;
use anyhow::Result;
use log::debug;

use crate::sources::{TransactionSource, LocalQueue, NatsConsumer};
use crate::types::BuiltTransaction;

/// Hybrid transaction source that prioritizes local queue over NATS
pub struct HybridSource {
    local_queue: LocalQueue,
    nats_consumer: NatsConsumer,
}

impl HybridSource {
    pub async fn new(
        buffer_size: usize,
        nats_url: &str,
        consumer_name: &str,
    ) -> Result<Self> {
        let local_queue = LocalQueue::new(buffer_size);
        let nats_consumer = NatsConsumer::new(nats_url, consumer_name, None).await?;
        
        Ok(Self {
            local_queue,
            nats_consumer,
        })
    }
    
    pub fn local_sender(&self) -> tokio::sync::mpsc::Sender<BuiltTransaction> {
        self.local_queue.sender()
    }
}

#[async_trait]
impl TransactionSource for HybridSource {
    async fn receive(&mut self) -> Result<Option<BuiltTransaction>> {
        // Always check local queue first (priority)
        if let Some(tx) = self.local_queue.receive().await? {
            debug!("Received transaction from local queue: {}", tx.id);
            return Ok(Some(tx));
        }
        
        // If local queue is empty, check NATS
        match self.nats_consumer.receive().await? {
            Some(tx) => {
                debug!("Received transaction from NATS: {}", tx.id);
                Ok(Some(tx))
            }
            None => Ok(None),
        }
    }
    
    async fn ack(&mut self, tx_id: &str) -> Result<()> {
        // Try to ack in both sources (one will be no-op)
        let _ = self.local_queue.ack(tx_id).await;
        let _ = self.nats_consumer.ack(tx_id).await;
        Ok(())
    }
    
    async fn nack(&mut self, tx_id: &str) -> Result<()> {
        // Try to nack in both sources (one will be no-op)
        let _ = self.local_queue.nack(tx_id).await;
        let _ = self.nats_consumer.nack(tx_id).await;
        Ok(())
    }
    
    fn name(&self) -> &str {
        "HybridSource"
    }
}