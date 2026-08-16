//! Core RPC Pool Implementation
//!
//! Provides a robust RPC client pool with failover, load balancing,
//! and safe deserialization for Solana RPC responses.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use anyhow::{anyhow, Result};
use base64::prelude::*;
use reqwest::Client;
use serde_json::json;
use solana_sdk::{hash::Hash, pubkey::Pubkey, signature::Signature, transaction::Transaction};

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

/// A `getProgramAccounts` entry with the account body ignored, for callers that
/// only asked which accounts exist. Deliberately not `ProgramAccountsItem` —
/// this must keep deserializing whatever a zero-length `dataSlice` returns.
#[derive(Debug, serde::Deserialize)]
struct ProgramAccountKey {
    pubkey: String,
}

/// `getProgramAccounts` with `withContext: true`, which wraps the array so the
/// slot the snapshot was taken at is available.
#[derive(Debug, serde::Deserialize)]
struct ProgramAccountsWithContext {
    context: ResponseContext,
    value: Vec<ProgramAccountsItem>,
}

#[derive(Debug, serde::Deserialize)]
struct ResponseContext {
    slot: u64,
}

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

/// How long a cached blockhash is served before being refreshed.
///
/// A blockhash is valid for ~150 slots (~60s), so a value a couple of seconds
/// old is indistinguishable in practice from a freshly fetched one — but
/// fetching it sits on the critical path between a trigger firing and the
/// transaction going out.
const BLOCKHASH_TTL: Duration = Duration::from_secs(2);

