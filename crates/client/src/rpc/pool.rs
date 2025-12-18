//! Core RPC Pool Implementation
//!
//! Provides a robust RPC client pool with failover, load balancing,
//! and safe deserialization for Solana RPC responses.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, Result};
use base64::prelude::*;
use reqwest::Client;
use serde_json::json;
use solana_sdk::{
    hash::Hash,
    pubkey::Pubkey,
    signature::Signature,
    transaction::Transaction,
};

use super::config::{EndpointConfig, LoadBalanceStrategy, RpcPoolConfig};
use super::endpoint::{EndpointHealth, EndpointState};
use super::response::{RpcResponse, SafeSimulationResult, SafeUiAccount};

/// Error types for RPC operations
#[derive(Debug, thiserror::Error)]
pub enum RpcError {
    #[error("No healthy endpoints available")]
    NoHealthyEndpoints,
    #[error("All endpoints failed: {0}")]
    AllEndpointsFailed(String),
    #[error("Request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),
    #[error("JSON parsing failed: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("RPC error: {0}")]
    RpcError(String),
    #[error("Simulation error: {0}")]
    SimulationError(String),
    #[error("Invalid response: {0}")]
    InvalidResponse(String),
}

/// RPC response wrapper for JSON-RPC
#[derive(Debug, serde::Deserialize)]
struct JsonRpcResponse<T> {
    result: Option<T>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, serde::Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

/// Blockhash response
#[derive(Debug, serde::Deserialize)]
struct BlockhashResponse {
    value: BlockhashValue,
}

#[derive(Debug, serde::Deserialize)]
struct BlockhashValue {
    blockhash: String,
    #[serde(rename = "lastValidBlockHeight")]
    last_valid_block_height: u64,
}


/// Account response wrapper
#[derive(Debug, serde::Deserialize)]
struct AccountResponse {
    value: Option<SafeUiAccount>,
}

/// Program accounts response
#[derive(Debug, serde::Deserialize)]
struct ProgramAccountsItem {
    pubkey: String,
    account: SafeUiAccount,
}

/// Core RPC client pool
pub struct RpcPool {
    /// HTTP client with connection pooling
    http_client: Client,
    /// Configured endpoints with state tracking
    endpoints: Vec<Arc<EndpointState>>,
    /// Pool configuration
    config: RpcPoolConfig,
    /// Round-robin index for load balancing
    round_robin_idx: AtomicUsize,
}

impl RpcPool {
    /// Create a new RPC pool from configuration
    pub fn new(endpoint_configs: Vec<EndpointConfig>, config: RpcPoolConfig) -> Result<Self> {
        if endpoint_configs.is_empty() {
            return Err(anyhow!("At least one endpoint is required"));
        }

        // Build HTTP client with configuration
        let http_client = Client::builder()
            .connect_timeout(config.http.connect_timeout)
            .timeout(config.http.request_timeout)
            .pool_idle_timeout(config.http.pool_idle_timeout)
            .pool_max_idle_per_host(config.http.pool_max_idle_per_host)
            .build()?;

        // Create endpoint states
        let endpoints: Vec<Arc<EndpointState>> = endpoint_configs
            .into_iter()
            .map(|cfg| Arc::new(EndpointState::new(cfg)))
            .collect();

        Ok(Self {
            http_client,
            endpoints,
            config,
            round_robin_idx: AtomicUsize::new(0),
        })
    }

    /// Create a pool with a single endpoint URL
    pub fn with_url(url: impl Into<String>) -> Result<Self> {
        Self::new(
            vec![EndpointConfig::new(url)],
            RpcPoolConfig::default(),
        )
    }

