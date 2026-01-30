//! Account caching with Moka
//!
//! Provides a thread-safe account cache as the single source of truth for account data.
//! Uses per-entry variable expiration:
//! - Time triggers: expire after trigger_time + grace_period
//! - Slot/Epoch/Account triggers: no TTL (persist until capacity eviction)

use crate::config::CacheConfig;
use crate::rpc::RpcPool;
use anchor_lang::AccountDeserialize;
use antegen_thread_program::state::{Schedule, Thread, Trigger};
use base64::prelude::*;
use moka::future::Cache;
use moka::notification::RemovalCause;
use moka::policy::Expiry;
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Trigger type for cache expiration logic
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheTriggerType {
    /// Time-based trigger with next execution timestamp (unix seconds)
    Time { next_timestamp: i64 },
    /// Block-based trigger (slot/epoch) - no TTL
    Block,
    /// Account-based trigger - no TTL
    Account,
    /// Unknown/immediate - no TTL
    Unknown,
}

impl CacheTriggerType {
    /// Extract trigger type from Thread data
    pub fn from_thread(thread: &Thread) -> Self {
        match &thread.trigger {
            Trigger::Timestamp { .. }
            | Trigger::Cron { .. }
            | Trigger::Interval { .. }
            | Trigger::Immediate { .. } => {
                // Extract next execution time from schedule
                if let Schedule::Timed { next, .. } = &thread.schedule {
                    CacheTriggerType::Time {
                        next_timestamp: *next,
                    }
                } else {
                    CacheTriggerType::Unknown
                }
            }
            Trigger::Slot { .. } | Trigger::Epoch { .. } => CacheTriggerType::Block,
            Trigger::Account { .. } => CacheTriggerType::Account,
        }
    }
}

/// Cached account data with metadata
#[derive(Debug, Clone)]
pub struct CachedAccount {
    pub data: Vec<u8>,
    pub slot: u64,
    pub hash: u64,
    /// Trigger type for expiration calculation
    pub trigger_type: CacheTriggerType,
}

/// Per-entry expiration policy
/// - Time triggers: expire after trigger_time + grace_period + eviction_buffer
/// - Other triggers: no expiration
struct ThreadExpiry {
    grace_period: u64,
    eviction_buffer: u64,
}

impl Expiry<Pubkey, CachedAccount> for ThreadExpiry {
    fn expire_after_create(
        &self,
        _key: &Pubkey,
        value: &CachedAccount,
        _created_at: Instant,
    ) -> Option<Duration> {
        self.calculate_ttl(value)
    }

    fn expire_after_update(
        &self,
        _key: &Pubkey,
        value: &CachedAccount,
        _updated_at: Instant,
        _duration_until_expiry: Option<Duration>,
    ) -> Option<Duration> {
        // Recalculate TTL on update (trigger context may have changed)
        self.calculate_ttl(value)
    }

    fn expire_after_read(
        &self,
        _key: &Pubkey,
        _value: &CachedAccount,
        _read_at: Instant,
        duration_until_expiry: Option<Duration>,
        _last_modified_at: Instant,
    ) -> Option<Duration> {
        // Don't change TTL on read
        duration_until_expiry
    }
}

impl ThreadExpiry {
    fn calculate_ttl(&self, value: &CachedAccount) -> Option<Duration> {
        match value.trigger_type {
            CacheTriggerType::Time { next_timestamp } => {
                // 0 or i64::MAX means "no next execution" (e.g., Immediate trigger after first exec)
                // Treat as no expiration
                if next_timestamp == 0 || next_timestamp == i64::MAX {
                    return None;
                }

                let now = chrono::Utc::now().timestamp();
                // Cache TTL = trigger_time + grace_period + eviction_buffer
                // This gives time for takeover attempts before cache eviction
                let expire_at = next_timestamp
                    .saturating_add(self.grace_period as i64)
                    .saturating_add(self.eviction_buffer as i64);

                if expire_at > now {
                    // Safe subtraction since we checked expire_at > now
                    let diff = expire_at.saturating_sub(now);
                    // Clamp to reasonable max (1 day) to prevent issues
                    let secs = (diff as u64).min(86400);
                    Some(Duration::from_secs(secs))
                } else {
                    // Already past expiration, set minimal TTL to allow cleanup
                    Some(Duration::from_secs(1))
                }
            }
            // Block, Account, Unknown triggers: no expiration
            _ => None,
        }
    }
}