/// Deadline for RPC calls that sit between a trigger firing and the transaction
/// going out.
///
/// The pool-wide HTTP timeout is sized for bulk calls like `getProgramAccounts`;
/// applying it here means a single hung endpoint stalls an execution for that
/// long before failover even begins. These calls are cheap, so anything slower
/// than this is better served by moving to the next endpoint.
const HOT_PATH_TIMEOUT: Duration = Duration::from_millis(1_500);

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
    /// Most recent blockhash, with the instant it was fetched.
    blockhash: RwLock<Option<(Hash, u64, Instant)>>,
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
            blockhash: RwLock::new(None),
        })
    }

    /// Create a pool with a single endpoint URL
    pub fn with_url(url: impl Into<String>) -> Result<Self> {
        Self::new(vec![EndpointConfig::new(url)], RpcPoolConfig::default())
    }

    /// Get the latest blockhash, served from a short-lived cache.
    ///
    /// Callers on the execution path hit this repeatedly; going to the network
    /// each time added round trips after the trigger deadline had already
    /// passed. Use [`RpcPool::refresh_blockhash`] from a background task to keep
    /// the cache warm.
    pub async fn get_latest_blockhash(&self) -> Result<(Hash, u64)> {
        if let Some((hash, height, fetched_at)) = *self.blockhash.read().await {
            if fetched_at.elapsed() < BLOCKHASH_TTL {
                return Ok((hash, height));
            }
        }
        self.refresh_blockhash().await
    }

    /// Fetch a blockhash from the network and update the cache.
    pub async fn refresh_blockhash(&self) -> Result<(Hash, u64)> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getLatestBlockhash",
            "params": [{
                "commitment": "confirmed"
            }]
        });

        let response: JsonRpcResponse<BlockhashResponse> = self
            .execute_with_deadline(&body, true, Some(HOT_PATH_TIMEOUT))
            .await?;

        let result = response
            .result
            .ok_or_else(|| anyhow!("No result in blockhash response"))?;

        let hash = result
            .value
            .blockhash
            .parse()
            .map_err(|e| anyhow!("Failed to parse blockhash: {}", e))?;

        let height = result.value.last_valid_block_height;
        *self.blockhash.write().await = Some((hash, height, Instant::now()));

        Ok((hash, height))
    }

    /// Seed the blockhash cache directly. Test-only.
    #[cfg(test)]
    pub(crate) async fn prime_blockhash(&self, hash: Hash, height: u64, fetched_at: Instant) {
        *self.blockhash.write().await = Some((hash, height, fetched_at));
    }

    /// Keep the blockhash cache warm so the execution path never has to fetch
    /// one synchronously. Runs until the pool is dropped.
    pub fn spawn_blockhash_refresher(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let pool = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(BLOCKHASH_TTL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let Some(pool) = pool.upgrade() else {
                    log::debug!("RPC pool dropped, stopping blockhash refresher");
                    return;
                };
                if let Err(e) = pool.refresh_blockhash().await {
                    log::debug!("Blockhash refresh failed: {}", e);
                }
            }
        })
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
                "skipPreflight": self.config.skip_preflight,
                "preflightCommitment": "confirmed",
                "maxRetries": 3
            }]
        });

        let response: JsonRpcResponse<String> = self
            .execute_with_deadline(&body, false, Some(HOT_PATH_TIMEOUT))
            .await?;

        let signature_str = response
            .result
            .ok_or_else(|| anyhow!("No result in send transaction response"))?;

        signature_str
            .parse()
            .map_err(|e| anyhow!("Failed to parse signature: {}", e))
    }

    /// Send a transaction and wait for confirmation
    ///
    /// Polls signature status until confirmed or timeout (30 seconds).
    pub async fn send_and_confirm_transaction(
        &self,
        transaction: &Transaction,
    ) -> Result<Signature> {
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

        let response: JsonRpcResponse<AccountResponse> = self
            .execute_with_deadline(&body, true, Some(HOT_PATH_TIMEOUT))
            .await?;

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

        let response: JsonRpcResponse<BalanceResponse> =
            self.execute_with_failover(&body, true).await?;

        response
            .result
            .map(|r| r.value)
            .ok_or_else(|| anyhow!("No result in balance response"))
    }

    /// Get multiple accounts
    pub async fn get_multiple_accounts(
        &self,
        pubkeys: &[Pubkey],
    ) -> Result<Vec<Option<SafeUiAccount>>> {
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

        let response: JsonRpcResponse<MultipleAccountsResponse> = self
            .execute_with_deadline(&body, true, Some(HOT_PATH_TIMEOUT))
            .await?;

        Ok(response.result.map(|r| r.value).unwrap_or_default())
    }

    /// Get program accounts with optional filters.
    ///
    /// Returns the slot the snapshot was taken at alongside the accounts.
    /// Callers need it: feeding these into the cache without a real slot makes
    /// every entry look older than what is already cached, so the whole snapshot
    /// is discarded as stale.
    pub async fn get_program_accounts(
        &self,
        program_id: &Pubkey,
        filters: Option<Vec<serde_json::Value>>,
    ) -> Result<(u64, Vec<(Pubkey, SafeUiAccount)>)> {
        let mut params = json!({
            "encoding": "base64+zstd",
            "commitment": "confirmed",
            "withContext": true
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

        let response: JsonRpcResponse<ProgramAccountsWithContext> =
            self.execute_with_failover(&body, true).await?;

        let Some(result) = response.result else {
            return Ok((0, Vec::new()));
        };

        let slot = result.context.slot;
        let mut accounts = Vec::with_capacity(result.value.len());

        for item in result.value {
            let pubkey: Pubkey = item
                .pubkey
                .parse()
                .map_err(|e| anyhow!("Failed to parse pubkey: {}", e))?;
            accounts.push((pubkey, item.account));
        }

        Ok((slot, accounts))
    }

    /// List the pubkeys of a program's accounts, without their data.
    ///
    /// `dataSlice` with a zero length makes the validator return the account
    /// envelopes and nothing else, so this answers "which accounts exist" for a
    /// fraction of the bytes a full `getProgramAccounts` moves. Reconciliation
    /// runs on a timer and almost always finds nothing missing, so paying for
    /// the account data on every pass would be the bulk of its cost for no
    /// information.
    pub async fn get_program_account_keys(
        &self,
        program_id: &Pubkey,
        filters: Option<Vec<serde_json::Value>>,
    ) -> Result<Vec<Pubkey>> {
        let mut params = json!({
            "encoding": "base64",
            "commitment": "confirmed",
            "dataSlice": { "offset": 0, "length": 0 }
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

        let response: JsonRpcResponse<Vec<ProgramAccountKey>> =
            self.execute_with_failover(&body, true).await?;

        let Some(result) = response.result else {
            return Ok(Vec::new());
        };

        result
            .into_iter()
            .map(|item| {
                item.pubkey
                    .parse()
                    .map_err(|e| anyhow!("Failed to parse pubkey: {}", e))
            })
            .collect()
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

        let response: RpcResponse<SafeSimulationResult> = self
            .execute_with_deadline(&body, true, Some(HOT_PATH_TIMEOUT))
            .await?;

        // Check for simulation error — surface program logs before returning.
        //
        // A trigger-not-ready simulation is a routine consequence of firing on a
        // projected clock that can be marginally ahead of the chain: the caller
        // simply waits and retries. Logging it at WARN, with the full program
        // log, made an expected control-flow path look like a fault.
        if let Some(err) = &response.result.value.err {
            let rendered = format!("{:?}", err);
            let expected = rendered.contains("6004") || rendered.contains("6006");
            if let Some(logs) = &response.result.value.logs {
                for log in logs {
                    if expected {
                        log::debug!("  SIM LOG: {}", log);
                    } else {
                        log::warn!("  SIM LOG: {}", log);
                    }
                }
            }
            return Err(anyhow!("Simulation error: {}", rendered));
        }

        Ok(response.result)
    }

    /// Get statuses for many signatures in one call.
    ///
    /// `getSignatureStatuses` accepts up to 256 signatures, so polling every
    /// in-flight transaction costs one request rather than one per transaction.
    /// Each entry is `None` while unconfirmed, `Some(Ok(()))` once confirmed, or
    /// `Some(Err(raw))` with the error exactly as the RPC reported it.
    pub async fn get_signature_statuses(
        &self,
        signatures: &[Signature],
    ) -> Result<Vec<Option<Result<(), String>>>> {
        if signatures.is_empty() {
            return Ok(Vec::new());
        }

        let strings: Vec<String> = signatures.iter().map(|s| s.to_string()).collect();
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getSignatureStatuses",
            "params": [strings]
        });

        let response: JsonRpcResponse<SignatureStatusResponse> = self
            .execute_with_deadline(&body, true, Some(HOT_PATH_TIMEOUT))
            .await?;

        let statuses = response.result.map(|r| r.value).unwrap_or_default();
        Ok(statuses
            .into_iter()
            .map(|status| {
                let status = status?;
                let confirmed = status
                    .confirmation_status
                    .as_deref()
                    .is_some_and(|s| s == "confirmed" || s == "finalized");
                if !confirmed {
                    return None;
                }
                Some(match status.err {
                    Some(err) => Err(err.to_string()),
                    None => Ok(()),
                })
            })
            .collect())
    }

    /// Get signature status for confirmation checking.
    ///
    /// On failure the raw `err` value is returned verbatim rather than being
    /// coerced into a `TransactionError`. Coercing loses the error kind — every
    /// non-`InstructionError` failure (`BlockhashNotFound`, `AlreadyProcessed`,
    /// `AccountInUse`) collapses to `Custom(0)`, which callers cannot tell apart
    /// from a genuine program error and so treat as permanently fatal.
    pub async fn get_signature_status(
        &self,
        signature: &Signature,
    ) -> Result<Option<Result<(), String>>> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getSignatureStatuses",
            "params": [[signature.to_string()]]
        });

        let response: JsonRpcResponse<SignatureStatusResponse> = self
            .execute_with_deadline(&body, true, Some(HOT_PATH_TIMEOUT))
            .await?;

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

            // Check for error — preserve it exactly as the RPC reported it.
            if let Some(err) = status.err {
                return Ok(Some(Err(err.to_string())));
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
        self.execute_with_deadline(body, read_only, None).await
    }

    /// Execute with an explicit per-call deadline, overriding the pool-wide
    /// HTTP timeout. `None` keeps the pool default.
    async fn execute_with_deadline<T>(
        &self,
        body: &serde_json::Value,
        read_only: bool,
        timeout: Option<Duration>,
    ) -> Result<T>
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

            match self.execute_request(endpoint, body, timeout).await {
                Ok(response) => {
                    endpoint.record_success(start.elapsed());
                    return Ok(response);
                }
                Err(e) => {
                    endpoint.record_failure();
                    log::warn!("RPC request failed for {}: {}", endpoint.url(), e);
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
        timeout: Option<Duration>,
    ) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let mut request = self.http_client.post(endpoint.url()).json(body);
        if let Some(deadline) = timeout {
            request = request.timeout(deadline);
        }
        let response = request.send().await?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "HTTP error: {} - {}",
                response.status(),
                response.text().await.unwrap_or_default()
            ));
        }

        let text = response.text().await?;

        // Try to parse as JSON-RPC error first
        if let Ok(error_response) =
            serde_json::from_str::<JsonRpcResponse<serde_json::Value>>(&text)
        {
            if let Some(error) = error_response.error {
                return Err(anyhow!(RpcError::RpcError(format!(
                    "code {}: {}",
                    error.code, error.message
                ))));
            }
        }

        serde_json::from_str(&text).map_err(|e| {
            anyhow!(
                "JSON parse error: {} - Response: {}",
                e,
                &text[..text.len().min(500)]
            )
        })
    }

    /// Select endpoints for a request based on load balancing strategy
    fn select_endpoints(&self, read_only: bool) -> Vec<Arc<EndpointState>> {
        let role_ok = |e: &Arc<EndpointState>| {
            if read_only {
                e.can_fetch()
            } else {
                e.can_submit()
            }
        };

        // Filter by role and health
        let available: Vec<_> = self
            .endpoints
            .iter()
            .filter(|e| role_ok(e) && e.is_available())
            .cloned()
            .collect();

        if available.is_empty() {
            // Every eligible endpoint is marked unhealthy. Health only improves
            // on a *successful* request, so refusing to send here means no
            // request is ever attempted, nothing can succeed, and the pool stays
            // wedged permanently — fatal for the common single-endpoint
            // deployment, and recoverable only by restarting the process.
            //
            // Attempt them anyway. A failure re-marks them; a success restores
            // health. Unhealthy should mean deprioritised, not unusable when it
            // is all we have.
            let last_resort: Vec<_> = self
                .endpoints
                .iter()
                .filter(|e| role_ok(e))
                .cloned()
                .collect();

            if !last_resort.is_empty() {
                log::debug!(
                    "All {} eligible endpoint(s) marked unhealthy; trying anyway to allow recovery",
                    last_resort.len()
                );
            }
            return last_resort;
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

    #[tokio::test]
    async fn fresh_blockhash_is_served_from_cache() {
        // Endpoint is unroutable, so any network attempt would fail. A cache
        // hit must not touch it.
        let pool = RpcPool::with_url("http://127.0.0.1:1").unwrap();
        let hash = Hash::new_unique();
        pool.prime_blockhash(hash, 4242, Instant::now()).await;

        let (got, height) = pool.get_latest_blockhash().await.unwrap();
        assert_eq!(got, hash);
        assert_eq!(height, 4242);
    }

    #[tokio::test]
    async fn stale_blockhash_is_not_served_from_cache() {
        let pool = RpcPool::with_url("http://127.0.0.1:1").unwrap();
        let stale = Instant::now()
            .checked_sub(BLOCKHASH_TTL * 2)
            .expect("clock far enough from start");
        pool.prime_blockhash(Hash::new_unique(), 1, stale).await;

        // Past the TTL it must go to the network, which here cannot succeed.
        assert!(pool.get_latest_blockhash().await.is_err());
    }

    #[test]
    fn unhealthy_endpoints_are_still_attempted_as_a_last_resort() {
        // Regression: health only improves on a successful request, so
        // returning no endpoints here meant nothing was ever attempted, nothing
        // could succeed, and a single-endpoint pool stayed wedged until the
        // process restarted.
        let pool = RpcPool::with_url("http://127.0.0.1:1").unwrap();

        for e in &pool.endpoints {
            for _ in 0..50 {
                e.record_failure();
            }
            assert!(!e.is_available(), "endpoint should be marked unhealthy");
        }

        assert_eq!(
            pool.select_endpoints(true).len(),
            1,
            "an unhealthy endpoint must still be tried so it can recover"
        );
        assert_eq!(pool.select_endpoints(false).len(), 1);
    }

    #[test]
    fn healthy_endpoints_are_preferred_over_unhealthy_ones() {
        let pool = RpcPool::new(
            vec![
                EndpointConfig::new("http://127.0.0.1:1"),
                EndpointConfig::new("http://127.0.0.1:2"),
            ],
            RpcPoolConfig::default(),
        )
        .unwrap();

        for _ in 0..50 {
            pool.endpoints[0].record_failure();
        }

        // While a healthy endpoint exists, the unhealthy one stays excluded.
        let selected = pool.select_endpoints(true);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].url(), "http://127.0.0.1:2");
    }

    #[test]
    fn skip_preflight_defaults_on() {
        // The client already simulates before signing; preflight is a third,
        // server-side simulation on the critical path.
        assert!(RpcPoolConfig::default().skip_preflight);
    }

    #[test]
    fn hot_path_timeout_is_well_under_the_pool_default() {
        // Otherwise a hung endpoint stalls an execution for the bulk-call
        // timeout before failover even begins.
        assert!(HOT_PATH_TIMEOUT < RpcPoolConfig::default().http.request_timeout);
    }

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
