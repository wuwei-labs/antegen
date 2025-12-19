//! Shared resources module
//!
//! Contains all shared resources (RPC client, TPU client, unified cache) and a wrapper
//! for easy passing to actors and components.
//!
//! The AccountCache serves dual purposes:
//! - Caching account data for RPC fetches
//! - Deduplication of account updates via `put_if_newer()`

mod cache;

pub use cache::{AccountCache, CachedAccount, CacheTriggerType};

use crate::config::{ClientConfig, EndpointRole};
use crate::rpc::{EndpointConfig, RpcPool, RpcPoolConfig};
use crate::tpu::{TpuClient, TpuClientConfig};
use anyhow::Result;
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Shared resources used across all actors
///
/// All actors share these resources via `Arc`, enabling efficient resource sharing
/// without duplication. The `TpuClient` is particularly designed for concurrent
/// access from multiple `WorkerActor` instances.
#[derive(Clone)]
pub struct SharedResources {
    /// Custom RPC client with safe deserialization, failover, and health tracking
    pub rpc_client: Arc<RpcPool>,
    /// Unified cache for account data - serves as both cache AND deduplication
    pub cache: Arc<AccountCache>,
    /// TPU client for direct validator transaction submission (optional)
    ///
    /// When enabled, transactions are sent via TPU first with RPC fallback.
    /// All workers share this single instance via Arc for efficient QUIC
    /// connection management.
    pub tpu_client: Option<Arc<TpuClient>>,
}

impl SharedResources {
    /// Create shared resources from configuration
    ///
    /// Returns (resources, eviction_receiver) - the receiver should be passed to StagingActor.
    ///
    /// This method is async because TPU client initialization requires network operations
    /// (connecting to RPC for leader schedule and WebSocket for slot updates).
    pub async fn new(config: &ClientConfig) -> Result<(Self, mpsc::UnboundedReceiver<Pubkey>)> {
        // Create channel for cache eviction notifications
        let (eviction_tx, eviction_rx) = mpsc::unbounded_channel();

        // Custom RPC client with safe deserialization
        let endpoint_configs = EndpointConfig::from_rpc_config(&config.rpc);
        let rpc_client = Arc::new(RpcPool::new(endpoint_configs, RpcPoolConfig::default())?);

        let cache = Arc::new(AccountCache::with_config(
            &config.cache,
            config.load_balancer.grace_period_secs,
            config.load_balancer.eviction_buffer_secs,
            Some(eviction_tx),
        ));

        // Initialize TPU client if enabled
        let tpu_client = if config.tpu.enabled {
            // Use first submission endpoint for TPU leader updates
            // This shares the same endpoint URL as the RpcPool
            let submission_endpoint = config
                .rpc
                .endpoints
                .iter()
                .find(|e| matches!(e.role, EndpointRole::Submission | EndpointRole::Both))
                .expect("Config validation ensures submission endpoint exists");

            let tpu_config = TpuClientConfig {
                rpc_url: submission_endpoint.url.clone(),
                websocket_url: submission_endpoint.get_ws_url(),
                num_connections: config.tpu.num_connections,
                leaders_fanout: config.tpu.leaders_fanout,
                worker_channel_size: config.tpu.worker_channel_size,
            };

            match TpuClient::new(tpu_config).await {
                Ok(client) => {
                    log::info!("TPU client initialized successfully");
                    Some(Arc::new(client))
                }
                Err(e) => {
                    log::warn!("Failed to initialize TPU client, using RPC only: {}", e);
                    None
                }
            }
        } else {
            log::info!("TPU client disabled in config");
            None
        };

        Ok((
            Self {
                rpc_client,
                cache,
                tpu_client,
            },
            eviction_rx,
        ))
    }

    /// Create with custom settings (for testing)
    #[cfg(test)]
    pub fn with_custom(rpc_client: Arc<RpcPool>, cache: Arc<AccountCache>) -> Self {
        Self {
            rpc_client,
            cache,
            tpu_client: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_resources_creation() {
        // Disable TPU for tests since it requires network
        let mut config = ClientConfig::default();
        config.tpu.enabled = false;
        let result = SharedResources::new(&config).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_resources_clone() {
        // Disable TPU for tests since it requires network
        let mut config = ClientConfig::default();
        config.tpu.enabled = false;
        let (resources, _eviction_rx) = SharedResources::new(&config).await.unwrap();
        let _cloned = resources.clone();

        // Arc counts should be incremented
        assert_eq!(Arc::strong_count(&resources.rpc_client), 2);
        assert_eq!(Arc::strong_count(&resources.cache), 2);
    }
}
