//! CLI commands

use anyhow::{anyhow, Context, Result};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{read_keypair_file, Keypair, Signer};
use std::path::{Path, PathBuf};

/// Minimum lamports required for executor operation (0.001 SOL)
/// Must exceed rent-exempt minimum for a system account (~890,880 lamports)
pub(crate) const MIN_BALANCE_LAMPORTS: u64 = 1_000_000;

pub mod client;
pub mod config;
pub mod geyser;
pub mod info;
pub mod program;
pub mod run;
pub mod service;
pub mod thread;
pub mod update;

/// Get RPC URL from arg or Solana CLI config
pub(crate) fn get_rpc_url(rpc: Option<String>) -> Result<String> {
    if let Some(url) = rpc {
        return Ok(url);
    }
    let config_file = solana_cli_config::CONFIG_FILE
        .as_ref()
        .ok_or_else(|| anyhow!("Unable to find Solana CLI config file"))?;
    let config = solana_cli_config::Config::load(config_file)
        .map_err(|e| anyhow!("Failed to load Solana CLI config: {}", e))?;
    Ok(config.json_rpc_url)
}

/// Get keypair from arg or Solana CLI config
pub(crate) fn get_keypair(keypair_path: Option<PathBuf>) -> Result<Keypair> {
    let path = if let Some(p) = keypair_path {
        p
    } else {
        let config_file = solana_cli_config::CONFIG_FILE
            .as_ref()
            .ok_or_else(|| anyhow!("Unable to find Solana CLI config file"))?;
        let config = solana_cli_config::Config::load(config_file)
            .map_err(|e| anyhow!("Failed to load Solana CLI config: {}", e))?;
        PathBuf::from(config.keypair_path)
    };
    read_keypair_file(&path).map_err(|e| anyhow!("Failed to read keypair from {:?}: {}", path, e))
}

/// Default config file path: `<config_dir>/antegen/antegen.toml`
///
/// On macOS this is `~/Library/Application Support/antegen/antegen.toml`.
/// On Linux this is `~/.config/antegen/antegen.toml`.
pub fn default_config_path() -> Result<PathBuf> {
    dirs::config_dir()
        .map(|p| p.join("antegen").join("antegen.toml"))
        .ok_or_else(|| anyhow!("Could not determine config directory"))
}

/// Expand ~ in path to home directory
pub fn expand_tilde(path: &str) -> Result<PathBuf> {
    if path.starts_with("~/") {
        let home =
            dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
        Ok(home.join(&path[2..]))
    } else {
        Ok(PathBuf::from(path))
    }
}

/// Ensure keypair exists at path, generating if needed. Returns the pubkey.
pub fn ensure_keypair_exists(keypair_path: &Path) -> Result<Pubkey> {
    if keypair_path.exists() {
        let keypair = read_keypair_file(keypair_path)
            .map_err(|e| anyhow::anyhow!("Failed to read keypair: {}", e))?;
        return Ok(keypair.pubkey());
    }

    // Create parent directory if needed
    if let Some(parent) = keypair_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }

    // Generate new keypair
    let keypair = Keypair::new();
    let keypair_bytes = keypair.to_bytes();
    let json = serde_json::to_string(&keypair_bytes.to_vec())?;
    std::fs::write(keypair_path, json)
        .with_context(|| format!("Failed to write keypair to: {}", keypair_path.display()))?;

    Ok(keypair.pubkey())
}
