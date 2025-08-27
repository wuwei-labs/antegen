use anyhow::Result;
use async_trait::async_trait;
use crossbeam::channel::{Receiver, TryRecvError};
use log::error;
use solana_program::pubkey::Pubkey;
use std::collections::HashSet;

use crate::events::{EventSource, ObservedEvent};

/// Event source that receives events from Geyser plugin
pub struct GeyserEventSource {
    receiver: Receiver<ObservedEvent>,
    subscribed_threads: HashSet<Pubkey>,
    current_slot: u64,
    running: bool,
}

impl GeyserEventSource {
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
impl EventSource for GeyserEventSource {
    async fn start(&mut self) -> Result<()> {
        self.running = true;
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.running = false;
        Ok(())
    }

    async fn next_event(&mut self) -> Result<Option<ObservedEvent>> {
        if !self.running {
            // Not running
            return Ok(None);
        }

        // Try to receive event from channel (non-blocking)
        match self.receiver.try_recv() {
            Ok(event) => {
                // Log the received event
                match &event {
                    ObservedEvent::Account {  slot, .. } => {
                        // Account update
                        // Update current slot if it's newer
                        if *slot > self.current_slot {
                            self.current_slot = *slot;
                        }
                    }
                }

                Ok(Some(event))
            }
            Err(TryRecvError::Empty) => {
                // No events available right now
                Ok(None)
            }
            Err(TryRecvError::Disconnected) => {
                error!("GEYSER_SOURCE: Channel disconnected!");
                Err(anyhow::anyhow!("Geyser channel disconnected"))
            }
        }
    }

    async fn subscribe_thread(&mut self, thread_pubkey: Pubkey) -> Result<()> {
        self.subscribed_threads.insert(thread_pubkey);
        Ok(())
    }

    async fn unsubscribe_thread(&mut self, thread_pubkey: Pubkey) -> Result<()> {
        self.subscribed_threads.remove(&thread_pubkey);
        Ok(())
    }

    async fn get_current_slot(&self) -> Result<u64> {
        Ok(self.current_slot)
    }

    fn name(&self) -> &str {
        "GeyserEventSource"
    }
}
