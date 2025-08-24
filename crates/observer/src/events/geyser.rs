use anyhow::Result;
use async_trait::async_trait;
use log::{debug, error, info};
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
        info!("GEYSER_SOURCE: Starting Geyser event source (running={})", self.running);
        self.running = true;
        info!("GEYSER_SOURCE: Event source started and ready to receive events");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        info!("Stopping Geyser event source");
        self.running = false;
        Ok(())
    }

    async fn next_event(&mut self) -> Result<Option<ObservedEvent>> {
        if !self.running {
            debug!("GEYSER_SOURCE: Not running, returning None");
            return Ok(None);
        }

        // Try to receive event from channel (non-blocking)
        match self.receiver.try_recv() {
            Ok(event) => {
                // Log the received event
                match &event {
                    ObservedEvent::ThreadExecutable { thread_pubkey, slot, .. } => {
                        info!("GEYSER_SOURCE: Received ThreadExecutable event from Geyser plugin: thread={}, slot={}", thread_pubkey, slot);
                    }
                    ObservedEvent::ThreadUpdate { thread_pubkey, slot, .. } => {
                        info!("GEYSER_SOURCE: Received ThreadUpdate event from Geyser plugin: thread={}, slot={}", thread_pubkey, slot);
                    }
                    ObservedEvent::ClockUpdate { slot, epoch, unix_timestamp } => {
                        info!("GEYSER_SOURCE: Received ClockUpdate event from Geyser plugin: slot={}, epoch={}, timestamp={}", slot, epoch, unix_timestamp);
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
                            info!("GEYSER_SOURCE: Filtering out unsubscribed thread: {} (subscribed threads: {})", thread_pubkey, self.subscribed_threads.len());
                            return Ok(None);
                        }
                        info!("GEYSER_SOURCE: Thread {} passed subscription filter, forwarding to observer", thread_pubkey);
                    }
                    _ => {}
                }

                Ok(Some(event))
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                // No events available right now
                Ok(None)
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                error!("GEYSER_SOURCE: Channel disconnected!");
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
