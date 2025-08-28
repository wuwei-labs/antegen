use anyhow::{anyhow, Result};
use log::info;
use solana_program::{pubkey::Pubkey, sysvar};
use solana_sdk::{
    compute_budget::ComputeBudgetInstruction, instruction::Instruction, signature::Keypair,
    signer::Signer,
};
use std::{sync::Arc, time::Duration};

use anchor_lang::{AccountDeserialize, AnchorDeserialize, InstructionData, ToAccountMetas};
use antegen_sdk::accounts::ThreadExec;
use antegen_sdk::instruction::ExecThread;
use antegen_thread_program::state::{FiberState, Thread, Trigger, TriggerContext};

use crate::clock::SharedClock;
use crate::metrics::ProcessorMetrics;
use crate::types::ExecutableThread;
use antegen_sdk::rpc::CachedRpcClient;

/// Executor logic integrated into submitter
#[derive(Clone)]
pub struct ExecutorLogic {
    /// Executor keypair for signing transactions
    keypair: Arc<Keypair>,
    /// RPC client for queries (cached)
    rpc_client: Arc<CachedRpcClient>,
    /// Shared blockchain clock
    clock: SharedClock,
    /// Whether to forgo executor commission
    forgo_executor_commission: bool,
    /// Metrics collector
    pub metrics: Arc<ProcessorMetrics>,
}

impl ExecutorLogic {
    pub fn new(
        keypair: Arc<Keypair>,
        rpc_client: Arc<CachedRpcClient>,
        clock: SharedClock,
        forgo_executor_commission: bool,
        metrics: Arc<ProcessorMetrics>,
    ) -> Self {
        Self {
            keypair,
            rpc_client,
            clock,
            forgo_executor_commission,
            metrics,
        }
    }

    /// Update clock state
    pub async fn update_clock(&self, slot: u64, epoch: u64, unix_timestamp: i64) {
        self.clock.update(slot, epoch, unix_timestamp).await;
        // Clock update handled by parser
    }

    /// Check if a thread's trigger is ready to execute
    pub async fn is_trigger_ready(&self, thread: &Thread) -> bool {
        match &thread.trigger_context {
            TriggerContext::Timestamp { next, .. } => {
                let current_time = self.clock.get_timestamp().await;
                let is_ready = current_time >= *next;
                if is_ready {
                    info!("EXECUTOR: Trigger ready - clock: {}, trigger: {}, delta: {}s",
                        current_time, *next, current_time - *next);
                }
                is_ready
            }
            TriggerContext::Block { next, .. } => {
                // Block context is used for both Slot and Epoch triggers
                // Check the actual trigger type to determine which to compare
                match &thread.trigger {
                    Trigger::Slot { .. } => {
                        let slot = self.clock.get_slot().await;
                        slot >= *next
                    }
                    Trigger::Epoch { .. } => {
                        let epoch = self.clock.get_epoch().await;
                        epoch >= *next
                    }
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
        
        let blockchain_time = self.clock.get_timestamp().await;
        info!("EXECUTOR: Building transaction for thread {} at blockchain time {}",
            thread_pubkey, blockchain_time);

        // Get fiber state if not provided
        let fiber_state = match fiber {
            Some(f) => f.clone(),
            None => {
                // Get fiber PDA using the thread method
                let fiber_pubkey = thread.fiber(thread_pubkey);
                // Get fiber account and deserialize (retry logic is in get_account)
                let account = self.rpc_client.get_account(&fiber_pubkey).await?;
                FiberState::try_deserialize(&mut account.data.as_slice())
                    .map_err(|e| anyhow!("Failed to deserialize fiber: {}", e))?
            }
        };

        // Build execute instruction
        let execute_ix = self
            .build_execute_instruction(thread_pubkey, thread, &fiber_state)
            .await?;

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
                self.rpc_client
                    .get_thread_config()
                    .await
                    .map_err(|e| anyhow!("Failed to fetch thread config: {}", e))?
            }
        };

        // Get fiber PDA using the trait method
        let fiber_pubkey = thread.fiber(thread_pubkey);

        // Deserialize the compiled instruction to get referenced accounts
        use antegen_thread_program::state::CompiledInstructionV0;
        let compiled =
            CompiledInstructionV0::deserialize(&mut fiber.compiled_instruction.as_slice())?;

        // Process fiber accounts

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
        }
        .to_account_metas(Some(false));

        // Add ALL accounts referenced by the fiber instruction as remaining accounts
        // These are needed for the CPI - even if they're already in base accounts
        // The program's invoke_signed passes ALL remaining_accounts to the CPI
        use solana_sdk::instruction::AccountMeta;

        for (account_index, pubkey) in compiled.accounts.iter().enumerate() {
            // Determine if account should be writable based on its position in the sorted accounts
            // The accounts are sorted by: rw_signers, ro_signers, rw_non_signers, ro_non_signers
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

            accounts.push(AccountMeta {
                pubkey: *pubkey,
                is_signer: false, // CPI accounts don't need to be signers at the transaction level
                is_writable,
            });
        }

        // Build instruction accounts

        // Build instruction data using Anchor-generated type
        let data = ExecThread {
            forgo_commission: self.forgo_executor_commission,
        }
        .data();

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
        self.clock.get_slot().await
    }
    
    /// Get current blockchain timestamp
    pub async fn current_timestamp(&self) -> i64 {
        self.clock.get_timestamp().await
    }
}
