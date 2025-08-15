use async_trait::async_trait;
use anyhow::Result;
use tokio::sync::mpsc::{Receiver, Sender, channel};
use log::debug;

use crate::sources::TransactionSource;
use crate::types::BuiltTransaction;

/// Local in-memory queue for transactions
pub struct LocalQueue {
    receiver: Receiver<BuiltTransaction>,
    sender: Sender<BuiltTransaction>,
}

impl LocalQueue {
    pub fn new(buffer_size: usize) -> Self {
        let (sender, receiver) = channel(buffer_size);
        Self {
            receiver,
            sender,
        }
    }
    
    pub fn from_receiver(receiver: Receiver<BuiltTransaction>, sender: Sender<BuiltTransaction>) -> Self {
        Self {
            receiver,
            sender,
        }
    }
    
    pub fn sender(&self) -> Sender<BuiltTransaction> {
        self.sender.clone()
    }
}

#[async_trait]
impl TransactionSource for LocalQueue {
    async fn receive(&mut self) -> Result<Option<BuiltTransaction>> {
        // Check channel
        match self.receiver.try_recv() {
            Ok(tx) => {
                debug!("Received transaction from local queue: {}", tx.id);
                Ok(Some(tx))
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                Err(anyhow::anyhow!("Local queue channel disconnected"))
            }
        }
    }
    
    async fn ack(&mut self, tx_id: &str) -> Result<()> {
        debug!("Transaction acknowledged: {}", tx_id);
        // No-op for local queue (already removed from queue)
        Ok(())
    }
    
    async fn nack(&mut self, tx_id: &str) -> Result<()> {
        debug!("Transaction failed, not retrying locally: {}", tx_id);
        // Could implement retry by re-queuing, but for now just log
        Ok(())
    }
    
    fn name(&self) -> &str {
        "LocalQueue"
    }
}