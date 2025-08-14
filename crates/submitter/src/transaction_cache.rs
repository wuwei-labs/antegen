use anyhow::Result;
use solana_sdk::{pubkey::Pubkey, signature::Signature};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Stores built transactions for sharing between builders and repeaters
#[derive(Clone)]
pub struct TransactionCache {
    /// Map of thread pubkey to built transaction and builder info
    cache: Arc<RwLock<HashMap<Pubkey, CachedTransaction>>>,
}

#[derive(Clone, Debug)]
pub struct CachedTransaction {
    pub transaction: Vec<u8>, // Serialized transaction without fee payer
    pub builder_id: u32,
    pub timestamp: i64,
    pub signature: Option<Signature>,
}

impl TransactionCache {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Store a built transaction
    pub async fn store(&self, thread_pubkey: Pubkey, tx: CachedTransaction) -> Result<()> {
        let mut cache = self.cache.write().await;
        cache.insert(thread_pubkey, tx);
        Ok(())
    }

    /// Retrieve a built transaction
    pub async fn get(&self, thread_pubkey: &Pubkey) -> Option<CachedTransaction> {
        let cache = self.cache.read().await;
        cache.get(thread_pubkey).cloned()
    }

    /// Remove a transaction from cache
    pub async fn remove(&self, thread_pubkey: &Pubkey) -> Option<CachedTransaction> {
        let mut cache = self.cache.write().await;
        cache.remove(thread_pubkey)
    }

    /// Clean up old transactions
    pub async fn cleanup_old(&self, current_timestamp: i64, max_age_seconds: i64) {
        let mut cache = self.cache.write().await;
        cache.retain(|_, tx| current_timestamp - tx.timestamp < max_age_seconds);
    }

    /// Get all cached transactions
    pub async fn get_all(&self) -> Vec<(Pubkey, CachedTransaction)> {
        let cache = self.cache.read().await;
        cache.iter().map(|(k, v)| (*k, v.clone())).collect()
    }
}

impl Default for TransactionCache {
    fn default() -> Self {
        Self::new()
    }
}
