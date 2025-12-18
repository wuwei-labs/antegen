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
    #[serde(default = "default_commitment")]
    pub commitment: String,
}

impl DatasourceConfig {
    /// Get the thread program ID
    pub fn program_id(&self) -> Pubkey {
        antegen_thread_program::ID
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
    /// Grace period in seconds - used by cache eviction AND load balancer takeover
    /// Threads with time triggers expire from cache after trigger_time + grace_period_secs
    #[serde(default = "default_grace_period_secs")]
    pub grace_period_secs: u64,
}

fn default_grace_period_secs() -> u64 {
    10
}

impl Default for LoadBalancerConfigFile {
    fn default() -> Self {
        Self {
            grace_period_secs: default_grace_period_secs(),
        }
    }
}

/// Load balancer runtime configuration (includes on-chain values)
/// Used internally - not serialized to config file
#[derive(Debug, Clone)]
pub struct LoadBalancerConfig {
    /// Grace period from config file
    pub grace_period_secs: u64,
    /// Capacity threshold (from on-chain ThreadConfig)
    pub capacity_threshold: u32,
    /// Takeover delay for overdue threads (from on-chain ThreadConfig)
    pub takeover_delay_secs: i64,
}

impl Default for LoadBalancerConfig {
    fn default() -> Self {
        Self {
            grace_period_secs: default_grace_period_secs(),
            // Default values, should be overridden by on-chain ThreadConfig
            capacity_threshold: 100,
            takeover_delay_secs: 300,
        }
    }
}

impl From<&LoadBalancerConfigFile> for LoadBalancerConfig {
    fn from(file_config: &LoadBalancerConfigFile) -> Self {
        Self {
            grace_period_secs: file_config.grace_period_secs,
            // On-chain values use defaults, will be updated at runtime
            capacity_threshold: 100,
            takeover_delay_secs: 300,
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
        let content = toml::to_string_pretty(self)
            .context("Failed to serialize config")?;

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
        let has_rpc_datasource = self.rpc.endpoints.iter()
            .any(|e| matches!(e.role, EndpointRole::Datasource | EndpointRole::Both));

        if !has_rpc_datasource {
            anyhow::bail!("At least one RPC datasource endpoint must be configured for standalone mode");
        }

        // Ensure at least one submission endpoint (required for both modes)
        let has_submission_endpoint = self.rpc.endpoints.iter()
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
                anyhow::bail!("RPC endpoint URL must start with http:// or https://: {}", endpoint.url);
            }
        }

        // Validate commitment level
        let valid_commitments = ["processed", "confirmed", "finalized"];
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
                endpoints: vec![
                    RpcEndpoint {
                        url: "http://localhost:8899".to_string(),
                        ws_url: None,
                        role: EndpointRole::Both,
                        priority: 1,
                    },
                ],
            },
            datasources: DatasourceConfig {
                commitment: "confirmed".to_string(),
            },
            processor: ProcessorConfig {
                max_concurrent_threads: 10,
            },
            cache: CacheConfig::default(),
            load_balancer: LoadBalancerConfigFile::default(),
            observability: ObservabilityConfig::default(),
            tpu: TpuConfig::default(),
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
