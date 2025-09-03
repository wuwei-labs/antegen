use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Default localnet configuration embedded in the binary
pub const DEFAULT_CONFIG: &str = r#"
# Antegen Localnet Configuration
# This is the default configuration used when no custom config is provided

[validator]
type = "solana"
rpc_url = "http://localhost:8899"
ws_url = "ws://localhost:8900"
ledger_dir = "test-ledger"
reset = true

# No clients by default - start with just validator
# Add clients with --client flag or configure below

# Example: Geyser plugin (efficient streaming)
# [[clients]]
# type = "geyser"
# name = "geyser-default"

# Example: Carbon RPC client (polling-based)
# [[clients]]
# type = "carbon"
# name = "carbon-rpc"
"#;

/// Main localnet configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalnetConfig {
    pub validator: ValidatorConfig,
    #[serde(default)]
    pub clients: Vec<ClientConfig>,
}

/// Validator configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorConfig {
    /// Type of validator: "solana", "firedancer", etc.
    #[serde(rename = "type")]
    pub validator_type: String,
    
    /// RPC URL for the validator
    pub rpc_url: String,
    
    /// WebSocket URL for the validator
    pub ws_url: String,
    
    /// Ledger directory
    #[serde(default = "default_ledger_dir")]
    pub ledger_dir: String,
    
    /// Reset ledger on start
    #[serde(default = "default_true")]
    pub reset: bool,
    
    /// Additional validator arguments
    #[serde(default)]
    pub extra_args: Vec<String>,
}

/// Client configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    /// Type of client: "geyser", "carbon"
    #[serde(rename = "type")]
    pub client_type: String,
    
    /// Name for this client instance (for logging)
    pub name: String,
    
    /// Client-specific configuration
    pub config: toml::Value,
}

/// Geyser client configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeyserClientConfig {
    pub keypath: String,
    pub forgo_commission: bool,
    pub enable_replay: bool,
    pub thread_count: usize,
    #[serde(default = "default_transaction_timeout")]
    pub transaction_timeout_threshold: u64,
    pub nats_url: Option<String>,
    pub replay_delay_ms: Option<u64>,
    // These will be populated from validator config if not specified
    pub rpc_url: Option<String>,
    pub ws_url: Option<String>,
}

/// Carbon client configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CarbonClientConfig {
    pub datasource: String,  // "rpc", "helius", "yellowstone"
    pub rpc_url: Option<String>,
    pub endpoint: Option<String>,
    pub token: Option<String>,
    pub thread_program_id: String,
    pub keypair_path: String,
    pub forgo_commission: Option<bool>,
}

impl LocalnetConfig {
    /// Load default configuration
    pub fn default() -> Result<Self, toml::de::Error> {
        toml::from_str(DEFAULT_CONFIG)
    }
    
    /// Load from file
    pub fn from_file(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&content)?)
    }
    
    /// Merge with another config (other takes precedence)
    #[allow(dead_code)]
    pub fn merge(mut self, other: LocalnetConfig) -> Self {
        // Override validator if provided
        self.validator = other.validator;
        
        // Replace clients if provided, otherwise keep defaults
        if !other.clients.is_empty() {
            self.clients = other.clients;
        }
        
        self
    }
}

fn default_ledger_dir() -> String {
    "test-ledger".to_string()
}

fn default_transaction_timeout() -> u64 {
    150
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_default_config_parses() {
        let config = LocalnetConfig::default().expect("Default config should parse");
        assert_eq!(config.validator.validator_type, "solana");
        assert!(!config.clients.is_empty());
    }
}