use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DatasourceType {
    Rpc,
    Helius,
    Yellowstone,
}

impl FromStr for DatasourceType {
    type Err = anyhow::Error;
    
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "rpc" => Ok(DatasourceType::Rpc),
            "helius" => Ok(DatasourceType::Helius),
            "yellowstone" => Ok(DatasourceType::Yellowstone),
            _ => Err(anyhow!("Invalid datasource type: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeliusConfig {
    pub ws_url: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YellowstoneConfig {
    pub endpoint: String,
    pub token: String,
}

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
    pub datasource: DatasourceType,
    pub rpc_url: String,
    pub keypair_path: PathBuf,
    pub thread_program_id: Pubkey,
    pub debug: bool,
    pub forgo_commission: bool,
    pub replay: ReplayConfig,
    pub helius: Option<HeliusConfig>,
    pub yellowstone: Option<YellowstoneConfig>,
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
        // Parse datasource type
        let datasource = args
            .datasource
            .ok_or_else(|| anyhow!("Datasource type is required"))?
            .parse::<DatasourceType>()?;
        
        // Parse thread program ID
        let thread_program_id = args
            .thread_program_id
            .ok_or_else(|| anyhow!("Thread program ID is required"))?
            .parse::<Pubkey>()
            .map_err(|e| anyhow!("Invalid thread program ID: {}", e))?;
        
        // Build Helius config if datasource is Helius
        let helius = if matches!(datasource, DatasourceType::Helius) {
            Some(HeliusConfig {
                ws_url: args
                    .helius_ws_url
                    .ok_or_else(|| anyhow!("Helius WebSocket URL is required for Helius datasource"))?,
                api_key: args
                    .helius_api_key
                    .ok_or_else(|| anyhow!("Helius API key is required for Helius datasource"))?,
            })
        } else {
            None
        };
        
        // Build Yellowstone config if datasource is Yellowstone
        let yellowstone = if matches!(datasource, DatasourceType::Yellowstone) {
            Some(YellowstoneConfig {
                endpoint: args
                    .yellowstone_endpoint
                    .ok_or_else(|| anyhow!("Yellowstone endpoint is required for Yellowstone datasource"))?,
                token: args
                    .yellowstone_token
                    .ok_or_else(|| anyhow!("Yellowstone token is required for Yellowstone datasource"))?,
            })
        } else {
            None
        };
        
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
            datasource,
            rpc_url: args
                .rpc_url
                .ok_or_else(|| anyhow!("RPC URL is required"))?,
            keypair_path: args
                .keypair
                .ok_or_else(|| anyhow!("Keypair path is required"))?,
            thread_program_id,
            debug: args.debug,
            forgo_commission: args.forgo_commission,
            replay,
            helius,
            yellowstone,
        })
    }
}