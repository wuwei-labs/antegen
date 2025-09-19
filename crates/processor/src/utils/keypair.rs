use anyhow::Result;
use log::info;
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    pubkey::Pubkey,
    signature::{read_keypair_file, write_keypair_file, Keypair, Signer},
};
use std::path::{Path, PathBuf};

pub struct KeypairManager {
    keypair_path: PathBuf,
    rpc_url: String,
}

impl KeypairManager {
    pub fn new(keypair_path: impl AsRef<Path>, rpc_url: String) -> Self {
        Self {
            keypair_path: keypair_path.as_ref().to_path_buf(),
            rpc_url,
        }
    }

    /// Complete initialization: wait for RPC, load/create keypair, ensure funded
    pub async fn initialize(&self, min_balance: u64) -> Result<Keypair> {
        // Step 1: Wait for validator to be ready
        info!("Waiting for validator connection...");
        self.wait_for_validator().await?;

        // Step 2: Load or create keypair
        let keypair = self.get_or_create_keypair().await?;

        // Step 3: Ensure account is funded
        self.ensure_funded(&keypair, min_balance).await?;

        Ok(keypair)
    }

    /// Initialize without waiting for validator (for Geyser plugin context)
    /// This is used when running inside the validator to avoid deadlock
    pub async fn initialize_without_wait(&self) -> Result<Keypair> {
        info!("Initializing keypair without validator wait (Geyser context)");

        // Just load or create keypair, skip validator wait and funding check
        let keypair = self.get_or_create_keypair().await?;

        info!("Keypair initialized: {}", keypair.pubkey());
        Ok(keypair)
    }

    async fn wait_for_validator(&self) -> Result<()> {
        let client = RpcClient::new(&self.rpc_url);
        let mut attempts = 0;
        let max_attempts = 30;

        loop {
            attempts += 1;

            // Try to get version (simplest RPC call)
            match client.get_version() {
                Ok(_) => {
                    info!("Validator is ready (attempt {}/{})", attempts, max_attempts);
                    return Ok(());
                }
                Err(_) if attempts < max_attempts => {
                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                }
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "Validator failed to become ready after {} attempts: {}",
                        max_attempts,
                        e
                    ));
                }
            }
        }
    }

    async fn get_or_create_keypair(&self) -> Result<Keypair> {
        if self.keypair_path.exists() {
            info!("Loading existing keypair from: {:?}", self.keypair_path);
            read_keypair_file(&self.keypair_path)
                .map_err(|e| anyhow::anyhow!("Failed to read keypair: {}", e))
        } else {
            info!("Keypair not found, creating new one at: {:?}", self.keypair_path);

            // Ensure parent directory exists
            if let Some(parent) = self.keypair_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            // Generate new keypair
            let keypair = Keypair::new();
            write_keypair_file(&keypair, &self.keypair_path)
                .map_err(|e| anyhow::anyhow!("Failed to write keypair: {}", e))?;

            info!("Generated new executor keypair: {}", keypair.pubkey());
            Ok(keypair)
        }
    }

    async fn ensure_funded(&self, keypair: &Keypair, min_balance: u64) -> Result<()> {
        let client = RpcClient::new(&self.rpc_url);
        let pubkey = keypair.pubkey();
        let mut displayed_instructions = false;

        loop {
            match client.get_balance(&pubkey) {
                Ok(balance) => {
                    if balance >= min_balance {
                        if displayed_instructions {
                            println!("\n✓ Account funded successfully!");
                        }
                        info!("Executor account has sufficient balance: {} lamports", balance);
                        return Ok(());
                    }

                    // Display funding instructions once
                    if !displayed_instructions {
                        self.display_funding_instructions(&pubkey, balance);
                        displayed_instructions = true;
                    }
                }
                Err(_) => {
                    // Account might not exist yet, treat as 0 balance
                    if !displayed_instructions {
                        self.display_funding_instructions(&pubkey, 0);
                        displayed_instructions = true;
                    }
                }
            }

            // Wait before next check
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            print!(".");
            let _ = std::io::Write::flush(&mut std::io::stdout());
        }
    }

    fn display_funding_instructions(&self, pubkey: &Pubkey, current_balance: u64) {
        println!("\n╭────────────────────────────────────────────────────────────╮");
        if current_balance == 0 {
            println!("│ ⚠️  Executor account has 0 SOL balance                      │");
        } else {
            println!("│ ⚠️  Executor account has insufficient balance               │");
            println!(
                "│    Current: {:.6} SOL                                 │",
                current_balance as f64 / 1_000_000_000.0
            );
        }
        println!("╰────────────────────────────────────────────────────────────╯");

        let is_localnet = self.rpc_url.contains("localhost") || self.rpc_url.contains("127.0.0.1");

        if is_localnet {
            println!("\nPlease fund this account to start processing:");
            println!("  solana airdrop 1 {} --url {}", pubkey, self.rpc_url);
        } else {
            println!("\nPlease transfer SOL to this account to start processing:");
            println!("\n  {}", pubkey);
            println!("\nMinimum recommended balance: 0.1 SOL");
        }

        println!("\nWaiting for account to be funded");
    }
}