//! Program configuration and deployment commands
//!
//! Commands for managing the thread program's global configuration and deploying via Anchor.

use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use antegen_client::rpc::RpcPool;
use antegen_thread_program::state::ThreadConfig;
use anyhow::{anyhow, Result};
use solana_sdk::{
    instruction::Instruction, message::Message, signer::Signer,
    transaction::Transaction,
};
use std::path::PathBuf;

use super::{get_keypair, get_rpc_url};

// =============================================================================
// Deploy command
// =============================================================================

/// Deploy the program binary to a Solana cluster using `solana program deploy`
///
/// When `rpc` and `keypair_path` are None, solana CLI reads its own config —
/// this is the desired default so users' Solana CLI settings are respected.
pub async fn deploy(
    program_binary: PathBuf,
    rpc: Option<String>,
    keypair_path: Option<PathBuf>,
    program_id: Option<String>,
    skip_init: bool,
    skip_verify: bool,
) -> Result<()> {
    use std::process::Command;

    // 1. Validate binary exists
    if !program_binary.exists() {
        return Err(anyhow!(
            "Program binary not found: {}\n\
             Build your program first, then pass the path to the .so file.",
            program_binary.display()
        ));
    }
    println!("Binary: {}", program_binary.display());

    // 2. Check solana CLI is installed
    {
        let output = Command::new("solana")
            .arg("--version")
            .output()
            .map_err(|_| anyhow!("'solana' CLI not found. Install it: https://solana.com/docs/intro/installation"))?;
        let version = String::from_utf8_lossy(&output.stdout);
        println!("Solana: {}", version.trim());
    }

    // 3. Deploy — only pass --url / --keypair when the user explicitly provided them
    println!("\n--- Deploying ---");
    let mut deploy_args: Vec<String> = vec![
        "program".to_string(),
        "deploy".to_string(),
        program_binary.to_string_lossy().to_string(),
    ];

    if let Some(ref url) = rpc {
        deploy_args.push("--url".to_string());
        deploy_args.push(url.clone());
    }

    if let Some(ref kp) = keypair_path {
        deploy_args.push("--keypair".to_string());
        deploy_args.push(kp.to_string_lossy().to_string());
    }

    if let Some(ref pid) = program_id {
        deploy_args.push("--program-id".to_string());
        deploy_args.push(pid.clone());
    }

    let deploy_args_ref: Vec<&str> = deploy_args.iter().map(|s| s.as_str()).collect();
    let status = Command::new("solana")
        .args(&deploy_args_ref)
        .status()
        .map_err(|e| anyhow!("Failed to run 'solana program deploy': {}", e))?;

    if !status.success() {
        return Err(anyhow!("'solana program deploy' failed with status: {}", status));
    }
    println!("Deploy complete.");

    // 4. Post-deploy: resolve RPC via Solana CLI config fallback for our own calls
    let rpc_url = get_rpc_url(rpc.clone())?;

    // 5. Init config
    if !skip_init {
        println!("\n--- Initializing ThreadConfig ---");
        match config_init(Some(rpc_url.clone()), keypair_path.clone()).await {
            Ok(()) => {}
            Err(e) => {
                println!("Warning: config init failed: {}", e);
                println!("You can run it manually: antegen program config init");
            }
        }
    } else {
        println!("\nSkipping config init (--skip-init)");
    }

    // 6. Verify
    if !skip_verify {
        println!("\n--- Verifying ---");
        let client = RpcPool::with_url(&rpc_url)
            .map_err(|e| anyhow!("Failed to create RPC client: {}", e))?;

        let pid = antegen_thread_program::ID;
        println!("Program ID: {}", pid);

        match client.get_account(&pid).await {
            Ok(Some(account)) => {
                if account.executable {
                    println!("Program is deployed and executable.");
                } else {
                    println!("Warning: Account exists but is not marked executable.");
                }
            }
            Ok(None) => {
                println!("Warning: Program account not found at {}", pid);
            }
            Err(e) => {
                println!("Warning: Failed to verify program: {}", e);
            }
        }
    } else {
        println!("\nSkipping verification (--skip-verify)");
    }

    println!("\nDone.");
    Ok(())
}

// =============================================================================
// Config commands
// =============================================================================

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
