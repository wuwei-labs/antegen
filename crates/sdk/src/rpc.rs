use anchor_lang::{AccountDeserialize, Discriminator};
use antegen_thread_program::state::{FiberState, Thread, ThreadConfig};
use anyhow::Result;
use log::debug;
use moka::future::Cache;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{account::Account, pubkey::Pubkey};
use std::sync::Arc;
use std::time::Duration;

/// Cache configuration for RPC client
#[derive(Clone, Debug)]
pub struct CacheConfig {
    /// Maximum number of cached accounts
    pub max_cached_accounts: u64,
    /// TTL for cached accounts in seconds
    pub account_ttl_secs: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_cached_accounts: 10_000,
            account_ttl_secs: 60,
        }
    }
}

/// RPC client wrapper with caching capabilities
#[derive(Clone)]
pub struct CachedRpcClient {
    /// Inner RPC client
    inner: Arc<RpcClient>,

    /// Account cache (for all accounts including thread config, fibers, etc)
    account_cache: Arc<Cache<Pubkey, Account>>,
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
        }
    }
    
    /// Create with metrics support (compatibility method)
    pub fn with_metrics(
        rpc_client: RpcClient,
        config: CacheConfig,
        _metrics: Arc<impl std::any::Any>,
    ) -> Self {
        // For now, just ignore metrics and create normally
        Self::new(rpc_client, config)
    }

    /// Get the inner RPC client for direct, non-cached operations
    pub fn bypass(&self) -> &Arc<RpcClient> {
        &self.inner
    }

    /// Get account with caching and retry logic for race conditions
    pub async fn get_account(&self, pubkey: &Pubkey) -> Result<Account> {
        use log::warn;
        use std::cmp;
        use tokio::time::sleep;
        
        // Check cache first
        if let Some(account) = self.account_cache.get(pubkey).await {
            // Cache hit
            return Ok(account);
        }

        // Cache miss - fetch with retry logic
        let mut attempt = 0;
        let mut delay = Duration::from_millis(500);
        let max_delay = Duration::from_secs(5);
        let start_time = std::time::Instant::now();
        let mut last_log = std::time::Instant::now();
        
        loop {
            attempt += 1;
            
            match self.inner.get_account(pubkey).await {
                Ok(account) => {
                    if attempt > 1 {
                        debug!("Successfully fetched account {} on attempt {} (waited ~{:.1}s total)",
                            pubkey, attempt, 
                            start_time.elapsed().as_secs_f32());
                    }
                    self.account_cache.insert(*pubkey, account.clone()).await;
                    return Ok(account);
                }
                Err(e) => {
                    let error_str = e.to_string();
                    
                    // Check if it's an account not found error (expected during race conditions)
                    if error_str.contains("AccountNotFound") || error_str.contains("could not find account") {
                        if last_log.elapsed() > Duration::from_secs(30) {
                            // Log progress every 30 seconds
                            debug!("Still waiting for account {} to exist (elapsed: {:.0}s)...",
                                pubkey, start_time.elapsed().as_secs());
                            last_log = std::time::Instant::now();
                        }
                    } else {
                        // For non-AccountNotFound errors, fail after first attempt
                        // These are likely network or RPC errors that won't resolve with retries
                        warn!("RPC error fetching account {}: {}", pubkey, e);
                        return Err(e.into());
                    }
                    
                    sleep(delay).await;
                    // Exponential backoff with cap at max_delay
                    delay = cmp::min(delay * 2, max_delay);
                }
            }
        }
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
    }

    /// Clear all caches
    pub async fn clear_all_caches(&self) {
        self.account_cache.invalidate_all();
    }

    /// Check if we should process this Thread update based on exec_count
    /// Returns true if:
    /// - Thread is not in cache (new thread)
    /// - Thread's exec_count is greater than cached version
    pub async fn should_process_thread(&self, pubkey: &Pubkey, new_thread: &Thread) -> bool {
        // Check if we have a cached version
        if let Some(cached_account) = self.account_cache.get(pubkey).await {
            // Try to deserialize cached thread to check exec_count
            if let Ok(cached_thread) = Thread::try_deserialize(&mut cached_account.data.as_slice()) {
                // Only process if exec_count increased
                let should_process = new_thread.exec_count > cached_thread.exec_count;
                
                return should_process;
            }
        }
        
        // Not cached or failed to deserialize - process it
        true
    }

    /// Update account in cache selectively based on account type
    /// - Always cache: Clock sysvar and Thread accounts
    /// - Conditionally cache: Everything else only if already in cache
    pub async fn update_account_selectively(&self, pubkey: &Pubkey, account: Account) {
        // Always cache Clock sysvar
        if *pubkey == solana_sdk::sysvar::clock::ID {
            self.account_cache.insert(*pubkey, account).await;
            debug!("Cached Clock sysvar update");
            return;
        }
        
        // Check if it's a Thread account (not fiber/config)
        if account.owner == antegen_thread_program::ID && account.data.len() > 8 {
            let discriminator = &account.data[0..8];
            
            // Only cache Thread accounts, not FiberState or ThreadConfig
            if discriminator == Thread::DISCRIMINATOR {
                self.account_cache.insert(*pubkey, account).await;
                debug!("Cached Thread account {}", pubkey);
                return;
            }
        }
        
        // For all other accounts (including fibers/configs), only update if already cached
        // This preserves accounts that were fetched on-demand
        if self.account_cache.contains_key(pubkey) {
            self.account_cache.insert(*pubkey, account).await;
            debug!("Updated existing cached account {}", pubkey);
        } else {
            // Skip uncached account
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