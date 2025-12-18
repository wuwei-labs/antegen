//! Executor Logic
//!
//! Handles building and simulating thread execution transactions.
//! Includes batching detection via Signal and compute budget estimation.
//!
//! Batching is determined by the fiber's Signal return value:
//! - Signal::Chain → batch another exec for next fiber in sequence
//! - Signal::Close → batch a delete instruction
//! - Other signals → no batching needed

use crate::resources::SharedResources;
use crate::rpc::response::decode_account_data;
use anchor_lang::{AccountDeserialize, AnchorDeserialize, InstructionData, ToAccountMetas};
use antegen_thread_program::{
    accounts::ThreadExec,
    instruction::ExecThread,
    state::{CompiledInstructionV0, FiberState, Signal, Thread, ThreadConfig, PAYER_PUBKEY},
};
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_sdk::{
    account::Account,
    instruction::{AccountMeta, Instruction},
    message::Message,
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    sysvar,
    transaction::Transaction,
};

use anyhow::{anyhow, Result};
use log::{debug, info, warn};
use std::sync::Arc;

/// Executor logic for building thread execution transactions
#[derive(Clone)]
pub struct ExecutorLogic {
    /// Executor keypair for signing transactions
    keypair: Arc<Keypair>,
    /// Shared resources (RPC pool, cache)
    resources: SharedResources,
    /// Whether to forgo executor commission
    forgo_executor_commission: bool,
}

impl ExecutorLogic {
    /// Create a new executor logic instance
    pub fn new(
        keypair: Arc<Keypair>,
        resources: SharedResources,
        forgo_executor_commission: bool,
    ) -> Self {
        Self {
            keypair,
            resources,
            forgo_executor_commission,
        }
    }

    /// Get executor pubkey
    pub fn pubkey(&self) -> Pubkey {
        self.keypair.pubkey()
    }

    /// Get the keypair reference
    pub fn keypair(&self) -> &Arc<Keypair> {
        &self.keypair
    }

    /// Build a transaction to execute a thread with automatic batching
    ///
    /// Simulates to detect chaining signals and estimate CU consumption.
    /// Batching is determined by the Signal returned from the fiber:
    /// - Signal::Chain → batch another exec for next fiber in sequence
    /// - Signal::Close → batch a delete instruction
    ///
    /// Returns (instructions, priority_fee)
    pub async fn build_execute_transaction(
        &self,
        thread_pubkey: &Pubkey,
        thread: &Thread,
    ) -> Result<(Vec<Instruction>, u64)> {
        // Log thread state for debugging
        self.log_thread_debug(thread, thread_pubkey);

        const MAX_BATCHED_EXECS: usize = 5;
        let mut priority_fee: u64 = 0;
        let mut ixs: Vec<Instruction> = Vec::new();

        // Track fiber_cursor through the chaining loop
        // Signal::Chain tells us to execute next fiber in sequence
        let mut current_fiber_cursor = thread.fiber_cursor;

        // Build and add first instruction
        debug!(
            "Building first thread_exec instruction for fiber_cursor={}",
            current_fiber_cursor
        );
        self.build_thread_exec(
            &mut priority_fee,
            &mut ixs,
            thread_pubkey,
            thread,
            current_fiber_cursor,
        )
        .await?;
        debug!(
            "First instruction built successfully, priority_fee: {}",
            priority_fee
        );

        loop {
            if ixs.len() >= MAX_BATCHED_EXECS {
                warn!(
                    "Reached max batched executions ({}), stopping",
                    MAX_BATCHED_EXECS
                );
                break;
            }

            // Simulate to check for batching signals
            debug!(
                "Simulating transaction with {} instruction(s) to check for batching...",
                ixs.len()
            );
            let (signal, units) = self.simulate_transaction(&ixs, thread_pubkey).await?;
            debug!(
                "Simulation result: signal={:?}, units_consumed={}",
                signal, units
            );

            // Handle signal - only Chain and Close trigger batching
            match signal {
                Signal::Chain => {
                    // Calculate next fiber in sequence
                    current_fiber_cursor =
                        Self::next_fiber_in_sequence(&thread.fiber_ids, current_fiber_cursor);
                    info!(
                        "Batching: Signal::Chain, adding thread_exec for fiber {}",
                        current_fiber_cursor
                    );
                    self.build_thread_exec(
                        &mut priority_fee,
                        &mut ixs,
                        thread_pubkey,
                        thread,
                        current_fiber_cursor,
                    )
                    .await?;
                }
                Signal::Close => {
                    // Build thread_exec that executes the pre-compiled close_fiber
                    // The close_fiber CPIs to thread_delete with thread signing as authority
                    info!("Signal::Close detected - building thread_exec with close_fiber");
                    let close_ix = self
                        .build_close_thread_exec(thread_pubkey, thread)
                        .await?;
                    ixs.push(close_ix);
                    break;
                }
                _ => {
                    // No batching needed for None, Repeat, Next, UpdateTrigger
                    debug!("No batching needed for signal: {:?}", signal);
                    break;
                }
            }
        }

        // Final simulation to get accurate compute units
        debug!("Final simulation to get accurate compute units...");
        let (_, units_consumed) = self.simulate_transaction(&ixs, thread_pubkey).await?;
        debug!("Final simulation: units_consumed={}", units_consumed);

        // Add compute budget instruction at the beginning with measured CU
        // Add 10% buffer for safety
        let compute_units = ((units_consumed as f64) * 1.1) as u32;
        ixs.insert(
            0,
            ComputeBudgetInstruction::set_compute_unit_limit(compute_units),
        );

        // Add priority fee if specified
        if priority_fee > 0 {
            ixs.insert(
                1,
                ComputeBudgetInstruction::set_compute_unit_price(priority_fee),
            );
        }

        Ok((ixs, priority_fee))
    }

