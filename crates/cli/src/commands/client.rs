//! Client commands - identity management and utilities

use anyhow::{Context, Result};
use antegen_client::ClientConfig;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use antegen_client::rpc::RpcPool;
use solana_sdk::native_token::LAMPORTS_PER_SOL;
use solana_sdk::signature::{read_keypair_file, Keypair, Signer};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::message::Message;
use solana_sdk::transaction::Transaction;
use std::str::FromStr;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tar::{Archive, Builder};

/// Manifest file included in the backup archive
#[derive(Debug, Serialize, Deserialize)]
struct BackupManifest {
    /// CLI version that created this backup
    version: String,
    /// Original observability storage path from config
    original_observability_path: String,
    /// Timestamp of backup creation
    created_at: String,
}

/// Expand ~ in path to home directory
fn expand_path(path: &str) -> Result<PathBuf> {
    let expanded = shellexpand::tilde(path);
    Ok(PathBuf::from(expanded.as_ref()))
}

/// Load keypair from config's keypair_path
fn load_keypair_from_config(config: &ClientConfig) -> Result<solana_sdk::signature::Keypair> {
    let keypair_path = expand_path(&config.executor.keypair_path)?;
    read_keypair_file(&keypair_path)
        .map_err(|e| anyhow::anyhow!("Failed to read keypair from {:?}: {}", keypair_path, e))
}

/// Show executor public key
pub fn address(config_path: PathBuf) -> Result<()> {
    // Load config
    let config = ClientConfig::load(&config_path)
        .with_context(|| format!("Failed to load config from {:?}", config_path))?;

    let keypair_path = expand_path(&config.executor.keypair_path)?;

    // Check if keypair exists
    if !keypair_path.exists() {
        println!("Keypair not found at: {}", keypair_path.display());
        println!();
        println!("Run 'antegen start' to auto-generate a keypair, or specify an existing keypair in your config.");
        return Ok(());
    }

    // Load and display keypair
    let keypair = load_keypair_from_config(&config)?;
    println!("{}", keypair.pubkey());

    Ok(())
}

/// Show executor SOL balance
pub async fn balance(config_path: PathBuf, rpc_override: Option<String>) -> Result<()> {
    // Load config
    let config = ClientConfig::load(&config_path)
        .with_context(|| format!("Failed to load config from {:?}", config_path))?;

    let keypair_path = expand_path(&config.executor.keypair_path)?;

    // Check if keypair exists
    if !keypair_path.exists() {
        println!("Keypair not found at: {}", keypair_path.display());
        println!();
        println!("Run 'antegen start' to auto-generate a keypair, or specify an existing keypair in your config.");
        return Ok(());
    }

    // Load keypair
    let keypair = load_keypair_from_config(&config)?;
    let pubkey = keypair.pubkey();

    // Determine RPC endpoint
    let rpc_url = if let Some(url) = rpc_override {
        url
    } else {
        // Use first endpoint from config that can be used for queries
        config
            .rpc
            .endpoints
            .first()
            .map(|e| e.url.clone())
            .ok_or_else(|| anyhow::anyhow!("No RPC endpoints configured"))?
    };

    // Query balance
    let client = RpcPool::with_url(&rpc_url)
        .with_context(|| format!("Failed to create RPC client for {}", rpc_url))?;
    let balance = client
        .get_balance(&pubkey)
        .await
        .with_context(|| format!("Failed to get balance from {}", rpc_url))?;

    let sol = balance as f64 / LAMPORTS_PER_SOL as f64;

    println!("Address: {}", pubkey);
    println!("Balance: {:.9} SOL ({} lamports)", sol, balance);

    Ok(())
}

