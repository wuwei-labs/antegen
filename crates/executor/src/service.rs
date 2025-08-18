use anyhow::{anyhow, Result};
use log::{debug, error, info};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_program::pubkey::Pubkey;
use solana_sdk::{
    compute_budget::ComputeBudgetInstruction,
    instruction::{AccountMeta, Instruction},
    signature::{Keypair, Signature, Signer},
    transaction::Transaction,
};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::Receiver;

use crate::retry_queue::{RetryConfig, RetryQueue, TaskResult};
use crate::sources::{ExecutorEvent, RpcSource};
use crate::transaction::{TransactionMonitor, TransactionStatus, TransactionSubmitter};
use anchor_lang::{AccountDeserialize, InstructionData};
use antegen_thread_program::state::{FiberState, Thread, ThreadConfig};

/// Mode of operation for the executor
#[derive(Debug, Clone)]
pub enum ExecutorMode {
    /// Paired with observer - can claim and execute
    WithObserver,
    /// Standalone - only execute unclaimed or late threads
    Standalone,
}

/// Task for claiming a thread
#[derive(Clone)]
struct ClaimTask {
    thread_pubkey: Pubkey,
    thread: Thread,
    slot: u64,
}

/// Task for executing a thread
#[derive(Clone)]
struct ExecutionTask {
    thread_pubkey: Pubkey,
    thread: Thread,
    claimed: bool, // True if we claimed it, False if unclaimed
    claimed_at: Option<i64>, // When it was claimed (if claimed)
}

/// Current cluster clock state
#[derive(Clone, Debug, Default)]
struct ClockState {
    slot: u64,
    epoch: u64,
    unix_timestamp: i64,
}

/// Executor service that claims and/or executes threads
pub struct ExecutorService {
    /// Mode of operation
    mode: ExecutorMode,
    /// RPC client for queries
    rpc_client: Arc<RpcClient>,
    /// Transaction submitter for TPU/RPC submission
    tx_submitter: TransactionSubmitter,
    /// Transaction monitor for confirmations
    tx_monitor: TransactionMonitor,
    /// Executor keypair for signing transactions
    keypair: Arc<Keypair>,
    /// Current cluster clock state
    clock: ClockState,
    /// Queue of threads waiting to be claimed (only in WithObserver mode)
    claim_queue: RetryQueue<Pubkey, ClaimTask>,
    /// Queue of threads waiting to be executed
    execution_queue: RetryQueue<Pubkey, ExecutionTask>,
    /// Optional receiver for executor events from observer
    event_receiver: Option<Receiver<ExecutorEvent>>,
    /// Optional RPC source for standalone mode
    rpc_source: Option<RpcSource>,
}

impl ExecutorService {
    /// Create executor paired with observer
    pub async fn new_with_observer(
        rpc_client: Arc<RpcClient>,
        keypair: Arc<Keypair>,
        event_receiver: Receiver<ExecutorEvent>,
        tpu_client_config: Option<String>,
    ) -> Result<Self> {
        let tx_submitter = TransactionSubmitter::new(rpc_client.clone(), tpu_client_config).await?;

        let tx_monitor = TransactionMonitor::new(rpc_client.clone());

        let retry_config = RetryConfig {
            initial_delay_ms: 100,
            max_delay_ms: 60_000,
            backoff_multiplier: 2.0,
        };

        Ok(Self {
            mode: ExecutorMode::WithObserver,
            rpc_client,
            tx_submitter,
            tx_monitor,
            keypair,
            clock: ClockState::default(),
            claim_queue: RetryQueue::with_config(retry_config.clone()),
            execution_queue: RetryQueue::with_config(retry_config),
            event_receiver: Some(event_receiver),
            rpc_source: None,
        })
    }