    /// Log thread state for debugging
    fn log_thread_debug(&self, thread: &Thread, thread_pubkey: &Pubkey) {
        debug!("Building execute transaction for thread: {}", thread_pubkey);
        debug!("  fiber_cursor: {}", thread.fiber_cursor);
        debug!("  fiber_ids: {:?}", thread.fiber_ids);
        debug!("  has_default_fiber: {}", thread.default_fiber.is_some());
        debug!("  fiber_signal: {:?}", thread.fiber_signal);
        debug!("  trigger: {:?}", thread.trigger);
        debug!("  schedule: {:?}", thread.schedule);
        debug!("  paused: {}", thread.paused);
        debug!("  exec_count: {}", thread.exec_count);
    }

    /// Calculate the next fiber index in sequence
    /// Used for Signal::Chain which always chains to the next consecutive fiber
    fn next_fiber_in_sequence(fiber_ids: &[u8], current_cursor: u8) -> u8 {
        if fiber_ids.is_empty() {
            return 0;
        }
        if let Some(current_pos) = fiber_ids.iter().position(|&x| x == current_cursor) {
            let next_pos = (current_pos + 1) % fiber_ids.len();
            fiber_ids[next_pos]
        } else {
            fiber_ids.first().copied().unwrap_or(0)
        }
    }