    /// Get the latest blockhash
    pub async fn get_latest_blockhash(&self) -> Result<(Hash, u64)> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getLatestBlockhash",
            "params": [{
                "commitment": "confirmed"
            }]
        });

        let response: JsonRpcResponse<BlockhashResponse> = self.execute_with_failover(&body, true).await?;

        let result = response.result.ok_or_else(|| {
            anyhow!("No result in blockhash response")
        })?;

        let hash = result.value.blockhash.parse().map_err(|e| {
            anyhow!("Failed to parse blockhash: {}", e)
        })?;

        Ok((hash, result.value.last_valid_block_height))
    }

    /// Send a transaction
    pub async fn send_transaction(&self, transaction: &Transaction) -> Result<Signature> {
        let tx_bytes = bincode::serialize(transaction)?;
        let tx_base64 = BASE64_STANDARD.encode(&tx_bytes);

        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendTransaction",
            "params": [tx_base64, {
                "encoding": "base64",
                "skipPreflight": false,
                "preflightCommitment": "confirmed",
                "maxRetries": 3
            }]
        });

        let response: JsonRpcResponse<String> = self.execute_with_failover(&body, false).await?;

        let signature_str = response.result.ok_or_else(|| {
            anyhow!("No result in send transaction response")
        })?;

        signature_str.parse().map_err(|e| {
            anyhow!("Failed to parse signature: {}", e)
        })
    }

    /// Send a transaction and wait for confirmation
    ///
    /// Polls signature status until confirmed or timeout (30 seconds).
    pub async fn send_and_confirm_transaction(&self, transaction: &Transaction) -> Result<Signature> {
        let signature = self.send_transaction(transaction).await?;

        // Poll for confirmation with timeout
        let timeout = std::time::Duration::from_secs(30);
        let poll_interval = std::time::Duration::from_millis(500);
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                return Err(anyhow!("Transaction confirmation timeout: {}", signature));
            }

            match self.get_signature_status(&signature).await? {
                Some(Ok(())) => {
                    // Transaction confirmed successfully
                    return Ok(signature);
                }
                Some(Err(e)) => {
                    // Transaction failed
                    return Err(anyhow!("Transaction failed: {:?}", e));
                }
                None => {
                    // Not yet confirmed, keep polling
                    tokio::time::sleep(poll_interval).await;
                }
            }
        }
    }

    /// Get account info with safe deserialization
    pub async fn get_account(&self, pubkey: &Pubkey) -> Result<Option<SafeUiAccount>> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getAccountInfo",
            "params": [pubkey.to_string(), {
                "encoding": "base64+zstd",
                "commitment": "confirmed"
            }]
        });

        let response: JsonRpcResponse<AccountResponse> = self.execute_with_failover(&body, true).await?;

        Ok(response.result.and_then(|r| r.value))
    }

    /// Get account balance in lamports
    pub async fn get_balance(&self, pubkey: &Pubkey) -> Result<u64> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getBalance",
            "params": [pubkey.to_string(), {
                "commitment": "confirmed"
            }]
        });

        #[derive(serde::Deserialize)]
        struct BalanceResponse {
            value: u64,
        }

        let response: JsonRpcResponse<BalanceResponse> = self.execute_with_failover(&body, true).await?;

        response.result
            .map(|r| r.value)
            .ok_or_else(|| anyhow!("No result in balance response"))
    }

    /// Get multiple accounts
    pub async fn get_multiple_accounts(&self, pubkeys: &[Pubkey]) -> Result<Vec<Option<SafeUiAccount>>> {
        let addresses: Vec<String> = pubkeys.iter().map(|p| p.to_string()).collect();

        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getMultipleAccounts",
            "params": [addresses, {
                "encoding": "base64+zstd",
                "commitment": "confirmed"
            }]
        });

        #[derive(serde::Deserialize)]
        struct MultipleAccountsResponse {
            value: Vec<Option<SafeUiAccount>>,
        }

        let response: JsonRpcResponse<MultipleAccountsResponse> = self.execute_with_failover(&body, true).await?;

        Ok(response.result.map(|r| r.value).unwrap_or_default())
    }

    /// Get program accounts with optional filters
    pub async fn get_program_accounts(
        &self,
        program_id: &Pubkey,
        filters: Option<Vec<serde_json::Value>>,
    ) -> Result<Vec<(Pubkey, SafeUiAccount)>> {
        let mut params = json!({
            "encoding": "base64+zstd",
            "commitment": "confirmed"
        });

        if let Some(f) = filters {
            params["filters"] = json!(f);
        }

        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getProgramAccounts",
            "params": [program_id.to_string(), params]
        });

        let response: JsonRpcResponse<Vec<ProgramAccountsItem>> = self.execute_with_failover(&body, true).await?;

        let items = response.result.unwrap_or_default();
        let mut accounts = Vec::with_capacity(items.len());

        for item in items {
            let pubkey: Pubkey = item.pubkey.parse().map_err(|e| {
                anyhow!("Failed to parse pubkey: {}", e)
            })?;
            accounts.push((pubkey, item.account));
        }

        Ok(accounts)
    }

    /// Simulate a transaction and return accounts
    pub async fn simulate_transaction(
        &self,
        transaction: &Transaction,
        account_addresses: &[Pubkey],
    ) -> Result<SafeSimulationResult> {
        let tx_bytes = bincode::serialize(transaction)?;
        let tx_base64 = BASE64_STANDARD.encode(&tx_bytes);

        let addresses: Vec<String> = account_addresses.iter().map(|p| p.to_string()).collect();

        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "simulateTransaction",
            "params": [tx_base64, {
                "sigVerify": false,
                "replaceRecentBlockhash": true,
                "commitment": "processed",
                "encoding": "base64",
                "accounts": {
                    "encoding": "base64+zstd",
                    "addresses": addresses
                }
            }]
        });

        let response: RpcResponse<SafeSimulationResult> = self.execute_with_failover(&body, true).await?;

        // Check for simulation error
        if let Some(err) = &response.result.value.err {
            return Err(anyhow!("Simulation error: {:?}", err));
        }

        Ok(response.result)
    }

    /// Get signature status for confirmation checking
    pub async fn get_signature_status(
        &self,
        signature: &Signature,
    ) -> Result<Option<Result<(), solana_sdk::transaction::TransactionError>>> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getSignatureStatuses",
            "params": [[signature.to_string()]]
        });

        #[derive(serde::Deserialize)]
        struct SignatureStatusResponse {
            value: Vec<Option<SignatureStatus>>,
        }

        #[derive(serde::Deserialize)]
        struct SignatureStatus {
            err: Option<serde_json::Value>,
            #[serde(rename = "confirmationStatus")]
            confirmation_status: Option<String>,
        }

        let response: JsonRpcResponse<SignatureStatusResponse> =
            self.execute_with_failover(&body, true).await?;

        let statuses = response.result.map(|r| r.value).unwrap_or_default();

        if let Some(Some(status)) = statuses.into_iter().next() {
            // Check confirmation status
            let confirmed = status
                .confirmation_status
                .map(|s| s == "confirmed" || s == "finalized")
                .unwrap_or(false);

            if !confirmed {
                return Ok(None); // Not yet confirmed
            }

            // Check for error
            if let Some(err) = status.err {
                // Try to parse into TransactionError
                // For now, return a generic error
                return Ok(Some(Err(
                    solana_sdk::transaction::TransactionError::InstructionError(
                        0,
                        solana_sdk::instruction::InstructionError::Custom(
                            err.get("InstructionError")
                                .and_then(|e| e.get(1))
                                .and_then(|e| e.get("Custom"))
                                .and_then(|e| e.as_u64())
                                .unwrap_or(0) as u32,
                        ),
                    ),
                )));
            }

            Ok(Some(Ok(())))
        } else {
            Ok(None) // Signature not found
        }
    }

    /// Execute a request with failover across healthy endpoints
    async fn execute_with_failover<T>(&self, body: &serde_json::Value, read_only: bool) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let endpoints = self.select_endpoints(read_only);

        if endpoints.is_empty() {
            return Err(anyhow!(RpcError::NoHealthyEndpoints));
        }

        let mut last_error = None;

        for endpoint in &endpoints {
            let start = Instant::now();

            match self.execute_request(endpoint, body).await {
                Ok(response) => {
                    endpoint.record_success(start.elapsed());
                    return Ok(response);
                }
                Err(e) => {
                    endpoint.record_failure();
                    log::warn!(
                        "RPC request failed for {}: {}",
                        endpoint.url(),
                        e
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("No endpoints to try")))
    }

    /// Execute a single request to an endpoint
    async fn execute_request<T>(
        &self,
        endpoint: &EndpointState,
        body: &serde_json::Value,
    ) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let response = self
            .http_client
            .post(endpoint.url())
            .json(body)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "HTTP error: {} - {}",
                response.status(),
                response.text().await.unwrap_or_default()
            ));
        }

        let text = response.text().await?;

        // Try to parse as JSON-RPC error first
        if let Ok(error_response) = serde_json::from_str::<JsonRpcResponse<serde_json::Value>>(&text) {
            if let Some(error) = error_response.error {
                return Err(anyhow!(RpcError::RpcError(format!(
                    "code {}: {}",
                    error.code, error.message
                ))));
            }
        }

        serde_json::from_str(&text).map_err(|e| {
            anyhow!("JSON parse error: {} - Response: {}", e, &text[..text.len().min(500)])
        })
    }

    /// Select endpoints for a request based on load balancing strategy
    fn select_endpoints(&self, read_only: bool) -> Vec<Arc<EndpointState>> {
        // Filter by role and health
        let available: Vec<_> = self
            .endpoints
            .iter()
            .filter(|e| {
                let role_ok = if read_only {
                    e.can_fetch()
                } else {
                    e.can_submit()
                };
                role_ok && e.is_available()
            })
            .cloned()
            .collect();

        if available.is_empty() {
            return vec![];
        }

        match self.config.load_balance_strategy {
            LoadBalanceStrategy::RoundRobin => {
                let idx = self.round_robin_idx.fetch_add(1, Ordering::Relaxed);
                let start = idx % available.len();
                // Return all available endpoints starting from round-robin index
                let mut result = Vec::with_capacity(available.len());
                for i in 0..available.len() {
                    result.push(available[(start + i) % available.len()].clone());
                }
                result
            }
            LoadBalanceStrategy::Priority => {
                // Sort by priority (lower = higher priority)
                let mut sorted = available;
                sorted.sort_by_key(|e| e.priority());
                sorted
            }
            LoadBalanceStrategy::LeastLatency => {
                // Sort by average latency
                let mut sorted = available;
                sorted.sort_by_key(|e| e.avg_latency());
                sorted
            }
            LoadBalanceStrategy::WeightedRoundRobin => {
                // Weighted by inverse priority (lower priority = more weight)
                // For now, just use priority-based ordering
                let mut sorted = available;
                sorted.sort_by_key(|e| e.priority());
                sorted
            }
        }
    }

    /// Get statistics for all endpoints
    pub fn stats(&self) -> Vec<(String, super::endpoint::EndpointStats)> {
        self.endpoints
            .iter()
            .map(|e| (e.url().to_string(), e.stats()))
            .collect()
    }

    /// Get number of healthy endpoints
    pub fn healthy_count(&self) -> usize {
        self.endpoints
            .iter()
            .filter(|e| e.health() == EndpointHealth::Healthy)
            .count()
    }

    /// Get total number of endpoints
    pub fn endpoint_count(&self) -> usize {
        self.endpoints.len()
    }

    /// Mark an endpoint as unhealthy by URL
    pub fn mark_unhealthy(&self, url: &str) {
        if let Some(endpoint) = self.endpoints.iter().find(|e| e.url() == url) {
            endpoint.mark_unhealthy();
        }
    }

    /// Mark an endpoint as healthy by URL
    pub fn mark_healthy(&self, url: &str) {
        if let Some(endpoint) = self.endpoints.iter().find(|e| e.url() == url) {
            endpoint.mark_healthy();
        }
    }
}

