use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayConfig {
    pub enabled: bool,
    pub nats_url: String,
    pub delay_ms: u64,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            nats_url: "nats://localhost:4222".to_string(),
            delay_ms: 30000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub rpc_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ws_url: Option<String>,
    pub keypair_path: PathBuf,
    pub thread_program_id: Pubkey,
    pub debug: bool,
    pub forgo_commission: bool,
    pub replay: ReplayConfig,
}

impl Config {
    /// Load config from file
    pub fn from_file(path: PathBuf) -> Result<Self> {
        let content = std::fs::read_to_string(&path)
            .map_err(|e| anyhow!("Failed to read config file: {}", e))?;
        
        let config: Config = toml::from_str(&content)
            .map_err(|e| anyhow!("Failed to parse config file: {}", e))?;
        
        Ok(config)
    }
    
    /// Build config from CLI arguments
    pub fn from_args(args: crate::Args) -> Result<Self> {
        // Parse thread program ID
        let thread_program_id = args
            .thread_program_id
            .ok_or_else(|| anyhow!("Thread program ID is required"))?
            .parse::<Pubkey>()
            .map_err(|e| anyhow!("Invalid thread program ID: {}", e))?;
        
        // Build replay config
        let replay = if args.enable_replay {
            ReplayConfig {
                enabled: true,
                nats_url: args
                    .nats_url
                    .unwrap_or_else(|| "nats://localhost:4222".to_string()),
                delay_ms: args.replay_delay_ms,
            }
        } else {
            ReplayConfig::default()
        };
        
        Ok(Config {
            rpc_url: args
                .rpc_url
                .ok_or_else(|| anyhow!("RPC URL is required"))?,
            ws_url: None, // Will be derived from rpc_url if needed
            keypair_path: args
                .keypair
                .ok_or_else(|| anyhow!("Keypair path is required"))?,
            thread_program_id,
            debug: args.debug,
            forgo_commission: args.forgo_commission,
            replay,
        })
    }
}