    /// Build thread_exec instruction for a specific fiber
    ///
    /// Determines which fiber to execute based on fiber_cursor:
    /// - If fiber_cursor == 0 and default_fiber exists: use inline default fiber
    /// - Otherwise: fetch external fiber account
    async fn build_thread_exec(
        &self,
        priority_fee: &mut u64,
        ixs: &mut Vec<Instruction>,
        thread_pubkey: &Pubkey,
        thread: &Thread,
        fiber_cursor: u8,
    ) -> Result<()> {
        // Check if using default fiber (fiber_cursor 0 with default_fiber)
        let is_default_fiber = fiber_cursor == 0 && thread.default_fiber.is_some();

        debug!(
            "build_thread_exec: fiber_cursor={}, is_default_fiber={}",
            fiber_cursor, is_default_fiber
        );

        // Get priority fee and fiber state
        let (fiber_priority_fee, fiber_state_opt) = if is_default_fiber {
            // Default fiber: priority fee from thread, no fiber account needed
            debug!(
                "Using default fiber (inline), priority_fee={}",
                thread.default_fiber_priority_fee
            );
            (thread.default_fiber_priority_fee, None)
        } else {
            // Account-based fiber: fetch fiber state
            let fiber_pubkey = thread.fiber_at_index(thread_pubkey, fiber_cursor);

            debug!(
                "Fetching external fiber account: {} (fiber_cursor={})",
                fiber_pubkey, fiber_cursor
            );

            let account = self.fetch_account(&fiber_pubkey).await?;
            let fiber_state = FiberState::try_deserialize(&mut account.data.as_slice())
                .map_err(|e| anyhow!("Failed to deserialize fiber {}: {}", fiber_pubkey, e))?;

            debug!(
                "External fiber fetched, priority_fee={}",
                fiber_state.priority_fee
            );
            (fiber_state.priority_fee, Some(fiber_state))
        };

        // Build execute instruction
        let ix = self
            .build_execute_instruction(
                thread_pubkey,
                thread,
                fiber_cursor,
                fiber_state_opt.as_ref(),
            )
            .await?;

        *priority_fee = (*priority_fee).max(fiber_priority_fee);
        ixs.push(ix);

        Ok(())
    }

    /// Build base ThreadExec accounts (shared by build_execute_instruction and build_close_thread_exec)
    async fn build_thread_exec_base_accounts(
        &self,
        thread_pubkey: &Pubkey,
        thread: &Thread,
        fiber_pubkey: Option<Pubkey>,
    ) -> Result<(Vec<AccountMeta>, ThreadConfig)> {
        let config_pubkey = ThreadConfig::pubkey();
        let config = self.fetch_thread_config(&config_pubkey).await?;
        let has_nonce = thread.has_nonce_account();

        debug!(
            "Building ThreadExec accounts: executor={}, thread={}, fiber={:?}, has_nonce={}",
            self.keypair.pubkey(),
            thread_pubkey,
            fiber_pubkey,
            has_nonce
        );

        let accounts = ThreadExec {
            executor: self.keypair.pubkey(),
            thread: *thread_pubkey,
            fiber: fiber_pubkey,
            config: config_pubkey,
            admin: config.admin,
            nonce_account: if has_nonce {
                Some(thread.nonce_account)
            } else {
                None
            },
            recent_blockhashes: if has_nonce {
                Some(sysvar::recent_blockhashes::ID)
            } else {
                None
            },
            system_program: solana_system_interface::program::ID,
        }
        .to_account_metas(Some(false));

        Ok((accounts, config))
    }

    /// Add compiled instruction accounts to the account list
    fn add_compiled_accounts(&self, accounts: &mut Vec<AccountMeta>, compiled: &CompiledInstructionV0) {
        debug!(
            "Adding remaining accounts: {} accounts from compiled.accounts",
            compiled.accounts.len()
        );

        for (account_index, pubkey) in compiled.accounts.iter().enumerate() {
            // Replace PAYER_PUBKEY with executor
            let actual_pubkey = if pubkey.eq(&PAYER_PUBKEY) {
                self.keypair.pubkey()
            } else {
                *pubkey
            };

            // Determine writability based on position in sorted accounts
            let account_idx = account_index as u8;
            let is_writable = if account_idx < compiled.num_rw_signers {
                true // Read-write signer
            } else if account_idx < compiled.num_rw_signers + compiled.num_ro_signers {
                false // Read-only signer
            } else if account_idx
                < compiled.num_rw_signers + compiled.num_ro_signers + compiled.num_rw
            {
                true // Read-write non-signer
            } else {
                false // Read-only non-signer
            };

            debug!(
                "  remaining[{}]: {} (is_writable={})",
                account_index, actual_pubkey, is_writable
            );
            accounts.push(AccountMeta {
                pubkey: actual_pubkey,
                is_signer: false, // CPI accounts don't need to be signers at transaction level
                is_writable,
            });
        }
    }

