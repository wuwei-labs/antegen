//! Run command - Start the client in standalone mode

use anyhow::{Context, Result};
use antegen_client::rpc::websocket::WsClient;
use antegen_client::rpc::RpcPool;
use antegen_client::ClientConfig;
use solana_sdk::native_token::LAMPORTS_PER_SOL;
use solana_sdk::signature::{read_keypair_file, Keypair, Signer};
use std::path::{Path, PathBuf};

/// Minimum lamports required to start (100x base fee = 100 transactions)
const MIN_BALANCE_LAMPORTS: u64 = 500_000; // 0.0005 SOL

/// Execute the run command (standalone mode)
pub async fn execute(config_path: PathBuf, log_level: Option<crate::LogLevel>) -> Result<()> {
    // Initialize logging
    let mut builder = env_logger::Builder::new();

    // If --log-level is provided, use it and ignore RUST_LOG
    // Otherwise, read from RUST_LOG with fallback to "info"
    if let Some(level) = log_level {
        builder.filter_level(level.to_level_filter());
    } else {
        builder.parse_env(env_logger::Env::default().default_filter_or("info"));
    }

    // Suppress noisy ractor internal logs (they log at info for every actor lifecycle event)
    builder.filter_module("ractor", log::LevelFilter::Warn);

    // Suppress noisy TPU connection timeout warnings (expected behavior - RPC fallback works)
    builder.filter_module("solana_tpu_client_next::connection_worker", log::LevelFilter::Error);

    // Suppress pws WebSocket logs (auto-reconnect handles disconnects gracefully)
    builder.filter_module("pws", log::LevelFilter::Off);

    builder.format_timestamp_millis().init();

    log::info!("Antegen CLI - Standalone Mode");

    // Auto-generate default config if it doesn't exist
    if !config_path.exists() {
        log::warn!("Config file not found: {}", config_path.display());
        log::info!("Generating default configuration...");

        ClientConfig::default().save(&config_path)?;

        // Get absolute path for logging
        let abs_config_path = config_path.canonicalize()
            .unwrap_or_else(|_| std::env::current_dir()
                .map(|p| p.join(&config_path))
                .unwrap_or(config_path.clone()));

        log::info!("✓ Generated default config at: {}", abs_config_path.display());
        log::warn!("⚠️  IMPORTANT: Review and edit {} before running in production!", abs_config_path.display());
        log::warn!("   - Configure RPC endpoints");
        log::warn!("   - Adjust thread program ID if needed");
        log::info!("");
        log::info!("Starting with default configuration...");
    } else {
        log::info!("Loading configuration from: {}", config_path.display());
    }

    // Load configuration
    let config = ClientConfig::load(&config_path)?;

    // Ensure keypair exists (generate if needed)
    let keypair_path = expand_tilde(&config.executor.keypair_path)?;
    ensure_keypair_exists(&keypair_path)?;

    // Check balance and wait if necessary
    let rpc_endpoint = config
        .rpc
        .endpoints
        .first()
        .ok_or_else(|| anyhow::anyhow!("No RPC endpoints configured"))?;

    check_balance_or_wait(
        &rpc_endpoint.url,
        &rpc_endpoint.get_ws_url(),
        &keypair_path,
    )
    .await?;

    // Run the client
    antegen_client::run_standalone(config).await
}

/// Expand ~ in path to home directory
fn expand_tilde(path: &str) -> Result<PathBuf> {
    if path.starts_with("~/") {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
        Ok(home.join(&path[2..]))
    } else {
        Ok(PathBuf::from(path))
    }
}

/// Ensure keypair exists, generating a new one if needed
fn ensure_keypair_exists(keypair_path: &Path) -> Result<()> {
    if keypair_path.exists() {
        log::info!("Using existing keypair: {}", keypair_path.display());
        return Ok(());
    }

    log::warn!("Keypair not found: {}", keypair_path.display());
    log::info!("Generating new keypair...");

    // Create parent directory if needed
    if let Some(parent) = keypair_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }

    // Generate new keypair
    let keypair = Keypair::new();

    // Write to file (JSON format compatible with solana-keygen)
    let keypair_bytes = keypair.to_bytes();
    let json = serde_json::to_string(&keypair_bytes.to_vec())?;
    std::fs::write(keypair_path, json)
        .with_context(|| format!("Failed to write keypair to: {}", keypair_path.display()))?;

    log::info!("✓ Generated new keypair: {}", keypair_path.display());
    log::info!("  Pubkey: {}", keypair.pubkey());
    log::warn!("⚠️  IMPORTANT: Fund this address with SOL before executing threads!");

    Ok(())
}

/// Check if executor has sufficient balance, wait for funding if not
async fn check_balance_or_wait(rpc_url: &str, ws_url: &str, keypair_path: &Path) -> Result<()> {
    let keypair = read_keypair_file(keypair_path)
        .map_err(|e| anyhow::anyhow!("Failed to read keypair: {}", e))?;
    let pubkey = keypair.pubkey();

    let client = RpcPool::with_url(rpc_url)
        .with_context(|| format!("Failed to create RPC client for {}", rpc_url))?;

    // Check initial balance via RPC
    let balance = client
        .get_balance(&pubkey)
        .await
        .with_context(|| format!("Failed to get balance from {}", rpc_url))?;

    if balance >= MIN_BALANCE_LAMPORTS {
        let sol = balance as f64 / LAMPORTS_PER_SOL as f64;
        log::info!("Executor balance: {:.4} SOL", sol);
        return Ok(());
    }

    // Insufficient balance - wait for funding via WebSocket
    let min_sol = MIN_BALANCE_LAMPORTS as f64 / LAMPORTS_PER_SOL as f64;
    log::warn!("Insufficient balance: {} lamports", balance);
    log::warn!(
        "Minimum required: {:.4} SOL ({} lamports)",
        min_sol,
        MIN_BALANCE_LAMPORTS
    );
    log::info!("Fund address: {}", pubkey);
    log::info!("Waiting for deposit...");

    // Wait until balance meets threshold
    let account = WsClient::wait_until(ws_url, &pubkey, |acc| {
        acc.lamports >= MIN_BALANCE_LAMPORTS
    })
    .await?;

    let sol = account.lamports as f64 / LAMPORTS_PER_SOL as f64;
    log::info!("Funded! Executor balance: {:.4} SOL", sol);

    Ok(())
}
