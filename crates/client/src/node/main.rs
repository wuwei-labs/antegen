//! Antegen Node — standalone executor binary
//!
//! This is the executor process that runs Solana threads.
//! Managed by `anm` (Antegen Node Manager) via `anm use/update/install`.

use anyhow::{Context, Result};
use antegen_client::config::{EndpointRole, RpcEndpoint};
use antegen_client::rpc::websocket::WsClient;
use antegen_client::rpc::RpcPool;
use antegen_client::ClientConfig;
use clap::Parser;
use solana_sdk::native_token::LAMPORTS_PER_SOL;
use solana_sdk::signature::{read_keypair_file, Keypair, Signer};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Minimum lamports required for executor operation (0.001 SOL)
const MIN_BALANCE_LAMPORTS: u64 = 1_000_000;

#[derive(Parser)]
#[command(name = "antegen-node")]
#[command(about = "Antegen executor node — runs Solana threads", version)]
struct Cli {
    /// Path to configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// RPC endpoint URL (overrides config file)
    #[arg(long)]
    rpc: Option<String>,

    /// Set the logging level
    #[arg(long, value_name = "LEVEL", value_enum)]
    log_level: Option<LogLevel>,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Off,
}

impl LogLevel {
    fn to_level_filter(&self) -> log::LevelFilter {
        match self {
            LogLevel::Trace => log::LevelFilter::Trace,
            LogLevel::Debug => log::LevelFilter::Debug,
            LogLevel::Info => log::LevelFilter::Info,
            LogLevel::Warn => log::LevelFilter::Warn,
            LogLevel::Error => log::LevelFilter::Error,
            LogLevel::Off => log::LevelFilter::Off,
        }
    }
}

/// Expand ~ in path to home directory
fn expand_tilde(path: &str) -> Result<PathBuf> {
    let expanded = shellexpand::tilde(path);
    Ok(PathBuf::from(expanded.as_ref()))
}

/// Ensure keypair exists at path, generating if needed. Returns the pubkey.
fn ensure_keypair_exists(keypair_path: &Path) -> Result<solana_sdk::pubkey::Pubkey> {
    if keypair_path.exists() {
        let keypair = read_keypair_file(keypair_path)
            .map_err(|e| anyhow::anyhow!("Failed to read keypair: {}", e))?;
        return Ok(keypair.pubkey());
    }

    if let Some(parent) = keypair_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }

    let keypair = Keypair::new();
    let keypair_bytes = keypair.to_bytes();
    let json = serde_json::to_string(&keypair_bytes.to_vec())?;
    std::fs::write(keypair_path, json)
        .with_context(|| format!("Failed to write keypair to: {}", keypair_path.display()))?;

    Ok(keypair.pubkey())
}

/// Resolve the config path: use --config if provided, else default platform path
fn resolve_config_path(config: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = config {
        return Ok(p);
    }
    dirs::config_dir()
        .map(|p| p.join("antegen").join("antegen.toml"))
        .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = resolve_config_path(cli.config)?;

    // Initialize logging
    let mut builder = env_logger::Builder::new();

    if let Some(level) = cli.log_level {
        builder.filter_level(level.to_level_filter());
    } else {
        builder.parse_env(env_logger::Env::default().default_filter_or("info"));
    }

    builder.filter_module("ractor", log::LevelFilter::Warn);
    builder.filter_module("solana_tpu_client_next::connection_worker", log::LevelFilter::Error);
    builder.filter_module("pws", log::LevelFilter::Off);
    builder.format_timestamp_millis().init();

    log::info!("Antegen Node - Standalone Mode");

    // Auto-generate default config if it doesn't exist
    if !config_path.exists() {
        log::warn!("Config file not found: {}", config_path.display());
        log::info!("Generating default configuration...");

        ClientConfig::default().save(&config_path)?;

        let abs_config_path = config_path
            .canonicalize()
            .unwrap_or_else(|_| {
                std::env::current_dir()
                    .map(|p| p.join(&config_path))
                    .unwrap_or(config_path.clone())
            });

        log::info!("Generated default config at: {}", abs_config_path.display());
        log::warn!("IMPORTANT: Review and edit {} before running in production!", abs_config_path.display());
        log::warn!("   - Configure RPC endpoints");
        log::warn!("   - Adjust thread program ID if needed");
        log::info!("");
        log::info!("Starting with default configuration...");
    } else {
        log::info!("Loading configuration from: {}", config_path.display());
    }

    // Load configuration
    let mut config = ClientConfig::load(&config_path)?;

    // Override RPC if provided via CLI
    if let Some(rpc_url) = cli.rpc {
        log::info!("Using RPC override: {}", rpc_url);
        config.rpc.endpoints = vec![RpcEndpoint {
            url: rpc_url,
            ws_url: None,
            role: EndpointRole::Both,
            priority: 1,
        }];
    }

    // Ensure keypair exists (generate if needed)
    let keypair_path = expand_tilde(&config.executor.keypair_path)?;
    let pubkey = ensure_keypair_exists(&keypair_path)?;
    log::info!("Executor pubkey: {}", pubkey);

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

/// Check if executor has sufficient balance, wait for funding if not
async fn check_balance_or_wait(rpc_url: &str, ws_url: &str, keypair_path: &Path) -> Result<()> {
    let keypair = read_keypair_file(keypair_path)
        .map_err(|e| anyhow::anyhow!("Failed to read keypair: {}", e))?;
    let pubkey = keypair.pubkey();

    let client = RpcPool::with_url(rpc_url)
        .with_context(|| format!("Failed to create RPC client for {}", rpc_url))?;

    let balance = client
        .get_balance(&pubkey)
        .await
        .with_context(|| format!("Failed to get balance from {}", rpc_url))?;

    if balance >= MIN_BALANCE_LAMPORTS {
        let sol = balance as f64 / LAMPORTS_PER_SOL as f64;
        log::info!("Executor balance: {:.4} SOL", sol);
        return Ok(());
    }

    let min_sol = MIN_BALANCE_LAMPORTS as f64 / LAMPORTS_PER_SOL as f64;
    log::warn!("Insufficient balance: {} lamports", balance);
    log::warn!(
        "Minimum required: {:.4} SOL ({} lamports)",
        min_sol,
        MIN_BALANCE_LAMPORTS
    );
    log::info!("Fund address: {}", pubkey);
    log::info!("Waiting for deposit...");

    let ws_future =
        WsClient::wait_until(ws_url, &pubkey, |acc| acc.lamports >= MIN_BALANCE_LAMPORTS);

    let poll_future = async {
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            if let Ok(bal) = client.get_balance(&pubkey).await {
                if bal >= MIN_BALANCE_LAMPORTS {
                    return bal;
                }
            }
        }
    };

    tokio::select! {
        ws_result = ws_future => {
            let account = ws_result?;
            let sol = account.lamports as f64 / LAMPORTS_PER_SOL as f64;
            log::info!("Funded! Executor balance: {:.4} SOL", sol);
        }
        balance = poll_future => {
            let sol = balance as f64 / LAMPORTS_PER_SOL as f64;
            log::info!("Funded! Executor balance: {:.4} SOL", sol);
        }
    }

    Ok(())
}