    /// Build exec_thread instruction
    async fn build_execute_instruction(
        &self,
        thread_pubkey: &Pubkey,
        thread: &Thread,
        fiber_cursor: u8,
        fiber: Option<&FiberState>,
    ) -> Result<Instruction> {
        debug!(
            "Building exec_thread instruction: thread={}, fiber_cursor={}, has_fiber_arg={}",
            thread_pubkey,
            fiber_cursor,
            fiber.is_some()
        );

        // Get compiled instruction from either inline fiber or fiber account
        let (compiled, fiber_pubkey_opt) = if let Some(fiber_state) = fiber {
            // Account-based fiber
            let fiber_pubkey = thread.fiber_at_index(thread_pubkey, fiber_cursor);
            let compiled = CompiledInstructionV0::deserialize(
                &mut fiber_state.compiled_instruction.as_slice(),
            )?;
            debug!(
                "Using account-based fiber: pubkey={}, compiled_accounts={}",
                fiber_pubkey,
                compiled.accounts.len()
            );
            (compiled, Some(fiber_pubkey))
        } else {
            // Default fiber
            let default_fiber = thread
                .default_fiber
                .as_ref()
                .ok_or_else(|| anyhow!("Thread has no default fiber"))?;
            let compiled = CompiledInstructionV0::deserialize(&mut default_fiber.as_slice())?;
            debug!(
                "Using default fiber (inline), compiled_accounts={}",
                compiled.accounts.len()
            );
            (compiled, None)
        };

        // Build base accounts
        let (mut accounts, _config) = self
            .build_thread_exec_base_accounts(thread_pubkey, thread, fiber_pubkey_opt)
            .await?;

        // Add compiled instruction accounts as remaining accounts
        self.add_compiled_accounts(&mut accounts, &compiled);

        // Build instruction data using Anchor-generated type
        let data = ExecThread {
            forgo_commission: self.forgo_executor_commission,
            fiber_cursor,
        }
        .data();

        debug!(
            "Instruction built: program={}, total_accounts={}, data_len={}",
            antegen_thread_program::ID,
            accounts.len(),
            data.len()
        );

        Ok(Instruction {
            program_id: antegen_thread_program::ID,
            accounts,
            data,
        })
    }

    /// Build thread_exec instruction that executes close_fiber to delete the thread
    ///
    /// When Signal::Close is detected, we build a thread_exec that:
    /// 1. Executes the pre-compiled close_fiber (which CPIs to thread_delete)
    /// 2. Passes all external fiber accounts as remaining_accounts for cleanup
    async fn build_close_thread_exec(
        &self,
        thread_pubkey: &Pubkey,
        thread: &Thread,
    ) -> Result<Instruction> {
        debug!(
            "Building close thread_exec: thread={}, fiber_ids={:?}",
            thread_pubkey, thread.fiber_ids
        );

        // Build base accounts - no fiber account needed since we use close_fiber (inline)
        let (mut accounts, _config) = self
            .build_thread_exec_base_accounts(thread_pubkey, thread, None)
            .await?;

        // Add external fiber accounts as remaining_accounts for thread_delete to close
        // Skip fiber index 0 if it's the inline default fiber
        for &fiber_index in &thread.fiber_ids {
            if fiber_index == 0 && thread.default_fiber.is_some() {
                continue; // Skip inline fiber - thread_delete handles it via thread account
            }
            let fiber_pda = thread.fiber_at_index(thread_pubkey, fiber_index);
            debug!("Adding fiber account for deletion: {} (index={})", fiber_pda, fiber_index);
            accounts.push(AccountMeta {
                pubkey: fiber_pda,
                is_signer: false,
                is_writable: true, // Needs to be writable to close
            });
        }

        // Add accounts needed for close_fiber CPI to thread_delete
        // The close_fiber is a compiled thread_delete instruction that needs these accounts
        // in remaining_accounts for invoke_signed to find them:

        // 1. Thread account (needed for CPI even though it's in ThreadExec accounts)
        accounts.push(AccountMeta {
            pubkey: *thread_pubkey,
            is_signer: false,
            is_writable: true,
        });

        // 2. close_to (thread.authority - receives rent)
        accounts.push(AccountMeta {
            pubkey: thread.authority,
            is_signer: false,
            is_writable: true,
        });

        // 3. Thread program ID (for CPI)
        accounts.push(AccountMeta {
            pubkey: antegen_thread_program::ID,
            is_signer: false,
            is_writable: false,
        });

        // Build instruction data - fiber_cursor doesn't matter since Signal::Close is set
        let data = ExecThread {
            forgo_commission: self.forgo_executor_commission,
            fiber_cursor: 0,
        }
        .data();

        debug!(
            "Close thread_exec built: {} accounts, {} external fibers",
            accounts.len(),
            thread.fiber_ids.iter().filter(|&&i| !(i == 0 && thread.default_fiber.is_some())).count()
        );

        Ok(Instruction {
            program_id: antegen_thread_program::ID,
            accounts,
            data,
        })
    }