    /// Create standalone executor
    pub async fn new_standalone(
        rpc_client: Arc<RpcClient>,
        keypair: Arc<Keypair>,
        tpu_client_config: Option<String>,
    ) -> Result<Self> {
        let tx_submitter = TransactionSubmitter::new(rpc_client.clone(), tpu_client_config).await?;

        let tx_monitor = TransactionMonitor::new(rpc_client.clone());

        let retry_config = RetryConfig {
            initial_delay_ms: 100,
            max_delay_ms: 60_000,
            backoff_multiplier: 2.0,
        };

        // Create RPC source for monitoring unclaimed threads
        let rpc_source = RpcSource::new(
            rpc_client.clone(),
            Duration::from_secs(5), // Poll every 5 seconds
            None,                   // No specific observer keypair in standalone mode
        );

        Ok(Self {
            mode: ExecutorMode::Standalone,
            rpc_client,
            tx_submitter,
            tx_monitor,
            keypair,
            clock: ClockState::default(),
            claim_queue: RetryQueue::with_config(retry_config.clone()),
            execution_queue: RetryQueue::with_config(retry_config),
            event_receiver: None,
            rpc_source: Some(rpc_source),
        })
    }

    /// Main service loop
    pub async fn run(&mut self) -> Result<()> {
        match &self.mode {
            ExecutorMode::WithObserver => {
                info!(
                    "EXECUTOR: Starting in WithObserver mode (observer={})",
                    self.keypair.pubkey()
                );
                self.run_with_observer().await
            }
            ExecutorMode::Standalone => {
                info!("EXECUTOR: Starting in Standalone mode");
                self.run_standalone().await
            }
        }
    }

