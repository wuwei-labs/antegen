use anyhow::Result;
use log::{debug, error, info, warn};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_program::pubkey::Pubkey;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{channel, Sender};

use crate::events::{CarbonEventSource, EventSource, GeyserEventSource, ObservedEvent};

// Re-export from executor crate
pub use antegen_executor::{ExecutableThread, ExecutorEvent};

/// Observer service that monitors events and notifies executor
pub struct ObserverService {
    /// Event source for blockchain events
    event_source: Box<dyn EventSource>,
    /// Observer keypair pubkey for validation
    observer_pubkey: Pubkey,
    /// RPC client for queries
    rpc_client: Arc<RpcClient>,
    /// Output channel for executor events
    pub event_sender: Sender<ExecutorEvent>,
    /// Buffer for events when channel is full
    event_buffer: VecDeque<ExecutorEvent>,
    /// Maximum buffer size before dropping events
    max_buffer_size: usize,
}

impl ObserverService {
    /// Create a Carbon event source
    pub fn create_carbon_source(
        receiver: tokio::sync::mpsc::Receiver<ObservedEvent>,
    ) -> Box<dyn EventSource> {
        Box::new(CarbonEventSource::new(receiver))
    }

    /// Create a Geyser event source
    pub fn create_geyser_source(
        receiver: tokio::sync::mpsc::Receiver<ObservedEvent>,
    ) -> Box<dyn EventSource> {
        Box::new(GeyserEventSource::new(receiver))
    }

    /// Create observer service (always with executor)
    pub fn new(
        event_source: Box<dyn EventSource>,
        observer_pubkey: Pubkey,
        rpc_client: Arc<RpcClient>,
    ) -> (Self, tokio::sync::mpsc::Receiver<ExecutorEvent>) {
        let (tx, rx) = channel(100);

        (
            Self {
                event_source,
                observer_pubkey,
                rpc_client,
                event_sender: tx,
                event_buffer: VecDeque::new(),
                max_buffer_size: 1000, // Buffer up to 1000 events
            },
            rx,
        )
    }

    /// Wait for thread config to exist with exponential backoff
    async fn wait_for_thread_config(&self) -> Result<()> {
        let config_pubkey =
            Pubkey::find_program_address(&[b"thread_config"], &antegen_thread_program::ID).0;
        info!(
            "OBSERVER: Waiting for thread config {} to be created...",
            config_pubkey
        );

        let mut attempts = 0;
        let mut delay_ms: u64 = 100;
        const MAX_DELAY_MS: u64 = 600_000;
        const BACKOFF_MULTIPLIER: f64 = 1.5;

        loop {
            match self.rpc_client.get_account(&config_pubkey).await {
                Ok(_account) => {
                    info!("OBSERVER: Thread config found");
                    return Ok(());
                }
                Err(e) => {
                    if attempts == 0 {
                        info!("OBSERVER: Thread config not found yet, will keep checking...");
                    } else if attempts % 10 == 0 {
                        info!(
                            "OBSERVER: Still waiting for thread config (attempt {})",
                            attempts
                        );
                    } else {
                        debug!(
                            "OBSERVER: Thread config check #{} failed: {:?}",
                            attempts, e
                        );
                    }
                }
            }

            attempts += 1;
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            delay_ms = ((delay_ms as f64 * BACKOFF_MULTIPLIER) as u64).min(MAX_DELAY_MS);
        }
    }

    /// Main service loop
    pub async fn run(&mut self) -> Result<()> {
        info!(
            "OBSERVER: Service starting (observer={}, source={})",
            self.observer_pubkey,
            self.event_source.name()
        );

        // Wait for thread config to exist
        self.wait_for_thread_config().await?;

        // Start event source
        self.event_source.start().await?;
        info!("OBSERVER: Event source started, monitoring for claimable threads");

        let mut event_count = 0;
        loop {
            // First, try to drain any buffered events
            self.drain_event_buffer().await;

            // Get next event from event source
            match self.event_source.next_event().await? {
                Some(event) => {
                    event_count += 1;
                    debug!("OBSERVER: Received event #{}", event_count);

                    if let Err(e) = self.process_event(event).await {
                        error!("OBSERVER: Error processing event: {}", e);
                    }
                }
                None => {
                    // No new events, brief pause
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    /// Try to drain buffered events to executor
    async fn drain_event_buffer(&mut self) {
        while let Some(event) = self.event_buffer.pop_front() {
            match self.event_sender.try_send(event.clone()) {
                Ok(()) => match &event {
                    ExecutorEvent::ExecutableThread(thread) => {
                        debug!(
                            "OBSERVER: Sent buffered thread {} to executor",
                            thread.thread_pubkey
                        );
                    }
                    ExecutorEvent::ClockUpdate { slot, .. } => {
                        debug!(
                            "OBSERVER: Sent buffered clock update (slot {}) to executor",
                            slot
                        );
                    }
                },
                Err(tokio::sync::mpsc::error::TrySendError::Full(msg)) => {
                    // Channel still full, put it back
                    self.event_buffer.push_front(msg);
                    break;
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    error!("OBSERVER: Executor channel closed!");
                    break;
                }
            }
        }

        if !self.event_buffer.is_empty() {
            debug!(
                "OBSERVER: {} events still buffered",
                self.event_buffer.len()
            );
        }
    }

    /// Process an observed event
    async fn process_event(&mut self, event: ObservedEvent) -> Result<()> {
        let executor_event = match event {
            ObservedEvent::ThreadExecutable {
                thread_pubkey,
                thread,
                slot,
            } => {
                // Don't check trigger readiness here - let executor handle with accurate clock
                info!(
                    "OBSERVER: Thread {} is potentially executable",
                    thread_pubkey
                );

                let executable = ExecutableThread {
                    thread_pubkey,
                    thread,
                    slot,
                };

                ExecutorEvent::ExecutableThread(executable)
            }
            ObservedEvent::ClockUpdate {
                slot,
                epoch,
                unix_timestamp,
            } => {
                debug!(
                    "OBSERVER: Clock update - slot: {}, epoch: {}, timestamp: {}",
                    slot, epoch, unix_timestamp
                );

                ExecutorEvent::ClockUpdate {
                    slot,
                    epoch,
                    unix_timestamp,
                }
            }
            _ => {
                debug!("OBSERVER: Ignoring other event type");
                return Ok(());
            }
        };

        // Try to send to executor
        match self.event_sender.try_send(executor_event.clone()) {
            Ok(()) => match &executor_event {
                ExecutorEvent::ExecutableThread(thread) => {
                    info!(
                        "OBSERVER: Notified executor about thread {}",
                        thread.thread_pubkey
                    );
                }
                ExecutorEvent::ClockUpdate { slot, .. } => {
                    debug!("OBSERVER: Sent clock update (slot {}) to executor", slot);
                }
            },
            Err(tokio::sync::mpsc::error::TrySendError::Full(msg)) => {
                // Channel full, buffer it
                if self.event_buffer.len() < self.max_buffer_size {
                    self.event_buffer.push_back(msg);
                    debug!(
                        "OBSERVER: Channel full, buffered event (buffer size: {})",
                        self.event_buffer.len()
                    );
                } else {
                    warn!("OBSERVER: Buffer full, dropping event");
                }
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                error!("OBSERVER: Executor channel closed!");
            }
        }

        Ok(())
    }
}

/// Configuration for observer service
#[derive(Debug, Clone)]
pub struct ObserverConfig {
    pub observer_pubkey: Pubkey,
    pub rpc_url: String,
}
