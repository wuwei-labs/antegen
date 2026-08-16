//! Shared resources module
//!
//! Contains all shared resources (RPC client, TPU client, unified cache) and a wrapper
//! for easy passing to actors and components.
//!
//! The AccountCache serves dual purposes:
//! - Caching account data for RPC fetches
//! - Deduplication of account updates via `put_if_newer()`

mod cache;
mod ingest;
pub mod latency;

pub use cache::{AccountCache, CacheTriggerType, CachedAccount, FetchError};
pub use ingest::{IngestSnapshot, IngestStats};

use crate::config::{ClientConfig, EndpointRole};
use crate::confirm::SignatureWatcher;
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
    /// Thread program ID (configurable, defaults to compiled-in value)
    pub program_id: Pubkey,
    /// Per-endpoint ingest attribution — which datasource is winning the race.
    pub ingest_stats: Arc<IngestStats>,
    /// Rolling execution-latency window, so the node reports its own
    /// percentiles instead of requiring logs to be shipped and parsed.
    pub latency_stats: Arc<latency::LatencyStats>,
    /// Commitment for the thread program subscription.
    pub commitment: Arc<str>,
    /// Commitment for the clock sysvar subscription.
    pub clock_commitment: Arc<str>,
    /// Shared confirmation watcher — one batched poll for every in-flight
    /// transaction rather than one poll per worker.
    pub confirmations: SignatureWatcher,
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
        let pool_config = RpcPoolConfig {
            skip_preflight: config.rpc.skip_preflight,
            ..RpcPoolConfig::default()
        };
        let rpc_client = Arc::new(RpcPool::new(endpoint_configs, pool_config)?);
        // Keep a blockhash ready so the execution path never fetches one after
        // the trigger deadline has already passed.
        rpc_client.spawn_blockhash_refresher();

        let cache = Arc::new(AccountCache::with_config(
            &config.cache,
            config.load_balancer.grace_period,
            config.load_balancer.eviction_buffer,
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

        // Started after the TPU client so it can rebroadcast through it.
        let confirmations = SignatureWatcher::spawn(rpc_client.clone(), tpu_client.clone());

        Ok((
            Self {
                rpc_client,
                cache,
                tpu_client,
                program_id: config.datasources.program_id,
                ingest_stats: Arc::new(IngestStats::new()),
                latency_stats: Arc::new(latency::LatencyStats::new()),
                commitment: config.datasources.commitment.as_str().into(),
                clock_commitment: config.datasources.clock_commitment.as_str().into(),
                confirmations,
            },
            eviction_rx,
        ))
    }

    /// Create with custom settings (for testing)
    #[cfg(test)]
    pub fn with_custom(rpc_client: Arc<RpcPool>, cache: Arc<AccountCache>) -> Self {
        let rpc_client_for_watcher = rpc_client.clone();
        Self {
            rpc_client,
            cache,
            tpu_client: None,
            program_id: antegen_thread_program::ID,
            ingest_stats: Arc::new(IngestStats::new()),
            latency_stats: Arc::new(latency::LatencyStats::new()),
            commitment: "confirmed".into(),
            clock_commitment: "processed".into(),
            confirmations: SignatureWatcher::spawn(rpc_client_for_watcher, None),
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

        // Cloning must share the underlying resources rather than duplicating
        // them. Asserted as a delta, not an absolute count: background tasks
        // (the blockhash refresher, the confirmation watcher) legitimately hold
        // their own handles.
        let rpc_before = Arc::strong_count(&resources.rpc_client);
        let cache_before = Arc::strong_count(&resources.cache);

        let cloned = resources.clone();

        assert_eq!(Arc::strong_count(&resources.rpc_client), rpc_before + 1);
        assert_eq!(Arc::strong_count(&resources.cache), cache_before + 1);
        assert!(Arc::ptr_eq(&resources.rpc_client, &cloned.rpc_client));
        assert!(Arc::ptr_eq(&resources.cache, &cloned.cache));
    }
}
