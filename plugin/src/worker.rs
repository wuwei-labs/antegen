use antegen_builder::{BuilderService, EventSource, ObservedEvent};
use antegen_submitter::{ClockEvent, SubmitterService};
use antegen_thread_program::state::Thread;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use log::{debug, error, info};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_program::{clock::Clock, pubkey::Pubkey};
use solana_sdk::{signature::read_keypair_file, signer::keypair::Keypair};
use std::fmt::Debug;
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::mpsc::{self, Receiver, Sender};

/// Worker that runs both builder and submitter in the plugin
pub struct PluginWorker {
    /// Channel to send events from Geyser to builder
    event_sender: Sender<ObservedEvent>,
    /// Channel to send clock events to submitter
    clock_sender: Sender<ClockEvent>,
    /// Builder service (owned until start is called)
    builder_service: Option<BuilderService>,
    /// Submitter service (owned until start is called)  
    submitter_service: Option<SubmitterService>,
}

impl Debug for PluginWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginWorker")
            .field("event_sender", &"Sender<ObservedEvent>")
            .finish()
    }
}

impl PluginWorker {
    pub async fn new(
        builder_id: u32,
        rpc_url: String,
        ws_url: String,
        keypair_path: String,
    ) -> Result<Self> {
        info!("=== Initializing PluginWorker ===");
        info!("Builder ID: {}", builder_id);
        info!("RPC URL: {}", rpc_url);
        info!("WS URL: {}", ws_url);

        // Channel 1: Geyser -> Builder (thread events)
        let (thread_tx, thread_rx) = mpsc::channel(1000);
        info!("Created thread event channel (Geyser->Builder) with capacity 1000");

        // Channel 2: Builder -> Submitter (built transactions)
        let (tx_tx, tx_rx) = mpsc::channel(100);
        info!("Created transaction channel (Builder->Submitter) with capacity 100");

        // Channel 3: Geyser -> Submitter (clock events)
        let (clock_tx, clock_rx) = mpsc::channel(100);
        info!("Created clock event channel (Geyser->Submitter) with capacity 100");

        // Create Geyser data source that receives thread events
        let event_source = Box::new(GeyserPluginEventSource::new(thread_rx));
        info!("Created GeyserPluginEventSource for builder");

        // Create RPC client
        let rpc_client = Arc::new(RpcClient::new(rpc_url.clone()));

        // Load keypair
        let keypair = load_keypair(&keypair_path)?;
        let keypair = Arc::new(keypair);
        info!("Loaded keypair from {}", keypair_path);

        // Create builder with transaction sender ONLY
        let builder_service = BuilderService::new_with_outputs(
            event_source,
            builder_id,
            rpc_client,
            keypair.clone(),  // Pass keypair for signing
            Some(tx_tx),      // Transaction sender to submitter
            None,             // No NATS for now
        )
        .await?;
        info!("Created builder service");

        // Create submitter with transaction receiver ONLY
        // We'll use a dummy sender since with_channel expects both
        let (dummy_tx, _) = mpsc::channel(1);
        let submitter_service = SubmitterService::with_channel(
            rpc_url,
            ws_url,
            keypair,
            dummy_tx,  // Not used, just to satisfy API
            tx_rx,     // Transaction receiver from builder
        );
        info!("Created submitter service");

        // Set clock receiver for the submitter
        let mut submitter_service = submitter_service;
        submitter_service.set_clock_receiver(clock_rx);
        info!("Set clock receiver for submitter");

        info!("=== PluginWorker initialization complete ===");

        Ok(Self {
            event_sender: thread_tx,
            clock_sender: clock_tx,
            builder_service: Some(builder_service),
            submitter_service: Some(submitter_service),
        })
    }

    /// Start the builder and submitter services
    pub fn start(&mut self, runtime: Handle) -> Result<()> {
        info!("=== Starting Worker Services ===");
        
        // Take ownership of the services
        let mut builder_service = self.builder_service.take()
            .ok_or_else(|| anyhow!("Builder service already started"))?;
        let mut submitter_service = self.submitter_service.take()
            .ok_or_else(|| anyhow!("Submitter service already started"))?;
            
        info!("Spawning builder service task...");
        
        // Spawn builder service
        runtime.spawn(async move {
            info!("BUILDER: Task started, entering event loop");
            match builder_service.run().await {
                Ok(()) => info!("BUILDER: Service completed normally"),
                Err(e) => error!("BUILDER: Service stopped with error: {}", e),
            }
            info!("BUILDER: Task exiting");
        });
        
        info!("Spawning submitter service task...");
        
        // Spawn submitter service
        runtime.spawn(async move {
            info!("SUBMITTER: Task started, entering event loop");
            match submitter_service.run().await {
                Ok(()) => info!("SUBMITTER: Service completed normally"),
                Err(e) => error!("SUBMITTER: Service stopped with error: {}", e),
            }
            info!("SUBMITTER: Task exiting");
        });
        
        info!("=== Worker Services Started ===");
        Ok(())
    }
    