/// Thread-safe account cache - single source of truth for account data
pub struct AccountCache {
    cache: Cache<Pubkey, CachedAccount>,
    grace_period: u64,
    /// Channel to notify when cache entries expire (for refetch)
    /// Note: Stored here for lifetime management; actual send happens in eviction_listener closure
    _eviction_tx: Option<mpsc::UnboundedSender<Pubkey>>,
}

impl AccountCache {
    /// Create a new account cache with default settings
    pub fn new() -> Self {
        Self::with_config(&CacheConfig::default(), 10, 20, None)
    }

    /// Create a new account cache from config
    pub fn with_config(
        config: &CacheConfig,
        grace_period: u64,
        eviction_buffer: u64,
        eviction_tx: Option<mpsc::UnboundedSender<Pubkey>>,
    ) -> Self {
        let expiry = ThreadExpiry {
            grace_period,
            eviction_buffer,
        };
        let eviction_tx_clone = eviction_tx.clone();

        Self {
            cache: Cache::builder()
                .max_capacity(config.max_capacity)
                .expire_after(expiry)
                .eviction_listener(move |key: Arc<Pubkey>, _value, cause| {
                    // Log evictions for debugging
                    match cause {
                        RemovalCause::Expired => {
                            log::debug!("Cache entry expired: {}", key);
                            // Notify listener to trigger refetch
                            if let Some(ref tx) = eviction_tx_clone {
                                let _ = tx.send(*key); // Fire and forget
                            }
                        }
                        RemovalCause::Size => {
                            log::debug!("Cache entry evicted (capacity): {}", key);
                        }
                        RemovalCause::Explicit => {
                            log::trace!("Cache entry explicitly removed: {}", key);
                        }
                        RemovalCause::Replaced => {
                            log::trace!("Cache entry replaced: {}", key);
                        }
                    }
                })
                .build(),
            grace_period,
            _eviction_tx: eviction_tx,
        }
    }

    /// Create a new account cache with custom capacity (for testing)
    #[cfg(test)]
    pub fn with_capacity(max_capacity: u64) -> Self {
        Self {
            cache: Cache::builder().max_capacity(max_capacity).build(),
            grace_period: 10,
            _eviction_tx: None,
        }
    }

    /// Get a cached account
    pub async fn get(&self, key: &Pubkey) -> Option<CachedAccount> {
        self.cache.get(key).await
    }

    /// Put an account in the cache with trigger type for expiration
    pub async fn put(&self, key: Pubkey, data: Vec<u8>, slot: u64, trigger_type: CacheTriggerType) {
        let hash = seahash::hash(&data);
        self.cache
            .insert(
                key,
                CachedAccount {
                    data,
                    slot,
                    hash,
                    trigger_type,
                },
            )
            .await;
    }

    /// Put an account in the cache (legacy, uses Unknown trigger type)
    pub async fn put_simple(&self, key: Pubkey, data: Vec<u8>, slot: u64) {
        self.put(key, data, slot, CacheTriggerType::Unknown).await;
    }

    /// Invalidate a specific account
    pub async fn invalidate(&self, key: &Pubkey) {
        self.cache.invalidate(key).await;
    }

    /// Put account data only if it's newer than cached version
    /// Returns true if data was actually updated (not a duplicate)
    /// This serves as both caching AND deduplication in one operation
    pub async fn put_if_newer(&self, key: Pubkey, data: Vec<u8>, slot: u64) -> bool {
        let new_hash = seahash::hash(&data);

        if let Some(existing) = self.cache.get(&key).await {
            // Same hash = identical data (duplicate)
            if existing.hash == new_hash {
                return false;
            }
            // Older slot = stale data
            if slot < existing.slot {
                return false;
            }
        }

        // Try to deserialize to get trigger type
        let trigger_type = if let Ok(thread) = Thread::try_deserialize(&mut data.as_slice()) {
            CacheTriggerType::from_thread(&thread)
        } else {
            CacheTriggerType::Unknown
        };

        self.cache
            .insert(
                key,
                CachedAccount {
                    data,
                    slot,
                    hash: new_hash,
                    trigger_type,
                },
            )
            .await;
        true
    }

