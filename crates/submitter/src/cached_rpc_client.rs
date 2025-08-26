use crate::metrics::SubmitterMetrics;
use anchor_lang::prelude::*;
use antegen_thread_program::state::{FiberState, ThreadConfig};
use anyhow::Result;
use log::debug;
use moka::future::Cache;
use solana_client::{
    nonblocking::rpc_client::RpcClient,
    rpc_config::RpcSimulateTransactionConfig,
    rpc_response::{Response, RpcSimulateTransactionResult},
};
use solana_sdk::{
    account::Account,
    commitment_config::CommitmentConfig,
    hash::Hash,
    pubkey::Pubkey,
    signature::Signature,
    transaction::{Transaction, TransactionError},
};
use std::sync::Arc;
use std::time::Duration;

/// Configuration for the cached RPC client
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// TTL for account cache in seconds  
    pub account_ttl_secs: u64,
    /// Maximum number of cached accounts
    pub max_cached_accounts: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            account_ttl_secs: 48 * 3600,  // 48 hours - safe with observer updates
            max_cached_accounts: 10000,   // Increased limit with longer TTL
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

    /// Get latest blockhash (no caching for transaction uniqueness)
    pub async fn get_latest_blockhash(&self) -> Result<Hash> {
        if let Some(ref metrics) = self.metrics {
            metrics.rpc_request("get_latest_blockhash");
        }
        Ok(self.inner.get_latest_blockhash().await?)
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

    /// Get thread config with caching (uses regular account cache)
    pub async fn get_thread_config(&self) -> Result<ThreadConfig> {
        let config_pubkey =
            Pubkey::find_program_address(&[b"thread_config"], &antegen_thread_program::ID).0;

        // Use regular account cache
        let account = self.get_account(&config_pubkey).await?;
        let config = ThreadConfig::try_deserialize(&mut account.data.as_slice())?;

        Ok(config)
    }

    /// Get fiber state (uses account cache)
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

    /// Update account in cache
    /// We update unconditionally to ensure fresh data is always available
    pub async fn update_account_if_cached(&self, pubkey: &Pubkey, account: Account) {
        // Update the account cache with fresh data from observer
        self.account_cache.insert(*pubkey, account).await;
        debug!("Updated account cache for {} from observer", pubkey);
        
        if let Some(ref metrics) = self.metrics {
            metrics.cache_hit("account_update");
        }
    }

    // ===== Pass-through methods (no caching) =====

    /// Send transaction (no caching)
    pub async fn send_transaction(&self, transaction: &Transaction) -> Result<Signature> {
        if let Some(ref metrics) = self.metrics {
            metrics.rpc_request("send_transaction");
        }
        Ok(self.inner.send_transaction(transaction).await?)
    }

    /// Send and confirm transaction (no caching)
    pub async fn send_and_confirm_transaction(
        &self,
        transaction: &Transaction,
    ) -> Result<Signature> {
        if let Some(ref metrics) = self.metrics {
            metrics.rpc_request("send_and_confirm_transaction");
        }
        Ok(self.inner.send_and_confirm_transaction(transaction).await?)
    }

    /// Simulate transaction (no caching)
    pub async fn simulate_transaction_with_config(
        &self,
        transaction: &Transaction,
        config: RpcSimulateTransactionConfig,
    ) -> Result<Response<RpcSimulateTransactionResult>> {
        if let Some(ref metrics) = self.metrics {
            metrics.rpc_request("simulate_transaction");
        }
        Ok(self
            .inner
            .simulate_transaction_with_config(transaction, config)
            .await?)
    }

    /// Get signature status (no caching)  
    pub async fn get_signature_status(
        &self,
        signature: &Signature,
    ) -> Result<Option<Result<(), TransactionError>>> {
        if let Some(ref metrics) = self.metrics {
            metrics.rpc_request("get_signature_status");
        }
        Ok(self.inner.get_signature_status(signature).await?)
    }

    /// Get signature status with commitment (no caching)
    pub async fn get_signature_status_with_commitment(
        &self,
        signature: &Signature,
        commitment: CommitmentConfig,
    ) -> Result<Option<Result<(), TransactionError>>> {
        if let Some(ref metrics) = self.metrics {
            metrics.rpc_request("get_signature_status");
        }
        Ok(self
            .inner
            .get_signature_status_with_commitment(signature, commitment)
            .await?)
    }

    /// Get health (no caching)
    pub async fn get_health(&self) -> Result<()> {
        Ok(self.inner.get_health().await?)
    }

    /// Get slot with commitment (no caching)
    pub async fn get_slot_with_commitment(&self, commitment: CommitmentConfig) -> Result<u64> {
        if let Some(ref metrics) = self.metrics {
            metrics.rpc_request("get_slot");
        }
        Ok(self.inner.get_slot_with_commitment(commitment).await?)
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

    /// Get the underlying RPC URL
    pub fn url(&self) -> String {
        self.inner.url().to_string()
    }
}
