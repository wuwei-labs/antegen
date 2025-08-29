use anyhow::{anyhow, Result};
use async_trait::async_trait;
use crossbeam::channel::{bounded, Receiver, Sender, TryRecvError};
use log::{error, info};
use solana_program::pubkey::Pubkey;
use solana_sdk::{
    signature::read_keypair_file,
    signer::{keypair::Keypair, Signer},
};
use std::fmt::Debug;
use tokio::runtime::Handle;

use antegen_adapter::builder::AdapterBuilder;
use antegen_adapter::events::{EventSource, ObservedEvent};
use antegen_client::AntegenClient;
use antegen_processor::builder::ProcessorBuilder;
use antegen_submitter::builder::SubmitterBuilder;

/// Worker that uses the builder pattern for simplified setup
pub struct PluginWorkerBuilder {
    /// Channel to send ObservedEvents from Geyser to the client
    event_sender: Sender<ObservedEvent>,
    /// The built Antegen client
    client: Option<AntegenClient>,
}

impl Debug for PluginWorkerBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginWorkerBuilder")
            .field("event_sender", &"Sender<ObservedEvent>")
            .field("client", &self.client.is_some())
            .finish()
    }
}

impl PluginWorkerBuilder {
    pub async fn new(
        rpc_url: String,
        _ws_url: String,
        keypair_path: String,
        forgo_executor_commission: bool,
        enable_replay: bool,
        nats_url: Option<String>,
    ) -> Result<Self> {
        info!("=== Initializing PluginWorkerBuilder ===");
        info!("RPC URL: {}", rpc_url);

        // Create channel for Geyser -> Adapter
        let (observed_tx, observed_rx) = bounded(1000);
        info!("Created observed event channel (Geyser->Adapter) with capacity 1000");

        // Load keypair
        let keypair = std::sync::Arc::new(load_keypair(&keypair_path)?);
        let executor_pubkey = keypair.pubkey();
        info!(
            "Loaded keypair from {}, pubkey: {}",
            keypair_path, executor_pubkey
        );

        // Create Geyser event source
        let event_source = Box::new(GeyserPluginEventSource::new(observed_rx));

        // Build the client using the builder pattern
        let mut client_builder = AntegenClient::builder()
            .adapter(
                AdapterBuilder::geyser()
                    .event_source(event_source)
                    .adapter_pubkey(executor_pubkey),
            )
            .processor(
                ProcessorBuilder::new()
                    .keypair(keypair_path.clone())
                    .rpc_url(rpc_url.clone())
                    .forgo_commission(forgo_executor_commission),
            );

        // Add submitter with replay if configured
        if enable_replay {
            let mut replay_config = antegen_submitter::ReplayConfig::default();
            replay_config.enable_replay = true;
            replay_config.nats_url = nats_url;

            client_builder = client_builder.submitter(
                SubmitterBuilder::new()
                    .rpc_url(rpc_url)
                    .executor_keypair(keypair.clone())
                    .replay_config(replay_config),
            );
        } else {
            client_builder = client_builder.submitter(
                SubmitterBuilder::new()
                    .rpc_url(rpc_url)
                    .executor_keypair(keypair.clone()),
            );
        }

        let client = client_builder.build().await?;

        info!("=== PluginWorkerBuilder initialization complete ===");

        Ok(Self {
            event_sender: observed_tx,
            client: Some(client),
        })
    }

    /// Start the client
    pub fn start(&mut self, runtime: Handle) -> Result<()> {
        info!("=== Starting Worker Services with Builder ===");

        let client = self
            .client
            .take()
            .ok_or_else(|| anyhow!("Client already started"))?;

        runtime.spawn(async move {
            info!("Starting AntegenClient");
            match client.run().await {
                Ok(()) => info!("AntegenClient completed normally"),
                Err(e) => error!("AntegenClient error: {}", e),
            }
        });

        info!("=== Worker Services Started ===");
        Ok(())
    }

    /// Send account update from Geyser to the client
    pub async fn send_account_event(
        &self,
        pubkey: Pubkey,
        account: solana_sdk::account::Account,
        slot: u64,
    ) -> Result<()> {
        let event = ObservedEvent::Account {
            pubkey,
            account,
            slot,
        };

        self.event_sender
            .send(event)
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