impl std::fmt::Debug for RpcPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpcPool")
            .field("endpoints", &self.endpoints.len())
            .field("healthy", &self.healthy_count())
            .field("strategy", &self.config.load_balance_strategy)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_creation() {
        let pool = RpcPool::with_url("https://api.devnet.solana.com").unwrap();
        assert_eq!(pool.endpoint_count(), 1);
        assert_eq!(pool.healthy_count(), 1);
    }

    #[test]
    fn test_pool_requires_endpoints() {
        let result = RpcPool::new(vec![], RpcPoolConfig::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_multiple_endpoints() {
        let pool = RpcPool::new(
            vec![
                EndpointConfig::new("https://api.devnet.solana.com"),
                EndpointConfig::new("https://api.mainnet-beta.solana.com"),
            ],
            RpcPoolConfig::default(),
        )
        .unwrap();

        assert_eq!(pool.endpoint_count(), 2);
        assert_eq!(pool.healthy_count(), 2);
    }

    #[test]
    fn test_mark_unhealthy() {
        let pool = RpcPool::new(
            vec![
                EndpointConfig::new("https://api.devnet.solana.com"),
                EndpointConfig::new("https://api.mainnet-beta.solana.com"),
            ],
            RpcPoolConfig::default(),
        )
        .unwrap();

        pool.mark_unhealthy("https://api.devnet.solana.com");
        assert_eq!(pool.healthy_count(), 1);

        pool.mark_healthy("https://api.devnet.solana.com");
        assert_eq!(pool.healthy_count(), 2);
    }
}
