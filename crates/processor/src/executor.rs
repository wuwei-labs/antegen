use anyhow::{Result, anyhow};
use log::{debug, info, warn};
use solana_program::{sysvar, pubkey::Pubkey};
use solana_sdk::{
    compute_budget::ComputeBudgetInstruction,
    instruction::Instruction,
    signature::Keypair,
    signer::Signer,
};
use std::{sync::Arc, cmp, time::Duration};
use tokio::sync::RwLock;

use antegen_thread_program::state::{Thread, FiberState, Trigger, TriggerContext};
use anchor_lang::{AnchorDeserialize, InstructionData, ToAccountMetas};
use antegen_sdk::accounts::ThreadExec;
use antegen_sdk::instruction::ExecThread;

use antegen_submitter::CachedRpcClient;
use crate::types::ExecutableThread;
use crate::metrics::ProcessorMetrics;

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
    /// RPC client for queries (cached)
    rpc_client: Arc<CachedRpcClient>,
    /// Current clock state
    clock: Arc<RwLock<ClockState>>,
    /// Whether to forgo executor commission
    forgo_executor_commission: bool,
    /// Metrics collector
    pub metrics: Arc<ProcessorMetrics>,
}

impl ExecutorLogic {
    pub fn new(
        keypair: Arc<Keypair>,
        rpc_client: Arc<CachedRpcClient>,
        forgo_executor_commission: bool,
        metrics: Arc<ProcessorMetrics>,
    ) -> Self {
        Self {
            keypair,
            rpc_client,
            clock: Arc::new(RwLock::new(ClockState::default())),
            forgo_executor_commission,
            metrics,
        }
    }

