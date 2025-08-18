use async_trait::async_trait;
use anyhow::Result;
use tokio::sync::mpsc::{Receiver, Sender, channel};
use log::debug;
use solana_program::pubkey::Pubkey;

use crate::sources::{ClaimedThreadSource, ClaimedThread};

/// Local in-memory queue for claimed threads
pub struct LocalQueue {
    receiver: Receiver<ClaimedThread>,
    sender: Sender<ClaimedThread>,
}

impl LocalQueue {
    pub fn new(buffer_size: usize) -> Self {
        let (sender, receiver) = channel(buffer_size);
        Self {
            receiver,
            sender,
        }
    }
    
    pub fn from_receiver(receiver: Receiver<ClaimedThread>, sender: Sender<ClaimedThread>) -> Self {
        Self {
            receiver,
            sender,
        }
    }
    
    pub fn sender(&self) -> Sender<ClaimedThread> {
        self.sender.clone()
    }
}

#[async_trait]
impl ClaimedThreadSource for LocalQueue {
    async fn receive(&mut self) -> Result<Option<ClaimedThread>> {
        // Check channel
        match self.receiver.try_recv() {
            Ok(claimed) => {
                debug!("Received claimed thread from local queue: {}", claimed.thread_pubkey);
                Ok(Some(claimed))
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                Err(anyhow::anyhow!("Local queue channel disconnected"))
            }
        }
    }
    
    async fn ack(&mut self, thread_pubkey: &Pubkey) -> Result<()> {
        debug!("Thread execution acknowledged: {}", thread_pubkey);
        // No-op for local queue (already removed from queue)
        Ok(())
    }
    
    async fn nack(&mut self, thread_pubkey: &Pubkey) -> Result<()> {
        debug!("Thread execution failed, not retrying locally: {}", thread_pubkey);
        // Could implement retry by re-queuing, but for now just log
        Ok(())
    }
    
    fn name(&self) -> &str {
        "LocalQueue"
    }
}