    /// Get thread from cache, or fetch from RPC if not cached
    /// This is the primary method for getting thread data for execution
    pub async fn get_thread_or_fetch(
        &self,
        key: &Pubkey,
        rpc_client: &Arc<RpcPool>,
    ) -> Result<Thread, String> {
        // Try cache first
        if let Some(cached) = self.cache.get(key).await {
            // Deserialize thread from cached data
            return Thread::try_deserialize(&mut cached.data.as_slice())
                .map_err(|e| format!("Failed to deserialize cached thread: {}", e));
        }

        // Cache miss - fetch from RPC
        log::debug!("Cache miss for thread {}, fetching from RPC", key);

        let ui_account = rpc_client
            .get_account(key)
            .await
            .map_err(|e| format!("Failed to fetch account {}: {}", key, e))?
            .ok_or_else(|| format!("Account {} not found", key))?;

        // Decode base64 account data
        let account_data = BASE64_STANDARD
            .decode(&ui_account.data.0)
            .map_err(|e| format!("Failed to decode account data: {}", e))?;

        // Deserialize to get trigger type
        let thread = Thread::try_deserialize(&mut account_data.as_slice())
            .map_err(|e| format!("Failed to deserialize fetched thread: {}", e))?;

        let trigger_type = CacheTriggerType::from_thread(&thread);

        // Store in cache with trigger type
        let slot = 0; // We don't have slot info from get_account, use 0
        self.put(*key, account_data, slot, trigger_type).await;

        Ok(thread)
    }

    /// Get current cache size
    pub fn entry_count(&self) -> u64 {
        self.cache.entry_count()
    }

    /// Get cache stats (hits, misses, etc.)
    pub fn weighted_size(&self) -> u64 {
        self.cache.weighted_size()
    }

    /// Get configured grace period
    pub fn grace_period(&self) -> u64 {
        self.grace_period
    }

    /// Run pending maintenance tasks (for testing)
    #[cfg(test)]
    pub async fn run_pending_tasks(&self) {
        self.cache.run_pending_tasks().await;
    }
}

