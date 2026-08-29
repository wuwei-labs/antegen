//! Configuration types and loading
//!
//! This module contains the unified ClientConfig used by both
//! standalone and plugin deployment modes.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use std::fs;
use std::path::Path;

/// Main configuration for the Antegen client
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClientConfig {
    #[serde(default)]
    pub executor: ExecutorConfig,
    pub rpc: RpcConfig,
    pub datasources: DatasourceConfig,
    pub processor: ProcessorConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub load_balancer: LoadBalancerConfigFile,
    #[serde(default)]
    pub observability: ObservabilityConfig,
    #[serde(default)]
    pub tpu: TpuConfig,
    #[serde(default)]
    pub transaction: TransactionConfig,
    #[serde(default)]
    pub compute: ComputeConfig,
}

/// Compute-budget configuration.
///
/// Every value here is a lever on what the node *requests*, which under
/// SIMD-0553 is what it pays for. Exposed rather than hardcoded because the
/// right margin depends on the fibers a node actually serves, and the cost of
/// getting it wrong moves in opposite directions on either side.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ComputeConfig {
    /// Margin a thread's compute request starts at, in basis points over the
    /// simulated estimate. The historical unconditional pad was 2,500.
    #[serde(default = "default_initial_margin_bps")]
    pub initial_margin_bps: u32,
    /// Floor the margin decays toward as executions land.
    #[serde(default = "default_min_margin_bps")]
    pub min_margin_bps: u32,
    /// Ceiling the margin climbs to after repeated overruns.
    #[serde(default = "default_max_margin_bps")]
    pub max_margin_bps: u32,
    /// Absolute compute units added on top of the proportional margin.
    #[serde(default = "default_floor_units")]
    pub floor_units: u32,
}

fn default_initial_margin_bps() -> u32 {
    2_500
}

fn default_min_margin_bps() -> u32 {
    300
}

fn default_max_margin_bps() -> u32 {
    10_000
}

fn default_floor_units() -> u32 {
    3_000
}

impl Default for ComputeConfig {
    fn default() -> Self {
        Self {
            initial_margin_bps: default_initial_margin_bps(),
            min_margin_bps: default_min_margin_bps(),
            max_margin_bps: default_max_margin_bps(),
            floor_units: default_floor_units(),
        }
    }
}

impl ComputeConfig {
    pub fn oracle(&self) -> crate::resources::CuOracleConfig {
        crate::resources::CuOracleConfig {
            initial_margin_bps: self.initial_margin_bps,
            min_margin_bps: self.min_margin_bps,
            max_margin_bps: self.max_margin_bps,
            floor_units: self.floor_units,
            ..crate::resources::CuOracleConfig::default()
        }
    }
}

/// Transaction encoding configuration.
///
/// Separate from the executor so the format can be rolled forward — or rolled
/// back — on a node without touching anything else about how it executes.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TransactionConfig {
    /// Message format to emit. `legacy` today; `v0` unlocks address lookup
    /// tables, `v1` (SIMD-0385) moves resource limits into the header and
    /// raises the size ceiling to 4096 bytes.
    #[serde(default)]
    pub version: crate::tx::TxVersion,
}

/// Executor configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExecutorConfig {
    #[serde(default = "default_keypair_path")]
    pub keypair_path: String,
    #[serde(default)]
    pub forgo_commission: bool,
}

fn default_keypair_path() -> String {
    "~/.antegen/executor-keypair.json".to_string()
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            keypair_path: default_keypair_path(),
            forgo_commission: false,
        }
    }
}

/// RPC endpoint configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RpcConfig {
    pub endpoints: Vec<RpcEndpoint>,
    /// Skip the RPC node's preflight simulation when submitting.
    ///
    /// The client already simulates before signing, so preflight is a third,
    /// server-side simulation on the critical path. Skipping it means paying
    /// the fee for a transaction that would have been rejected at preflight —
    /// set to `false` if that shows up as meaningful fee waste. Note that TPU
    /// submission never preflights, so this only affects the RPC fallback.
    #[serde(default = "default_skip_preflight")]
    pub skip_preflight: bool,
}

fn default_skip_preflight() -> bool {
    true
}

