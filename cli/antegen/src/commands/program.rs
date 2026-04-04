//! Program configuration and deployment commands
//!
//! Commands for managing the thread program's global configuration and deploying via Anchor.

use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use antegen_client::rpc::RpcPool;
use antegen_thread_program::state::ThreadConfig;
use anyhow::{anyhow, Result};
use solana_sdk::{
    instruction::Instruction, message::Message, pubkey::Pubkey, signer::Signer,
    transaction::Transaction,
};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use antegen_cli_core::commands::{get_keypair, get_rpc_url};

// =============================================================================
// Deploy helpers
// =============================================================================

/// Which program a binary belongs to
enum DetectedProgram {
    Fiber,
    Thread,
    Unknown,
}

/// Try to figure out which program a binary + optional program-id refers to.
fn detect_program(program_id: &Option<String>, binary_path: &Path) -> DetectedProgram {
    // Check --program-id against known IDs
    if let Some(ref id_str) = program_id {
        // Could be a pubkey string or a keypair file path
        let pubkey = Pubkey::from_str(id_str).ok().or_else(|| {
            // Try reading as keypair file → extract pubkey
            solana_sdk::signature::read_keypair_file(id_str)
                .ok()
                .map(|kp| kp.pubkey())
        });
        if let Some(pk) = pubkey {
            if pk == antegen_fiber_program::ID {
                return DetectedProgram::Fiber;
            }
            if pk == antegen_thread_program::ID {
                return DetectedProgram::Thread;
            }
        }
    }

    // Fallback: check binary filename
    let name = binary_path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if name.contains("fiber") {
        return DetectedProgram::Fiber;
    }
    if name.contains("thread") {
        return DetectedProgram::Thread;
    }

    DetectedProgram::Unknown
}

/// Check solana CLI is available and print its version.
fn check_solana_cli() -> Result<()> {
    let output = Command::new("solana")
        .arg("--version")
        .output()
        .map_err(|_| {
            anyhow!(
                "'solana' CLI not found. Install it: https://solana.com/docs/intro/installation"
            )
        })?;
    let version = String::from_utf8_lossy(&output.stdout);
    println!("Solana: {}", version.trim());
    Ok(())
}

/// Deploy a single .so binary via `solana program deploy`.
///
/// `program_id_arg` is passed as `--program-id` — it can be a pubkey string
/// or a path to a keypair file.
fn deploy_single(
    binary: &Path,
    program_id_arg: Option<&str>,
    rpc: &Option<String>,
    keypair_path: &Option<PathBuf>,
) -> Result<()> {
    if !binary.exists() {
        return Err(anyhow!(
            "Program binary not found: {}\n\
             Build your program first, then pass the path to the .so file.",
            binary.display()
        ));
    }
    println!("Binary: {}", binary.display());

    let mut args: Vec<String> = vec![
        "program".into(),
        "deploy".into(),
        binary.to_string_lossy().to_string(),
    ];

    if let Some(ref url) = rpc {
        args.push("--url".into());
        args.push(url.clone());
    }
    if let Some(ref kp) = keypair_path {
        args.push("--keypair".into());
        args.push(kp.to_string_lossy().to_string());
    }
    if let Some(pid) = program_id_arg {
        args.push("--program-id".into());
        args.push(pid.to_string());
    }

    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let status = Command::new("solana")
        .args(&args_ref)
        .status()
        .map_err(|e| anyhow!("Failed to run 'solana program deploy': {}", e))?;

    if !status.success() {
        return Err(anyhow!(
            "'solana program deploy' failed with status: {}",
            status
        ));
    }
    println!("Deploy complete.");
    Ok(())
}

/// Verify a program is deployed and executable on-chain.
async fn verify_program(pubkey: Pubkey, rpc_url: &str) -> Result<()> {
    let client =
        RpcPool::with_url(rpc_url).map_err(|e| anyhow!("Failed to create RPC client: {}", e))?;

    println!("Program ID: {}", pubkey);
    match client.get_account(&pubkey).await {
        Ok(Some(account)) => {
            if account.executable {
                println!("Program is deployed and executable.");
            } else {
                println!("Warning: Account exists but is not marked executable.");
            }
        }
        Ok(None) => {
            println!("Warning: Program account not found at {}", pubkey);
        }
        Err(e) => {
            println!("Warning: Failed to verify program: {}", e);
        }
    }
    Ok(())
}

// =============================================================================
// Deploy command
// =============================================================================

