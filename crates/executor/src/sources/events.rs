use anyhow::Result;
use tokio::sync::mpsc::{Receiver, Sender, channel};
use log::debug;

use crate::sources::ExecutorEvent;

/// Event queue for receiving events from observer
pub struct EventQueue {
    receiver: Receiver<ExecutorEvent>,
    sender: Sender<ExecutorEvent>,
}

impl EventQueue {
    pub fn new(buffer_size: usize) -> Self {
        let (sender, receiver) = channel(buffer_size);
        Self {
            receiver,
            sender,
        }
    }
    
    pub fn from_channel(receiver: Receiver<ExecutorEvent>, sender: Sender<ExecutorEvent>) -> Self {
        Self {
            receiver,
            sender,
        }
    }
    
    pub fn sender(&self) -> Sender<ExecutorEvent> {
        self.sender.clone()
    }
    
    /// Try to receive the next event
    pub async fn receive(&mut self) -> Result<Option<ExecutorEvent>> {
        match self.receiver.try_recv() {
            Ok(event) => {
                debug!("EventQueue: Received event");
                Ok(Some(event))
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                Err(anyhow::anyhow!("Event queue channel disconnected"))
            }
        }
    }
    
    /// Blocking receive for the next event
    pub async fn receive_blocking(&mut self) -> Result<Option<ExecutorEvent>> {
        match self.receiver.recv().await {
            Some(event) => {
                debug!("EventQueue: Received event (blocking)");
                Ok(Some(event))
            }
            None => {
                Err(anyhow::anyhow!("Event queue channel closed"))
            }
        }
    }
}