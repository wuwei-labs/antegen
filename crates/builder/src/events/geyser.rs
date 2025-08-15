use anyhow::Result;
use async_trait::async_trait;
use log::{debug, info};
use solana_program::pubkey::Pubkey;
use std::collections::HashSet;
use tokio::sync::mpsc::Receiver;

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
        info!("Starting Geyser event source");
        self.running = true;
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        info!("Stopping Geyser event source");
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
                // Log the received event
                match &event {
                    ObservedEvent::ThreadExecutable { thread_pubkey, slot, .. } => {
                        debug!("Received ThreadExecutable event from Geyser: thread={}, slot={}", thread_pubkey, slot);
                    }
                    ObservedEvent::ThreadUpdate { thread_pubkey, slot, .. } => {
                        debug!("Received ThreadUpdate event from Geyser: thread={}, slot={}", thread_pubkey, slot);
                    }
                    ObservedEvent::BuilderUpdate { builder_pubkey, slot, .. } => {
                        debug!("Received BuilderUpdate event from Geyser: builder={}, slot={}", builder_pubkey, slot);
                    }
                    ObservedEvent::ClockUpdate { slot, epoch, unix_timestamp } => {
                        debug!("Received ClockUpdate event from Geyser: slot={}, epoch={}, timestamp={}", slot, epoch, unix_timestamp);
                        self.current_slot = *slot;
                    }
                    ObservedEvent::AccountUpdate { pubkey, slot, .. } => {
                        debug!("Received AccountUpdate event from Geyser: account={}, slot={}", pubkey, slot);
                    }
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
        "GeyserEventSource"
    }
}
