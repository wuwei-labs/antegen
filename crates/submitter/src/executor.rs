use anyhow::{Result, anyhow};
use log::{debug, info, warn};
use solana_client::{
    nonblocking::rpc_client::RpcClient,
    rpc_config::{RpcSimulateTransactionAccountsConfig, RpcSimulateTransactionConfig},
    rpc_custom_error::JSON_RPC_SERVER_ERROR_MIN_CONTEXT_SLOT_NOT_REACHED,
};
use solana_account_decoder::UiAccountEncoding;
use solana_program::{sysvar, pubkey::Pubkey};
use solana_sdk::{
    commitment_config::CommitmentConfig,
    compute_budget::ComputeBudgetInstruction,
    instruction::Instruction,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use std::{sync::Arc, cmp, time::Duration};

use antegen_thread_program::state::{Thread, ThreadConfig, FiberState, Trigger, TriggerContext};
use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use antegen_sdk::accounts::ThreadExec;
use antegen_sdk::instruction::ExecThread;

use crate::types::ExecutableThread;

/// Clock state from the cluster
#[derive(Clone, Debug, Default)]
pub struct ClockState {
    pub slot: u64,
    pub epoch: u64,
    pub unix_timestamp: i64,
}

/// Executor logic integrated into submitter
#[derive(Clone)]
pub struct ExecutorLogic {
    /// Executor keypair for signing transactions
    keypair: Arc<Keypair>,
    /// RPC client for queries
    rpc_client: Arc<RpcClient>,
    /// Current clock state
    clock: ClockState,
    /// Whether to forgo executor commission
    forgo_executor_commission: bool,
}

impl ExecutorLogic {
    pub fn new(
        keypair: Arc<Keypair>,
        rpc_client: Arc<RpcClient>,
        forgo_executor_commission: bool,
    ) -> Self {
        Self {
            keypair,
            rpc_client,
            clock: ClockState::default(),
            forgo_executor_commission,
        }
    }

    /// Update clock state
    pub fn update_clock(&mut self, slot: u64, epoch: u64, unix_timestamp: i64) {
        self.clock = ClockState {
            slot,
            epoch,
            unix_timestamp,
        };
        debug!(
            "EXECUTOR: Clock updated - slot: {}, epoch: {}, timestamp: {}",
            slot, epoch, unix_timestamp
        );
    }

    /// Get fiber state with retry logic for race conditions
    /// Will retry indefinitely for AccountNotFound errors (account will eventually exist)
    /// Uses exponential backoff capped at 5 seconds between retries
    async fn get_fiber_state_with_retry(&self, fiber_pubkey: &Pubkey) -> Result<FiberState> {
        let mut attempt = 0;
        let mut delay = Duration::from_millis(500);
        let max_delay = Duration::from_secs(5);
        let start_time = std::time::Instant::now();
        let mut last_log = std::time::Instant::now();
        
        loop {
            attempt += 1;
            
            match self.rpc_client.get_account(fiber_pubkey).await {
                Ok(account) => {
                    if attempt > 1 {
                        info!("Successfully fetched fiber account {} on attempt {} (waited ~{:.1}s total)",
                            fiber_pubkey, attempt, 
                            start_time.elapsed().as_secs_f32());
                    }
                    return FiberState::try_deserialize(&mut account.data.as_slice())
                        .map_err(|e| anyhow!("Failed to deserialize fiber state: {}", e));
                }
                Err(e) => {
                    let error_str = e.to_string();
                    
                    // Check if it's an account not found error (expected during race conditions)
                    if error_str.contains("AccountNotFound") || error_str.contains("could not find account") {
                        if attempt == 1 {
                            debug!("Fiber account {} not found, will retry until it exists...", fiber_pubkey);
                        } else if last_log.elapsed() > Duration::from_secs(30) {
                            // Log progress every 30 seconds
                            info!("Still waiting for fiber account {} to exist (attempt {}, elapsed: {:.0}s)...",
                                fiber_pubkey, attempt, start_time.elapsed().as_secs());
                            last_log = std::time::Instant::now();
                        }
                    } else {
                        // For non-AccountNotFound errors, we might want to fail after some attempts
                        // but for now, keep retrying with logging
                        if attempt <= 5 {
                            debug!("Error fetching fiber account {} (attempt {}): {}, retrying in {:?}...",
                                fiber_pubkey, attempt, e, delay);
                        } else if last_log.elapsed() > Duration::from_secs(30) {
                            // After 5 attempts, only log every 30 seconds to avoid spam
                            info!("Still having errors fetching fiber account {} (attempt {}): {}",
                                fiber_pubkey, attempt, e);
                            last_log = std::time::Instant::now();
                        }
                    }
                    
                    tokio::time::sleep(delay).await;
                    // Exponential backoff with cap at max_delay
                    delay = cmp::min(delay * 2, max_delay);
                }
            }
        }
    }
    
    /// Check if a thread's trigger is ready to execute
    pub fn is_trigger_ready(&self, thread: &Thread) -> bool {
        match &thread.trigger_context {
            TriggerContext::Timestamp { next, .. } => {
                let current_time = self.clock.unix_timestamp;
                current_time >= *next
            }
            TriggerContext::Block { next, .. } => {
                // Block context is used for both Slot and Epoch triggers
                // Check the actual trigger type to determine which to compare
                match &thread.trigger {
                    Trigger::Slot { .. } => self.clock.slot >= *next,
                    Trigger::Epoch { .. } => self.clock.epoch >= *next,
                    _ => false,
                }
            }
            TriggerContext::Account { .. } => {
                // Account triggers are handled by observer
                false
            }
        }
    }

    /// Build a transaction to execute a thread
    pub async fn build_execute_transaction(
        &self,
        executable: &ExecutableThread,
    ) -> Result<Transaction> {
        let thread_pubkey = &executable.thread_pubkey;
        let thread = &executable.thread;

        // Get fiber PDA
        let fiber_pubkey = Pubkey::find_program_address(
            &[
                b"thread_fiber",
                thread_pubkey.as_ref(),
                &[thread.exec_index],
            ],
            &antegen_thread_program::ID,
        ).0;

        // Get fiber state with retry logic for race conditions
        let fiber = self.get_fiber_state_with_retry(&fiber_pubkey).await?;

        // Build execute instruction
        let execute_ix = self.build_execute_instruction(thread_pubkey, thread, &fiber).await?;

        // Add compute budget
        let ixs = vec![
            ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
            execute_ix,
        ];

        // Build transaction
        let recent_blockhash = self.rpc_client.get_latest_blockhash().await?;
        Ok(Transaction::new_signed_with_payer(
            &ixs,
            Some(&self.keypair.pubkey()),
            &[&*self.keypair],
            recent_blockhash,
        ))
    }

    /// Build exec_thread instruction
    async fn build_execute_instruction(
        &self,
        thread_pubkey: &Pubkey,
        thread: &Thread,
        _fiber: &FiberState,
    ) -> Result<Instruction> {
        // Get config for fee recipients
        let config_pubkey =
            Pubkey::find_program_address(&[b"thread_config"], &antegen_thread_program::ID).0;
        
        // Fetch config with simple retry (config should be stable, but handle edge cases)
        let config_account = match self.rpc_client.get_account(&config_pubkey).await {
            Ok(account) => account,
            Err(e) => {
                debug!("Failed to fetch config account on first attempt: {}, retrying...", e);
                tokio::time::sleep(Duration::from_millis(500)).await;
                self.rpc_client.get_account(&config_pubkey).await
                    .map_err(|e| anyhow!("Failed to fetch thread config account: {}", e))?
            }
        };
        let config = ThreadConfig::try_deserialize(&mut config_account.data.as_slice())?;

        // Get fiber PDA
        let fiber_pubkey = Pubkey::find_program_address(
            &[
                b"thread_fiber",
                thread_pubkey.as_ref(),
                &[thread.exec_index],
            ],
            &antegen_thread_program::ID,
        ).0;

        // Build accounts using Anchor-generated types
        let accounts = ThreadExec {
            executor: self.keypair.pubkey(),
            fee_payer: self.keypair.pubkey(),
            thread: *thread_pubkey,
            fiber: fiber_pubkey,
            config: config_pubkey,
            thread_authority: thread.authority,
            config_admin: config.admin,
            nonce_account: if thread.has_nonce_account() {
                Some(thread.nonce_account)
            } else {
                None
            },
            recent_blockhashes: if thread.has_nonce_account() {
                Some(sysvar::recent_blockhashes::ID)
            } else {
                None
            },
            system_program: solana_program::system_program::id(),
        }.to_account_metas(Some(false));

        // Build instruction data using Anchor-generated type
        let data = ExecThread {
            forgo_commission: self.forgo_executor_commission,
        }.data();

        // Create the instruction
        Ok(Instruction {
            program_id: antegen_thread_program::ID,
            accounts,
            data,
        })
    }

    /// Get executor pubkey
    pub fn pubkey(&self) -> Pubkey {
        self.keypair.pubkey()
    }

    /// Get current slot
    pub fn current_slot(&self) -> u64 {
        self.clock.slot
    }

    /// Simulate transaction and optimize compute units
    pub async fn simulate_and_optimize_transaction(
        &self,
        executable: &ExecutableThread,
        cu_multiplier: f64,
        max_compute_units: u32,
    ) -> Result<Transaction> {
        // Use the slot from the executable (when it was triggered)
        self.simulate_and_optimize_transaction_with_slot(
            executable,
            cu_multiplier,
            max_compute_units,
            executable.slot,
        ).await
    }
    
    /// Simulate transaction with specific trigger slot for min_context_slot
    pub async fn simulate_and_optimize_transaction_with_slot(
        &self,
        executable: &ExecutableThread,
        cu_multiplier: f64,
        max_compute_units: u32,
        trigger_slot: u64,
    ) -> Result<Transaction> {
        let thread_pubkey = executable.thread_pubkey;
        
        // Build initial transaction with default compute units
        let initial_tx = self.build_execute_transaction(executable).await?;
        
        debug!("Simulating transaction for thread {} with trigger slot {}", thread_pubkey, trigger_slot);
        
        // Simulate transaction with proper config
        let sim_result = match self.rpc_client.simulate_transaction_with_config(
            &initial_tx,
            RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: true,
                commitment: Some(CommitmentConfig::processed()),
                accounts: Some(RpcSimulateTransactionAccountsConfig {
                    encoding: Some(UiAccountEncoding::Base64Zstd),
                    addresses: vec![thread_pubkey.to_string()],
                }),
                min_context_slot: Some(trigger_slot),
                ..Default::default()
            },
        ).await {
            Ok(result) => result,
            Err(err) => {
                // Check for min context slot error
                if let solana_client::client_error::ClientErrorKind::RpcError(
                    solana_client::rpc_request::RpcError::RpcResponseError { code, .. }
                ) = &err.kind {
                    if *code == JSON_RPC_SERVER_ERROR_MIN_CONTEXT_SLOT_NOT_REACHED {
                        return Err(anyhow!("RPC not caught up to slot {}", trigger_slot));
                    }
                }
                return Err(anyhow!("Simulation failed: {}", err));
            }
        };
        
        // Check for simulation errors
        if let Some(err) = sim_result.value.err {
            let logs = sim_result.value.logs.unwrap_or_default();
            warn!("Simulation failed for thread {}: {:?}", thread_pubkey, err);
            warn!("Simulation logs: {:?}", logs);
            return Err(anyhow!("Simulation failed: {:?}, logs: {:?}", err, logs));
        }
        
        // Verify thread account was returned
        let _thread_account = sim_result.value.accounts
            .and_then(|accounts| accounts.get(0).cloned().flatten())
            .ok_or_else(|| anyhow!("No thread account in simulation response"))?;
        
        // Calculate optimized compute units with multiplier
        let optimized_cu = if let Some(units_consumed) = sim_result.value.units_consumed {
            let with_multiplier = (units_consumed as f64 * cu_multiplier) as u32;
            let final_cu = cmp::min(with_multiplier, max_compute_units);
            info!(
                "Simulation successful for thread {} - consumed: {}, with multiplier: {}, final: {}",
                thread_pubkey, units_consumed, with_multiplier, final_cu
            );
            final_cu
        } else {
            warn!("No compute units consumed in simulation, using max: {}", max_compute_units);
            max_compute_units
        };
        
        // Rebuild transaction with optimized compute units
        self.build_execute_transaction_with_cu(executable, optimized_cu).await
    }

    /// Build transaction with specific compute unit limit
    async fn build_execute_transaction_with_cu(
        &self,
        executable: &ExecutableThread,
        compute_units: u32,
    ) -> Result<Transaction> {
        let thread_pubkey = &executable.thread_pubkey;
        let thread = &executable.thread;

        // Get fiber state
        let fiber_pubkey = Pubkey::find_program_address(
            &[
                b"thread_fiber",
                thread_pubkey.as_ref(),
                &[thread.exec_index],
            ],
            &antegen_thread_program::ID,
        ).0;
        let fiber = self.get_fiber_state_with_retry(&fiber_pubkey).await?;

        // Build execute instruction
        let execute_ix = self.build_execute_instruction(thread_pubkey, thread, &fiber).await?;

        // Build instructions with specific compute budget
        let ixs = vec![
            ComputeBudgetInstruction::set_compute_unit_limit(compute_units),
            execute_ix,
        ];

        // Build transaction
        let recent_blockhash = self.rpc_client.get_latest_blockhash().await?;
        Ok(Transaction::new_signed_with_payer(
            &ixs,
            Some(&self.keypair.pubkey()),
            &[&*self.keypair],
            recent_blockhash,
        ))
    }
}