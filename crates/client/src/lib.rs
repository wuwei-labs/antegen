//! Antegen client - Ractor-based automation client
//!
//! This is the Antegen client using the ractor actor framework.
//!
//! ## Usage
//!
//! ### Standalone Mode
//! ```ignore
//! let config = ClientConfig::load("antegen.toml")?;
//! antegen_client::run_standalone(config).await?;
//! ```
//!
//! ### Plugin Mode
//! ```ignore
//! let config = ClientConfig::load("antegen.toml")?;
//! let handle = PluginHandle::spawn(config).await?;
//! // Later, in Geyser callbacks:
//! handle.try_send_update(account_update)?;
//! ```

pub mod actors;
pub mod config;
pub mod datasources;
pub mod executor;
pub mod load_balancer;
pub mod resources;
pub mod rpc;
pub mod tpu;
pub mod types;

// Re-exports
pub use config::ClientConfig;
pub use executor::ExecutorLogic;
pub use load_balancer::{LoadBalancer, LoadBalancerConfig, LoadBalancerStats, ProcessDecision};
pub use resources::{AccountCache, CachedAccount, SharedResources};
pub use rpc::RpcPool;
pub use tpu::{TpuClient, TpuClientConfig};
pub use types::{AccountUpdate, DurableTransactionMessage, ProcessorMessage, TransactionMessage};

use anyhow::Result;
use tokio::sync::mpsc;

/// Run the client in standalone mode (blocking)
///
/// This is the main entry point for the standalone binary. It will:
/// 1. Validate the configuration
/// 2. Create shared resources (RPC pool, unified cache)
/// 3. Spawn the root supervisor
/// 4. Spawn RPC datasource actors (all listening concurrently)
/// 5. Block until shutdown signal
///
/// # Example
/// ```ignore
/// let config = ClientConfig::load("antegen.toml")?;
/// run_standalone(config).await?;
/// ```
pub async fn run_standalone(config: ClientConfig) -> Result<()> {
    // Validate configuration
    config.validate()?;

    log::debug!("Starting Antegen client in standalone mode");
    log::debug!("Thread program: {}", config.datasources.program_id());
    log::debug!(
        "Max concurrent threads: {}",
        config.processor.max_concurrent_threads
    );

    // Create shared resources (async for TPU client initialization)
    let (resources, eviction_rx) = SharedResources::new(&config).await?;
    log::debug!("Created shared resources (RPC pool, unified cache, TPU client)");

    // Spawn RootSupervisor (no geyser channel in standalone mode)
    let (_root_ref, root_handle) = ractor::Actor::spawn(
        Some("root-supervisor".to_string()),
        actors::RootSupervisor,
        (config, resources, None, eviction_rx),
    )
    .await
    .map_err(|e| anyhow::anyhow!("Failed to spawn RootSupervisor: {}", e))?;

    // Block until supervisor exits (via signal handler)
    match root_handle.await {
        Ok(_) => {
            log::info!("Shutdown complete");
            Ok(())
        }
        Err(e) => {
            log::error!("RootSupervisor error: {:?}", e);
            Err(anyhow::anyhow!("Actor system failure: {:?}", e))
        }
    }
}

/// Handle for plugin mode
///
/// This provides a way for the Geyser plugin to send account updates
/// to the client without blocking the validator.
pub struct PluginHandle {
    account_sender: mpsc::Sender<AccountUpdate>,
    // Root supervisor runs in background, handle is not stored but actor tree remains alive
}

impl PluginHandle {
    /// Spawn the client in plugin mode
    ///
    /// This creates a channel for account updates and spawns the actor tree.
    /// The Geyser plugin should call `try_send_update()` with account updates.
    ///
    /// # Example
    /// ```ignore
    /// let mut config = ClientConfig::load("antegen.toml")?;
    /// let handle = PluginHandle::spawn(config).await?;
    /// ```
    pub async fn spawn(config: ClientConfig) -> Result<Self> {
        config.validate()?;

        log::debug!("Starting Antegen client in plugin mode");
        log::debug!("Thread program: {}", config.datasources.program_id());
        log::debug!(
            "Max concurrent threads: {}",
            config.processor.max_concurrent_threads
        );

        // Create channel for plugin -> processor communication
        let (tx, rx) = mpsc::channel(1000);

        // Create shared resources (async for TPU client initialization)
        let (resources, eviction_rx) = SharedResources::new(&config).await?;
        log::debug!("Created shared resources (RPC pool, unified cache, TPU client)");

        // Spawn RootSupervisor with geyser channel receiver
        let (_root_ref, root_handle) = ractor::Actor::spawn(
            Some("root-supervisor".to_string()),
            actors::RootSupervisor,
            (config, resources, Some(rx), eviction_rx),
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to spawn RootSupervisor: {}", e))?;

        // Spawn background task to log any supervisor errors
        tokio::spawn(async move {
            match root_handle.await {
                Ok(_) => log::info!("Plugin mode: RootSupervisor shutdown complete"),
                Err(e) => log::error!("Plugin mode: RootSupervisor error: {:?}", e),
            }
        });

        log::info!("Plugin mode: Actor tree spawned successfully");

        Ok(Self { account_sender: tx })
    }

    /// Send an account update to the processor (non-blocking)
    ///
    /// Returns an error if the channel is full or closed.
    /// The Geyser plugin should call this from `update_account()` callbacks.
    pub fn try_send_update(&self, update: AccountUpdate) -> Result<()> {
        self.account_sender
            .try_send(update)
            .map_err(|e| anyhow::anyhow!("Failed to send account update: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_run_standalone_validates_config() {
        let mut config = ClientConfig::default();
        config.rpc.endpoints.clear();

        // Should fail validation
        let result = run_standalone(config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_plugin_handle_spawn() {
        let config = ClientConfig::default();
        let result = PluginHandle::spawn(config).await;
        // Result depends on whether keypair file exists at the default path
        // In tests, we mainly want to verify the method doesn't panic on validation
        // The actual RootSupervisor spawn will fail if keypair file missing - that's OK
        match &result {
            Ok(_) => {}
            Err(e) => {
                // Expected failures: keypair file not found, validation, etc.
                let err_str = e.to_string();
                assert!(
                    err_str.contains("keypair") || err_str.contains("RootSupervisor"),
                    "Unexpected error: {}",
                    err_str
                );
            }
        }
    }

    #[tokio::test]
    async fn test_plugin_handle_send_update() {
        let config = ClientConfig::default();
        // Spawn may fail if keypair file doesn't exist, which is OK for this test
        if let Ok(handle) = PluginHandle::spawn(config).await {
            // Test sending an update
            let update =
                AccountUpdate::new(solana_sdk::pubkey::Pubkey::new_unique(), vec![1, 2, 3], 100);
            assert!(handle.try_send_update(update).is_ok());
        }
    }
}
