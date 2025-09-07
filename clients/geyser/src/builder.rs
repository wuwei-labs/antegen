use anyhow::{anyhow, Result};
use log::{error, info};
use solana_program::pubkey::Pubkey;
use solana_sdk::{
    signature::read_keypair_file,
    signer::{keypair::Keypair, Signer},
};
use std::fmt::Debug;
use tokio::runtime::Handle;

use antegen_client::{AntegenClient, GeyserDatasource, GeyserPluginHelper};
use antegen_processor::builder::ProcessorBuilder;
use antegen_submitter::builder::SubmitterBuilder;

/// Worker that uses the builder pattern with pre-built Geyser datasource
pub struct PluginWorkerBuilder {
    /// Helper for Geyser plugin to send events
    plugin_helper: GeyserPluginHelper,
    /// The built Antegen client
    client: Option<AntegenClient>,
}

impl Debug for PluginWorkerBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginWorkerBuilder")
            .field("plugin_helper", &"GeyserPluginHelper")
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
        info!("=== Initializing PluginWorkerBuilder with pre-built datasource ===");
        info!("RPC URL: {}", rpc_url);

        // Create Geyser datasource with channel
        let mut geyser_datasource = GeyserDatasource::new();
        let plugin_sender = geyser_datasource.get_plugin_sender();
        let plugin_helper = GeyserPluginHelper::new(plugin_sender);
        info!("Created Geyser datasource with channel capacity 1000");

        // Load keypair
        let keypair = std::sync::Arc::new(load_keypair(&keypair_path)?);
        let executor_pubkey = keypair.pubkey();
        info!(
            "Loaded keypair from {}, pubkey: {}",
            keypair_path, executor_pubkey
        );

        // Build the client using the pre-built Geyser datasource
        let mut client_builder = AntegenClient::builder()
            .datasource(Box::new(geyser_datasource))
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
                    .replay_config(replay_config)
                    .tpu_enabled(),
            );
        } else {
            client_builder = client_builder.submitter(
                SubmitterBuilder::new()
                    .rpc_url(rpc_url)
                    .executor_keypair(keypair.clone())
                    .tpu_enabled(),
            );
        }

        let client = client_builder.build().await?;

        info!("=== PluginWorkerBuilder initialization complete ===");

        Ok(Self {
            plugin_helper,
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
        _slot: u64,
    ) -> Result<()> {
        self.plugin_helper.send_account_update(pubkey, account)
    }
    
    /// Check if the channel is still connected
    pub fn is_connected(&self) -> bool {
        self.plugin_helper.is_connected()
    }
}

fn load_keypair(path: &str) -> Result<Keypair> {
    let keypair =
        read_keypair_file(path).map_err(|e| anyhow!("Failed to read keypair file: {}", e))?;
    Ok(keypair)
}
