//! Thread inspection and test commands

use anchor_lang::AccountDeserialize;
use antegen_client::rpc::RpcPool;
use antegen_thread_program::state::Thread;
use anyhow::{anyhow, Result};
use solana_sdk::pubkey::Pubkey;
#[cfg(feature = "dev")]
use solana_sdk::signature::{read_keypair_file, Keypair};
#[cfg(feature = "dev")]
use std::path::PathBuf;
use std::str::FromStr;

// =============================================================================
// Common utilities (always available)
// =============================================================================

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
#[cfg(feature = "dev")]
fn get_keypair(keypair_path: Option<PathBuf>) -> Result<Keypair> {
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

// =============================================================================
// Thread inspection commands (always available)
// =============================================================================

/// Fetch and display a thread account
pub async fn get(address: String, rpc_url: Option<String>) -> Result<()> {
    // Parse the public key
    let thread_pubkey =
        Pubkey::from_str(&address).map_err(|e| anyhow!("Invalid pubkey '{}': {}", address, e))?;

    // Get RPC URL
    let rpc_url = get_rpc_url(rpc_url)?;
    println!("Fetching thread {} from {}", thread_pubkey, rpc_url);

    let client = RpcPool::with_url(&rpc_url)
        .map_err(|e| anyhow!("Failed to create RPC client: {}", e))?;

    // Fetch the account
    let account = client
        .get_account(&thread_pubkey)
        .await
        .map_err(|e| anyhow!("Failed to fetch account: {}", e))?
        .ok_or_else(|| anyhow!("Account not found: {}", thread_pubkey))?;

    // Decode account data
    let data = account.decode_data()
        .map_err(|e| anyhow!("Failed to decode account data: {}", e))?;
    let owner = account.owner_pubkey()
        .map_err(|e| anyhow!("Failed to parse owner: {}", e))?;

    println!("\n=== Account Info ===");
    println!("Owner: {}", owner);
    println!("Lamports: {}", account.lamports);
    println!("Data length: {} bytes", data.len());
    println!("Executable: {}", account.executable);

    // Deserialize as Thread
    println!("\n=== Thread Deserialization ===");
    match Thread::try_deserialize(&mut data.as_slice()) {
        Ok(thread) => {
            println!("Success!\n");
            print_thread(&thread);
        }
        Err(e) => {
            println!("Failed to deserialize: {:?}", e);
            println!(
                "\nRaw data (first 100 bytes): {:?}",
                &data[..100.min(data.len())]
            );
        }
    }

    Ok(())
}

fn print_thread(thread: &Thread) {
    println!("=== Thread State ===");
    println!();

    // Identity
    println!("--- Identity ---");
    println!("  version: {}", thread.version);
    println!("  bump: {}", thread.bump);
    println!("  authority: {}", thread.authority);
    println!(
        "  id: {:?} ({})",
        thread.id,
        String::from_utf8_lossy(&thread.id)
    );
    println!("  name: {}", thread.name);
    println!("  created_at: {}", thread.created_at);
    println!();

    // Scheduling
    println!("--- Scheduling ---");
    println!("  trigger: {:?}", thread.trigger);
    println!("  schedule: {:?}", thread.schedule);
    println!();

    // Default fiber
    println!("--- Default Fiber ---");
    println!("  has_default_fiber: {}", thread.default_fiber.is_some());
    if let Some(ref fiber) = thread.default_fiber {
        println!("  default_fiber_len: {} bytes", fiber.len());
    }
    println!(
        "  default_fiber_priority_fee: {}",
        thread.default_fiber_priority_fee
    );
    println!();

    // Fibers
    println!("--- Fibers ---");
    println!("  fiber_ids: {:?}", thread.fiber_ids);
    println!("  fiber_cursor: {}", thread.fiber_cursor);
    println!("  fiber_next_id: {}", thread.fiber_next_id);
    println!("  fiber_signal: {:?}", thread.fiber_signal);
    println!();

    // Lifecycle
    println!("--- Lifecycle ---");
    println!("  paused: {}", thread.paused);
    println!();

    // Execution tracking
    println!("--- Execution Tracking ---");
    println!("  exec_count: {}", thread.exec_count);
    println!("  last_executor: {}", thread.last_executor);
    println!("  last_error_time: {:?}", thread.last_error_time);
    println!();

    // Nonce
    println!("--- Nonce ---");
    println!("  nonce_account: {}", thread.nonce_account);
    println!("  last_nonce: {}", thread.last_nonce);
}

// =============================================================================
// Admin commands (only available with `dev` feature)
// =============================================================================

/// Admin: force delete a thread (skips all checks)
#[cfg(feature = "dev")]
pub async fn admin_delete(
    address: String,
    rpc_url: Option<String>,
    keypair_path: Option<std::path::PathBuf>,
) -> Result<()> {
    use anchor_lang::{InstructionData, ToAccountMetas};
    use solana_sdk::{instruction::Instruction, message::Message, signer::Signer, transaction::Transaction};

    let thread_pubkey =
        Pubkey::from_str(&address).map_err(|e| anyhow!("Invalid pubkey '{}': {}", address, e))?;

    let rpc_url = get_rpc_url(rpc_url)?;
    let admin = get_keypair(keypair_path)?;

    println!("Admin delete thread: {}", thread_pubkey);
    println!("Admin: {}", admin.pubkey());
    println!("RPC: {}", rpc_url);

    let client = RpcPool::with_url(&rpc_url)
        .map_err(|e| anyhow!("Failed to create RPC client: {}", e))?;

    // Get config PDA
    let (config_pubkey, _) = Pubkey::find_program_address(
        &[antegen_thread_program::SEED_CONFIG],
        &antegen_thread_program::ID,
    );

    // Build ThreadDelete instruction
    let accounts = antegen_thread_program::accounts::ThreadDelete {
        admin: admin.pubkey(),
        config: config_pubkey,
        thread: thread_pubkey,
    };

    let data = antegen_thread_program::instruction::DeleteThread {}.data();

    let ix = Instruction {
        program_id: antegen_thread_program::ID,
        accounts: accounts.to_account_metas(None),
        data,
    };

    // Build and send transaction
    let (blockhash, _) = client
        .get_latest_blockhash()
        .await
        .map_err(|e| anyhow!("Failed to get blockhash: {}", e))?;

    let message = Message::new(&[ix], Some(&admin.pubkey()));
    let tx = Transaction::new(&[&admin], message, blockhash);

    let sig = client
        .send_and_confirm_transaction(&tx)
        .await
        .map_err(|e| anyhow!("Failed to delete thread: {}", e))?;

    println!("\n✓ Thread deleted successfully!");
    println!("Signature: {}", sig);

    Ok(())
}

// =============================================================================
// Test commands (only available with `dev` feature)
// =============================================================================

#[cfg(feature = "dev")]
mod test_commands {
    use super::*;
    use anchor_lang::{InstructionData, ToAccountMetas};
    use antegen_thread_program::state::{SerializableInstruction, Signal, Trigger};
    use chrono::Utc;
    use serde::{Deserialize, Serialize};
    use solana_sdk::{
        instruction::Instruction, message::Message, native_token::LAMPORTS_PER_SOL, signer::Signer,
        transaction::Transaction,
    };
    use std::collections::HashMap;

    /// Registry for tracking managed test threads
    #[derive(Serialize, Deserialize, Default)]
    struct TestThreadRegistry {
        next_id: u32,
        threads: HashMap<String, TestThreadEntry>,
    }

    /// Entry for a single managed test thread
    #[derive(Serialize, Deserialize, Clone)]
    struct TestThreadEntry {
        pubkey: String,
        trigger: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signal: Option<String>,
        #[serde(default = "default_fiber_count")]
        fibers: u8,
        created_at: String,
    }

    fn default_fiber_count() -> u8 {
        1
    }

    impl TestThreadRegistry {
        /// Load registry from disk, or return default if not exists
        fn load() -> Result<Self> {
            let path = get_registry_path()?;
            if path.exists() {
                let content = std::fs::read_to_string(&path)
                    .map_err(|e| anyhow!("Failed to read registry: {}", e))?;
                serde_yaml::from_str(&content)
                    .map_err(|e| anyhow!("Failed to parse registry: {}", e))
            } else {
                Ok(Self::default())
            }
        }

        /// Save registry to disk
        fn save(&self) -> Result<()> {
            let path = get_registry_path()?;
            let content = serde_yaml::to_string(self)
                .map_err(|e| anyhow!("Failed to serialize registry: {}", e))?;
            std::fs::write(&path, content)
                .map_err(|e| anyhow!("Failed to write registry: {}", e))?;
            Ok(())
        }

        /// Generate next auto-incremented thread ID
        fn next_thread_id(&mut self) -> String {
            self.next_id += 1;
            format!("test-{}", self.next_id)
        }

        /// Add a thread to the registry
        fn add_thread(&mut self, id: String, entry: TestThreadEntry) {
            self.threads.insert(id, entry);
        }

        /// Remove a thread from the registry
        fn remove_thread(&mut self, id: &str) -> Option<TestThreadEntry> {
            self.threads.remove(id)
        }

        /// Get a thread by ID
        fn get_thread(&self, id: &str) -> Option<&TestThreadEntry> {
            self.threads.get(id)
        }

        /// Reset next_id counter when registry is empty
        fn reset_if_empty(&mut self) {
            if self.threads.is_empty() {
                self.next_id = 0;
            }
        }
    }

    /// Get path to registry file
    fn get_registry_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| anyhow!("Could not find config directory"))?
            .join("antegen");
        std::fs::create_dir_all(&config_dir)
            .map_err(|e| anyhow!("Failed to create config dir: {}", e))?;
        Ok(config_dir.join("test-threads.yaml"))
    }

    /// Get or create the CLI test keypair
    /// Stored at ~/.config/antegen/test-keypair.json
    /// This keypair is used as authority for all CLI test threads,
    /// allowing create/delete operations without using the main Solana keypair.
    fn get_or_create_test_keypair() -> Result<Keypair> {
        use solana_sdk::signature::Keypair;

        let config_dir = dirs::config_dir()
            .ok_or_else(|| anyhow!("Could not find config directory"))?
            .join("antegen");

        let keypair_path = config_dir.join("test-keypair.json");

        if keypair_path.exists() {
            read_keypair_file(&keypair_path)
                .map_err(|e| anyhow!("Failed to read test keypair from {:?}: {}", keypair_path, e))
        } else {
            // Create config directory if needed
            std::fs::create_dir_all(&config_dir)
                .map_err(|e| anyhow!("Failed to create config dir {:?}: {}", config_dir, e))?;

            // Generate new keypair
            let keypair = Keypair::new();
            let keypair_bytes: Vec<u8> = keypair.to_bytes().to_vec();
            let json = serde_json::to_string(&keypair_bytes)
                .map_err(|e| anyhow!("Failed to serialize keypair: {}", e))?;

            std::fs::write(&keypair_path, &json).map_err(|e| {
                anyhow!("Failed to write test keypair to {:?}: {}", keypair_path, e)
            })?;

            println!("Created new test keypair at {:?}", keypair_path);
            Ok(keypair)
        }
    }

    /// Parse trigger string into Trigger enum
    fn parse_trigger(trigger_str: &str) -> Result<Trigger> {
        match trigger_str {
            "immediate" => Ok(Trigger::Immediate { jitter: 0 }),
            s if s.starts_with("cron:") => Ok(Trigger::Cron {
                schedule: s.trim_start_matches("cron:").to_string(),
                skippable: true,
                jitter: 0,
            }),
            s if s.starts_with("interval:") => {
                let seconds: i64 = s
                    .trim_start_matches("interval:")
                    .parse()
                    .map_err(|_| anyhow!("Invalid interval seconds"))?;
                Ok(Trigger::Interval {
                    seconds,
                    skippable: true,
                    jitter: 0,
                })
            }
            s if s.starts_with("timestamp:") => {
                let unix_ts: i64 = s
                    .trim_start_matches("timestamp:")
                    .parse()
                    .map_err(|_| anyhow!("Invalid timestamp"))?;
                Ok(Trigger::Timestamp { unix_ts, jitter: 0 })
            }
            s if s.starts_with("slot:") => {
                let slot: u64 = s
                    .trim_start_matches("slot:")
                    .parse()
                    .map_err(|_| anyhow!("Invalid slot number"))?;
                Ok(Trigger::Slot { slot })
            }
            s if s.starts_with("epoch:") => {
                let epoch: u64 = s
                    .trim_start_matches("epoch:")
                    .parse()
                    .map_err(|_| anyhow!("Invalid epoch number"))?;
                Ok(Trigger::Epoch { epoch })
            }
            s if s.starts_with("account:") => {
                let address = s.trim_start_matches("account:");
                let pubkey = Pubkey::from_str(address)
                    .map_err(|e| anyhow!("Invalid account pubkey: {}", e))?;
                Ok(Trigger::Account {
                    address: pubkey,
                    offset: 0,
                    size: 100, // Watch first 100 bytes by default
                })
            }
            _ => Err(anyhow!(
                "Unknown trigger: {}. Options: immediate, cron:<schedule>, interval:<secs>, \
                 timestamp:<unix>, slot:<num>, epoch:<num>, account:<pubkey>",
                trigger_str
            )),
        }
    }

    /// Parse signal string into Signal enum (for single fiber signals like fiber add)
    fn parse_single_fiber_signal(s: Option<&str>) -> Result<Option<Signal>> {
        match s {
            None => Ok(None),
            Some("none") => Ok(Some(Signal::None)),
            Some("repeat") => Ok(Some(Signal::Repeat)),
            Some("close") => Ok(Some(Signal::Close)),
            Some("chain") => Ok(Some(Signal::Chain)),
            Some(sig) if sig.starts_with("next:") => {
                let idx: u8 = sig
                    .trim_start_matches("next:")
                    .parse()
                    .map_err(|_| anyhow!("Invalid next index"))?;
                Ok(Some(Signal::Next { index: idx }))
            }
            Some(sig) => Err(anyhow!(
                "Unknown signal: {}. Options: chain, next:T, repeat, close",
                sig
            )),
        }
    }

    /// Parsed signal configuration for fibers
    struct FiberSignalConfig {
        per_fiber_signals: HashMap<u8, Signal>,
    }

    impl FiberSignalConfig {
        /// Calculate implicit fiber count from signals
        /// Returns max(all fiber indices referenced) + 1, or 1 if empty
        fn implied_fiber_count(&self) -> u8 {
            if self.per_fiber_signals.is_empty() {
                return 1;
            }

            let mut max_idx: u8 = 0;
            for (&fiber_idx, signal) in &self.per_fiber_signals {
                max_idx = max_idx.max(fiber_idx);
                // Also check target indices in Next signals (Chain always targets next fiber)
                if let Signal::Next { index } = signal {
                    max_idx = max_idx.max(*index);
                }
            }
            max_idx + 1
        }
    }

    /// Parse repeated --signal flags into FiberSignalConfig
    /// Formats: "repeat", "close" (fiber 0), or "F:chain", "F:next:T" (specific fiber)
    fn parse_fiber_signals(signals: &[String]) -> Result<FiberSignalConfig> {
        let mut config = FiberSignalConfig {
            per_fiber_signals: HashMap::new(),
        };

        for signal_str in signals {
            let (fiber_idx, signal) = parse_signal_with_fiber(signal_str)?;

            // Check for collision
            if config.per_fiber_signals.contains_key(&fiber_idx) {
                eprintln!("Warning: Fiber {} signal overwritten", fiber_idx);
            }
            config.per_fiber_signals.insert(fiber_idx, signal);
        }

        Ok(config)
    }

    /// Parse a single signal string that may include fiber index
    /// Formats: "repeat", "close", "none" (fiber 0), "F:chain", "F:repeat", "F:close", "F:none", or "F:next:T"
    fn parse_signal_with_fiber(s: &str) -> Result<(u8, Signal)> {
        match s {
            // Simple signals → apply to fiber 0
            "repeat" => Ok((0, Signal::Repeat)),
            "close" => Ok((0, Signal::Close)),
            "none" => Ok((0, Signal::None)),

            // Per-fiber formats: "F:signal" or "F:next:T"
            _ => {
                let parts: Vec<&str> = s.split(':').collect();

                // "F:signal" format (2 parts) - chain, repeat, close, none
                if parts.len() == 2 {
                    let fiber_idx: u8 = parts[0]
                        .parse()
                        .map_err(|_| anyhow!("Invalid fiber index: {}", parts[0]))?;
                    let signal = match parts[1] {
                        "chain" => Signal::Chain,
                        "repeat" => Signal::Repeat,
                        "close" => Signal::Close,
                        "none" => Signal::None,
                        _ => {
                            return Err(anyhow!(
                                "Unknown signal type: '{}'. Use chain, repeat, close, or none",
                                parts[1]
                            ))
                        }
                    };
                    return Ok((fiber_idx, signal));
                }

                // "F:next:T" format (3 parts)
                if parts.len() == 3 && parts[1] == "next" {
                    let fiber_idx: u8 = parts[0]
                        .parse()
                        .map_err(|_| anyhow!("Invalid fiber index: {}", parts[0]))?;
                    let target: u8 = parts[2]
                        .parse()
                        .map_err(|_| anyhow!("Invalid target index: {}", parts[2]))?;
                    return Ok((fiber_idx, Signal::Next { index: target }));
                }

                Err(anyhow!(
                    "Invalid signal format: '{}'. Expected F:signal or F:next:T",
                    s
                ))
            }
        }
    }

    /// Thread IDs for advanced tests
    const TEST_THREAD_RECURRING_ID: &str = "antegen-test-recurring";
    const TEST_THREAD_WATCHER_ID: &str = "antegen-test-watcher";
    const TEST_THREAD_CHAIN_ID: &str = "antegen-test-chain";

    /// Derive a thread PDA
    fn derive_thread_pda(authority: Pubkey, thread_id: &str) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[
                antegen_thread_program::SEED_THREAD,
                authority.as_ref(),
                thread_id.as_bytes(),
            ],
            &antegen_thread_program::ID,
        )
    }

    /// Build a thread_memo instruction with optional signal
    /// The thread signs this instruction via invoke_signed in thread_exec
    /// Note: The thread account appears both in ThreadExec and in remaining_accounts,
    /// but ThreadExec uses the `dup` constraint to allow this.
    fn build_thread_memo_instruction(
        thread_pubkey: Pubkey,
        memo: String,
        signal: Option<Signal>,
    ) -> Instruction {
        let accounts = antegen_thread_program::accounts::ThreadMemo {
            signer: thread_pubkey,
        }
        .to_account_metas(None);

        let data = antegen_thread_program::instruction::ThreadMemo { memo, signal }.data();

        Instruction {
            program_id: antegen_thread_program::ID,
            accounts,
            data,
        }
    }

    /// Build a fiber_create instruction (does not send)
    fn build_fiber_create_instruction(
        payer: &Keypair,
        authority: &Keypair,
        thread_pubkey: Pubkey,
        fiber_index: u8,
        signal: Option<Signal>,
    ) -> Instruction {
        // Derive fiber PDA
        let (fiber_pubkey, _) = Pubkey::find_program_address(
            &[
                antegen_thread_program::SEED_THREAD_FIBER,
                thread_pubkey.as_ref(),
                &[fiber_index],
            ],
            &antegen_thread_program::ID,
        );

        // Build thread_memo instruction for this fiber
        let memo_ix = build_thread_memo_instruction(
            thread_pubkey,
            format!("Fiber {} executed", fiber_index),
            signal,
        );
        let serializable_ix: SerializableInstruction = memo_ix.into();

        // Build fiber_create instruction
        let accounts = antegen_thread_program::accounts::FiberCreate {
            authority: authority.pubkey(),
            payer: payer.pubkey(),
            thread: thread_pubkey,
            fiber: fiber_pubkey,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(Some(false));

        let data = antegen_thread_program::instruction::CreateFiber {
            fiber_index,
            instruction: serializable_ix,
            signer_seeds: vec![],
            priority_fee: 0,
        }
        .data();

        Instruction {
            program_id: antegen_thread_program::ID,
            accounts,
            data,
        }
    }

    /// Create an additional fiber on an existing thread (separate transaction)
    async fn create_additional_fiber(
        client: &RpcPool,
        payer: &Keypair,
        authority: &Keypair,
        thread_pubkey: Pubkey,
        fiber_index: u8,
        signal: Option<Signal>,
    ) -> Result<()> {
        let ix = build_fiber_create_instruction(payer, authority, thread_pubkey, fiber_index, signal);

        // Send transaction
        let (blockhash, _) = client.get_latest_blockhash().await?;
        let message = Message::new(&[ix], Some(&payer.pubkey()));
        let tx = Transaction::new(&[payer, authority], message, blockhash);

        let sig = client
            .send_and_confirm_transaction(&tx)
            .await
            .map_err(|e| anyhow!("Failed to create fiber {}: {}", fiber_index, e))?;

        println!("  Fiber {} created: {}", fiber_index, sig);

        Ok(())
    }

    /// Create, list, or delete test threads
    pub async fn test(
        rpc: Option<String>,
        keypair_path: Option<PathBuf>,
        cmd: crate::TestCommands,
    ) -> Result<()> {
        use crate::TestCommands;

        // Handle list (doesn't require RPC)
        if matches!(cmd, TestCommands::List) {
            return list_test_threads();
        }

        // Get RPC and keypair (used as payer)
        let rpc_url = get_rpc_url(rpc)?;
        let payer = get_keypair(keypair_path)?;

        println!("RPC: {}", rpc_url);
        println!("Payer: {}", payer.pubkey());

        // Get or create the CLI test keypair for thread authority
        let test_authority = get_or_create_test_keypair()?;
        let authority = test_authority.pubkey();
        println!("Thread authority (test keypair): {}", authority);

        let client = RpcPool::with_url(&rpc_url)
            .map_err(|e| anyhow!("Failed to create RPC client: {}", e))?;

        match cmd {
            TestCommands::Create {
                trigger: trigger_str,
                signal: signals,
                fibers: fibers_override,
                test_type,
            } => {
                // Handle advanced test types
                if let Some(ref tt) = test_type {
                    match tt.as_str() {
                        "account" => {
                            return create_account_trigger_test(&client, &payer, &test_authority)
                                .await;
                        }
                        "chain" => {
                            return create_chain_test(&client, &payer, &test_authority).await;
                        }
                        _ => {
                            return Err(anyhow!(
                                "Unknown test type: {}. Options: account, chain",
                                tt
                            ));
                        }
                    }
                }

                // Load registry and generate auto ID
                let mut registry = TestThreadRegistry::load()?;
                let thread_id = registry.next_thread_id();

                // Parse signals into FiberSignalConfig
                let signal_config = parse_fiber_signals(&signals)?;

                // Determine fiber count: explicit override > implicit from signals > 1
                let fiber_count =
                    fibers_override.unwrap_or_else(|| signal_config.implied_fiber_count());

                // Calculate thread PDA
                let (thread_pubkey, _bump) = derive_thread_pda(authority, &thread_id);

                // Create the thread with fibers
                create_test_thread(
                    &client,
                    &payer,
                    &test_authority,
                    &thread_id,
                    &trigger_str,
                    &signal_config,
                    fiber_count,
                )
                .await?;

                // Format signal string for registry display
                let signal_str = if signals.is_empty() {
                    None
                } else {
                    Some(signals.join(", "))
                };

                // Save to registry
                registry.add_thread(
                    thread_id.clone(),
                    TestThreadEntry {
                        pubkey: thread_pubkey.to_string(),
                        trigger: trigger_str.clone(),
                        signal: signal_str,
                        fibers: fiber_count,
                        created_at: Utc::now().to_rfc3339(),
                    },
                );
                registry.save()?;

                println!("\nCreated test thread '{}' at {}", thread_id, thread_pubkey);
                println!("Fibers: {}", fiber_count);
                println!("To list: antegen thread test list");
                println!("To delete: antegen thread test delete --id {}", thread_id);
            }
            TestCommands::Delete { id, all, test_type } => {
                // Handle advanced test types deletion
                if let Some(ref tt) = test_type {
                    match tt.as_str() {
                        "account" => {
                            return delete_account_trigger_test(&client, &payer, &test_authority)
                                .await;
                        }
                        "chain" => {
                            return delete_chain_test(&client, &payer, &test_authority).await;
                        }
                        _ => {
                            return Err(anyhow!(
                                "Unknown test type: {}. Options: account, chain",
                                tt
                            ));
                        }
                    }
                }

                if all {
                    // Delete all threads in registry
                    return delete_all_registered_threads(&client, &payer, &test_authority).await;
                } else if let Some(thread_id) = id {
                    // Delete specific thread by ID
                    return delete_thread_by_id(&client, &payer, &test_authority, &thread_id).await;
                } else {
                    // No ID specified - show list and error
                    println!("Available test threads:");
                    list_test_threads()?;
                    return Err(anyhow!(
                        "Specify --id <name> to delete a specific thread, or --all to delete all"
                    ));
                }
            }
            TestCommands::List => {
                // Already handled above, but keep for exhaustiveness
                return list_test_threads();
            }
            TestCommands::Fiber(fiber_cmd) => {
                use crate::TestFiberCommands;
                match fiber_cmd {
                    TestFiberCommands::Add { id, signal } => {
                        return test_fiber_add(
                            &client,
                            &payer,
                            &test_authority,
                            &id,
                            signal.as_deref(),
                        )
                        .await;
                    }
                    TestFiberCommands::List { id } => {
                        return test_fiber_list(&client, &id).await;
                    }
                    TestFiberCommands::Delete { id, index } => {
                        return test_fiber_delete(&client, &payer, &test_authority, &id, index)
                            .await;
                    }
                }
            }
        }

        Ok(())
    }

    /// List all managed test threads from registry
    fn list_test_threads() -> Result<()> {
        let registry = TestThreadRegistry::load()?;

        if registry.threads.is_empty() {
            println!("No managed test threads found.");
            println!("\nCreate one with: antegen thread test create --trigger <type>");
            return Ok(());
        }

        println!("Managed test threads:");
        println!();

        // Sort by ID for consistent display
        let mut entries: Vec<_> = registry.threads.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));

        for (id, entry) in entries {
            let signal_str = entry.signal.as_deref().unwrap_or("-");
            println!(
                "  {}: {} (trigger: {}, fibers: {}, signal: {})",
                id, entry.pubkey, entry.trigger, entry.fibers, signal_str
            );
        }

        println!();
        println!("Delete with: antegen thread test delete --id <name>");
        println!("Delete all:  antegen thread test delete --all");
        println!("Add fiber:   antegen thread test fiber add <id> --signal <signal>");

        Ok(())
    }

    /// Delete a specific thread by ID from registry
    async fn delete_thread_by_id(
        client: &RpcPool,
        payer: &Keypair,
        authority: &Keypair,
        thread_id: &str,
    ) -> Result<()> {
        let mut registry = TestThreadRegistry::load()?;

        // Check if thread exists in registry
        let entry = registry
            .get_thread(thread_id)
            .ok_or_else(|| anyhow!("Thread '{}' not found in registry", thread_id))?
            .clone();

        let thread_pubkey = Pubkey::from_str(&entry.pubkey)
            .map_err(|e| anyhow!("Invalid pubkey in registry: {}", e))?;

        println!("Deleting thread '{}' ({})...", thread_id, thread_pubkey);

        // Delete from chain
        delete_test_thread(client, payer, authority, thread_pubkey).await?;

        // Remove from registry
        registry.remove_thread(thread_id);
        registry.reset_if_empty();
        registry.save()?;

        println!("Thread '{}' deleted and removed from registry.", thread_id);

        Ok(())
    }

    /// Delete all threads in the registry
    async fn delete_all_registered_threads(
        client: &RpcPool,
        payer: &Keypair,
        authority: &Keypair,
    ) -> Result<()> {
        let mut registry = TestThreadRegistry::load()?;

        if registry.threads.is_empty() {
            println!("No test threads to delete.");
            return Ok(());
        }

        println!("Deleting {} test threads...\n", registry.threads.len());

        let thread_ids: Vec<String> = registry.threads.keys().cloned().collect();

        for thread_id in thread_ids {
            if let Some(entry) = registry.get_thread(&thread_id) {
                match Pubkey::from_str(&entry.pubkey) {
                    Ok(thread_pubkey) => {
                        print!("  {}: ", thread_id);
                        match delete_test_thread(client, payer, authority, thread_pubkey).await {
                            Ok(_) => println!("deleted"),
                            Err(e) => println!("failed ({})", e),
                        }
                    }
                    Err(e) => {
                        println!("  {}: invalid pubkey ({})", thread_id, e);
                    }
                }
            }
            registry.remove_thread(&thread_id);
        }

        registry.reset_if_empty();
        registry.save()?;
        println!("\nAll test threads deleted.");

        Ok(())
    }

    /// Create a test thread with thread_memo as default fiber and optional additional fibers
    /// All instructions are bundled into a single transaction
    async fn create_test_thread(
        client: &RpcPool,
        payer: &Keypair,
        authority: &Keypair,
        thread_id: &str,
        trigger_str: &str,
        signal_config: &FiberSignalConfig,
        fiber_count: u8,
    ) -> Result<()> {
        // Derive thread PDA
        let (thread_pubkey, _) = derive_thread_pda(authority.pubkey(), thread_id);

        println!("\nCreating test thread '{}'...", thread_id);
        println!("Thread PDA: {}", thread_pubkey);
        println!("Fibers: {}", fiber_count);

        // Parse trigger
        let trigger = parse_trigger(trigger_str)?;
        println!("Trigger: {:?}", trigger);

        // Get signal for default fiber (index 0) if specified
        let default_signal = signal_config.per_fiber_signals.get(&0).cloned();
        if let Some(ref sig) = default_signal {
            println!("Default fiber signal: {:?}", sig);
        }

        // Build thread_memo instruction as default fiber
        let memo_instruction = build_thread_memo_instruction(
            thread_pubkey,
            format!("Test thread '{}' fiber 0 executed", thread_id),
            default_signal,
        );
        let serializable_ix: SerializableInstruction = memo_instruction.into();

        // Build ThreadCreate accounts
        let accounts = antegen_thread_program::accounts::ThreadCreate {
            authority: authority.pubkey(),
            payer: payer.pubkey(),
            thread: thread_pubkey,
            nonce_account: None,
            recent_blockhashes: None,
            rent: None,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(Some(false));

        // Build instruction data - use custom thread_id
        let data = antegen_thread_program::instruction::CreateThread {
            amount: LAMPORTS_PER_SOL / 10, // 0.1 SOL
            id: thread_id.into(),
            trigger,
            initial_instruction: Some(serializable_ix),
            priority_fee: Some(0),
        }
        .data();

        let thread_create_ix = Instruction {
            program_id: antegen_thread_program::ID,
            accounts,
            data,
        };

        // Build all instructions (thread_create + fiber_creates)
        let mut instructions = vec![thread_create_ix];

        // Add fiber_create instructions for additional fibers
        for i in 1..fiber_count {
            let fiber_signal = signal_config.per_fiber_signals.get(&i).cloned();
            let fiber_ix = build_fiber_create_instruction(payer, authority, thread_pubkey, i, fiber_signal);
            instructions.push(fiber_ix);
        }

        // Send all instructions in a single transaction
        let (blockhash, _) = client.get_latest_blockhash().await?;
        let message = Message::new(&instructions, Some(&payer.pubkey()));
        let tx = Transaction::new(&[payer, authority], message, blockhash);

        println!("Sending transaction with {} instructions...", instructions.len());
        let sig = client
            .send_and_confirm_transaction(&tx)
            .await
            .map_err(|e| anyhow!("Failed to send transaction: {}", e))?;

        println!("Thread and {} fibers created: {}", fiber_count, sig);

        Ok(())
    }

    /// Delete the test thread
    async fn delete_test_thread(
        client: &RpcPool,
        payer: &Keypair,
        authority: &Keypair,
        thread_pubkey: Pubkey,
    ) -> Result<()> {
        use solana_sdk::instruction::AccountMeta;

        println!("\nDeleting test thread...");

        // Fetch thread to get fiber_ids
        let account = client
            .get_account(&thread_pubkey)
            .await
            .map_err(|e| anyhow!("Failed to fetch thread: {}", e))?
            .ok_or_else(|| anyhow!("Thread account not found"))?;
        let data = account.decode_data()
            .map_err(|e| anyhow!("Failed to decode account data: {}", e))?;
        let thread = Thread::try_deserialize(&mut data.as_slice())
            .map_err(|e| anyhow!("Failed to deserialize thread: {:?}", e))?;

        // Build ThreadClose accounts
        // Close_to receives the lamports - send to payer
        let mut accounts = antegen_thread_program::accounts::ThreadClose {
            authority: authority.pubkey(),
            close_to: payer.pubkey(),
            thread: thread_pubkey,
        }
        .to_account_metas(Some(false));

        // Add fiber accounts to remaining_accounts so they can be closed
        // Skip fiber 0 if it's stored inline (default_fiber exists)
        for fiber_id in &thread.fiber_ids {
            // Fiber 0 with default_fiber is stored inline, not as separate account
            if *fiber_id == 0 && thread.default_fiber.is_some() {
                continue;
            }
            let (fiber_pubkey, _) = Pubkey::find_program_address(
                &[
                    antegen_thread_program::SEED_THREAD_FIBER,
                    thread_pubkey.as_ref(),
                    &[*fiber_id],
                ],
                &antegen_thread_program::ID,
            );
            accounts.push(AccountMeta::new(fiber_pubkey, false));
        }

        // Build instruction data
        let data = antegen_thread_program::instruction::CloseThread {}.data();

        let ix = Instruction {
            program_id: antegen_thread_program::ID,
            accounts,
            data,
        };

        // Send transaction - authority must sign, payer pays
        let (blockhash, _) = client.get_latest_blockhash().await?;
        let message = Message::new(&[ix], Some(&payer.pubkey()));
        let tx = Transaction::new(&[payer, authority], message, blockhash);

        println!("Sending transaction...");
        let sig = client
            .send_and_confirm_transaction(&tx)
            .await
            .map_err(|e| anyhow!("Failed to send transaction: {}", e))?;

        println!("Transaction confirmed: {}", sig);
        println!("\nTest thread deleted successfully!");

        Ok(())
    }

    /// Create account trigger test - two threads demonstrating account-based triggering
    async fn create_account_trigger_test(
        client: &RpcPool,
        payer: &Keypair,
        authority: &Keypair,
    ) -> Result<()> {
        println!("\nCreating account trigger test (two threads)...");

        // Thread A - recurring interval trigger
        let (thread_a_pubkey, _) = derive_thread_pda(authority.pubkey(), TEST_THREAD_RECURRING_ID);
        println!("Thread A (recurring): {}", thread_a_pubkey);

        // Thread B - account trigger watching Thread A
        let (thread_b_pubkey, _) = derive_thread_pda(authority.pubkey(), TEST_THREAD_WATCHER_ID);
        println!("Thread B (watcher): {}", thread_b_pubkey);

        // Create Thread A with interval trigger
        println!("\nCreating Thread A (interval:30)...");
        let memo_a =
            build_thread_memo_instruction(thread_a_pubkey, "Recurring update".to_string(), None);
        let serializable_a: SerializableInstruction = memo_a.into();

        let accounts_a = antegen_thread_program::accounts::ThreadCreate {
            authority: authority.pubkey(),
            payer: payer.pubkey(),
            thread: thread_a_pubkey,
            nonce_account: None,
            recent_blockhashes: None,
            rent: None,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(Some(false));

        let data_a = antegen_thread_program::instruction::CreateThread {
            amount: LAMPORTS_PER_SOL / 10,
            id: TEST_THREAD_RECURRING_ID.into(),
            trigger: Trigger::Interval {
                seconds: 30,
                skippable: true,
                jitter: 0,
            },
            initial_instruction: Some(serializable_a),
            priority_fee: Some(0),
        }
        .data();

        let ix_a = Instruction {
            program_id: antegen_thread_program::ID,
            accounts: accounts_a,
            data: data_a,
        };

        let (blockhash, _) = client.get_latest_blockhash().await?;
        let message = Message::new(&[ix_a], Some(&payer.pubkey()));
        let tx = Transaction::new(&[payer, authority], message, blockhash);

        let sig = client
            .send_and_confirm_transaction(&tx)
            .await
            .map_err(|e| anyhow!("Failed to create Thread A: {}", e))?;
        println!("Thread A created: {}", sig);

        // Create Thread B with account trigger watching Thread A
        println!("\nCreating Thread B (account trigger watching Thread A)...");
        let memo_b = build_thread_memo_instruction(
            thread_b_pubkey,
            "Detected change in Thread A!".to_string(),
            None,
        );
        let serializable_b: SerializableInstruction = memo_b.into();

        let accounts_b = antegen_thread_program::accounts::ThreadCreate {
            authority: authority.pubkey(),
            payer: payer.pubkey(),
            thread: thread_b_pubkey,
            nonce_account: None,
            recent_blockhashes: None,
            rent: None,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(Some(false));

        let data_b = antegen_thread_program::instruction::CreateThread {
            amount: LAMPORTS_PER_SOL / 10,
            id: TEST_THREAD_WATCHER_ID.into(),
            trigger: Trigger::Account {
                address: thread_a_pubkey,
                offset: 0,
                size: 100,
            },
            initial_instruction: Some(serializable_b),
            priority_fee: Some(0),
        }
        .data();

        let ix_b = Instruction {
            program_id: antegen_thread_program::ID,
            accounts: accounts_b,
            data: data_b,
        };

        let (blockhash, _) = client.get_latest_blockhash().await?;
        let message = Message::new(&[ix_b], Some(&payer.pubkey()));
        let tx = Transaction::new(&[payer, authority], message, blockhash);

        let sig = client
            .send_and_confirm_transaction(&tx)
            .await
            .map_err(|e| anyhow!("Failed to create Thread B: {}", e))?;
        println!("Thread B created: {}", sig);

        println!("\n=== Account Trigger Test Created ===");
        println!("Thread A (recurring): {}", thread_a_pubkey);
        println!("Thread B (watcher): {}", thread_b_pubkey);
        println!("\nHow it works:");
        println!("1. Thread A executes every 30 seconds");
        println!("2. When Thread A executes, its account data changes");
        println!("3. Thread B detects the change and triggers");
        println!("\nTo delete: antegen thread test delete --test-type account");

        Ok(())
    }

    /// Create multi-fiber chain test - demonstrates Signal::Chain bundling
    /// Creates 3 fibers that chain together: 0 → 1 → 2 in a single transaction
    async fn create_chain_test(
        client: &RpcPool,
        payer: &Keypair,
        authority: &Keypair,
    ) -> Result<()> {
        println!("\nCreating chain signal test (3 fibers)...");

        let (thread_pubkey, _) = derive_thread_pda(authority.pubkey(), TEST_THREAD_CHAIN_ID);
        println!("Thread: {}", thread_pubkey);

        // Create thread with default fiber (index 0) that chains to next fiber
        println!("\nCreating thread with fiber 0 (chains to next fiber)...");
        let memo = build_thread_memo_instruction(
            thread_pubkey,
            "Fiber 0 executed - chaining to next fiber".to_string(),
            Some(Signal::Chain),
        );
        let serializable: SerializableInstruction = memo.into();

        let accounts = antegen_thread_program::accounts::ThreadCreate {
            authority: authority.pubkey(),
            payer: payer.pubkey(),
            thread: thread_pubkey,
            nonce_account: None,
            recent_blockhashes: None,
            rent: None,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(Some(false));

        let data = antegen_thread_program::instruction::CreateThread {
            amount: LAMPORTS_PER_SOL / 10,
            id: TEST_THREAD_CHAIN_ID.into(),
            trigger: Trigger::Interval {
                seconds: 30,
                skippable: true,
                jitter: 0,
            },
            initial_instruction: Some(serializable),
            priority_fee: Some(0),
        }
        .data();

        let ix = Instruction {
            program_id: antegen_thread_program::ID,
            accounts,
            data,
        };

        let (blockhash, _) = client.get_latest_blockhash().await?;
        let message = Message::new(&[ix], Some(&payer.pubkey()));
        let tx = Transaction::new(&[payer, authority], message, blockhash);

        let sig = client
            .send_and_confirm_transaction(&tx)
            .await
            .map_err(|e| anyhow!("Failed to create thread: {}", e))?;
        println!("Thread created: {}", sig);

        // Create fiber 1 that chains to next fiber
        println!("\nCreating fiber 1 (chains to next fiber)...");
        create_additional_fiber(
            client,
            payer,
            authority,
            thread_pubkey,
            1,
            Some(Signal::Chain),
        )
        .await?;

        // Create fiber 2 (end of chain)
        println!("\nCreating fiber 2 (end of chain)...");
        create_additional_fiber(client, payer, authority, thread_pubkey, 2, None).await?;

        println!("\n=== Chain Signal Test Created ===");
        println!("Thread: {}", thread_pubkey);
        println!("\nFiber configuration:");
        println!("  Fiber 0: Signal::Chain → chains to next fiber (fiber 1)");
        println!("  Fiber 1: Signal::Chain → chains to next fiber (fiber 2)");
        println!("  Fiber 2: Signal::None → end of chain");
        println!("\nBehavior:");
        println!("  - Thread triggers every 30 seconds");
        println!("  - One trigger → all 3 fibers execute in single transaction");
        println!("  - Execution order: Fiber 0 → Fiber 1 → Fiber 2");
        println!("\nTo delete: antegen thread test delete --test-type chain");

        Ok(())
    }

    /// Delete account trigger test threads (recurring and watcher)
    async fn delete_account_trigger_test(
        client: &RpcPool,
        payer: &Keypair,
        authority: &Keypair,
    ) -> Result<()> {
        println!("\nDeleting account trigger test threads...");

        let test_thread_ids = [TEST_THREAD_RECURRING_ID, TEST_THREAD_WATCHER_ID];

        for thread_id in test_thread_ids {
            let (thread_pubkey, _) = derive_thread_pda(authority.pubkey(), thread_id);

            match client.get_account(&thread_pubkey).await {
                Ok(_) => {
                    println!("Deleting thread {} ({})...", thread_id, thread_pubkey);
                    match delete_test_thread(client, payer, authority, thread_pubkey).await {
                        Ok(_) => println!("  Deleted successfully"),
                        Err(e) => println!("  Failed to delete: {}", e),
                    }
                }
                Err(_) => {
                    println!("Thread {} does not exist, skipping", thread_id);
                }
            }
        }

        println!("\nAccount trigger test threads deleted!");
        Ok(())
    }

    /// Delete chain test thread
    async fn delete_chain_test(
        client: &RpcPool,
        payer: &Keypair,
        authority: &Keypair,
    ) -> Result<()> {
        println!("\nDeleting chain test thread...");

        let (thread_pubkey, _) = derive_thread_pda(authority.pubkey(), TEST_THREAD_CHAIN_ID);

        match client.get_account(&thread_pubkey).await {
            Ok(_) => {
                println!(
                    "Deleting thread {} ({})...",
                    TEST_THREAD_CHAIN_ID, thread_pubkey
                );
                delete_test_thread(client, payer, authority, thread_pubkey).await?;
                println!("Chain test thread deleted!");
            }
            Err(_) => {
                println!("Chain test thread does not exist.");
            }
        }

        Ok(())
    }

    /// Add a fiber to a test thread by registry ID
    async fn test_fiber_add(
        client: &RpcPool,
        payer: &Keypair,
        authority: &Keypair,
        thread_id: &str,
        signal_str: Option<&str>,
    ) -> Result<()> {
        // Look up thread from registry
        let registry = TestThreadRegistry::load()?;
        let entry = registry
            .get_thread(thread_id)
            .ok_or_else(|| anyhow!("Thread '{}' not found in registry", thread_id))?;
        let thread_pubkey = Pubkey::from_str(&entry.pubkey)
            .map_err(|e| anyhow!("Invalid pubkey in registry: {}", e))?;

        println!("Adding fiber to thread '{}' ({})...", thread_id, thread_pubkey);

        // Fetch thread to get fiber_next_id
        let account = client
            .get_account(&thread_pubkey)
            .await
            .map_err(|e| anyhow!("Failed to fetch thread: {}", e))?
            .ok_or_else(|| anyhow!("Thread account not found"))?;
        let data = account.decode_data()
            .map_err(|e| anyhow!("Failed to decode account data: {}", e))?;
        let thread = Thread::try_deserialize(&mut data.as_slice())
            .map_err(|e| anyhow!("Failed to deserialize thread: {:?}", e))?;
        let fiber_index = thread.fiber_next_id;

        println!("Next fiber index: {}", fiber_index);

        // Parse signal (simplified - no fiber index prefix)
        let signal = parse_single_fiber_signal(signal_str)?;
        if let Some(ref sig) = signal {
            println!("Signal: {:?}", sig);
        }

        // Create fiber
        create_additional_fiber(client, payer, authority, thread_pubkey, fiber_index, signal)
            .await?;

        println!(
            "\nAdded fiber {} to thread '{}' ({})",
            fiber_index, thread_id, thread_pubkey
        );

        Ok(())
    }

    /// List fibers on a test thread
    async fn test_fiber_list(client: &RpcPool, thread_id: &str) -> Result<()> {
        // Look up thread from registry
        let registry = TestThreadRegistry::load()?;
        let entry = registry
            .get_thread(thread_id)
            .ok_or_else(|| anyhow!("Thread '{}' not found in registry", thread_id))?;
        let thread_pubkey = Pubkey::from_str(&entry.pubkey)
            .map_err(|e| anyhow!("Invalid pubkey in registry: {}", e))?;

        // Fetch thread to get fiber info
        let account = client
            .get_account(&thread_pubkey)
            .await
            .map_err(|e| anyhow!("Failed to fetch thread: {}", e))?
            .ok_or_else(|| anyhow!("Thread account not found"))?;
        let data = account.decode_data()
            .map_err(|e| anyhow!("Failed to decode account data: {}", e))?;
        let thread = Thread::try_deserialize(&mut data.as_slice())
            .map_err(|e| anyhow!("Failed to deserialize thread: {:?}", e))?;

        println!("Fibers for thread '{}' ({}):", thread_id, thread_pubkey);
        println!();
        println!("  fiber_ids: {:?}", thread.fiber_ids);
        println!("  fiber_cursor: {}", thread.fiber_cursor);
        println!("  fiber_next_id: {}", thread.fiber_next_id);
        println!("  fiber_signal: {:?}", thread.fiber_signal);
        println!();
        println!(
            "  default_fiber: {}",
            if thread.default_fiber.is_some() {
                "present"
            } else {
                "none"
            }
        );

        // Try to fetch individual fiber accounts
        println!("\nFiber accounts:");
        for &fiber_id in &thread.fiber_ids {
            let (fiber_pubkey, _) = Pubkey::find_program_address(
                &[
                    antegen_thread_program::SEED_THREAD_FIBER,
                    thread_pubkey.as_ref(),
                    &[fiber_id],
                ],
                &antegen_thread_program::ID,
            );

            match client.get_account(&fiber_pubkey).await {
                Ok(Some(fiber_account)) => {
                    let data_len = fiber_account.decode_data()
                        .map(|d| d.len())
                        .unwrap_or(0);
                    println!(
                        "  Fiber {}: {} ({} bytes)",
                        fiber_id,
                        fiber_pubkey,
                        data_len
                    );
                }
                Ok(None) | Err(_) => {
                    println!("  Fiber {}: {} (not found)", fiber_id, fiber_pubkey);
                }
            }
        }

        Ok(())
    }

    /// Delete a fiber from a test thread
    async fn test_fiber_delete(
        client: &RpcPool,
        payer: &Keypair,
        authority: &Keypair,
        thread_id: &str,
        fiber_index: u8,
    ) -> Result<()> {
        // Look up thread from registry
        let registry = TestThreadRegistry::load()?;
        let entry = registry
            .get_thread(thread_id)
            .ok_or_else(|| anyhow!("Thread '{}' not found in registry", thread_id))?;
        let thread_pubkey = Pubkey::from_str(&entry.pubkey)
            .map_err(|e| anyhow!("Invalid pubkey in registry: {}", e))?;

        println!(
            "Deleting fiber {} from thread '{}' ({})...",
            fiber_index, thread_id, thread_pubkey
        );

        // Derive fiber PDA
        let (fiber_pubkey, _) = Pubkey::find_program_address(
            &[
                antegen_thread_program::SEED_THREAD_FIBER,
                thread_pubkey.as_ref(),
                &[fiber_index],
            ],
            &antegen_thread_program::ID,
        );

        // Build fiber_close instruction
        let accounts = antegen_thread_program::accounts::FiberClose {
            authority: authority.pubkey(),
            close_to: payer.pubkey(),
            thread: thread_pubkey,
            fiber: Some(fiber_pubkey),
        }
        .to_account_metas(Some(false));

        let data = antegen_thread_program::instruction::CloseFiber { fiber_index }.data();

        let ix = Instruction {
            program_id: antegen_thread_program::ID,
            accounts,
            data,
        };

        // Send transaction
        let (blockhash, _) = client.get_latest_blockhash().await?;
        let message = Message::new(&[ix], Some(&payer.pubkey()));
        let tx = Transaction::new(&[payer, authority], message, blockhash);

        let sig = client
            .send_and_confirm_transaction(&tx)
            .await
            .map_err(|e| anyhow!("Failed to delete fiber: {}", e))?;

        println!("Fiber {} deleted: {}", fiber_index, sig);

        Ok(())
    }
}

// Re-export the test function when dev feature is enabled
#[cfg(feature = "dev")]
pub use test_commands::test;
