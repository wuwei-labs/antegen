use anyhow::Result;
use async_trait::async_trait;
use crossbeam::channel::{Receiver, TryRecvError};
use log::{debug, info};
use solana_program::pubkey::Pubkey;
use std::collections::HashSet;

use crate::events::{EventSource, ObservedEvent};

/// Event source that receives events from Carbon indexer
/// Carbon handles the complexity of different sources (RPC, Geyser, etc.)
///
/// By default, receives events for ALL threads. When specific threads are
/// subscribed via subscribe_thread(), it switches to selective filtering mode
/// and only passes through events for subscribed threads.
pub struct CarbonEventSource {
    /// Receiver for events from Carbon pipeline
    receiver: Receiver<ObservedEvent>,
    /// Threads we're interested in
    /// - Empty set = monitor all threads (default behavior)
    /// - Non-empty set = only monitor threads in this set (selective mode)
    subscribed_threads: HashSet<Pubkey>,
    /// Current slot
    current_slot: u64,
    /// Whether the source is running
    running: bool,
}

impl CarbonEventSource {
    pub fn new(receiver: Receiver<ObservedEvent>) -> Self {
        Self {
            receiver,
            subscribed_threads: HashSet::new(),
            current_slot: 0,
            running: false,
        }
    }

    /// Create from Carbon pipeline configuration
    /// Carbon handles:
    /// - RPC polling
    /// - Geyser integration
    /// - WebSocket subscriptions
    /// - Account decoders
    /// - Update filtering
    pub async fn from_carbon_config(config: CarbonConfig) -> Result<Self> {
        info!("Initializing Carbon event source with config: {:?}", config);

        // In a real implementation, this would:
        // 1. Initialize Carbon pipeline with the config
        // 2. Set up decoders for Thread and Builder accounts
        // 3. Create channel for receiving decoded events
        // 4. Start the Carbon pipeline

        // Carbon is initialized externally and sends events through the channel
        let (_tx, rx) = crossbeam::channel::unbounded();

        // Carbon would be configured to send ObservedEvent to tx
        // based on its pipeline processing

        Ok(Self::new(rx))
    }
}

#[async_trait]
impl EventSource for CarbonEventSource {
    async fn start(&mut self) -> Result<()> {
        info!("Starting Carbon event source");
        self.running = true;
        // Carbon pipeline would be started here
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        info!("Stopping Carbon event source");
        self.running = false;
        // Carbon pipeline would be stopped here
        Ok(())
    }

    async fn next_event(&mut self) -> Result<Option<ObservedEvent>> {
        if !self.running {
            return Ok(None);
        }

        // Receive events from Carbon pipeline
        match self.receiver.try_recv() {
            Ok(event) => {
                // Update current slot from account events
                let ObservedEvent::Account { slot, pubkey, .. } = &event;
                self.current_slot = *slot;

                // Filter based on subscriptions (only if selective filtering is enabled)
                // By default, we're interested in all threads
                if !self.subscribed_threads.is_empty() && !self.subscribed_threads.contains(pubkey)
                {
                    debug!("Filtering out unsubscribed account: {}", pubkey);
                    return Ok(None);
                }

                Ok(Some(event))
            }
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(anyhow::anyhow!("Carbon channel disconnected")),
        }
    }

    async fn subscribe_thread(&mut self, thread_pubkey: Pubkey) -> Result<()> {
        info!("Subscribing to specific thread: {}", thread_pubkey);
        // Enable selective filtering by adding to the set
        // If the set is empty, we receive all threads by default
        self.subscribed_threads.insert(thread_pubkey);
        // Could notify Carbon to start monitoring this specific account
        Ok(())
    }

    async fn unsubscribe_thread(&mut self, thread_pubkey: Pubkey) -> Result<()> {
        info!("Unsubscribing from thread: {}", thread_pubkey);
        self.subscribed_threads.remove(&thread_pubkey);
        // If the set becomes empty after removal, we go back to receiving all threads
        if self.subscribed_threads.is_empty() {
            debug!("No specific subscriptions - receiving all threads");
        }
        // Could notify Carbon to stop monitoring this specific account
        Ok(())
    }

    async fn get_current_slot(&self) -> Result<u64> {
        Ok(self.current_slot)
    }

    fn name(&self) -> &str {
        "CarbonEventSource"
    }
}

/// Configuration for Carbon data source
#[derive(Debug, Clone)]
pub struct CarbonConfig {
    /// Data source type for Carbon to use
    pub source_type: CarbonSourceType,
    /// Thread program ID to monitor
    pub thread_program_id: Pubkey,
    /// Network program ID to monitor
    pub network_program_id: Pubkey,
    /// Whether to start from latest slot or genesis
    pub start_from_latest: bool,
}

#[derive(Debug, Clone)]
pub enum CarbonSourceType {
    /// Use RPC polling
    RpcPolling {
        rpc_url: String,
        poll_interval_ms: u64,
    },
    /// Use Geyser plugin
    Geyser,
    /// Use WebSocket subscriptions
    WebSocket { ws_url: String },
}