/// Deploy program(s) to a Solana cluster using `solana program deploy`.
///
/// Two modes:
/// - **Single-program**: pass a `.so` path → deploys that binary (backwards compat).
/// - **Full deploy**: omit the binary → auto-discovers fiber + thread from
///   `target/deploy/`, resolves keypairs from `--keys-dir`, and deploys both
///   in dependency order (fiber → thread → ThreadConfig init).
pub async fn deploy(
    program_binary: Option<PathBuf>,
    rpc: Option<String>,
    keypair_path: Option<PathBuf>,
    program_id: Option<String>,
    keys_dir: Option<PathBuf>,
    skip_init: bool,
    skip_verify: bool,
) -> Result<()> {
    check_solana_cli()?;

    if let Some(binary) = program_binary {
        // ── Single-program mode ──────────────────────────────────────────
        deploy_single(&binary, program_id.as_deref(), &rpc, &keypair_path)?;

        let detected = detect_program(&program_id, &binary);

        let rpc_url = get_rpc_url(rpc.clone())?;

        // Only run config init for thread program
        if !skip_init {
            if let DetectedProgram::Thread = detected {
                println!("\n--- Initializing ThreadConfig ---");
                match config_init(Some(rpc_url.clone()), keypair_path.clone()).await {
                    Ok(()) => {}
                    Err(e) => {
                        println!("Warning: config init failed: {}", e);
                        println!("You can run it manually: antegen program config init");
                    }
                }
            }
        } else {
            println!("\nSkipping config init (--skip-init)");
        }

        if !skip_verify {
            println!("\n--- Verifying ---");
            let pubkey = match detected {
                DetectedProgram::Fiber => antegen_fiber_program::ID,
                DetectedProgram::Thread => antegen_thread_program::ID,
                DetectedProgram::Unknown => antegen_thread_program::ID,
            };
            verify_program(pubkey, &rpc_url).await?;
        } else {
            println!("\nSkipping verification (--skip-verify)");
        }
    } else {
        // ── Full deploy mode (fiber → thread → init) ────────────────────
        let keys_dir = keys_dir.ok_or_else(|| {
            anyhow!(
                "--keys-dir is required when deploying both programs.\n\
                 Provide a directory containing program keypair files named {{program_id}}.json."
            )
        })?;

        let fiber_so = Path::new("target/deploy/antegen_fiber_program.so");
        let thread_so = Path::new("target/deploy/antegen_thread_program.so");

        if !fiber_so.exists() {
            return Err(anyhow!(
                "Fiber binary not found at {}. Run `cargo build-sbf` first.",
                fiber_so.display()
            ));
        }
        if !thread_so.exists() {
            return Err(anyhow!(
                "Thread binary not found at {}. Run `cargo build-sbf` first.",
                thread_so.display()
            ));
        }

        let fiber_id = antegen_fiber_program::ID;
        let thread_id = antegen_thread_program::ID;

        let fiber_keypair = keys_dir.join(format!("{}.json", fiber_id));
        let thread_keypair = keys_dir.join(format!("{}.json", thread_id));

        if !fiber_keypair.exists() {
            return Err(anyhow!(
                "Fiber keypair not found: {}\n\
                 Expected file named {}.json in --keys-dir",
                fiber_keypair.display(),
                fiber_id
            ));
        }
        if !thread_keypair.exists() {
            return Err(anyhow!(
                "Thread keypair not found: {}\n\
                 Expected file named {}.json in --keys-dir",
                thread_keypair.display(),
                thread_id
            ));
        }

        let rpc_url = get_rpc_url(rpc.clone())?;

        // Step 1: fiber
        println!("\n--- Step 1/3: Deploying fiber program ---");
        deploy_single(
            fiber_so,
            Some(&fiber_keypair.to_string_lossy()),
            &rpc,
            &keypair_path,
        )?;
        if !skip_verify {
            verify_program(fiber_id, &rpc_url).await?;
        }

        // Step 2: thread
        println!("\n--- Step 2/3: Deploying thread program ---");
        deploy_single(
            thread_so,
            Some(&thread_keypair.to_string_lossy()),
            &rpc,
            &keypair_path,
        )?;
        if !skip_verify {
            verify_program(thread_id, &rpc_url).await?;
        }

        // Step 3: init
        if !skip_init {
            println!("\n--- Step 3/3: Initializing ThreadConfig ---");
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

    let client =
        RpcPool::with_url(&rpc_url).map_err(|e| anyhow!("Failed to create RPC client: {}", e))?;

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

    let client =
        RpcPool::with_url(&rpc_url).map_err(|e| anyhow!("Failed to create RPC client: {}", e))?;

    let config_pubkey = ThreadConfig::pubkey();
    println!("Config PDA: {}", config_pubkey);

    let account = client
        .get_account(&config_pubkey)
        .await
        .map_err(|e| anyhow!("Failed to fetch config: {}", e))?
        .ok_or_else(|| {
            anyhow!("ThreadConfig not found. Run 'antegen program config init' to initialize.")
        })?;

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
    println!(
        "Executor Fee: {}% ({}bps)",
        config.executor_fee_bps / 100,
        config.executor_fee_bps
    );
    println!(
        "Core Team Fee: {}% ({}bps)",
        config.core_team_bps / 100,
        config.core_team_bps
    );
    println!();
    println!("=== Timing ===");
    println!("Grace Period: {} seconds", config.grace_period_seconds);
    println!("Fee Decay: {} seconds", config.fee_decay_seconds);
    println!(
        "Total Window: {} seconds",
        config.grace_period_seconds + config.fee_decay_seconds
    );

    Ok(())
}
