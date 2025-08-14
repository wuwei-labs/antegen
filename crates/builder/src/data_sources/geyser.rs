use anyhow::Result;
use async_trait::async_trait;
use log::{debug, info};
use solana_program::pubkey::Pubkey;
use std::collections::HashSet;
use tokio::sync::mpsc::Receiver;

use crate::data_source::{DataSource, ObservedEvent};

/// Data source that receives events from Geyser plugin
pub struct GeyserDataSource {
    receiver: Receiver<ObservedEvent>,
    subscribed_threads: HashSet<Pubkey>,
    current_slot: u64,
    running: bool,
}

impl GeyserDataSource {
    pub fn new(receiver: Receiver<ObservedEvent>) -> Self {
        Self {
            receiver,
            subscribed_threads: HashSet::new(),
            current_slot: 0,
            running: false,
        }
    }
}

#[async_trait]
impl DataSource for GeyserDataSource {
    async fn start(&mut self) -> Result<()> {
        info!("Starting Geyser data source");
        self.running = true;
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        info!("Stopping Geyser data source");
        self.running = false;
        Ok(())
    }

    async fn next_event(&mut self) -> Result<Option<ObservedEvent>> {
        if !self.running {
            return Ok(None);
        }

        // Try to receive event from channel (non-blocking)
        match self.receiver.try_recv() {
            Ok(event) => {
                // Update current slot if this is a clock update
                if let ObservedEvent::ClockUpdate { slot, .. } = &event {
                    self.current_slot = *slot;
                }

                // Filter thread events based on subscriptions
                match &event {
                    ObservedEvent::ThreadExecutable { thread_pubkey, .. }
                    | ObservedEvent::ThreadUpdate { thread_pubkey, .. } => {
                        if !self.subscribed_threads.is_empty()
                            && !self.subscribed_threads.contains(thread_pubkey)
                        {
                            debug!("Filtering out unsubscribed thread: {}", thread_pubkey);
                            return Ok(None);
                        }
                    }
                    _ => {}
                }

                Ok(Some(event))
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                Err(anyhow::anyhow!("Geyser channel disconnected"))
            }
        }
    }

    async fn subscribe_thread(&mut self, thread_pubkey: Pubkey) -> Result<()> {
        info!("Subscribing to thread: {}", thread_pubkey);
        self.subscribed_threads.insert(thread_pubkey);
        Ok(())
    }

    async fn unsubscribe_thread(&mut self, thread_pubkey: Pubkey) -> Result<()> {
        info!("Unsubscribing from thread: {}", thread_pubkey);
        self.subscribed_threads.remove(&thread_pubkey);
        Ok(())
    }

    async fn get_current_slot(&self) -> Result<u64> {
        Ok(self.current_slot)
    }

    fn name(&self) -> &str {
        "GeyserDataSource"
    }
}