    /// Simulate transaction and extract Signal and compute units consumed
    ///
    /// Uses the RPC pool for failover and health tracking.
    ///
    /// Returns (signal, units_consumed)
    /// - signal: The fiber_signal from post-simulation thread state (determines batching)
    /// - units_consumed: compute units used by the transaction
    async fn simulate_transaction(
        &self,
        instructions: &[Instruction],
        thread_pubkey: &Pubkey,
    ) -> Result<(Signal, u64)> {
        debug!(
            "Simulating transaction: thread={}, num_instructions={}",
            thread_pubkey,
            instructions.len(),
        );

        // Log instruction details
        for (i, ix) in instructions.iter().enumerate() {
            debug!(
                "  Instruction {}: program={}, num_accounts={}, data_len={}",
                i,
                ix.program_id,
                ix.accounts.len(),
                ix.data.len()
            );
            if i == 0 {
                for (j, acc) in ix.accounts.iter().enumerate() {
                    debug!(
                        "    account[{}]: {} (signer={}, writable={})",
                        j, acc.pubkey, acc.is_signer, acc.is_writable
                    );
                }
            }
        }

        // 1. Get blockhash from RPC pool
        let (blockhash, _) = self
            .resources
            .rpc_client
            .get_latest_blockhash()
            .await
            .map_err(|e| anyhow!("Failed to get blockhash for simulation: {}", e))?;
        debug!("Got blockhash for simulation: {}", blockhash);

        // 2. Build transaction
        let message = Message::new(instructions, Some(&self.keypair.pubkey()));
        let tx = Transaction::new(&[self.keypair.as_ref()], message, blockhash);

        // 3. Simulate via RPC pool (handles failover, returns result with accounts)
        let result = self
            .resources
            .rpc_client
            .simulate_transaction(&tx, &[*thread_pubkey])
            .await?;

        // Log simulation logs
        if let Some(logs) = &result.value.logs {
            debug!("Simulation logs ({} lines):", logs.len());
            for (i, log) in logs.iter().enumerate() {
                if i < 20 {
                    debug!("  [{}] {}", i, log);
                } else if i == 20 {
                    debug!("  ... ({} more log lines)", logs.len() - 20);
                    break;
                }
            }
        }

        // 4. Extract units_consumed (safely handles float)
        let units_consumed = result.value.units_consumed.unwrap_or(0);
        debug!("Simulation units_consumed: {}", units_consumed);

        // 5. Extract signal from thread account
        let signal = if let Some(accounts) = &result.value.accounts {
            if let Some(Some(ui_account)) = accounts.first() {
                // Decode account data (supports base64 and base64+zstd)
                match decode_account_data(&ui_account.data.0, &ui_account.data.1) {
                    Ok(data) => {
                        if data.len() < 8 {
                            debug!("Thread account has insufficient data (likely closed)");
                            Signal::None
                        } else {
                            match Thread::try_deserialize(&mut data.as_slice()) {
                                Ok(thread) => {
                                    debug!(
                                        "Extracted signal from simulation: {:?}",
                                        thread.fiber_signal
                                    );
                                    thread.fiber_signal
                                }
                                Err(e) => {
                                    warn!("Failed to deserialize thread from simulation: {}", e);
                                    Signal::None
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to decode account data: {}", e);
                        Signal::None
                    }
                }
            } else {
                debug!("No account data in simulation response (account is null)");
                Signal::None
            }
        } else {
            debug!("No accounts in simulation response");
            Signal::None
        };

        Ok((signal, units_consumed))
    }

    /// Fetch an account from RPC (with cache check)
    async fn fetch_account(&self, pubkey: &Pubkey) -> Result<Account> {
        // Check cache first
        if let Some(cached) = self.resources.cache.get(pubkey).await {
            return Ok(Account {
                lamports: 0, // Not stored in cache
                data: cached.data,
                owner: Pubkey::default(), // Not stored in cache
                executable: false,
                rent_epoch: 0,
            });
        }

        // Fetch from RPC using custom client
        let ui_account = self
            .resources
            .rpc_client
            .get_account(pubkey)
            .await
            .map_err(|e| anyhow!("Failed to fetch account {}: {}", pubkey, e))?
            .ok_or_else(|| anyhow!("Account {} not found", pubkey))?;

        // Decode account data (supports base64 and base64+zstd)
        let account_data = decode_account_data(&ui_account.data.0, &ui_account.data.1)
            .map_err(|e| anyhow!("Failed to decode account data: {}", e))?;

        // Cache the result (unknown trigger type for generic account fetch)
        self.resources
            .cache
            .put_simple(*pubkey, account_data.clone(), 0)
            .await;

        Ok(solana_sdk::account::Account {
            lamports: ui_account.lamports,
            data: account_data,
            owner: ui_account.owner.parse().unwrap_or_default(),
            executable: ui_account.executable,
            rent_epoch: ui_account.rent_epoch,
        })
    }

    /// Fetch thread config with caching
    async fn fetch_thread_config(&self, config_pubkey: &Pubkey) -> Result<ThreadConfig> {
        // Try cache first
        if let Some(cached) = self.resources.cache.get(config_pubkey).await {
            if let Ok(config) = ThreadConfig::try_deserialize(&mut cached.data.as_slice()) {
                return Ok(config);
            }
        }

        // Fetch from RPC using custom client
        let ui_account = self
            .resources
            .rpc_client
            .get_account(config_pubkey)
            .await
            .map_err(|e| anyhow!("Failed to fetch thread config {}: {}", config_pubkey, e))?
            .ok_or_else(|| anyhow!("Thread config {} not found", config_pubkey))?;

        // Decode account data (supports base64 and base64+zstd)
        let account_data = decode_account_data(&ui_account.data.0, &ui_account.data.1)
            .map_err(|e| anyhow!("Failed to decode account data: {}", e))?;

        let config = ThreadConfig::try_deserialize(&mut account_data.as_slice())
            .map_err(|e| anyhow!("Failed to deserialize thread config: {}", e))?;

        // Cache it (unknown trigger type for config accounts)
        self.resources
            .cache
            .put_simple(*config_pubkey, account_data, 0)
            .await;

        Ok(config)
    }

    /// Build thread error instruction for reporting execution failures
    pub async fn build_thread_error_instruction(
        &self,
        thread_pubkey: &Pubkey,
        error_code: u32,
        error_message: String,
    ) -> Result<Vec<Instruction>> {
        // Get config pubkey
        let config_pubkey = ThreadConfig::pubkey();

        // Fetch config to get admin
        let config = self.fetch_thread_config(&config_pubkey).await?;

        // Build the error instruction
        let accounts = antegen_thread_program::accounts::ThreadError {
            executor: self.keypair.pubkey(),
            thread: *thread_pubkey,
            config: config_pubkey,
            admin: config.admin,
            system_program: solana_system_interface::program::ID,
        };

        let data = antegen_thread_program::instruction::ErrorThread {
            error_code,
            error_message,
        }
        .data();

        Ok(vec![Instruction {
            program_id: antegen_thread_program::ID,
            accounts: accounts.to_account_metas(Some(true)),
            data,
        }])
    }
}

#[cfg(test)]
mod tests {
    // Integration tests would require RPC connection
    // Unit tests for the module structure
    #[test]
    fn test_executor_logic_creation() {
        // Just verify the struct can be created
        // Full tests require RPC mocking
    }
}