/// Transfer SOL to an address
pub async fn refill(
    address: String,
    amount: f64,
    keypair_path: Option<PathBuf>,
    rpc_override: Option<String>,
) -> Result<()> {
    // Parse destination address
    let destination = Pubkey::from_str(&address)
        .with_context(|| format!("Invalid destination address: {}", address))?;

    // Load Solana CLI config for defaults
    let solana_config = solana_cli_config::Config::load(
        solana_cli_config::CONFIG_FILE.as_ref().unwrap(),
    )
    .unwrap_or_default();

    // Get funding keypair (source)
    let funding_keypair_path = keypair_path
        .map(|p| expand_path(&p.to_string_lossy()).unwrap_or(p))
        .unwrap_or_else(|| PathBuf::from(&solana_config.keypair_path));

    let funding_keypair = read_keypair_file(&funding_keypair_path)
        .map_err(|e| anyhow::anyhow!("Failed to read funding keypair from {:?}: {}", funding_keypair_path, e))?;

    // Get RPC client
    let rpc_url = rpc_override.unwrap_or(solana_config.json_rpc_url);
    let client = RpcPool::with_url(&rpc_url)
        .context("Failed to create RPC client")?;

    // Convert SOL to lamports
    let lamports = (amount * LAMPORTS_PER_SOL as f64) as u64;

    println!("Transferring {:.9} SOL ({} lamports)", amount, lamports);
    println!("  From: {}", funding_keypair.pubkey());
    println!("  To:   {}", destination);

    // Create and send transfer
    let (recent_blockhash, _) = client.get_latest_blockhash()
        .await
        .context("Failed to get recent blockhash")?;

    let transfer_ix = solana_system_interface::instruction::transfer(
        &funding_keypair.pubkey(),
        &destination,
        lamports,
    );

    let message = Message::new(&[transfer_ix], Some(&funding_keypair.pubkey()));
    let tx = Transaction::new(&[&funding_keypair], message, recent_blockhash);

    let signature = client.send_and_confirm_transaction(&tx)
        .await
        .context("Failed to send transaction")?;

    let new_balance = client.get_balance(&destination)
        .await
        .context("Failed to get new balance")?;
    let new_sol = new_balance as f64 / LAMPORTS_PER_SOL as f64;

    println!();
    println!("Transaction: {}", signature);
    println!("New balance: {:.9} SOL", new_sol);

    Ok(())
}

