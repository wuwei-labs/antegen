use antegen_adapter::{EventSource, ObservedEvent, AdapterService};
use antegen_processor::ProcessorService;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use log::{error, info};
use solana_program::pubkey::Pubkey;
use solana_sdk::{
    signature::read_keypair_file,
    signer::{keypair::Keypair, Signer},
};
use std::fmt::Debug;
use tokio::runtime::Handle;
use crossbeam::channel::{bounded, Receiver, Sender, TryRecvError};

/// Worker that runs adapter and processor services in the plugin
pub struct PluginWorker {
    /// Channel to send ObservedEvents from Geyser to Adapter
    event_sender: Sender<ObservedEvent>,
    /// Adapter service (owned until start is called)
    adapter_service: Option<AdapterService>,
    /// Processor service for thread processing and execution
    processor_service: Option<ProcessorService>,
}

impl Debug for PluginWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginWorker")
            .field("event_sender", &"Sender<ObservedEvent>")
            .field("adapter_service", &self.adapter_service.is_some())
            .field("processor_service", &self.processor_service.is_some())
            .finish()
    }
}

impl PluginWorker {
    pub async fn new(
        rpc_url: String,
        ws_url: String,
        keypair_path: String,
        forgo_executor_commission: bool,
        enable_replay: bool,
        nats_url: Option<String>,
        replay_delay_ms: Option<u64>,
    ) -> Result<Self> {
        info!("=== Initializing PluginWorker ===");
        info!("RPC URL: {}", rpc_url);
        info!("WS URL: {}", ws_url);

        // Channel 1: Geyser -> Observer (observed events)
        let (observed_tx, observed_rx) = bounded(1000);
        info!("Created observed event channel (Geyser->Observer) with capacity 1000");

        // Create Geyser data source that receives observed events
        let event_source = Box::new(GeyserPluginEventSource::new(observed_rx));
        info!("Created GeyserPluginEventSource for observer");


        // Load keypair
        let keypair = load_keypair(&keypair_path)?;
        let observer_pubkey = keypair.pubkey();
        info!(
            "Loaded keypair from {}, pubkey: {}",
            keypair_path, observer_pubkey
        );

        // Create adapter service with event source
        // This returns the service and single account receiver
        let (adapter_service, account_receiver) =
            AdapterService::new(event_source, observer_pubkey);
        info!("Created adapter service with single account channel");

        // Create processor service with integrated executor functionality
        let mut processor_config = antegen_processor::ProcessorConfig::default();
        
        // Override specific fields for plugin operation
        processor_config.executor_keypair_path = keypair_path.clone();
        processor_config.rpc_url = rpc_url.clone();
        processor_config.forgo_executor_commission = forgo_executor_commission;
        
        // Configure replay if enabled
        if enable_replay {
            processor_config.replay_config.enable_replay = true;
            processor_config.replay_config.nats_url = nats_url;
            processor_config.replay_config.replay_delay_ms = replay_delay_ms.unwrap_or(30_000); // Default 30s
            processor_config.replay_config.replay_max_age_ms = 3600_000; // 1 hour
            processor_config.replay_config.replay_max_attempts = 3;
        }
        
        let processor_service = ProcessorService::new(processor_config, account_receiver).await?;
        info!("Created processor service with integrated executor and account channel");

        info!("=== PluginWorker initialization complete ===");

        Ok(Self {
            event_sender: observed_tx,
            adapter_service: Some(adapter_service),
            processor_service: Some(processor_service),
        })
    }

    /// Start the adapter and processor services
    pub fn start(&mut self, runtime: Handle) -> Result<()> {
        info!("=== Starting Worker Services ===");

        // Take ownership of the services
        let mut adapter_service = self
            .adapter_service
            .take()
            .ok_or_else(|| anyhow!("Adapter service already started"))?;
        let processor_service = self
            .processor_service
            .take()
            .ok_or_else(|| anyhow!("Processor service already started"))?;

        info!("Spawning adapter service task...");

        // Spawn adapter service
        runtime.spawn(async move {
            info!("ADAPTER: Task started, entering event loop");
            match adapter_service.run().await {
                Ok(()) => info!("ADAPTER: Service completed normally"),
                Err(e) => error!("ADAPTER: Service stopped with error: {}", e),
            }
            info!("ADAPTER: Task exiting");
        });

        info!("Spawning processor service task (with integrated executor)...");

        // Spawn processor service (now includes executor logic)
        let processor_service = processor_service;
        
        runtime.spawn(async move {
            info!("PROCESSOR: Task started in full mode, entering event loop");
            match processor_service.run().await {
                Ok(()) => info!("PROCESSOR: Service completed normally"),
                Err(e) => error!("PROCESSOR: Service stopped with error: {}", e),
            }
            // Processor exiting
        });

        info!("=== Worker Services Started ===");
        Ok(())
    }

    /// Send account update from Geyser to observer
    /// All accounts are forwarded - observer just passes them through to submitter
    pub async fn send_account_event(
        &self,
        pubkey: Pubkey,
        account: solana_sdk::account::Account,
        slot: u64,
    ) -> Result<()> {
        // Forward account update

        let event = ObservedEvent::Account {
            pubkey,
            account,
            slot,
        };

        // Use crossbeam's synchronous send
        self.event_sender.send(event)
            .map_err(|e| anyhow!("Failed to send account event: {}", e))?;
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
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
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
