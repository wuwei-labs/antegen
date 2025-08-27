use crate::metrics::SubmitterMetrics;
use anchor_lang::{prelude::*, Discriminator};
use antegen_thread_program::state::{FiberState, Thread, ThreadConfig};
use anyhow::Result;
use log::debug;
use moka::future::Cache;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{account::Account, pubkey::Pubkey};
use std::sync::Arc;
use std::time::Duration;

use crate::types::CacheConfig;


/// RPC client wrapper with caching capabilities
#[derive(Clone)]
pub struct CachedRpcClient {
    /// Inner RPC client
    inner: Arc<RpcClient>,

    /// Account cache (for all accounts including thread config, fibers, etc)
    account_cache: Arc<Cache<Pubkey, Account>>,

    /// Metrics
    metrics: Option<Arc<SubmitterMetrics>>,
}

impl CachedRpcClient {
    /// Create a new cached RPC client
    pub fn new(rpc_client: RpcClient, config: CacheConfig) -> Self {
        let account_cache = Cache::builder()
            .max_capacity(config.max_cached_accounts)
            .time_to_live(Duration::from_secs(config.account_ttl_secs))
            .build();

        Self {
            inner: Arc::new(rpc_client),
            account_cache: Arc::new(account_cache),
            metrics: None,
        }
    }

    /// Create with metrics support
    pub fn with_metrics(
        rpc_client: RpcClient,
        config: CacheConfig,
        metrics: Arc<SubmitterMetrics>,
    ) -> Self {
        let mut client = Self::new(rpc_client, config);
        client.metrics = Some(metrics);
        client
    }

    /// Get the inner RPC client for direct, non-cached operations
    pub fn bypass(&self) -> &Arc<RpcClient> {
        &self.inner
    }

    /// Get account with caching
    pub async fn get_account(&self, pubkey: &Pubkey) -> Result<Account> {
        // Check cache first
        if let Some(account) = self.account_cache.get(pubkey).await {
            debug!("Account cache hit for {}", pubkey);
            if let Some(ref metrics) = self.metrics {
                metrics.cache_hit("account");
            }
            return Ok(account);
        }

        // Cache miss - fetch and cache
        debug!("Account cache miss for {}, fetching from RPC", pubkey);
        if let Some(ref metrics) = self.metrics {
            metrics.cache_miss("account");
            metrics.rpc_request("get_account");
        }

        let account = self.inner.get_account(pubkey).await?;
        self.account_cache.insert(*pubkey, account.clone()).await;

        Ok(account)
    }

    /// Get thread config with caching
    pub async fn get_thread_config(&self) -> Result<ThreadConfig> {
        let config_pubkey =
            Pubkey::find_program_address(&[b"thread_config"], &antegen_thread_program::ID).0;

        // Use regular account cache
        let account = self.get_account(&config_pubkey).await?;
        let config = ThreadConfig::try_deserialize(&mut account.data.as_slice())?;

        Ok(config)
    }

    /// Get fiber state with caching
    pub async fn get_fiber_state(&self, fiber_pubkey: &Pubkey) -> Result<FiberState> {
        let account = self.get_account(fiber_pubkey).await?;
        FiberState::try_deserialize(&mut account.data.as_slice())
            .map_err(|e| anyhow::anyhow!("Failed to deserialize fiber: {}", e))
    }

    /// Invalidate specific account
    pub async fn invalidate_account(&self, pubkey: &Pubkey) {
        self.account_cache.invalidate(pubkey).await;
        debug!("Invalidated account cache for {}", pubkey);
    }

    /// Clear all caches
    pub async fn clear_all_caches(&self) {
        self.account_cache.invalidate_all();
        debug!("Cleared all caches");
    }

    /// Update account in cache selectively based on account type
    /// - Always cache: Clock sysvar and Thread accounts
    /// - Conditionally cache: Everything else only if already in cache
    pub async fn update_account_selectively(&self, pubkey: &Pubkey, account: Account) {
        // Always cache Clock sysvar
        if *pubkey == solana_sdk::sysvar::clock::ID {
            self.account_cache.insert(*pubkey, account).await;
            debug!("Cached Clock sysvar update");
            
            if let Some(ref metrics) = self.metrics {
                metrics.cache_hit("clock_update");
            }
            return;
        }
        
        // Check if it's a Thread account (not fiber/config)
        if account.owner == antegen_thread_program::ID && account.data.len() > 8 {
            let discriminator = &account.data[0..8];
            
            // Only cache Thread accounts, not FiberState or ThreadConfig
            if discriminator == Thread::DISCRIMINATOR {
                self.account_cache.insert(*pubkey, account).await;
                debug!("Cached Thread account {}", pubkey);
                
                if let Some(ref metrics) = self.metrics {
                    metrics.cache_hit("thread_update");
                }
                return;
            }
        }
        
        // For all other accounts (including fibers/configs), only update if already cached
        // This preserves accounts that were fetched on-demand
        if self.account_cache.contains_key(pubkey) {
            self.account_cache.insert(*pubkey, account).await;
            debug!("Updated existing cached account {}", pubkey);
            
            if let Some(ref metrics) = self.metrics {
                metrics.cache_hit("account_update");
            }
        } else {
            debug!("Skipping cache for uncached account {}", pubkey);
        }
    }

    /// Get multiple accounts with caching
    pub async fn get_multiple_accounts(&self, pubkeys: &[Pubkey]) -> Result<Vec<Option<Account>>> {
        let mut results = Vec::with_capacity(pubkeys.len());
        let mut uncached_indices = Vec::new();
        let mut uncached_pubkeys = Vec::new();

        // Check cache for each account
        for (i, pubkey) in pubkeys.iter().enumerate() {
            if let Some(account) = self.account_cache.get(pubkey).await {
                results.push(Some(account));
            } else {
                results.push(None);
                uncached_indices.push(i);
                uncached_pubkeys.push(*pubkey);
            }
        }

        // Fetch uncached accounts in batch
        if !uncached_pubkeys.is_empty() {
            debug!("Fetching {} uncached accounts", uncached_pubkeys.len());
            if let Some(ref metrics) = self.metrics {
                metrics.rpc_request("get_multiple_accounts");
            }

            let accounts = self.inner.get_multiple_accounts(&uncached_pubkeys).await?;

            // Update results and cache
            for (idx, account_opt) in uncached_indices.iter().zip(accounts.iter()) {
                if let Some(account) = account_opt {
                    results[*idx] = Some(account.clone());
                    self.account_cache
                        .insert(uncached_pubkeys[*idx], account.clone())
                        .await;
                }
            }
        }

        Ok(results)
    }
}
