use anyhow::{anyhow, Result};
use log::{debug, error, info};
use solana_program::pubkey::Pubkey;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{channel, Sender};

use crate::events::{CarbonEventSource, EventSource, GeyserEventSource, ObservedEvent};
use crate::metrics::ObserverMetrics;

// Re-export from submitter crate
pub use antegen_submitter::{AccountUpdate, ClockUpdate, ExecutableThread};

/// Observer service that monitors events and notifies executor
pub struct ObserverService {
    /// Event source for blockchain events
    event_source: Box<dyn EventSource>,
    /// Observer keypair pubkey for validation
    observer_pubkey: Pubkey,
    /// Output channel for executor events
    pub event_sender: Sender<ExecutableThread>,
    /// Output channel for clock updates
    pub clock_sender: Sender<ClockUpdate>,
    /// Output channel for account updates
    pub account_sender: Sender<AccountUpdate>,
    /// Metrics collector
    metrics: ObserverMetrics,
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
    ) -> (
        Self,
        tokio::sync::mpsc::Receiver<ExecutableThread>,
        tokio::sync::mpsc::Receiver<ClockUpdate>,
        tokio::sync::mpsc::Receiver<AccountUpdate>,
    ) {
        let (thread_tx, thread_rx) = channel(1000); // Large buffer to handle bursts
        let (clock_tx, clock_rx) = channel(100); // Smaller buffer for clock updates
        let (account_tx, account_rx) = channel(1000); // Large buffer for account updates

        (
            Self {
                event_source,
                observer_pubkey,
                event_sender: thread_tx,
                clock_sender: clock_tx,
                account_sender: account_tx,
                metrics: ObserverMetrics::default(),
            },
            thread_rx,
            clock_rx,
            account_rx,
        )
    }

    /// Main service loop
    pub async fn run(&mut self) -> Result<()> {
        info!("OBSERVER: run() method called");
        info!(
            "OBSERVER: Service starting (observer={}, source={})",
            self.observer_pubkey,
            self.event_source.name()
        );

        // Start event source
        info!("OBSERVER: Starting event source...");
        self.event_source.start().await?;
        info!("OBSERVER: Event source started successfully, entering main loop");

        let mut event_count = 0;
        let mut loop_iterations = 0;
        loop {
            loop_iterations += 1;
            if loop_iterations % 100 == 1 {
                debug!(
                    "OBSERVER: Main loop iteration {}, events processed: {}",
                    loop_iterations, event_count
                );
            }
            // Get next event from event source
            match self.event_source.next_event().await? {
                Some(event) => {
                    event_count += 1;
                    info!(
                        "OBSERVER: Received event #{} from event source: {:?}",
                        event_count, event
                    );

                    if let Err(e) = self.process_event(event).await {
                        error!("OBSERVER: Error processing event: {}", e);
                    }
                }
                None => {
                    // No new events, brief pause
                    if loop_iterations % 1000 == 0 {
                        debug!(
                            "OBSERVER: No events available, continuing to poll (iteration {})",
                            loop_iterations
                        );
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    /// Process an observed event
    async fn process_event(&mut self, event: ObservedEvent) -> Result<()> {
        match event {
            ObservedEvent::ThreadExecutable {
                thread_pubkey,
                thread,
                slot,
            } => {
                let start = Instant::now();
                
                // Don't check trigger readiness here - let executor handle with accurate clock
                info!(
                    "OBSERVER: Received ThreadExecutable event for thread {} at slot {} (trigger: {:?})",
                    thread_pubkey,
                    slot,
                    thread.trigger
                );

                // Record thread triggered metric
                let trigger_type = match &thread.trigger {
                    antegen_thread_program::state::Trigger::Account { .. } => "account",
                    antegen_thread_program::state::Trigger::Cron { .. } => "cron",
                    antegen_thread_program::state::Trigger::Now => "now",
                    antegen_thread_program::state::Trigger::Slot { .. } => "slot",
                    antegen_thread_program::state::Trigger::Epoch { .. } => "epoch",
                    antegen_thread_program::state::Trigger::Interval { .. } => "interval",
                    antegen_thread_program::state::Trigger::Timestamp { .. } => "timestamp",
                };
                self.metrics.thread_triggered(trigger_type);

                let executable = ExecutableThread {
                    thread_pubkey,
                    thread,
                    slot,
                };

                info!(
                    "OBSERVER: Creating ExecutableThread event for thread {} to send to submitter",
                    thread_pubkey
                );

                // Send to submitter (will wait if channel is full)
                info!(
                    "OBSERVER: Sending ExecutableThread to submitter for thread {}",
                    thread_pubkey
                );
                if let Err(e) = self.event_sender.send(executable).await {
                    error!("OBSERVER: Failed to send to submitter: {}", e);
                    return Err(anyhow!("Submitter channel closed"));
                }
                info!(
                    "OBSERVER->SUBMITTER: Successfully sent ExecutableThread event for thread {} to submitter",
                    thread_pubkey
                );
                
                // Record trigger evaluation time
                self.metrics.record_trigger_evaluation(start.elapsed().as_secs_f64(), trigger_type);
            }
            ObservedEvent::ClockUpdate {
                slot,
                epoch,
                unix_timestamp,
            } => {
                debug!(
                    "OBSERVER: Received ClockUpdate - slot: {}, epoch: {}, timestamp: {}",
                    slot, epoch, unix_timestamp
                );

                // Record account update metric (clock is a special account)
                self.metrics.account_update_processed("clock");

                // Forward clock update to submitter
                let clock_update = ClockUpdate {
                    slot,
                    epoch,
                    unix_timestamp,
                };

                info!(
                    "OBSERVER: Forwarding ClockUpdate to submitter - slot: {}",
                    slot
                );
                if let Err(e) = self.clock_sender.send(clock_update).await {
                    error!("OBSERVER: Failed to send clock update to submitter: {}", e);
                    return Err(anyhow!("Submitter clock channel closed"));
                }
                info!(
                    "OBSERVER->SUBMITTER: Successfully sent ClockUpdate for slot {}",
                    slot
                );
            }
            ObservedEvent::AccountUpdate {
                pubkey,
                account,
                slot,
            } => {
                debug!(
                    "OBSERVER: Received AccountUpdate for {} at slot {}",
                    pubkey, slot
                );

                // Record account update metric
                self.metrics.account_update_processed("account");

                // Forward account update to submitter
                let account_update = AccountUpdate {
                    pubkey,
                    account,
                    slot,
                };

                debug!(
                    "OBSERVER: Forwarding AccountUpdate to submitter - pubkey: {}",
                    pubkey
                );
                if let Err(e) = self.account_sender.send(account_update).await {
                    error!("OBSERVER: Failed to send account update to submitter: {}", e);
                    return Err(anyhow!("Submitter account channel closed"));
                }
                debug!(
                    "OBSERVER->SUBMITTER: Successfully sent AccountUpdate for {}",
                    pubkey
                );
            }
            _ => {
                debug!("OBSERVER: Processing ThreadUpdate or other event type");
                self.metrics.account_update_processed("other");
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
