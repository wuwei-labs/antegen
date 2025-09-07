use anyhow::Result;
use async_trait::async_trait;
use crossbeam::channel::{bounded, Receiver, Sender};
use log::{error, info};
use solana_program::pubkey::Pubkey;

use crate::builder::DatasourceBuilder;
use antegen_processor::types::AccountUpdate;

/// Configuration for Geyser datasource
#[derive(Clone, Debug)]
pub struct GeyserConfig {
    /// Channel capacity for buffering events
    pub channel_capacity: usize,
}

impl Default for GeyserConfig {
    fn default() -> Self {
        Self {
            channel_capacity: 1000,
        }
    }
}

/// Pre-built Geyser datasource for receiving events from Geyser plugin
///
/// This datasource creates a channel that the Geyser plugin can send events to,
/// and forwards those events to the processor.
pub struct GeyserDatasource {
    config: GeyserConfig,
    /// Receiver for events from Geyser plugin
    receiver: Option<Receiver<AccountUpdate>>,
    /// Sender that the Geyser plugin will use
    plugin_sender: Option<Sender<AccountUpdate>>,
}

impl GeyserDatasource {
    /// Create a new Geyser datasource with default configuration
    pub fn new() -> Self {
        Self::with_config(GeyserConfig::default())
    }

    /// Create a new Geyser datasource with custom configuration
    pub fn with_config(config: GeyserConfig) -> Self {
        let (tx, rx) = bounded(config.channel_capacity);
        Self {
            config,
            receiver: Some(rx),
            plugin_sender: Some(tx),
        }
    }

    /// Get the sender that the Geyser plugin should use to send events
    /// This should be called before building the client
    pub fn get_plugin_sender(&mut self) -> Sender<AccountUpdate> {
        self.plugin_sender
            .take()
            .expect("Plugin sender already taken")
    }

    /// Create a datasource that uses an existing channel
    /// This is useful when the channel is created elsewhere (e.g., in the plugin)
    pub fn from_receiver(receiver: Receiver<AccountUpdate>) -> Self {
        Self {
            config: GeyserConfig::default(),
            receiver: Some(receiver),
            plugin_sender: None,
        }
    }
}

#[async_trait]
impl DatasourceBuilder for GeyserDatasource {
    async fn run(&self, sender: Sender<AccountUpdate>) -> Result<()> {
        info!("Starting Geyser datasource");
        info!("Channel capacity: {}", self.config.channel_capacity);

        let receiver = self
            .receiver
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Geyser datasource receiver not available"))?;

        // Forward events from Geyser plugin to the processor
        loop {
            // Blocking receive - will wait for events
            match receiver.recv() {
                Ok(event) => {
                    // Forward event to the processor
                    if let Err(e) = sender.send(event) {
                        error!("Failed to send event to processor: {}", e);
                        break;
                    }
                }
                Err(e) => {
                    info!("Geyser datasource channel disconnected: {}", e);
                    break;
                }
            }
        }

        info!("Geyser datasource stopped");
        Ok(())
    }
}

/// Helper struct for Geyser plugin integration
/// This provides methods for the plugin to send account updates
pub struct GeyserPluginHelper {
    sender: Sender<AccountUpdate>,
}

impl GeyserPluginHelper {
    /// Create a new helper with the given sender
    pub fn new(sender: Sender<AccountUpdate>) -> Self {
        Self { sender }
    }

    /// Send an account update from the Geyser plugin
    pub fn send_account_update(
        &self,
        pubkey: Pubkey,
        account: solana_sdk::account::Account,
    ) -> Result<()> {
        let update = AccountUpdate { pubkey, account };
        self.sender
            .send(update)
            .map_err(|e| anyhow::anyhow!("Failed to send account update: {}", e))
    }

    /// Check if the channel is still connected
    pub fn is_connected(&self) -> bool {
        !self.sender.is_full()
    }
}
