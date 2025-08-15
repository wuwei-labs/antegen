use anyhow::Result;
use log::{debug, error, info, warn};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_program::pubkey::Pubkey;
use solana_sdk::signer::keypair::Keypair;
use std::sync::Arc;
use tokio::sync::mpsc::{channel, Sender};

use crate::events::{EventSource, ObservedEvent, CarbonEventSource, GeyserEventSource};
use crate::outputs::NatsPublisher;
use crate::transaction::exec::build_thread_exec_tx;
use antegen_submitter::BuiltTransaction;
use antegen_thread_program::state::Thread;
use antegen_network_program::state::Builder;

/// Builder service that observes events and builds transactions
pub struct BuilderService {
    /// Event source for blockchain events
    event_source: Box<dyn EventSource>,
    /// Builder ID
    builder_id: u32,
    /// RPC client for additional queries
    rpc_client: Arc<RpcClient>,
    /// Output channel for built transactions (Worker mode)
    tx_sender: Option<Sender<BuiltTransaction>>,
    /// NATS publisher (Builder mode)
    nats_publisher: Option<NatsPublisher>,
    /// Builder keypair for signing transactions
    keypair: Arc<Keypair>,
}

impl BuilderService {
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
    
    /// Create builder for Worker mode (outputs to channel)
    pub fn new_worker(
        event_source: Box<dyn EventSource>,
        builder_id: u32,
        rpc_client: Arc<RpcClient>,
        keypair: Arc<Keypair>,
    ) -> (Self, Sender<BuiltTransaction>) {
        let (tx, _rx) = channel(100);
        let sender = tx.clone();

        (
            Self {
                event_source,
                builder_id,
                rpc_client,
                tx_sender: Some(tx),
                nats_publisher: None,
                keypair,
            },
            sender,
        )
    }
    
    /// Create builder with explicit output channels
    pub async fn new_with_outputs(
        event_source: Box<dyn EventSource>,
        builder_id: u32,
        rpc_client: Arc<RpcClient>,
        keypair: Arc<Keypair>,
        local_sender: Option<Sender<BuiltTransaction>>,
        nats_url: Option<String>,
    ) -> Result<Self> {
        let nats_publisher = if let Some(url) = nats_url {
            Some(NatsPublisher::new(&url, None, None).await?)
        } else {
            None
        };
        
        Ok(Self {
            event_source,
            builder_id,
            rpc_client,
            tx_sender: local_sender,
            nats_publisher,
            keypair,
        })
    }

    /// Create builder for Builder mode (publishes to NATS)
    pub async fn new_builder(
        event_source: Box<dyn EventSource>,
        builder_id: u32,
        rpc_client: Arc<RpcClient>,
        keypair: Arc<Keypair>,
        nats_url: &str,
    ) -> Result<Self> {
        let nats_publisher = Some(NatsPublisher::new(nats_url, None, None).await?);

        Ok(Self {
            event_source,
            builder_id,
            rpc_client,
            tx_sender: None,
            nats_publisher,
            keypair,
        })
    }

    /// Wait for builder account to exist with exponential backoff
    async fn wait_for_builder_account(&self) -> Result<()> {
        use antegen_network_program::state::Builder;
        
        let builder_pubkey = Builder::pubkey(self.builder_id);
        info!("BUILDER: Waiting for builder account {} to be created...", builder_pubkey);
        
        let mut attempts = 0;
        let mut delay_ms: u64 = 100; // Start with 100ms
        const MAX_DELAY_MS: u64 = 600_000; // Max 10 minutes between checks
        const BACKOFF_MULTIPLIER: f64 = 1.5; // Exponential backoff factor
        
        loop {
            match self.rpc_client.get_account(&builder_pubkey).await {
                Ok(account) => {
                    // Try to deserialize to ensure it's a valid Builder account
                    match Builder::try_from(account.data.as_slice()) {
                        Ok(builder) => {
                            info!("BUILDER: Builder account found and verified after {} attempts (id={}, authority={}, signatory={})", 
                                  attempts, builder.id, builder.authority, builder.signatory);
                            return Ok(());
                        }
                        Err(e) => {
                            warn!("BUILDER: Account exists but failed to deserialize as Builder: {:?}", e);
                        }
                    }
                }
                Err(e) => {
                    if attempts == 0 {
                        info!("BUILDER: Builder account not found yet, will keep checking...");
                    } else if attempts % 10 == 0 {
                        info!("BUILDER: Still waiting for builder account (attempt {}, next check in {}s)", 
                              attempts, delay_ms / 1000);
                    } else {
                        debug!("BUILDER: Builder account check #{} failed: {:?}", attempts, e);
                    }
                }
            }
            
            attempts += 1;
            tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
            
            // Exponential backoff with cap
            delay_ms = ((delay_ms as f64 * BACKOFF_MULTIPLIER) as u64).min(MAX_DELAY_MS);
        }
    }

