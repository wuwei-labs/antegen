use anyhow::Result;
use async_trait::async_trait;
use log::{error, info};
use solana_program::pubkey::Pubkey;
use tokio::sync::mpsc;

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
    receiver: std::sync::Mutex<Option<mpsc::Receiver<AccountUpdate>>>,
    /// Sender that the Geyser plugin will use
    plugin_sender: std::sync::Mutex<Option<mpsc::Sender<AccountUpdate>>>,
}

impl GeyserDatasource {
    /// Create a new Geyser datasource with default configuration
    pub fn new() -> Self {
        Self::with_config(GeyserConfig::default())
    }

    /// Create a new Geyser datasource with custom configuration
    pub fn with_config(config: GeyserConfig) -> Self {
        let (tx, rx) = mpsc::channel(config.channel_capacity);
        Self {
            config,
            receiver: std::sync::Mutex::new(Some(rx)),
            plugin_sender: std::sync::Mutex::new(Some(tx)),
        }
    }

    /// Get the sender that the Geyser plugin should use to send events
    /// This should be called before building the client
    pub fn get_plugin_sender(&mut self) -> mpsc::Sender<AccountUpdate> {
        self.plugin_sender
            .lock()
            .unwrap()
            .take()
            .expect("Plugin sender already taken")
    }

    /// Create a datasource that uses an existing channel
    /// This is useful when the channel is created elsewhere (e.g., in the plugin)
    pub fn from_receiver(receiver: mpsc::Receiver<AccountUpdate>) -> Self {
        Self {
            config: GeyserConfig::default(),
            receiver: std::sync::Mutex::new(Some(receiver)),
            plugin_sender: std::sync::Mutex::new(None),
        }
    }
}

#[async_trait]
impl DatasourceBuilder for GeyserDatasource {
    async fn run(&self, sender: mpsc::Sender<AccountUpdate>) -> Result<()> {
        info!("Starting Geyser datasource");
        info!("Channel capacity: {}", self.config.channel_capacity);

        let mut receiver = self
            .receiver
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| anyhow::anyhow!("Geyser datasource receiver not available"))?;

        // Forward events from Geyser plugin to the processor
        loop {
            // Async receive - will wait for events
            match receiver.recv().await {
                Some(event) => {
                    log::debug!("Forwarding account update for {}", event.pubkey);
                    
                    // Forward event to the processor (async send)
                    if let Err(e) = sender.send(event).await {
                        error!("Failed to send event to processor: {}", e);
                        break;
                    }
                }
                None => {
                    info!("Geyser datasource channel disconnected");
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
    sender: mpsc::Sender<AccountUpdate>,
}

impl GeyserPluginHelper {
    /// Create a new helper with the given sender
    pub fn new(sender: mpsc::Sender<AccountUpdate>) -> Self {
        Self { sender }
    }

    /// Send an account update from the Geyser plugin (blocking)
    /// This uses try_send since the plugin may not be in an async context
    pub fn send_account_update(
        &self,
        pubkey: Pubkey,
        account: solana_sdk::account::Account,
    ) -> Result<()> {
        log::debug!("GeyserPluginHelper sending update for {}", pubkey);
        let update = AccountUpdate { pubkey, account };
        self.sender
            .try_send(update)
            .map_err(|e| anyhow::anyhow!("Failed to send account update: {}", e))?;
        log::debug!("GeyserPluginHelper sent update successfully");
        Ok(())
    }

    /// Check if the channel is still connected
    pub fn is_connected(&self) -> bool {
        !self.sender.is_closed()
    }
}
