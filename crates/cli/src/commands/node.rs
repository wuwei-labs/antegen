//! Executor daemon — `antegen node run`
//!
//! This is the process the service supervises. It was previously a separate
//! `antegen-node` binary in `antegen-client`; the runtime it starts has always
//! lived in the library, so the binary only ever duplicated startup wiring.

use crate::commands::{ensure_keypair_exists, expand_tilde, MIN_BALANCE_LAMPORTS};
use crate::config_cmd::LogLevel;
use antegen_client::config::{EndpointRole, RpcEndpoint};
use antegen_client::rpc::websocket::WsClient;
use antegen_client::rpc::RpcPool;
use antegen_client::ClientConfig;
use anyhow::{Context, Result};
use solana_sdk::native_token::LAMPORTS_PER_SOL;
use solana_sdk::signature::{read_keypair_file, Signer};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Run the executor in the foreground, blocking until it exits.
pub async fn run(
    config_path: PathBuf,
    rpc_override: Option<String>,
    log_level: Option<LogLevel>,
) -> Result<()> {
    init_logging(log_level);

    log::info!("Antegen Node - Standalone Mode");

    // Auto-generate default config if it doesn't exist
    if !config_path.exists() {
        log::warn!("Config file not found: {}", config_path.display());
        log::info!("Generating default configuration...");

        ClientConfig::default().save(&config_path)?;

        let abs_config_path = config_path.canonicalize().unwrap_or_else(|_| {
            std::env::current_dir()
                .map(|p| p.join(&config_path))
                .unwrap_or_else(|_| config_path.clone())
        });

        log::info!("Generated default config at: {}", abs_config_path.display());
        log::warn!(
            "IMPORTANT: Review and edit {} before running in production!",
            abs_config_path.display()
        );
        log::warn!("   - Configure RPC endpoints");
        log::warn!("   - Adjust thread program ID if needed");
        log::info!("");
        log::info!("Starting with default configuration...");
    } else {
        log::info!("Loading configuration from: {}", config_path.display());
    }

    let mut config = ClientConfig::load(&config_path)?;

    if let Some(rpc_url) = rpc_override {
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

    let rpc_endpoint = config
        .rpc
        .endpoints
        .first()
        .ok_or_else(|| anyhow::anyhow!("No RPC endpoints configured"))?;

    check_balance_or_wait(&rpc_endpoint.url, &rpc_endpoint.get_ws_url(), &keypair_path).await?;

    antegen_client::run_standalone(config).await
}

/// Configure the executor's log filters.
///
/// The per-execution latency line is emitted at DEBUG so it can be filtered
/// independently, but it is the primary signal for diagnosing trigger delay —
/// enable it even when the rest of the client is at INFO.
fn init_logging(log_level: Option<LogLevel>) {
    let mut builder = env_logger::Builder::new();

    if let Some(level) = log_level {
        builder.filter_level(level.to_level_filter());
    } else {
        builder.parse_env(env_logger::Env::default().default_filter_or("info"));
    }

    builder.filter_module("ractor", log::LevelFilter::Warn);
    builder.filter_module(
        "solana_tpu_client_next::connection_worker",
        log::LevelFilter::Error,
    );
    builder.filter_module("antegen_ws", log::LevelFilter::Off);
    builder.filter_module(
        antegen_client::actors::processor::LATENCY_TARGET,
        log::LevelFilter::Debug,
    );
    builder.format_timestamp_millis().init();
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