    /// Main service loop
    pub async fn run(&mut self) -> Result<()> {
        info!("BUILDER: Service starting (builder_id={}, source={})", 
              self.builder_id, self.event_source.name());

        // Wait for builder account to exist before processing events
        self.wait_for_builder_account().await?;

        // Start event source
        self.event_source.start().await?;
        info!("BUILDER: Event source started, entering main loop");

        let mut event_count = 0;
        loop {
            // Get next event from event source
            match self.event_source.next_event().await? {
                Some(event) => {
                    event_count += 1;
                    info!("BUILDER: Received event #{}", event_count);
                    
                    match self.process_event(event).await {
                        Ok(()) => {
                            info!("BUILDER: Event #{} processed successfully", event_count);
                        }
                        Err(e) => {
                            error!("BUILDER: Event #{} processing failed: {}", event_count, e);
                        }
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
                info!("BUILDER: Processing ThreadExecutable for thread {}", thread_pubkey);
                self.handle_executable_thread(thread_pubkey, thread, slot).await?;
            }
            _ => {
                debug!("BUILDER: Ignoring non-thread event");
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
        info!("BUILDER: Building real transaction for thread {}", thread_pubkey);
        
        // Build the actual thread execution transaction
        match build_thread_exec_tx(
            self.rpc_client.clone(),
            &*self.keypair,
            slot,
            thread.clone(),
            thread_pubkey,
            self.builder_id,
        ).await? {
            Some(versioned_tx) => {
                info!("BUILDER: Successfully built transaction for thread {}", thread_pubkey);
                
                // Serialize the transaction
                let tx_bytes = bincode::serialize(&versioned_tx)?;
                
                // Extract timing info
                let last_started_at = match &thread.trigger_context {
                    antegen_thread_program::state::TriggerContext::Timestamp { prev, .. } => *prev,
                    antegen_thread_program::state::TriggerContext::Block { prev, .. } => *prev as i64,
                    antegen_thread_program::state::TriggerContext::Account { .. } => 0,
                };
                
                // Create BuiltTransaction with real transaction bytes
                let mut built_tx = BuiltTransaction::new(
                    thread_pubkey,
                    self.builder_id,
                    tx_bytes,
                    vec![],
                );
                
                built_tx.trigger = thread.trigger.clone();
                built_tx.last_started_at = last_started_at;
                built_tx.slot = slot;
                
                info!("BUILDER: Sending real tx {} to outputs (signature: {:?})", 
                      built_tx.id, versioned_tx.signatures[0]);
                self.output_transaction(built_tx).await?;
                info!("BUILDER: Transaction sent successfully");
            }
            None => {
                warn!("BUILDER: Could not build transaction for thread {} (may be pending or invalid)", thread_pubkey);
            }
        }
        
        Ok(())
    }


    /// Claim a thread for building
    async fn claim_thread(&self, thread_pubkey: Pubkey) -> Result<()> {
        // This would create and submit a thread_claim transaction
        // For now, we'll just log
        info!("Would claim thread: {}", thread_pubkey);
        Ok(())
    }

    /// Output built transaction to all configured outputs
    async fn output_transaction(&mut self, tx: BuiltTransaction) -> Result<()> {
        let tx_id = tx.id.clone();
        
        // Send to local channel if configured
        if let Some(sender) = &self.tx_sender {
            info!("BUILDER->SUBMITTER: Sending tx {} via channel", tx_id);
            
            match sender.send(tx.clone()).await {
                Ok(()) => {
                    info!("BUILDER->SUBMITTER: Successfully sent tx {}", tx_id);
                }
                Err(e) => {
                    error!("BUILDER->SUBMITTER: Channel send failed: {}", e);
                    return Err(anyhow::anyhow!("Channel send failed: {}", e));
                }
            }
        } else {
            error!("BUILDER: No channel configured!");
            return Err(anyhow::anyhow!("No output channel configured"));
        }
        
        // NATS publishing would go here if configured
        if let Some(publisher) = &self.nats_publisher {
            debug!("BUILDER->NATS: Publishing tx {}", tx_id);
            publisher.publish(&tx).await?;
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