/// Individual RPC endpoint
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RpcEndpoint {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ws_url: Option<String>,
    pub role: EndpointRole,
    /// Priority for submission (lower = higher priority, 1 = highest)
    /// Only used for Submission and Both roles
    /// Ignored for Datasource-only endpoints (all datasources listen concurrently)
    #[serde(default = "default_priority")]
    pub priority: u8,
}

/// An endpoint URL with its credentials stripped, for logging.
///
/// Providers put API keys in the query string — `?api-key=…` for Helius,
/// `/v2/<key>` style paths elsewhere — and some accept them as userinfo. Logging
/// the URL verbatim writes the key to the journal at INFO on a healthy node, and
/// to anywhere those logs are shipped. Endpoints appear in dozens of log lines,
/// so the redaction belongs at the point a label is made rather than at each
/// call site.
///
/// Keeps scheme, host and port, which is all a reader needs to tell endpoints
/// apart. Anything that cannot be parsed is reduced to its host-looking prefix
/// rather than passed through, so a malformed URL cannot leak by falling back.
pub fn redact_endpoint(url: &str) -> String {
    let (scheme, rest) = match url.split_once("://") {
        Some((s, r)) => (s, r),
        // No scheme to anchor on: keep nothing but the first path-free segment.
        None => return url.split(['/', '?', '#']).next().unwrap_or("").to_string(),
    };

    // Drop userinfo (`user:pass@host`), then everything from the first path,
    // query or fragment separator — which is where every key we have seen lives.
    let rest = rest.rsplit_once('@').map(|(_, h)| h).unwrap_or(rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");

    format!("{}://{}", scheme, host)
}

impl RpcEndpoint {
    /// Get the WebSocket URL, deriving from HTTP URL if not explicitly provided
    pub fn get_ws_url(&self) -> String {
        self.ws_url.clone().unwrap_or_else(|| {
            // Auto-derive: http://... -> ws://..., https://... -> wss://...
            if self.url.starts_with("https://") {
                self.url.replace("https://", "wss://")
            } else if self.url.starts_with("http://") {
                self.url.replace("http://", "ws://")
            } else {
                // Fallback: assume https if no protocol
                format!("wss://{}", self.url.trim_start_matches("//"))
            }
        })
    }
}

fn default_priority() -> u8 {
    1
}

/// Role of an RPC endpoint
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EndpointRole {
    /// Only used for account subscriptions
    Datasource,
    /// Only used for transaction submission
    Submission,
    /// Used for both datasources and submission
    Both,
}

/// Datasource configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DatasourceConfig {
    /// Commitment for the thread program subscription.
    ///
    /// Kept at `confirmed`: thread state drives execution dedup, load-balancer
    /// ownership and `exec_count` checks, and acting on a `processed` update
    /// that is later rolled back would mean executing against state that never
    /// existed.
    #[serde(default = "default_commitment")]
    pub commitment: String,
    /// Commitment for the clock sysvar subscription.
    ///
    /// Defaults to `processed`, which is one to two slots ahead of `confirmed`.
    /// The clock is only a scheduling hint — the on-chain `require!` in
    /// `thread_exec` is the real gate — so firing a slot early costs at worst a
    /// short wait, while firing a slot late costs ~400ms on every execution.
    #[serde(default = "default_clock_commitment")]
    pub clock_commitment: String,
    #[serde(default = "default_program_id", with = "pubkey_string")]
    pub program_id: Pubkey,
}

fn default_clock_commitment() -> String {
    "processed".to_string()
}

fn default_program_id() -> Pubkey {
    antegen_thread_program::ID
}

mod pubkey_string {
    use serde::{self, Deserialize, Deserializer, Serializer};
    use solana_sdk::pubkey::Pubkey;
    use std::str::FromStr;

    pub fn serialize<S>(pubkey: &Pubkey, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&pubkey.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Pubkey, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Pubkey::from_str(&s).map_err(serde::de::Error::custom)
    }
}

fn default_commitment() -> String {
    "confirmed".to_string()
}

/// Processor configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProcessorConfig {
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_threads: usize,
}

fn default_max_concurrent() -> usize {
    10
}

/// Cache configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CacheConfig {
    /// Maximum number of accounts to cache
    #[serde(default = "default_cache_max_capacity")]
    pub max_capacity: u64,
}

fn default_cache_max_capacity() -> u64 {
    10_000
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_capacity: default_cache_max_capacity(),
        }
    }
}

