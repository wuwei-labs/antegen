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
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::Receiver;

use crate::queue::{ExecutionTask, Queue, RetryConfig, TaskResult};
use crate::sources::{ExecutorEvent, RpcSource};
use crate::transaction::{TransactionMonitor, TransactionStatus, TransactionSubmitter};
use anchor_lang::{AccountDeserialize, InstructionData};
use antegen_thread_program::state::{FiberState, Thread, ThreadConfig};

/// Clock state from the cluster
#[derive(Clone, Debug, Default)]
pub struct ClockState {
    pub slot: u64,
    pub epoch: u64,
    pub unix_timestamp: i64,
}

/// Mode of operation for the executor
#[derive(Debug, Clone)]
pub enum ExecutorMode {
    /// Paired with observer - can claim and execute
    WithObserver,
    /// Standalone - only execute unclaimed or late threads
    Standalone,
}

/// Executor service that executes threads
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
    /// Current clock state
    clock: ClockState,
    /// Queue of threads waiting to be executed
    execution_queue: Arc<Queue>,
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
        data_dir: Option<String>,
    ) -> Result<Self> {
        let tx_submitter = TransactionSubmitter::new(rpc_client.clone(), tpu_client_config).await?;
        let tx_monitor = TransactionMonitor::new(rpc_client.clone());

        let retry_config = RetryConfig {
            initial_delay_ms: 100,
            max_delay_ms: 60_000,
            backoff_multiplier: 2.0,
            max_attempts: 10,
        };

        // Setup sled-based queue
        let execution_queue = if let Some(dir) = data_dir {
            Arc::new(Queue::with_config(&dir, retry_config)?)
        } else {
            Arc::new(Queue::with_config("./executor_queue", retry_config)?)
        };

        Ok(Self {
            mode: ExecutorMode::WithObserver,
            rpc_client,
            tx_submitter,
            tx_monitor,
            keypair,
            clock: ClockState::default(),
            execution_queue,
            event_receiver: Some(event_receiver),
            rpc_source: None,
        })
    }

    /// Create standalone executor
    pub async fn new_standalone(
        rpc_client: Arc<RpcClient>,
        keypair: Arc<Keypair>,
        tpu_client_config: Option<String>,
        ws_url: Option<String>,
        data_dir: Option<String>,
    ) -> Result<Self> {
        let tx_submitter = TransactionSubmitter::new(rpc_client.clone(), tpu_client_config).await?;
        let tx_monitor = TransactionMonitor::new(rpc_client.clone());

        let retry_config = RetryConfig {
            initial_delay_ms: 100,
            max_delay_ms: 60_000,
            backoff_multiplier: 2.0,
            max_attempts: 10,
        };

        // Setup sled-based queue
        let execution_queue = if let Some(dir) = data_dir {
            Arc::new(Queue::with_config(&dir, retry_config)?)
        } else {
            Arc::new(Queue::with_config("./executor_queue", retry_config)?)
        };

        // Create RPC source with optional websocket for clock tracking
        let rpc_source = RpcSource::new(rpc_client.clone(), Duration::from_secs(5), ws_url).await?;

        Ok(Self {
            mode: ExecutorMode::Standalone,
            rpc_client,
            tx_submitter,
            tx_monitor,
            keypair,
            clock: ClockState::default(),
            execution_queue,
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
                        ExecutorEvent::ExecutableThread(executable) => {
                            info!(
                                "EXECUTOR: Received executable thread {} from observer",
                                executable.thread_pubkey
                            );

                            // Check if trigger is ready with current clock
                            if self.is_trigger_ready(&executable.thread) {
                                // Queue directly for execution
                                self.execution_queue
                                    .queue_task(executable.thread_pubkey, executable.thread)?;
                            } else {
                                debug!(
                                    "EXECUTOR: Thread {} trigger not ready yet",
                                    executable.thread_pubkey
                                );
                            }
                        }
                        ExecutorEvent::ClockUpdate {
                            slot,
                            epoch,
                            unix_timestamp,
                        } => {
                            debug!(
                                "EXECUTOR: Clock update from observer - slot: {}, epoch: {}, timestamp: {}",
                                slot, epoch, unix_timestamp
                            );

                            // Update our clock state
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

            // Process execution queue
            if let Err(e) = self.process_execution_queue().await {
                error!("EXECUTOR: Error processing execution queue: {}", e);
            }

            // Periodically save queue state (every 10 seconds)
            static LAST_SAVE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let last = LAST_SAVE.load(std::sync::atomic::Ordering::Relaxed);
            if now - last > 10 {
                if let Err(e) = self.save_queue() {
                    error!("EXECUTOR: Failed to save queue: {}", e);
                }
                LAST_SAVE.store(now, std::sync::atomic::Ordering::Relaxed);
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

        loop {
            // Get clock updates from RPC source
            if let Some(clock) = rpc_source.get_current_clock() {
                if clock.slot > self.clock.slot || clock.unix_timestamp > self.clock.unix_timestamp
                {
                    debug!(
                        "EXECUTOR: Clock update from RPC source - slot: {}, epoch: {}, timestamp: {}",
                        clock.slot, clock.epoch, clock.unix_timestamp
                    );
                    self.clock = clock;
                    self.check_ready_threads();
                }
            }

            // Check for executable threads via RPC
            use crate::sources::ThreadSource;
            match rpc_source.receive().await? {
                Some(thread) => {
                    info!(
                        "EXECUTOR: Found executable thread {} via RPC",
                        thread.thread_pubkey
                    );

                    // Check if trigger is ready with current clock
                    if self.is_trigger_ready(&thread.thread) {
                        // Queue directly for execution
                        self.execution_queue
                            .queue_task(thread.thread_pubkey, thread.thread)?;
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

            // Periodically save queue state (every 10 seconds)
            static LAST_SAVE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let last = LAST_SAVE.load(std::sync::atomic::Ordering::Relaxed);
            if now - last > 10 {
                if let Err(e) = self.save_queue() {
                    error!("EXECUTOR: Failed to save queue: {}", e);
                }
                LAST_SAVE.store(now, std::sync::atomic::Ordering::Relaxed);
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Process the execution queue
    async fn process_execution_queue(&mut self) -> Result<()> {
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let ready_tasks = self.execution_queue.get_ready_tasks(
            current_time,
            self.clock.slot,
            self.clock.epoch,
        )?;

        for task in ready_tasks {
            let pubkey = task.thread_pubkey;
            info!("EXECUTOR: Processing thread {}", pubkey);

            // Move task to processing
            self.execution_queue.move_to_processing(&task)?;

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
                        TaskResult::Retry
                    } else {
                        TaskResult::Retry
                    }
                }
            };

            // Handle the result
            self.execution_queue
                .handle_task_result(&pubkey, result, None)?;
        }

        Ok(())
    }

    /// Execute a thread on-chain
    async fn execute_thread(&self, task: &ExecutionTask) -> Result<Signature> {
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

        // Build execute instruction
        let execute_ix = self
            .build_execute_instruction(&task.thread_pubkey, &task.thread, &fiber)
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

    /// Check if a thread's trigger is ready to execute based on trigger context
    fn is_trigger_ready(&self, thread: &Thread) -> bool {
        use antegen_thread_program::state::{Trigger, TriggerContext};

        // Use the trigger_context which has the accurate "next" execution time
        match &thread.trigger_context {
            TriggerContext::Timestamp { next, .. } => {
                // For time-based triggers (Now, Timestamp, Interval, Cron)
                self.clock.unix_timestamp >= *next
            }
            TriggerContext::Block { next, .. } => {
                // For block-based triggers (Slot, Epoch)
                match &thread.trigger {
                    Trigger::Slot { .. } => self.clock.slot >= *next,
                    Trigger::Epoch { .. } => self.clock.epoch >= *next,
                    _ => false, // Shouldn't happen
                }
            }
            TriggerContext::Account { .. } => {
                // Account triggers are ready when observer detects changes
                // The hash is for detecting changes, not timing
                true
            }
        }
    }

    /// Check queued threads to see if any are now ready with updated clock
    fn check_ready_threads(&mut self) {
        // With sled-based queue, ready checks happen during get_ready_tasks
        // which considers current time, slot, and epoch
        debug!("EXECUTOR: Clock updated, ready tasks will be checked on next iteration");
    }

    /// Build thread_exec instruction
    async fn build_execute_instruction(
        &self,
        thread_pubkey: &Pubkey,
        thread: &Thread,
        fiber: &FiberState,
    ) -> Result<Instruction> {
        // Get config for fee recipients
        let config_pubkey =
            Pubkey::find_program_address(&[b"thread_config"], &antegen_thread_program::ID).0;
        let config_account = self.rpc_client.get_account(&config_pubkey).await?;
        let config = ThreadConfig::try_deserialize(&mut config_account.data.as_slice())?;

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
            data: antegen_thread_program::instruction::ExecThread {}.data(),
        })
    }

    /// Save the current queue state to disk
    pub fn save_queue(&self) -> Result<()> {
        // Sled automatically persists to disk, just flush to ensure durability
        self.execution_queue.flush()?;
        debug!("EXECUTOR: Flushed queue to disk");
        Ok(())
    }

    /// Shutdown the executor gracefully
    pub async fn shutdown(&mut self) -> Result<()> {
        info!("EXECUTOR: Shutting down gracefully");

        // Save the queue state
        if let Err(e) = self.save_queue() {
            error!("EXECUTOR: Failed to save queue on shutdown: {}", e);
        }

        // Clean up RPC source if present
        if let Some(source) = self.rpc_source.take() {
            drop(source);
        }

        Ok(())
    }
}
