//! CLI commands

use anyhow::{Context, Result};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{read_keypair_file, Keypair, Signer};
use std::path::{Path, PathBuf};

pub mod client;
pub mod config;
pub mod geyser;
pub mod program;
pub mod run;
pub mod service;
pub mod thread;
pub mod update;

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
