use anyhow::Result;
use log::{debug, error, info, warn};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_program::pubkey::Pubkey;
use std::sync::Arc;
use tokio::sync::mpsc::{channel, Sender};

use crate::data_source::{DataSource, ObservedEvent};
use crate::data_sources::{CarbonDataSource, GeyserDataSource};
use crate::nats_publisher::NatsPublisher;
use antegen_submitter::BuiltTransaction;
use antegen_thread_program::state::{Thread, Trigger};

/// Builder service that observes events and builds transactions
pub struct BuilderService {
    /// Data source for blockchain events
    data_source: Box<dyn DataSource>,
    /// Builder ID
    builder_id: u32,
    /// RPC client for additional queries
    rpc_client: Arc<RpcClient>,
    /// Output channel for built transactions (Worker mode)
    tx_sender: Option<Sender<BuiltTransaction>>,
    /// NATS publisher (Builder mode)
    nats_publisher: Option<NatsPublisher>,
}

impl BuilderService {
    /// Create a Carbon data source
    pub fn create_carbon_source(
        receiver: tokio::sync::mpsc::Receiver<ObservedEvent>,
    ) -> Box<dyn DataSource> {
        Box::new(CarbonDataSource::new(receiver))
    }
    
    /// Create a Geyser data source
    pub fn create_geyser_source(
        receiver: tokio::sync::mpsc::Receiver<ObservedEvent>,
    ) -> Box<dyn DataSource> {
        Box::new(GeyserDataSource::new(receiver))
    }
    
    /// Create builder for Worker mode (outputs to channel)
    pub fn new_worker(
        data_source: Box<dyn DataSource>,
        builder_id: u32,
        rpc_client: Arc<RpcClient>,
    ) -> (Self, Sender<BuiltTransaction>) {
        let (tx, _rx) = channel(100);
        let sender = tx.clone();

        (
            Self {
                data_source,
                builder_id,
                rpc_client,
                tx_sender: Some(tx),
                nats_publisher: None,
            },
            sender,
        )
    }

    /// Create builder for Builder mode (publishes to NATS)
    pub async fn new_builder(
        data_source: Box<dyn DataSource>,
        builder_id: u32,
        rpc_client: Arc<RpcClient>,
        nats_url: &str,
    ) -> Result<Self> {
        let nats_publisher = Some(NatsPublisher::new(nats_url, None, None).await?);

        Ok(Self {
            data_source,
            builder_id,
            rpc_client,
            tx_sender: None,
            nats_publisher,
        })
    }

    /// Main service loop
    pub async fn run(&mut self) -> Result<()> {
        info!("Starting builder service (builder_id: {})", self.builder_id);

        // Start data source
        self.data_source.start().await?;

        loop {
            // Get next event from data source
            match self.data_source.next_event().await? {
                Some(event) => {
                    if let Err(e) = self.process_event(event).await {
                        error!("Error processing event: {}", e);
                    }
                }
                None => {
                    // No events available, brief pause
                    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                }
            }
        }
    }

    /// Process a single event
    async fn process_event(&mut self, event: ObservedEvent) -> Result<()> {
        match event {
            ObservedEvent::ThreadExecutable {
                thread_pubkey,
                thread,
                slot,
            } => {
                debug!("Thread executable: {} at slot {}", thread_pubkey, slot);
                self.handle_executable_thread(thread_pubkey, thread, slot)
                    .await?;
            }
            ObservedEvent::ClockUpdate { slot, .. } => {
                debug!("Clock update: slot {}", slot);
                // Could trigger time-based threads here
            }
            _ => {
                // Other events we might not care about for building
            }
        }
        Ok(())
    }

    /// Handle an executable thread
    async fn handle_executable_thread(
        &mut self,
        thread_pubkey: Pubkey,
        thread: Thread,
        slot: u64,
    ) -> Result<()> {
        // Check if thread is truly executable
        if !self.is_thread_executable(&thread, slot).await? {
            return Ok(());
        }

        // Check if we should claim this thread
        if thread.builders.contains(&self.builder_id) {
            debug!("Already claimed thread: {}", thread_pubkey);
            return Ok(());
        }

        // Try to claim the thread
        match self.claim_thread(thread_pubkey).await {
            Ok(()) => {
                info!("Claimed thread: {}", thread_pubkey);
            }
            Err(e) => {
                warn!("Failed to claim thread {}: {}", thread_pubkey, e);
                return Ok(());
            }
        }

        // Build the transaction
        match crate::thread_exec::build_thread_exec_tx(
            self.rpc_client.clone(),
            &self.get_builder_keypair()?,
            slot,
            thread.clone(),
            thread_pubkey,
            self.builder_id,
        )
        .await?
        {
            Some(tx) => {
                info!("Built transaction for thread: {}", thread_pubkey);

                // Create BuiltTransaction
                let built_tx = BuiltTransaction::new(
                    thread_pubkey,
                    self.builder_id,
                    bincode::serialize(&tx)?,
                    vec![], // remaining_accounts would be populated from thread_exec
                );

                // Output transaction
                self.output_transaction(built_tx).await?;
            }
            None => {
                debug!("No transaction built for thread: {}", thread_pubkey);
            }
        }

        Ok(())
    }

    /// Check if thread is executable
    async fn is_thread_executable(&self, thread: &Thread, slot: u64) -> Result<bool> {
        if thread.paused {
            return Ok(false);
        }

        if !thread.builders.is_empty() {
            // Someone is already building
            return Ok(false);
        }

        // Check trigger
        match &thread.trigger {
            Trigger::Now => Ok(true),
            Trigger::Slot { slot: trigger_slot } => Ok(slot >= *trigger_slot),
            _ => Ok(false), // Other triggers need more complex logic
        }
    }

    /// Claim a thread for building
    async fn claim_thread(&self, thread_pubkey: Pubkey) -> Result<()> {
        // This would create and submit a thread_claim transaction
        // For now, we'll just log
        info!("Would claim thread: {}", thread_pubkey);
        Ok(())
    }

    /// Output built transaction based on mode
    async fn output_transaction(&mut self, tx: BuiltTransaction) -> Result<()> {
        if let Some(sender) = &self.tx_sender {
            // Worker mode: send to channel
            sender.send(tx).await?;
        } else if let Some(publisher) = &self.nats_publisher {
            // Builder mode: publish to NATS
            publisher.publish(&tx).await?;
        } else {
            return Err(anyhow::anyhow!("No output configured"));
        }
        Ok(())
    }

    /// Get builder keypair (placeholder)
    fn get_builder_keypair(&self) -> Result<solana_sdk::signer::keypair::Keypair> {
        // In real implementation, load from file or config
        Ok(solana_sdk::signer::keypair::Keypair::new())
    }
}

/// Configuration for builder service
#[derive(Debug, Clone)]
pub struct BuilderConfig {
    pub builder_id: u32,
    pub rpc_url: String,
    pub nats_url: Option<String>,
    pub keypair_path: String,
}
