//! Client commands - executor fund and withdraw operations

use anyhow::{Context, Result};
use antegen_client::ClientConfig;
use antegen_client::rpc::RpcPool;
use solana_sdk::native_token::LAMPORTS_PER_SOL;
use solana_sdk::signature::{read_keypair_file, Signer};
use solana_sdk::message::Message;
use solana_sdk::transaction::Transaction;
use std::path::PathBuf;

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

/// Fund the executor with SOL from Solana CLI wallet
///
/// - `amount = Some(x)`: transfers exactly `x` SOL to executor
/// - `amount = None`: checks executor balance, tops up deficit to MIN_BALANCE_LAMPORTS
pub async fn fund(
    config_path: PathBuf,
    amount: Option<f64>,
    keypair_path: Option<PathBuf>,
    rpc_override: Option<String>,
) -> Result<()> {
    use super::MIN_BALANCE_LAMPORTS;

    // Load config to get executor pubkey
    let config = ClientConfig::load(&config_path)
        .with_context(|| format!("Failed to load config from {:?}", config_path))?;

    let executor_keypair_path = expand_path(&config.executor.keypair_path)?;
    if !executor_keypair_path.exists() {
        anyhow::bail!(
            "Executor keypair not found at: {}\n\nRun 'antegen node start' to generate one first.",
            executor_keypair_path.display()
        );
    }

    let executor_keypair = load_keypair_from_config(&config)?;
    let destination = executor_keypair.pubkey();

    // Funding keypair: flag → Solana CLI config
    let funding_keypair = super::get_keypair(keypair_path)?;

    // RPC: flag → Solana CLI config
    let rpc_url = super::get_rpc_url(rpc_override)?;
    let client = RpcPool::with_url(&rpc_url)
        .context("Failed to create RPC client")?;

    // Determine lamports to transfer
    let lamports = if let Some(sol) = amount {
        (sol * LAMPORTS_PER_SOL as f64) as u64
    } else {
        // Smart top-up: check current balance and top up to minimum
        let balance = client
            .get_balance(&destination)
            .await
            .context("Failed to get executor balance")?;

        if balance >= MIN_BALANCE_LAMPORTS {
            let sol = balance as f64 / LAMPORTS_PER_SOL as f64;
            println!("Executor balance: {:.9} SOL ({} lamports)", sol, balance);
            println!("Already at or above minimum ({} lamports). No funding needed.", MIN_BALANCE_LAMPORTS);
            return Ok(());
        }

        let deficit = MIN_BALANCE_LAMPORTS - balance;
        println!("Executor balance: {} lamports (below minimum {})", balance, MIN_BALANCE_LAMPORTS);
        deficit
    };

    let sol_amount = lamports as f64 / LAMPORTS_PER_SOL as f64;

    println!("Transferring {:.9} SOL ({} lamports)", sol_amount, lamports);
    println!("  From (CLI wallet): {}", funding_keypair.pubkey());
    println!("  To (executor):     {}", destination);

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
    println!("Executor balance: {:.9} SOL", new_sol);

    Ok(())
}

/// Withdraw SOL from executor to Solana CLI keypair
///
/// - `amount = Some(x)`: withdraws exactly `x` SOL from executor
/// - `amount = None`: withdraws everything above MIN_BALANCE_LAMPORTS + fee buffer
pub async fn withdraw(
    config_path: PathBuf,
    amount: Option<f64>,
    rpc_override: Option<String>,
) -> Result<()> {
    use super::MIN_BALANCE_LAMPORTS;

    // Load config
    let config = ClientConfig::load(&config_path)
        .with_context(|| format!("Failed to load config from {:?}", config_path))?;

    let keypair_path = expand_path(&config.executor.keypair_path)?;

    // Check if keypair exists
    if !keypair_path.exists() {
        anyhow::bail!(
            "Executor keypair not found at: {}\n\nRun 'antegen node start' to generate one first.",
            keypair_path.display()
        );
    }

    // Load executor keypair (source)
    let executor_keypair = load_keypair_from_config(&config)?;
    let executor_pubkey = executor_keypair.pubkey();

    // Load Solana CLI keypair for destination
    let destination_keypair = super::get_keypair(None)?;
    let destination = destination_keypair.pubkey();

    // RPC: flag → Solana CLI config
    let rpc_url = super::get_rpc_url(rpc_override)?;

    let client = RpcPool::with_url(&rpc_url)
        .context("Failed to create RPC client")?;

    // Get current balance
    let balance = client
        .get_balance(&executor_pubkey)
        .await
        .context("Failed to get executor balance")?;

    // Calculate amount to withdraw
    let fee_buffer = 5000u64;
    let lamports = if let Some(sol) = amount {
        let requested = (sol * LAMPORTS_PER_SOL as f64) as u64;
        if requested + fee_buffer > balance {
            anyhow::bail!(
                "Insufficient balance. Requested: {} lamports + {} fee, available: {} lamports",
                requested,
                fee_buffer,
                balance
            );
        }
        requested
    } else {
        // Smart withdraw: everything above minimum + fee buffer
        let reserve = MIN_BALANCE_LAMPORTS + fee_buffer;
        if balance <= reserve {
            let sol = balance as f64 / LAMPORTS_PER_SOL as f64;
            anyhow::bail!(
                "Balance ({:.9} SOL) is at or below minimum reserve ({} lamports). Nothing to withdraw.",
                sol,
                reserve
            );
        }
        balance - reserve
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
    println!("  Executor:   {:.9} SOL", new_executor_balance as f64 / LAMPORTS_PER_SOL as f64);
    println!("  CLI wallet: {:.9} SOL", new_destination_balance as f64 / LAMPORTS_PER_SOL as f64);

    Ok(())
}