impl Default for AccountCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;

    #[tokio::test]
    async fn test_cache_put_and_get() {
        let cache = AccountCache::new();
        let pubkey = Pubkey::new_unique();
        let data = vec![1, 2, 3, 4];
        let slot = 100;

        // Put
        cache.put_simple(pubkey, data.clone(), slot).await;

        // Get
        let cached = cache.get(&pubkey).await.unwrap();
        assert_eq!(cached.data, data);
        assert_eq!(cached.slot, slot);
        assert_eq!(cached.hash, seahash::hash(&data));
    }

    #[tokio::test]
    async fn test_cache_miss() {
        let cache = AccountCache::new();
        let pubkey = Pubkey::new_unique();

        // Should return None for non-existent key
        assert!(cache.get(&pubkey).await.is_none());
    }

    #[tokio::test]
    async fn test_cache_invalidate() {
        let cache = AccountCache::new();
        let pubkey = Pubkey::new_unique();
        let data = vec![1, 2, 3, 4];

        cache.put_simple(pubkey, data.clone(), 100).await;
        assert!(cache.get(&pubkey).await.is_some());

        cache.invalidate(&pubkey).await;
        assert!(cache.get(&pubkey).await.is_none());
    }

    #[tokio::test]
    async fn test_cache_update() {
        let cache = AccountCache::new();
        let pubkey = Pubkey::new_unique();

        // Initial value
        cache.put_simple(pubkey, vec![1, 2, 3], 100).await;
        let cached = cache.get(&pubkey).await.unwrap();
        assert_eq!(cached.data, vec![1, 2, 3]);
        assert_eq!(cached.slot, 100);

        // Update
        cache.put_simple(pubkey, vec![4, 5, 6], 200).await;
        let cached = cache.get(&pubkey).await.unwrap();
        assert_eq!(cached.data, vec![4, 5, 6]);
        assert_eq!(cached.slot, 200);
    }

    #[tokio::test]
    async fn test_cache_entry_count() {
        let cache = AccountCache::new();

        assert_eq!(cache.entry_count(), 0);

        cache.put_simple(Pubkey::new_unique(), vec![1], 100).await;
        cache.put_simple(Pubkey::new_unique(), vec![2], 100).await;

        // Give cache time to update entry count
        cache.run_pending_tasks().await;

        assert_eq!(cache.entry_count(), 2);
    }

    #[tokio::test]
    async fn test_no_ttl_for_block_triggers() {
        // Create cache with small capacity but block trigger type
        let cache = AccountCache::with_capacity(100);
        let pubkey = Pubkey::new_unique();

        cache
            .put(pubkey, vec![1, 2, 3], 100, CacheTriggerType::Block)
            .await;
        assert!(cache.get(&pubkey).await.is_some());

        // Wait a bit - should NOT be evicted (block triggers have no TTL)
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        // Should still be there
        assert!(cache.get(&pubkey).await.is_some());
    }

    #[tokio::test]
    async fn test_ttl_for_time_triggers() {
        // Create cache with grace period and time trigger in the past
        let config = CacheConfig { max_capacity: 100 };
        let cache = AccountCache::with_config(&config, 1, 0, None); // 1 second grace period, no eviction buffer
        let pubkey = Pubkey::new_unique();

        // Set trigger time in the past (should expire quickly)
        let past_timestamp = chrono::Utc::now().timestamp() - 10;
        cache
            .put(
                pubkey,
                vec![1, 2, 3],
                100,
                CacheTriggerType::Time {
                    next_timestamp: past_timestamp,
                },
            )
            .await;

        assert!(cache.get(&pubkey).await.is_some());

        // Wait for expiration (past trigger + grace period = already expired, uses 1s min TTL)
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        cache.run_pending_tasks().await;

        // Should be evicted
        assert!(cache.get(&pubkey).await.is_none());
    }

    #[tokio::test]
    async fn test_put_if_newer_dedup() {
        let cache = AccountCache::new();
        let pubkey = Pubkey::new_unique();
        let data = vec![1, 2, 3, 4];

        // First insert should succeed
        assert!(cache.put_if_newer(pubkey, data.clone(), 100).await);

        // Same data, same slot = duplicate, should return false
        assert!(!cache.put_if_newer(pubkey, data.clone(), 100).await);

        // Same data, higher slot = duplicate content, should return false
        assert!(!cache.put_if_newer(pubkey, data.clone(), 200).await);

        // Different data, higher slot = new data, should succeed
        assert!(cache.put_if_newer(pubkey, vec![5, 6, 7, 8], 300).await);

        // Verify the new data is stored
        let cached = cache.get(&pubkey).await.unwrap();
        assert_eq!(cached.data, vec![5, 6, 7, 8]);
        assert_eq!(cached.slot, 300);
    }

    #[tokio::test]
    async fn test_put_if_newer_stale() {
        let cache = AccountCache::new();
        let pubkey = Pubkey::new_unique();

        // Insert at slot 200
        assert!(cache.put_if_newer(pubkey, vec![1, 2, 3], 200).await);

        // Try to insert different data at older slot = stale, should return false
        assert!(!cache.put_if_newer(pubkey, vec![4, 5, 6], 100).await);

        // Verify original data is still there
        let cached = cache.get(&pubkey).await.unwrap();
        assert_eq!(cached.data, vec![1, 2, 3]);
        assert_eq!(cached.slot, 200);
    }

    #[tokio::test]
    async fn test_trigger_type_extraction() {
        // Test Unknown trigger type (no expiration)
        let trigger = CacheTriggerType::Unknown;
        assert_eq!(trigger, CacheTriggerType::Unknown);

        // Test Time trigger type
        let trigger = CacheTriggerType::Time {
            next_timestamp: 12345,
        };
        match trigger {
            CacheTriggerType::Time { next_timestamp } => assert_eq!(next_timestamp, 12345),
            _ => panic!("Expected Time trigger"),
        }

        // Test Block trigger type
        let trigger = CacheTriggerType::Block;
        assert_eq!(trigger, CacheTriggerType::Block);

        // Test Account trigger type
        let trigger = CacheTriggerType::Account;
        assert_eq!(trigger, CacheTriggerType::Account);
    }
}