/// Load balancer configuration (file-based portion)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoadBalancerConfigFile {
    /// Grace period in seconds - for fee decay calculations
    #[serde(default = "default_grace_period")]
    pub grace_period: u64,

    /// Extra time in seconds to keep threads in cache after grace period
    /// Allows takeover attempts before cache eviction
    /// Cache TTL = trigger_time + grace_period + eviction_buffer
    #[serde(default = "default_eviction_buffer")]
    pub eviction_buffer: u64,

    /// Delay in seconds before claiming new threads (default: 0)
    /// Slower clients can set higher values to avoid wasting fees on races
    #[serde(default)]
    pub thread_process_delay: u64,
}

fn default_grace_period() -> u64 {
    10
}

fn default_eviction_buffer() -> u64 {
    20
}

impl Default for LoadBalancerConfigFile {
    fn default() -> Self {
        Self {
            grace_period: default_grace_period(),
            eviction_buffer: default_eviction_buffer(),
            thread_process_delay: 0,
        }
    }
}

/// Load balancer runtime configuration (includes on-chain values)
/// Used internally - not serialized to config file
#[derive(Debug, Clone)]
pub struct LoadBalancerConfig {
    /// Grace period from config file (seconds)
    pub grace_period: u64,
    /// Eviction buffer from config file (seconds)
    pub eviction_buffer: u64,
    /// Capacity threshold (from on-chain ThreadConfig)
    pub capacity_threshold: u32,
    /// Takeover delay for overdue threads (from on-chain ThreadConfig, seconds)
    pub thread_takeover_delay: i64,
    /// Delay before claiming new threads (seconds)
    pub thread_process_delay: u64,
}

impl Default for LoadBalancerConfig {
    fn default() -> Self {
        Self {
            grace_period: default_grace_period(),
            eviction_buffer: default_eviction_buffer(),
            // Default values, should be overridden by on-chain ThreadConfig
            capacity_threshold: 100,
            thread_takeover_delay: 300,
            thread_process_delay: 0,
        }
    }
}

impl From<&LoadBalancerConfigFile> for LoadBalancerConfig {
    fn from(file_config: &LoadBalancerConfigFile) -> Self {
        Self {
            grace_period: file_config.grace_period,
            eviction_buffer: file_config.eviction_buffer,
            // On-chain values use defaults, will be updated at runtime
            capacity_threshold: 100,
            thread_takeover_delay: 300,
            thread_process_delay: file_config.thread_process_delay,
        }
    }
}

/// Observability configuration (loa-core agent)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ObservabilityConfig {
    /// Enable loa-core observability agent
    #[serde(default = "default_observability_enabled")]
    pub enabled: bool,
    /// Storage path for loa-core data (metrics, identity)
    #[serde(default = "default_observability_storage_path")]
    pub storage_path: String,
}

/// TPU client configuration for direct validator transaction submission
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TpuConfig {
    /// Enable TPU client for transaction submission (TPU first, RPC fallback)
    #[serde(default = "default_tpu_enabled")]
    pub enabled: bool,
    /// Number of QUIC connections per leader
    #[serde(default = "default_tpu_num_connections")]
    pub num_connections: usize,
    /// Number of leaders to fan out transactions to
    #[serde(default = "default_tpu_leaders_fanout")]
    pub leaders_fanout: usize,
    /// Channel buffer size for transaction batches
    #[serde(default = "default_tpu_worker_channel_size")]
    pub worker_channel_size: usize,
}

fn default_tpu_enabled() -> bool {
    true
}

fn default_tpu_num_connections() -> usize {
    4
}

fn default_tpu_leaders_fanout() -> usize {
    4
}

fn default_tpu_worker_channel_size() -> usize {
    256
}

impl Default for TpuConfig {
    fn default() -> Self {
        Self {
            enabled: default_tpu_enabled(),
            num_connections: default_tpu_num_connections(),
            leaders_fanout: default_tpu_leaders_fanout(),
            worker_channel_size: default_tpu_worker_channel_size(),
        }
    }
}

fn default_observability_enabled() -> bool {
    true
}

fn default_observability_storage_path() -> String {
    "~/.antegen/observability".to_string()
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            enabled: default_observability_enabled(),
            storage_path: default_observability_storage_path(),
        }
    }
}

