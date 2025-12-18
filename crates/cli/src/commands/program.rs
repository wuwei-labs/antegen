//! Program configuration commands
//!
//! Commands for managing the thread program's global configuration.

use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use antegen_client::rpc::RpcPool;
use antegen_thread_program::state::ThreadConfig;
use anyhow::{anyhow, Result};
use solana_sdk::{
    instruction::Instruction, message::Message, signature::read_keypair_file, signer::Signer,
    transaction::Transaction,
};
use std::path::PathBuf;

/// Get RPC URL from arg or Solana CLI config
fn get_rpc_url(rpc: Option<String>) -> Result<String> {
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
fn get_keypair(keypair_path: Option<PathBuf>) -> Result<solana_sdk::signature::Keypair> {
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

/// Initialize the ThreadConfig account
pub async fn config_init(rpc: Option<String>, keypair_path: Option<PathBuf>) -> Result<()> {
    let rpc_url = get_rpc_url(rpc)?;
    let admin = get_keypair(keypair_path)?;

    println!("RPC: {}", rpc_url);
    println!("Admin: {}", admin.pubkey());

    let client = RpcPool::with_url(&rpc_url)
        .map_err(|e| anyhow!("Failed to create RPC client: {}", e))?;

    // Check if config already exists
    let config_pubkey = ThreadConfig::pubkey();
    println!("Config PDA: {}", config_pubkey);

    match client.get_account(&config_pubkey).await {
        Ok(Some(_)) => {
            println!("\nThreadConfig already exists at {}", config_pubkey);
            println!("Use 'antegen program config get' to view current configuration.");
            return Ok(());
        }
        Ok(None) => {
            println!("\nThreadConfig does not exist, initializing...");
        }
        Err(e) => {
            println!("Warning: Failed to check config account: {}", e);
            println!("Proceeding with initialization...");
        }
    }

    // Build ConfigInit instruction
    let accounts = antegen_thread_program::accounts::ConfigInit {
        admin: admin.pubkey(),
        config: config_pubkey,
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);

    let data = antegen_thread_program::instruction::InitConfig {}.data();

    let ix = Instruction {
        program_id: antegen_thread_program::ID,
        accounts,
        data,
    };

    // Send transaction
    let (blockhash, _) = client.get_latest_blockhash().await?;
    let message = Message::new(&[ix], Some(&admin.pubkey()));
    let tx = Transaction::new(&[&admin], message, blockhash);

    let sig = client
        .send_and_confirm_transaction(&tx)
        .await
        .map_err(|e| anyhow!("Failed to initialize config: {}", e))?;

    println!("\nThreadConfig initialized successfully!");
    println!("Transaction: {}", sig);
    println!("Config address: {}", config_pubkey);

    Ok(())
}

/// Display the current ThreadConfig
pub async fn config_get(rpc: Option<String>) -> Result<()> {
    let rpc_url = get_rpc_url(rpc)?;
    println!("RPC: {}", rpc_url);

    let client = RpcPool::with_url(&rpc_url)
        .map_err(|e| anyhow!("Failed to create RPC client: {}", e))?;

    let config_pubkey = ThreadConfig::pubkey();
    println!("Config PDA: {}", config_pubkey);

    let account = client
        .get_account(&config_pubkey)
        .await
        .map_err(|e| anyhow!("Failed to fetch config: {}", e))?
        .ok_or_else(|| anyhow!("ThreadConfig not found. Run 'antegen program config init' to initialize."))?;

    let data = account
        .decode_data()
        .map_err(|e| anyhow!("Failed to decode account data: {}", e))?;

    let config = ThreadConfig::try_deserialize(&mut data.as_slice())
        .map_err(|e| anyhow!("Failed to deserialize ThreadConfig: {}", e))?;

    println!("\n=== ThreadConfig ===");
    println!("Version: {}", config.version);
    println!("Bump: {}", config.bump);
    println!("Admin: {}", config.admin);
    println!("Paused: {}", config.paused);
    println!();
    println!("=== Commission Settings ===");
    println!("Commission Fee: {} lamports", config.commission_fee);
    println!("Executor Fee: {}% ({}bps)", config.executor_fee_bps / 100, config.executor_fee_bps);
    println!("Core Team Fee: {}% ({}bps)", config.core_team_bps / 100, config.core_team_bps);
    println!();
    println!("=== Timing ===");
    println!("Grace Period: {} seconds", config.grace_period_seconds);
    println!("Fee Decay: {} seconds", config.fee_decay_seconds);
    println!("Total Window: {} seconds", config.grace_period_seconds + config.fee_decay_seconds);

    Ok(())
}