    /// Update clock state
    pub async fn update_clock(&self, slot: u64, epoch: u64, unix_timestamp: i64) {
        let mut clock = self.clock.write().await;
        *clock = ClockState {
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
            self.metrics.rpc_request("get_account");
            match self.rpc_client.get_fiber_state(fiber_pubkey).await {
                Ok(fiber_state) => {
                    if attempt > 1 {
                        info!("Successfully fetched fiber account {} on attempt {} (waited ~{:.1}s total)",
                            fiber_pubkey, attempt, 
                            start_time.elapsed().as_secs_f32());
                    }
                    return Ok(fiber_state);
                }
                Err(e) => {
                    let error_str = e.to_string();
                    
                    // Check if it's an account not found error (expected during race conditions)
                    if error_str.contains("AccountNotFound") || error_str.contains("could not find account") {
                        if last_log.elapsed() > Duration::from_secs(30) {
                            // Log progress every 30 seconds
                            info!("Still waiting for fiber account {} to exist (elapsed: {:.0}s)...",
                                fiber_pubkey, start_time.elapsed().as_secs());
                            last_log = std::time::Instant::now();
                        }
                    } else {
                        // For non-AccountNotFound errors, we might want to fail after some attempts
                        // but for now, keep retrying with logging
                        if last_log.elapsed() > Duration::from_secs(30) {
                            // Log errors every 30 seconds to avoid spam
                            warn!("Error fetching fiber account {} after {:.0}s: {}",
                                fiber_pubkey, start_time.elapsed().as_secs(), e);
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
    pub async fn is_trigger_ready(&self, thread: &Thread) -> bool {
        match &thread.trigger_context {
            TriggerContext::Timestamp { next, .. } => {
                let clock = self.clock.read().await;
                let current_time = clock.unix_timestamp;
                current_time >= *next
            }
            TriggerContext::Block { next, .. } => {
                // Block context is used for both Slot and Epoch triggers
                // Check the actual trigger type to determine which to compare
                match &thread.trigger {
                    Trigger::Slot { .. } => {
                        let clock = self.clock.read().await;
                        clock.slot >= *next
                    },
                    Trigger::Epoch { .. } => {
                        let clock = self.clock.read().await;
                        clock.epoch >= *next
                    },
                    _ => false,
                }
            }
            TriggerContext::Account { .. } => {
                // Account triggers are handled by observer
                false
            }
        }
    }

    /// Build a transaction to execute a thread (unified method)
    pub async fn build_execute_transaction(
        &self,
        executable: &ExecutableThread,
        fiber: Option<&FiberState>,
        compute_units: Option<u32>,
    ) -> Result<Vec<Instruction>> {
        let thread_pubkey = &executable.thread_pubkey;
        let thread = &executable.thread;

        // Get fiber state if not provided
        let fiber_state = match fiber {
            Some(f) => f.clone(),
            None => {
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
                self.get_fiber_state_with_retry(&fiber_pubkey).await?
            }
        };

        // Build execute instruction
        let execute_ix = self.build_execute_instruction(thread_pubkey, thread, &fiber_state).await?;

        // Build instructions with optional compute budget
        let mut ixs = vec![];
        
        if let Some(cu_limit) = compute_units {
            ixs.push(ComputeBudgetInstruction::set_compute_unit_limit(cu_limit));
        }
        
        ixs.push(execute_ix);
        
        Ok(ixs)
    }

    /// Build exec_thread instruction
    async fn build_execute_instruction(
        &self,
        thread_pubkey: &Pubkey,
        thread: &Thread,
        fiber: &FiberState,
    ) -> Result<Instruction> {
        // Get config for fee recipients
        let config_pubkey =
            Pubkey::find_program_address(&[b"thread_config"], &antegen_thread_program::ID).0;
        
        // Fetch config with caching
        self.metrics.rpc_request("get_account");
        let config = match self.rpc_client.get_thread_config().await {
            Ok(config) => config,
            Err(_) => {
                // Retry once if config fetch fails
                tokio::time::sleep(Duration::from_millis(500)).await;
                self.metrics.rpc_request("get_account");
                self.rpc_client.get_thread_config().await
                    .map_err(|e| anyhow!("Failed to fetch thread config: {}", e))?
            }
        };

        // Get fiber PDA
        let fiber_pubkey = Pubkey::find_program_address(
            &[
                b"thread_fiber",
                thread_pubkey.as_ref(),
                &[thread.exec_index],
            ],
            &antegen_thread_program::ID,
        ).0;

        // Deserialize the compiled instruction to get referenced accounts
        use antegen_thread_program::state::CompiledInstructionV0;
        let compiled = CompiledInstructionV0::deserialize(&mut fiber.compiled_instruction.as_slice())?;
        
        debug!("Fiber instruction has {} accounts", compiled.accounts.len());
        
        // Build base accounts using Anchor-generated types
        let mut accounts = ThreadExec {
            executor: self.keypair.pubkey(),
            thread: *thread_pubkey,
            fiber: fiber_pubkey,
            config: config_pubkey,
            thread_authority: thread.authority,
            admin: config.admin,
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

        // Add ALL accounts referenced by the fiber instruction as remaining accounts
        // These are needed for the CPI - even if they're already in base accounts
        // The program's invoke_signed passes ALL remaining_accounts to the CPI
        use solana_sdk::instruction::AccountMeta;
        
        for (account_index, pubkey) in compiled.accounts.iter().enumerate() {
            // Determine if account should be writable based on its position in the sorted accounts
            // The accounts are sorted by: rw_signers, ro_signers, rw_non_signers, ro_non_signers
            let account_idx = account_index as u8;
            let is_writable = if account_idx < compiled.num_rw_signers {
                true  // Read-write signer
            } else if account_idx < compiled.num_rw_signers + compiled.num_ro_signers {
                false  // Read-only signer
            } else if account_idx < compiled.num_rw_signers + compiled.num_ro_signers + compiled.num_rw {
                true  // Read-write non-signer
            } else {
                false  // Read-only non-signer
            };
            
            accounts.push(AccountMeta {
                pubkey: *pubkey,
                is_signer: false,  // CPI accounts don't need to be signers at the transaction level
                is_writable,
            });
        }
        
        debug!("Total accounts for instruction: {}", accounts.len());

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
    pub async fn current_slot(&self) -> u64 {
        let clock = self.clock.read().await;
        clock.slot
    }
}