    /// Run with observer - process events from observer
    async fn run_with_observer(&mut self) -> Result<()> {
        let mut event_rx = self
            .event_receiver
            .take()
            .ok_or_else(|| anyhow!("No event receiver in WithObserver mode"))?;

        loop {
            // Check for new events from observer
            match event_rx.try_recv() {
                Ok(event) => {
                    match event {
                        ExecutorEvent::ClaimableThread(claimable) => {
                            info!(
                                "EXECUTOR: Received claimable thread {} from observer",
                                claimable.thread_pubkey
                            );

                            // Check if trigger is ready with current clock
                            if self.is_trigger_ready(&claimable.thread) {
                                if let ExecutorMode::WithObserver = self.mode {
                                    // Queue for claiming
                                    let claim_task = ClaimTask {
                                        thread_pubkey: claimable.thread_pubkey,
                                        thread: claimable.thread,
                                        slot: claimable.slot,
                                    };
                                    self.claim_queue
                                        .queue_task(claimable.thread_pubkey, claim_task);
                                }
                            } else {
                                debug!(
                                    "EXECUTOR: Thread {} trigger not ready yet",
                                    claimable.thread_pubkey
                                );
                            }
                        }
                        ExecutorEvent::ClockUpdate {
                            slot,
                            epoch,
                            unix_timestamp,
                        } => {
                            debug!(
                                "EXECUTOR: Clock update - slot: {}, epoch: {}, timestamp: {}",
                                slot, epoch, unix_timestamp
                            );

                            // Update clock state
                            self.clock = ClockState {
                                slot,
                                epoch,
                                unix_timestamp,
                            };

                            // Check if any queued threads are now ready
                            self.check_ready_threads();
                        }
                    }
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    // No new events
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    return Err(anyhow!("Observer channel disconnected"));
                }
            }

            // Process claim queue
            if let Err(e) = self.process_claim_queue().await {
                error!("EXECUTOR: Error processing claim queue: {}", e);
            }

            // Process execution queue
            if let Err(e) = self.process_execution_queue().await {
                error!("EXECUTOR: Error processing execution queue: {}", e);
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Run standalone - monitor RPC for executable threads
    async fn run_standalone(&mut self) -> Result<()> {
        let mut rpc_source = self
            .rpc_source
            .take()
            .ok_or_else(|| anyhow!("No RPC source in Standalone mode"))?;

        let mut last_clock_update = Instant::now();
        let clock_update_interval = Duration::from_secs(2); // Update clock every 2 seconds

        loop {
            // Update clock from RPC periodically
            if last_clock_update.elapsed() > clock_update_interval {
                if let Err(e) = self.update_clock_from_rpc().await {
                    debug!("EXECUTOR: Failed to update clock from RPC: {}", e);
                }
                last_clock_update = Instant::now();
            }

            // Check for unclaimed/late threads via RPC
            use crate::sources::ClaimedThreadSource;
            match rpc_source.receive().await? {
                Some(thread) => {
                    info!(
                        "EXECUTOR: Found executable thread {} via RPC",
                        thread.thread_pubkey
                    );

                    // Check if trigger is ready with current clock
                    if self.is_trigger_ready(&thread.thread) {
                        // Queue directly for execution (no claiming in standalone)
                        let exec_task = ExecutionTask {
                            thread_pubkey: thread.thread_pubkey,
                            thread: thread.thread,
                            claimed: false,
                            claimed_at: None,
                        };
                        self.execution_queue
                            .queue_task(thread.thread_pubkey, exec_task);
                    } else {
                        debug!(
                            "EXECUTOR: Thread {} trigger not ready yet",
                            thread.thread_pubkey
                        );
                    }
                }
                None => {
                    // No threads found
                }
            }

            // Process execution queue
            if let Err(e) = self.process_execution_queue().await {
                error!("EXECUTOR: Error processing execution queue: {}", e);
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Update clock state from RPC
    async fn update_clock_from_rpc(&mut self) -> Result<()> {
        // Get slot
        let slot = self.rpc_client.get_slot().await?;

        // Get epoch info
        let epoch_info = self.rpc_client.get_epoch_info().await?;

        // Get block time for current slot (this might fail for very recent slots)
        let unix_timestamp = match self.rpc_client.get_block_time(slot).await {
            Ok(timestamp) => timestamp,
            Err(_) => {
                // Fallback to system time if block time not available
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64
            }
        };

        self.clock = ClockState {
            slot,
            epoch: epoch_info.epoch,
            unix_timestamp,
        };

        debug!(
            "EXECUTOR: Updated clock from RPC - slot: {}, epoch: {}, timestamp: {}",
            slot, epoch_info.epoch, unix_timestamp
        );

        // Check if any queued threads are now ready
        self.check_ready_threads();

        Ok(())
    }

    /// Process the claim queue (only in WithObserver mode)
    async fn process_claim_queue(&mut self) -> Result<()> {
        let ready_keys = self.claim_queue.get_ready_keys();

        for pubkey in ready_keys {
            if let Some(queued_task) = self.claim_queue.get_mut(&pubkey) {
                let task = queued_task.task.clone();

                if queued_task.attempts == 0 {
                    info!("EXECUTOR: First claim attempt for thread {}", pubkey);
                } else {
                    debug!(
                        "EXECUTOR: Claim attempt {} for thread {}",
                        queued_task.attempts + 1,
                        pubkey
                    );
                }

                queued_task.attempts += 1;

                // Try to claim the thread
                let result = match self.claim_thread(&task).await {
                    Ok((signature, claimed_at)) => {
                        info!(
                            "EXECUTOR: Successfully claimed thread {} with signature {}",
                            pubkey, signature
                        );

                        // Queue for execution
                        let exec_task = ExecutionTask {
                            thread_pubkey: task.thread_pubkey,
                            thread: task.thread,
                            claimed: true,
                            claimed_at: Some(claimed_at),
                        };
                        self.execution_queue
                            .queue_task(task.thread_pubkey, exec_task);

                        TaskResult::Success
                    }
                    Err(e) => {
                        error!("EXECUTOR: Claim error for thread {}: {}", pubkey, e);

                        if e.to_string().contains("already claimed") {
                            TaskResult::Success // Don't retry if already claimed
                        } else {
                            TaskResult::Retry
                        }
                    }
                };

                self.claim_queue.handle_task_result(&pubkey, result);
            }
        }

        Ok(())
    }

    /// Claim a thread on-chain
    async fn claim_thread(&self, task: &ClaimTask) -> Result<(Signature, i64)> {
        // Get fiber PDA
        let fiber_pubkey = Pubkey::find_program_address(
            &[
                b"thread_fiber",
                task.thread_pubkey.as_ref(),
                &[task.thread.exec_index],
            ],
            &antegen_thread_program::ID,
        )
        .0;

        // Check if fiber exists and is unclaimed
        match self.rpc_client.get_account(&fiber_pubkey).await {
            Ok(account) => {
                let fiber = FiberState::try_deserialize(&mut account.data.as_slice())?;
                if fiber.observer.is_some() {
                    return Err(anyhow!("Fiber already claimed"));
                }
            }
            Err(_) => {
                return Err(anyhow!("Fiber doesn't exist yet"));
            }
        }

        // Build claim instruction - simplified without Observer account
        let claim_ix = Instruction {
            program_id: antegen_thread_program::ID,
            accounts: vec![
                AccountMeta::new(self.keypair.pubkey(), true), // observer signer
                AccountMeta::new_readonly(task.thread_pubkey, false),
                AccountMeta::new(fiber_pubkey, false),
            ],
            data: antegen_thread_program::instruction::ThreadClaim {}.data(),
        };

        // Add compute budget
        let ixs = vec![
            ComputeBudgetInstruction::set_compute_unit_limit(200_000),
            claim_ix,
        ];

        // Build and submit transaction
        let recent_blockhash = self.rpc_client.get_latest_blockhash().await?;
        let tx = Transaction::new_signed_with_payer(
            &ixs,
            Some(&self.keypair.pubkey()),
            &[&*self.keypair],
            recent_blockhash,
        );

        let signature = self.tx_submitter.submit(&tx).await?;

        // Monitor for confirmation
        use solana_sdk::commitment_config::CommitmentConfig;
        match self
            .tx_monitor
            .monitor_transaction(&signature, CommitmentConfig::confirmed())
            .await?
        {
            TransactionStatus::Confirmed(_) => {
                info!("EXECUTOR: Claim transaction {} confirmed", signature);
                let claimed_at = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;
                Ok((signature, claimed_at))
            }
            TransactionStatus::Failed(err) => Err(anyhow!("Claim transaction failed: {}", err)),
            _ => Err(anyhow!("Claim transaction not confirmed within timeout")),
        }
    }

    /// Process the execution queue
    async fn process_execution_queue(&mut self) -> Result<()> {
        let ready_keys = self.execution_queue.get_ready_keys();

        for pubkey in ready_keys {
            if let Some(queued_task) = self.execution_queue.get_mut(&pubkey) {
                let task = queued_task.task.clone();

                if queued_task.attempts == 0 {
                    info!("EXECUTOR: First execution attempt for thread {}", pubkey);
                } else {
                    debug!(
                        "EXECUTOR: Execution attempt {} for thread {}",
                        queued_task.attempts + 1,
                        pubkey
                    );
                }

                queued_task.attempts += 1;

                // Try to execute the thread
                let result = match self.execute_thread(&task).await {
                    Ok(signature) => {
                        info!(
                            "EXECUTOR: Successfully executed thread {} with signature {}",
                            pubkey, signature
                        );
                        TaskResult::Success
                    }
                    Err(e) => {
                        error!("EXECUTOR: Execution error for thread {}: {}", pubkey, e);

                        if e.to_string().contains("already executed") {
                            TaskResult::Success
                        } else if e.to_string().contains("priority window") {
                            // Still in someone else's priority window
                            TaskResult::Retry
                        } else {
                            TaskResult::Retry
                        }
                    }
                };

                self.execution_queue.handle_task_result(&pubkey, result);
            }
        }

        Ok(())
    }

    /// Execute a thread on-chain
    async fn execute_thread(&self, task: &ExecutionTask) -> Result<Signature> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Get config for timing windows
        let config_pubkey = Pubkey::find_program_address(
            &[b"thread_config"],
            &antegen_thread_program::ID,
        )
        .0;
        let config_account = self.rpc_client.get_account(&config_pubkey).await?;
        let config = ThreadConfig::try_deserialize(&mut config_account.data.as_slice())?;

        // Get fiber PDA
        let fiber_pubkey = Pubkey::find_program_address(
            &[
                b"thread_fiber",
                task.thread_pubkey.as_ref(),
                &[task.thread.exec_index],
            ],
            &antegen_thread_program::ID,
        )
        .0;

        // Get fiber account
        let fiber_account = self.rpc_client.get_account(&fiber_pubkey).await?;
        let fiber = FiberState::try_deserialize(&mut fiber_account.data.as_slice())?;

        // Check if we can execute
        if let Some(claimed_observer) = fiber.observer {
            // Thread is claimed
            let in_priority_window = now < fiber.claimed_at + config.priority_window;

            if claimed_observer != self.keypair.pubkey() && in_priority_window {
                // Not our claim and still in priority window
                return Err(anyhow!(
                    "Thread claimed by another observer, still in priority window"
                ));
            }
        } else if self.mode.matches(&ExecutorMode::Standalone) {
            // In standalone mode, we can only execute unclaimed threads
            debug!(
                "EXECUTOR: Executing unclaimed thread {}",
                task.thread_pubkey
            );
        }

        // Build execute instruction
        let execute_ix = self
            .build_execute_instruction(&task.thread_pubkey, &task.thread, &fiber, task.claimed)
            .await?;

        // Add compute budget
        let ixs = vec![
            ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
            execute_ix,
        ];

        // Build and submit transaction
        let recent_blockhash = self.rpc_client.get_latest_blockhash().await?;
        let tx = Transaction::new_signed_with_payer(
            &ixs,
            Some(&self.keypair.pubkey()),
            &[&*self.keypair],
            recent_blockhash,
        );

        let signature = self.tx_submitter.submit(&tx).await?;

        // Monitor for confirmation
        use solana_sdk::commitment_config::CommitmentConfig;
        match self
            .tx_monitor
            .monitor_transaction(&signature, CommitmentConfig::confirmed())
            .await?
        {
            TransactionStatus::Confirmed(_) => {
                info!("EXECUTOR: Execute transaction {} confirmed", signature);
                Ok(signature)
            }
            TransactionStatus::Failed(err) => Err(anyhow!("Execute transaction failed: {}", err)),
            _ => Err(anyhow!("Execute transaction not confirmed within timeout")),
        }
    }

    /// Check if a thread's trigger is ready to execute
    fn is_trigger_ready(&self, thread: &Thread) -> bool {
        use antegen_thread_program::state::Trigger;

        match &thread.trigger {
            Trigger::Now => true,
            Trigger::Timestamp { unix_ts } => self.clock.unix_timestamp >= *unix_ts,
            Trigger::Interval { .. } => {
                // Check if enough time has passed since last execution
                // This would need the last execution time from trigger_context
                // For now, assume it's ready if we have a valid clock
                self.clock.unix_timestamp > 0
            }
            Trigger::Cron { .. } => {
                // Cron evaluation would need the cron parser
                // For now, assume ready if we have a valid clock
                self.clock.unix_timestamp > 0
            }
            Trigger::Account { .. } => {
                // Account triggers are ready when the account changes
                // This is handled by the observer detecting the change
                true
            }
            Trigger::Slot { slot } => self.clock.slot >= *slot,
            Trigger::Epoch { epoch } => self.clock.epoch >= *epoch,
        }
    }

    /// Check queued threads to see if any are now ready with updated clock
    fn check_ready_threads(&mut self) {
        // Check claim queue for threads that might now be ready
        let pending_keys: Vec<Pubkey> = self.claim_queue.get_all_keys();

        for pubkey in pending_keys {
            if let Some(task) = self.claim_queue.get(&pubkey) {
                if self.is_trigger_ready(&task.task.thread) {
                    debug!(
                        "EXECUTOR: Thread {} is now ready after clock update",
                        pubkey
                    );
                    // The task is already queued, it will be processed in the next iteration
                }
            }
        }
    }

    /// Build thread_exec instruction
    async fn build_execute_instruction(
        &self,
        thread_pubkey: &Pubkey,
        thread: &Thread,
        fiber: &FiberState,
        claimed: bool,
    ) -> Result<Instruction> {
        // Get config for fee recipients
        let config_pubkey = Pubkey::find_program_address(
            &[b"thread_config"],
            &antegen_thread_program::ID,
        )
        .0;
        let config_account = self.rpc_client.get_account(&config_pubkey).await?;
        let config = ThreadConfig::try_deserialize(&mut config_account.data.as_slice())?;
        
        // Observer account is the keypair pubkey if claimed, otherwise default
        let observer_account = if claimed {
            self.keypair.pubkey()
        } else {
            Pubkey::default()
        };

        // Get fiber PDA
        let fiber_pubkey = Pubkey::find_program_address(
            &[
                b"thread_fiber",
                thread_pubkey.as_ref(),
                &[thread.exec_index],
            ],
            &antegen_thread_program::ID,
        )
        .0;

        // Parse the fiber's compiled instruction
        use anchor_lang::AnchorDeserialize;
        let compiled = antegen_thread_program::state::CompiledInstructionV0::deserialize(
            &mut fiber.compiled_instruction.as_slice(),
        )?;
        let instruction = antegen_thread_program::state::decompile_instruction(&compiled)?;

        // Build account metas for thread_exec
        let mut account_metas = vec![
            AccountMeta::new(self.keypair.pubkey(), true), // Executor (signer)
            AccountMeta::new(self.keypair.pubkey(), true), // Fee payer (same as executor)
            AccountMeta::new(*thread_pubkey, false),       // Thread account
            AccountMeta::new(fiber_pubkey, false),         // Fiber account
            AccountMeta::new_readonly(config_pubkey, false), // Config account
            AccountMeta::new(thread.authority, false),     // Thread authority (for fees)
            AccountMeta::new(observer_account, false),     // Observer account (for fees if claimed)
            AccountMeta::new(config.admin, false),         // Config admin (for fees)
            AccountMeta::new_readonly(solana_program::system_program::ID, false),
        ];

        // Add nonce account if thread uses durable nonces
        if thread.has_nonce_account() {
            account_metas.push(AccountMeta::new(thread.nonce_account, false));
            #[allow(deprecated)]
            account_metas.push(AccountMeta::new_readonly(
                solana_program::sysvar::recent_blockhashes::ID,
                false,
            ));
        }

        // Add accounts from the fiber instruction
        for account in &instruction.accounts {
            account_metas.push(AccountMeta {
                pubkey: account.pubkey,
                is_signer: false,
                is_writable: account.is_writable,
            });
        }

        // Add trigger account if needed
        match &thread.trigger {
            antegen_thread_program::state::Trigger::Account { address, .. } => {
                account_metas.push(AccountMeta {
                    pubkey: *address,
                    is_signer: false,
                    is_writable: false,
                });
            }
            _ => {}
        }

        Ok(Instruction {
            program_id: antegen_thread_program::ID,
            accounts: account_metas,
            data: antegen_thread_program::instruction::ThreadExec {}.data(),
        })
    }
}

impl ExecutorMode {
    fn matches(&self, other: &ExecutorMode) -> bool {
        matches!(
            (self, other),
            (ExecutorMode::Standalone, ExecutorMode::Standalone)
                | (ExecutorMode::WithObserver, ExecutorMode::WithObserver)
        )
    }
}
