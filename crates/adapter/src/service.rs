use anyhow::{anyhow, Result};
use crossbeam::channel::{bounded, Sender};
use log::{debug, error, info};
use solana_program::pubkey::Pubkey;
use std::time::Duration;

use crate::events::{CarbonEventSource, EventSource, GeyserEventSource, ObservedEvent};
use crate::metrics::AdapterMetrics;

// Re-export AccountUpdate from processor crate
pub use antegen_processor::AccountUpdate;

/// Adapter service that bridges data sources and forwards account updates
/// All processing logic has been moved to the processor
pub struct AdapterService {
    /// Event source for blockchain events
    event_source: Box<dyn EventSource>,
    /// Adapter keypair pubkey for validation
    adapter_pubkey: Pubkey,
    /// Single output channel for all account updates
    pub account_sender: Sender<AccountUpdate>,
    /// Metrics collector
    metrics: AdapterMetrics,
}

impl AdapterService {
    /// Create a Carbon event source
    pub fn create_carbon_source(
        receiver: crossbeam::channel::Receiver<ObservedEvent>,
    ) -> Box<dyn EventSource> {
        Box::new(CarbonEventSource::new(receiver))
    }

    /// Create a Geyser event source
    pub fn create_geyser_source(
        receiver: crossbeam::channel::Receiver<ObservedEvent>,
    ) -> Box<dyn EventSource> {
        Box::new(GeyserEventSource::new(receiver))
    }

    /// Create adapter service with single account channel
    pub fn new(
        event_source: Box<dyn EventSource>,
        adapter_pubkey: Pubkey,
    ) -> (Self, crossbeam::channel::Receiver<AccountUpdate>) {
        // Single channel for all account updates
        let (account_tx, account_rx) = bounded(1000);

        (
            Self {
                event_source,
                adapter_pubkey,
                account_sender: account_tx,
                metrics: AdapterMetrics::default(),
            },
            account_rx,
        )
    }

    /// Main service loop
    pub async fn run(&mut self) -> Result<()> {
        info!("OBSERVER: run() method called");
        info!(
            "ADAPTER: Service starting (adapter={}, source={})",
            self.adapter_pubkey,
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

    /// Process an observed event - simplified to just forward accounts
    async fn process_event(&mut self, event: ObservedEvent) -> Result<()> {
        match event {
            ObservedEvent::Account {
                pubkey,
                account,
                slot,
            } => {
                debug!(
                    "OBSERVER: Received account update for {} at slot {}",
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

                // Use crossbeam's synchronous send (no await needed)
                if let Err(e) = self.account_sender.send(account_update) {
                    error!("OBSERVER: Failed to send account update to submitter: {}", e);
                    return Err(anyhow!("Submitter account channel closed"));
                }
                
                debug!(
                    "OBSERVER->SUBMITTER: Forwarded account update for {}",
                    pubkey
                );
            }
        }

        Ok(())
    }
}

/// Configuration for adapter service
#[derive(Debug, Clone)]
pub struct ObserverConfig {
    pub adapter_pubkey: Pubkey,
    pub rpc_url: String,
}