/// Export client identity to a portable archive
///
/// NOTE: This does NOT export the executor keypair for safety.
/// The archive is safe to share - it only contains config and observability identity.
/// On import, a new keypair will be generated or you can specify an existing one.
pub fn export(config_path: PathBuf, output_path: PathBuf) -> Result<()> {
    // Load config
    let config = ClientConfig::load(&config_path)
        .with_context(|| format!("Failed to load config from {:?}", config_path))?;

    // Check if output already exists
    if output_path.exists() {
        anyhow::bail!(
            "Output file already exists: {}. Remove it first or choose a different name.",
            output_path.display()
        );
    }

    // Resolve paths
    let observability_path = expand_path(&config.observability.storage_path)?;
    let agent_id_path = observability_path.join("agent_id.key");

    println!("Exporting client identity...");
    println!();

    // Create the archive
    let output_file = File::create(&output_path)
        .with_context(|| format!("Failed to create output file: {:?}", output_path))?;
    let encoder = GzEncoder::new(output_file, Compression::default());
    let mut archive = Builder::new(encoder);

    // Add config file
    println!("  + antegen.toml");
    add_file_to_archive(&mut archive, &config_path, "antegen.toml")?;

    // Add agent_id.key if it exists
    if agent_id_path.exists() {
        println!("  + agent_id.key");
        add_file_to_archive(&mut archive, &agent_id_path, "agent_id.key")?;
    } else {
        println!("  - agent_id.key (not found, skipping)");
    }

    // Create and add manifest
    let manifest = BackupManifest {
        version: env!("CARGO_PKG_VERSION").to_string(),
        original_observability_path: config.observability.storage_path.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    println!("  + manifest.json");
    add_bytes_to_archive(&mut archive, manifest_json.as_bytes(), "manifest.json")?;

    // Finish archive
    archive.finish()?;

    println!();
    println!("Exported to: {}", output_path.display());
    println!();
    println!("This archive is safe to share - it does NOT contain any keypairs.");
    println!();
    println!("To restore on another machine:");
    println!("  antegen client import --input {}", output_path.display());

    Ok(())
}

/// Import client identity from a backup archive
///
/// If `keypair_path` is provided, that path will be used in the config (not copied).
/// If `keypair_path` is None, a new keypair will be generated at ~/.antegen/executor-keypair.json.
pub fn import(input_path: PathBuf, keypair_path: Option<PathBuf>, force: bool) -> Result<()> {
    // Check input exists
    if !input_path.exists() {
        anyhow::bail!("Input file not found: {}", input_path.display());
    }

    println!("Importing client identity...");
    println!();

    // Open and decompress archive
    let input_file =
        File::open(&input_path).with_context(|| format!("Failed to open {:?}", input_path))?;
    let decoder = GzDecoder::new(input_file);
    let mut archive = Archive::new(decoder);

    // Extract to temp directory first
    let temp_dir = tempfile::tempdir().context("Failed to create temp directory")?;
    archive
        .unpack(temp_dir.path())
        .context("Failed to extract archive")?;

    // Read manifest
    let manifest_path = temp_dir.path().join("manifest.json");
    let manifest: BackupManifest = if manifest_path.exists() {
        let content = fs::read_to_string(&manifest_path)?;
        serde_json::from_str(&content)?
    } else {
        anyhow::bail!("Invalid backup: manifest.json not found");
    };

    println!("Backup created: {}", manifest.created_at);
    println!("Backup version: {}", manifest.version);
    println!();

    // Determine destination paths
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    let antegen_dir = home.join(".antegen");
    let observability_dir = antegen_dir.join("observability");

    // Create directories
    fs::create_dir_all(&antegen_dir).context("Failed to create ~/.antegen directory")?;
    fs::create_dir_all(&observability_dir)
        .context("Failed to create ~/.antegen/observability directory")?;

    // Destination paths
    let dest_keypair = antegen_dir.join("executor-keypair.json");
    let dest_config = PathBuf::from("antegen.toml");
    let dest_agent_id = observability_dir.join("agent_id.key");

    // Check for existing files (only config and agent_id, keypair is handled separately)
    let mut conflicts = Vec::new();
    if dest_config.exists() {
        conflicts.push(dest_config.display().to_string());
    }
    if dest_agent_id.exists() {
        conflicts.push(dest_agent_id.display().to_string());
    }

    if !conflicts.is_empty() && !force {
        println!("The following files already exist:");
        for path in &conflicts {
            println!("  - {}", path);
        }
        println!();
        println!("Use --force to overwrite existing files.");
        return Ok(());
    }

    // Handle keypair - either use provided path or generate new one
    let (keypair_path_for_config, pubkey) = if let Some(user_keypair) = keypair_path {
        // User provided keypair - just reference it in config (don't copy)
        let expanded = expand_path(&user_keypair.to_string_lossy())?;
        let kp = read_keypair_file(&expanded)
            .map_err(|e| anyhow::anyhow!("Failed to read keypair from {:?}: {}", expanded, e))?;
        println!("  Using keypair: {}", user_keypair.display());
        (user_keypair.to_string_lossy().to_string(), kp.pubkey())
    } else {
        // Generate new keypair at ~/.antegen/ or use existing
        if dest_keypair.exists() && !force {
            // Use existing keypair
            let kp = read_keypair_file(&dest_keypair)
                .map_err(|e| anyhow::anyhow!("Failed to read existing keypair: {}", e))?;
            println!("  Using existing keypair: {}", dest_keypair.display());
            ("~/.antegen/executor-keypair.json".to_string(), kp.pubkey())
        } else {
            // Generate new keypair
            let new_kp = Keypair::new();
            let keypair_bytes: Vec<u8> = new_kp.to_bytes().to_vec();
            let keypair_json = serde_json::to_string(&keypair_bytes)?;
            fs::write(&dest_keypair, &keypair_json)?;

            // Set restrictive permissions on keypair
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&dest_keypair, fs::Permissions::from_mode(0o600))?;
            }

            println!("  -> {} (generated)", dest_keypair.display());
            ("~/.antegen/executor-keypair.json".to_string(), new_kp.pubkey())
        }
    };

    // Copy config file
    let src_config = temp_dir.path().join("antegen.toml");
    if src_config.exists() {
        // Load and update config with new paths
        let mut config: ClientConfig = {
            let content = fs::read_to_string(&src_config)?;
            toml::from_str(&content)?
        };

        // Update paths
        config.executor.keypair_path = keypair_path_for_config;
        config.observability.storage_path = "~/.antegen/observability".to_string();

        // Save updated config
        println!("  -> {}", dest_config.display());
        let config_content = toml::to_string_pretty(&config)?;
        fs::write(&dest_config, config_content)?;
    }

    // Copy agent_id.key
    let src_agent_id = temp_dir.path().join("agent_id.key");
    if src_agent_id.exists() {
        println!("  -> {}", dest_agent_id.display());
        fs::copy(&src_agent_id, &dest_agent_id)?;
    }

    // Display summary
    println!();
    println!("Executor pubkey: {}", pubkey);
    println!();
    println!("Import complete!");
    println!();
    println!("Next steps:");
    println!("  1. Review antegen.toml and update RPC endpoints if needed");
    println!("  2. Fund the executor address: {}", pubkey);
    println!("  3. Run: antegen start");

    Ok(())
}