    /// Send thread event from Geyser to builder
    pub async fn send_thread_event(
        &self,
        thread: Thread,
        thread_pubkey: Pubkey,
        slot: u64,
    ) -> Result<()> {
        info!("GEYSER->BUILDER: Thread event for {} at slot {}", thread_pubkey, slot);
        
        // Skip paused threads
        if thread.paused {
            info!("GEYSER->BUILDER: Thread {} is paused, skipping", thread_pubkey);
            return Ok(());
        }

        // For testing, send ALL threads to builder (not just unclaimed ones)
        let event = ObservedEvent::ThreadExecutable {
            thread_pubkey,
            thread: thread.clone(),
            slot,
        };

        info!("GEYSER->BUILDER: Sending ThreadExecutable event for {}", thread_pubkey);
        match self.event_sender.send(event).await {
            Ok(()) => {
                info!("GEYSER->BUILDER: Successfully sent thread event for {}", thread_pubkey);
            }
            Err(e) => {
                error!("GEYSER->BUILDER: Failed to send thread event: {}", e);
                return Err(anyhow!("Failed to send thread event: {}", e));
            }
        }
        
        Ok(())
    }

    /// Send clock update to submitter only (builder doesn't need it)
    pub async fn send_clock_event(&self, clock: Clock, slot: u64, block_height: u64) -> Result<()> {
        // Only send to submitter (builder doesn't need clock events)
        let submitter_event = ClockEvent {
            slot,
            epoch: clock.epoch,
            timestamp: clock.unix_timestamp,
            block_height,
        };
        
        debug!("GEYSER->SUBMITTER: Sending clock event slot={}", slot);
        match self.clock_sender.send(submitter_event).await {
            Ok(()) => {
                debug!("GEYSER->SUBMITTER: Clock event sent successfully");
            }
            Err(e) => {
                error!("GEYSER->SUBMITTER: Failed to send clock event: {}", e);
                return Err(anyhow!("Failed to send clock event: {}", e));
            }
        }

        Ok(())
    }

    /// Send account update from Geyser to builder
    pub async fn send_account_event(
        &self,
        pubkey: Pubkey,
        account: solana_sdk::account::Account,
        slot: u64,
    ) -> Result<()> {
        debug!(
            "Forwarding AccountUpdate event to builder: pubkey={}, slot={}, data_len={}",
            pubkey,
            slot,
            account.data.len()
        );

        let event = ObservedEvent::AccountUpdate {
            pubkey,
            account,
            slot,
        };

        self.event_sender.send(event).await?;
        Ok(())
    }
}

/// Event source that receives events from the Geyser plugin
struct GeyserPluginEventSource {
    receiver: Receiver<ObservedEvent>,
    running: bool,
}

impl GeyserPluginEventSource {
    fn new(receiver: Receiver<ObservedEvent>) -> Self {
        Self {
            receiver,
            running: false,
        }
    }
}

#[async_trait]
impl EventSource for GeyserPluginEventSource {
    async fn start(&mut self) -> Result<()> {
        info!("Starting GeyserPluginEventSource");
        self.running = true;
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        info!("Stopping GeyserPluginEventSource");
        self.running = false;
        Ok(())
    }

    async fn next_event(&mut self) -> Result<Option<ObservedEvent>> {
        if !self.running {
            return Ok(None);
        }

        // Non-blocking receive
        match self.receiver.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => {
                error!("Event channel disconnected");
                self.running = false;
                Ok(None)
            }
        }
    }

    async fn subscribe_thread(&mut self, _thread_pubkey: Pubkey) -> Result<()> {
        // No-op - we receive all events from Geyser
        Ok(())
    }

    async fn unsubscribe_thread(&mut self, _thread_pubkey: Pubkey) -> Result<()> {
        // No-op - we receive all events from Geyser
        Ok(())
    }

    async fn get_current_slot(&self) -> Result<u64> {
        // This would need to be tracked from clock events
        Ok(0)
    }

    fn name(&self) -> &str {
        "GeyserPluginEventSource"
    }
}

fn load_keypair(path: &str) -> Result<Keypair> {
    let keypair =
        read_keypair_file(path).map_err(|e| anyhow!("Failed to read keypair file: {}", e))?;
    Ok(keypair)
}