impl ClientConfig {
    /// Load configuration from a TOML file
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        let config: ClientConfig = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

        config.validate()?;
        Ok(config)
    }

    /// Save configuration to a TOML file
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;

        fs::write(path.as_ref(), content)
            .with_context(|| format!("Failed to write config file: {}", path.as_ref().display()))?;

        Ok(())
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        // Validate keypair path
        if self.executor.keypair_path.is_empty() {
            anyhow::bail!("Executor keypair_path cannot be empty");
        }

        // Validate RPC endpoints
        if self.rpc.endpoints.is_empty() {
            anyhow::bail!("At least one RPC endpoint must be configured");
        }

        // Ensure at least one RPC datasource endpoint for standalone mode
        // (Plugin mode will use Geyser instead, but config should be valid for standalone)
        let has_rpc_datasource = self
            .rpc
            .endpoints
            .iter()
            .any(|e| matches!(e.role, EndpointRole::Datasource | EndpointRole::Both));

        if !has_rpc_datasource {
            anyhow::bail!(
                "At least one RPC datasource endpoint must be configured for standalone mode"
            );
        }

        // Ensure at least one submission endpoint (required for both modes)
        let has_submission_endpoint = self
            .rpc
            .endpoints
            .iter()
            .any(|e| matches!(e.role, EndpointRole::Submission | EndpointRole::Both));

        if !has_submission_endpoint {
            anyhow::bail!("At least one submission endpoint must be configured");
        }

        // Validate endpoint URLs
        for endpoint in &self.rpc.endpoints {
            if endpoint.url.is_empty() {
                anyhow::bail!("RPC endpoint URL cannot be empty");
            }

            // Basic URL validation
            if !endpoint.url.starts_with("http://") && !endpoint.url.starts_with("https://") {
                anyhow::bail!(
                    "RPC endpoint URL must start with http:// or https://: {}",
                    endpoint.url
                );
            }
        }

        // Validate commitment level
        let valid_commitments = ["processed", "confirmed", "finalized"];
        if !valid_commitments.contains(&self.datasources.clock_commitment.as_str()) {
            return Err(anyhow::anyhow!(
                "Invalid clock commitment level: {}. Must be one of: {}",
                self.datasources.clock_commitment,
                valid_commitments.join(", ")
            ));
        }
        if !valid_commitments.contains(&self.datasources.commitment.as_str()) {
            anyhow::bail!(
                "Invalid commitment level: {}. Must be one of: {}",
                self.datasources.commitment,
                valid_commitments.join(", ")
            );
        }

        // Validate processor config
        if self.processor.max_concurrent_threads == 0 {
            anyhow::bail!("max_concurrent_threads must be greater than 0");
        }

        // A margin band that cannot contain its own starting point would have
        // the oracle clamp on its first adjustment, silently ignoring the
        // configured start.
        let compute = &self.compute;
        if compute.min_margin_bps > compute.max_margin_bps {
            anyhow::bail!(
                "compute.min_margin_bps ({}) must not exceed compute.max_margin_bps ({})",
                compute.min_margin_bps,
                compute.max_margin_bps
            );
        }
        if compute.initial_margin_bps < compute.min_margin_bps
            || compute.initial_margin_bps > compute.max_margin_bps
        {
            anyhow::bail!(
                "compute.initial_margin_bps ({}) must fall within [{}, {}]",
                compute.initial_margin_bps,
                compute.min_margin_bps,
                compute.max_margin_bps
            );
        }

        // Refuse an unimplemented transaction format at startup. Left to the
        // execution path it would surface as every thread failing to build,
        // with nothing in the error naming the config line that caused it.
        if !self.transaction.version.is_implemented() {
            anyhow::bail!(
                "transaction version '{}' is not implemented yet; use 'legacy'",
                self.transaction.version
            );
        }

        Ok(())
    }
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            executor: ExecutorConfig {
                keypair_path: "~/.antegen/executor-keypair.json".to_string(),
                forgo_commission: false,
            },
            rpc: RpcConfig {
                endpoints: vec![RpcEndpoint {
                    url: "http://localhost:8899".to_string(),
                    ws_url: None,
                    role: EndpointRole::Both,
                    priority: 1,
                }],
                skip_preflight: default_skip_preflight(),
            },
            datasources: DatasourceConfig {
                commitment: default_commitment(),
                clock_commitment: default_clock_commitment(),
                program_id: default_program_id(),
            },
            processor: ProcessorConfig {
                max_concurrent_threads: 10,
            },
            cache: CacheConfig::default(),
            load_balancer: LoadBalancerConfigFile::default(),
            observability: ObservabilityConfig::default(),
            tpu: TpuConfig::default(),
            transaction: TransactionConfig::default(),
            compute: ComputeConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_default_config_is_valid() {
        let config = ClientConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_load_and_save() {
        let config = ClientConfig::default();

        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_path_buf();

        // Save
        config.save(&path).unwrap();

        // Load
        let loaded = ClientConfig::load(&path).unwrap();

        // Verify
        assert_eq!(loaded.executor.keypair_path, config.executor.keypair_path);
        assert_eq!(loaded.rpc.endpoints.len(), config.rpc.endpoints.len());
    }

    #[test]
    fn test_validation_requires_endpoints() {
        let mut config = ClientConfig::default();
        config.rpc.endpoints.clear();

        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validation_requires_datasource() {
        let mut config = ClientConfig::default();
        config.rpc.endpoints[0].role = EndpointRole::Submission;

        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validation_requires_submission_endpoint() {
        let mut config = ClientConfig::default();
        config.rpc.endpoints[0].role = EndpointRole::Datasource;

        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_commitment() {
        let mut config = ClientConfig::default();
        config.datasources.commitment = "invalid".to_string();

        assert!(config.validate().is_err());
    }

    #[test]
    fn test_ws_url_auto_derivation() {
        // HTTP to WS
        let endpoint = RpcEndpoint {
            url: "http://localhost:8899".to_string(),
            ws_url: None,
            role: EndpointRole::Both,
            priority: 1,
        };
        assert_eq!(endpoint.get_ws_url(), "ws://localhost:8899");

        // HTTPS to WSS
        let endpoint = RpcEndpoint {
            url: "https://api.mainnet-beta.solana.com".to_string(),
            ws_url: None,
            role: EndpointRole::Both,
            priority: 1,
        };
        assert_eq!(endpoint.get_ws_url(), "wss://api.mainnet-beta.solana.com");

        // Explicit ws_url takes precedence
        let endpoint = RpcEndpoint {
            url: "https://api.mainnet-beta.solana.com".to_string(),
            ws_url: Some("wss://custom-ws-url.com".to_string()),
            role: EndpointRole::Both,
            priority: 1,
        };
        assert_eq!(endpoint.get_ws_url(), "wss://custom-ws-url.com");
    }
}

#[cfg(test)]
mod redaction_tests {
    use super::redact_endpoint;

    /// The line that prompted this: a Helius key logged at WARN on a mainnet
    /// node, and so present in the journal and anywhere those logs are shipped.
    #[test]
    fn strips_an_api_key_from_the_query_string() {
        assert_eq!(
            redact_endpoint("wss://mainnet.helius-rpc.com/?api-key=cf11925f-9ff4-dead-beef"),
            "wss://mainnet.helius-rpc.com"
        );
    }

    #[test]
    fn strips_keys_carried_in_the_path() {
        assert_eq!(
            redact_endpoint("https://rpc.example.com/v2/secret-key-here"),
            "https://rpc.example.com"
        );
    }

    #[test]
    fn strips_userinfo() {
        assert_eq!(
            redact_endpoint("wss://user:password@rpc.example.com/ws"),
            "wss://rpc.example.com"
        );
    }

    #[test]
    fn keeps_what_distinguishes_endpoints() {
        assert_eq!(
            redact_endpoint("http://localhost:8899"),
            "http://localhost:8899"
        );
        assert_ne!(
            redact_endpoint("wss://a.example.com/?api-key=x"),
            redact_endpoint("wss://b.example.com/?api-key=x")
        );
    }

    /// A URL we cannot parse must not fall through verbatim — that is exactly
    /// how a key would escape.
    #[test]
    fn malformed_input_does_not_leak() {
        assert_eq!(
            redact_endpoint("rpc.example.com/?api-key=secret"),
            "rpc.example.com"
        );
        assert_eq!(redact_endpoint(""), "");
        assert!(!redact_endpoint("not a url ?api-key=secret").contains("secret"));
    }
}