/// Withdraw SOL from executor to Solana CLI keypair
pub async fn withdraw(
    config_path: PathBuf,
    amount: Option<f64>,
    all: bool,
    rpc_override: Option<String>,
) -> Result<()> {
    // Validate flags
    if amount.is_none() && !all {
        anyhow::bail!("Must specify either --amount or --all");
    }
    if amount.is_some() && all {
        anyhow::bail!("Cannot specify both --amount and --all");
    }

    // Load config
    let config = ClientConfig::load(&config_path)
        .with_context(|| format!("Failed to load config from {:?}", config_path))?;

    let keypair_path = expand_path(&config.executor.keypair_path)?;

    // Check if keypair exists
    if !keypair_path.exists() {
        anyhow::bail!(
            "Executor keypair not found at: {}\n\nRun 'antegen start' to generate one first.",
            keypair_path.display()
        );
    }

    // Load executor keypair (source)
    let executor_keypair = load_keypair_from_config(&config)?;
    let executor_pubkey = executor_keypair.pubkey();

    // Load Solana CLI config for destination
    let solana_config = solana_cli_config::Config::load(
        solana_cli_config::CONFIG_FILE.as_ref().unwrap(),
    )
    .context("Failed to load Solana CLI config")?;

    let destination_keypair = read_keypair_file(&solana_config.keypair_path)
        .map_err(|e| anyhow::anyhow!("Failed to read Solana CLI keypair: {}", e))?;
    let destination = destination_keypair.pubkey();

    // Get RPC client
    let rpc_url = if let Some(url) = rpc_override {
        url
    } else {
        config
            .rpc
            .endpoints
            .first()
            .map(|e| e.url.clone())
            .ok_or_else(|| anyhow::anyhow!("No RPC endpoints configured"))?
    };

    let client = RpcPool::with_url(&rpc_url)
        .context("Failed to create RPC client")?;

    // Get current balance
    let balance = client
        .get_balance(&executor_pubkey)
        .await
        .context("Failed to get executor balance")?;

    // Calculate amount to withdraw
    let lamports = if all {
        // Leave enough for transaction fee (5000 lamports is typical)
        let fee_buffer = 5000u64;
        if balance <= fee_buffer {
            anyhow::bail!(
                "Insufficient balance to withdraw. Current: {} lamports, need at least {} for fees.",
                balance,
                fee_buffer
            );
        }
        balance - fee_buffer
    } else {
        let requested = (amount.unwrap() * LAMPORTS_PER_SOL as f64) as u64;
        if requested > balance {
            anyhow::bail!(
                "Insufficient balance. Requested: {} lamports, available: {} lamports",
                requested,
                balance
            );
        }
        requested
    };

    let sol_amount = lamports as f64 / LAMPORTS_PER_SOL as f64;

    println!("Withdrawing {:.9} SOL ({} lamports)", sol_amount, lamports);
    println!("  From (executor): {}", executor_pubkey);
    println!("  To (CLI wallet): {}", destination);

    // Create and send transfer
    let (recent_blockhash, _) = client.get_latest_blockhash()
        .await
        .context("Failed to get recent blockhash")?;

    let transfer_ix = solana_system_interface::instruction::transfer(
        &executor_pubkey,
        &destination,
        lamports,
    );

    let message = Message::new(&[transfer_ix], Some(&executor_pubkey));
    let tx = Transaction::new(&[&executor_keypair], message, recent_blockhash);

    let signature = client.send_and_confirm_transaction(&tx)
        .await
        .context("Failed to send transaction")?;

    // Get new balances
    let new_executor_balance = client.get_balance(&executor_pubkey)
        .await
        .context("Failed to get executor balance")?;
    let new_destination_balance = client.get_balance(&destination)
        .await
        .context("Failed to get destination balance")?;

    println!();
    println!("Transaction: {}", signature);
    println!();
    println!("New balances:");
    println!("  Executor: {:.9} SOL", new_executor_balance as f64 / LAMPORTS_PER_SOL as f64);
    println!("  CLI wallet: {:.9} SOL", new_destination_balance as f64 / LAMPORTS_PER_SOL as f64);

    Ok(())
}

/// Add a file to the tar archive
fn add_file_to_archive<W: Write>(
    archive: &mut Builder<W>,
    source_path: &Path,
    archive_name: &str,
) -> Result<()> {
    let mut file = File::open(source_path)
        .with_context(|| format!("Failed to open {:?}", source_path))?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)?;
    add_bytes_to_archive(archive, &contents, archive_name)
}

/// Add bytes to the tar archive
fn add_bytes_to_archive<W: Write>(
    archive: &mut Builder<W>,
    data: &[u8],
    archive_name: &str,
) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    );
    header.set_cksum();

    archive
        .append_data(&mut header, archive_name, data)
        .with_context(|| format!("Failed to add {} to archive", archive_name))?;

    Ok(())
}
