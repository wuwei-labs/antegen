use anchor_lang::AccountDeserialize;
use anyhow::{anyhow, Result};
use crossbeam::channel::{Receiver, RecvTimeoutError};
use log::{debug, error, info, warn};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{read_keypair_file, Keypair},
};
use std::sync::Arc;
use std::time::Duration;

use crate::parser::{classify_account, AccountType};
use crate::executor::ExecutorLogic;
use crate::queue::{ThreadQueue, TriggerType};
use crate::types::{ProcessorConfig, AccountUpdate, ExecutableThread};
use crate::metrics::ProcessorMetrics;
use antegen_submitter::{
    DurableTransactionMessage, SubmissionService, SubmissionConfig, SubmitterMetrics
};

/// Main processor service that handles thread processing
pub struct ProcessorService {
    /// Configuration
    config: ProcessorConfig,
    /// Unified submission service for all RPC/TPU operations
    submission_service: Arc<SubmissionService>,
    /// Executor logic for building transactions
    executor_logic: Arc<ExecutorLogic>,
    /// Thread queue for scheduling
    thread_queue: Arc<ThreadQueue>,
    /// Metrics collector
    #[allow(dead_code)]
    metrics: Arc<ProcessorMetrics>,
    /// Executor keypair
    #[allow(dead_code)]
    executor_keypair: Arc<Keypair>,
    /// Account update receiver from adapter
    account_receiver: Receiver<AccountUpdate>,
}

impl ProcessorService {
    /// Create a new processor service
    pub async fn new(
        config: ProcessorConfig,
        account_receiver: Receiver<AccountUpdate>,
    ) -> Result<Self> {
        // Load executor keypair
        let executor_keypair = Arc::new(
            read_keypair_file(&config.executor_keypair_path)
                .map_err(|e| anyhow!("Failed to read executor keypair: {}", e))?
        );
        
        // Create metrics instances
        let processor_metrics = Arc::new(ProcessorMetrics::default());
        let submitter_metrics = Arc::new(SubmitterMetrics::default());
        
        // Create submission configuration
        let submission_config = SubmissionConfig {
            cache_config: config.cache_config.clone(),
            tpu_config: config.tpu_config.clone(),
            replay_config: config.replay_config.clone(),
        };
        
        // Create unified submission service
        let submission_service = Arc::new(
            SubmissionService::new(
                config.rpc_url.clone(),
                submission_config,
                Some(submitter_metrics.clone()),
            ).await?
        );
        
        // Initialize submission service (wait for RPC and create TPU client)
        submission_service.initialize().await?;
        
        // Create executor logic
        let executor_logic = Arc::new(ExecutorLogic::new(
            executor_keypair.clone(),
            submission_service.cached_rpc().clone(),
            config.forgo_executor_commission,
            processor_metrics.clone(),
        ));
        
        // Create thread queue with metrics
        let thread_queue = Arc::new(
            ThreadQueue::with_metrics(
                config.max_concurrent_threads,
                processor_metrics.clone(),
            )?
        );
        
        Ok(Self {
            config,
            submission_service,
            executor_logic,
            thread_queue,
            metrics: processor_metrics,
            executor_keypair,
            account_receiver,
        })
    }
    
    /// Start a task to process threads as they become ready
    fn start_thread_processing<F, Fut>(&self, processor_fn: F)
    where
        F: Fn(Pubkey, antegen_thread_program::state::Thread) -> Fut + Send + Sync + 'static + Clone,
        Fut: std::future::Future<Output = Result<solana_sdk::signature::Signature>> + Send + 'static,
    {
        let queue = self.thread_queue.clone();
        tokio::spawn(async move {
            queue.spawn_execution_tasks(processor_fn).await;
        });
    }
    
    /// Main processing loop
    pub async fn run(self) -> Result<()> {
        info!("Starting processor service");
        
        // Note: Replay consumer is started automatically by SubmissionService during initialization
        
        // Start processing threads as they become ready
        let processor_fn = self.create_processor_fn();
        self.start_thread_processing(processor_fn);
        
        // Main loop processing account updates
        loop {
            match self.account_receiver.recv_timeout(Duration::from_secs(1)) {
                Ok(account_update) => {
                    if let Err(e) = self.process_account_update(account_update).await {
                        error!("Error processing account update: {}", e);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    // Periodic health check or metrics update
                    debug!("No account updates received in the last second");
                }
                Err(RecvTimeoutError::Disconnected) => {
                    warn!("Account receiver disconnected, shutting down");
                    break;
                }
            }
        }
        
        // Shutdown
        info!("Shutting down processor service");
        // ThreadQueue and SubmissionService will be dropped and cleaned up automatically
        
        Ok(())
    }
    
    /// Process a single account update
    async fn process_account_update(&self, account_update: AccountUpdate) -> Result<()> {
        // Update cache with the account
        self.submission_service.cached_rpc().update_account_selectively(
            &account_update.pubkey,
            account_update.account.clone(),
        ).await;
        
        // Classify and process the account
        match classify_account(&account_update.pubkey, &account_update.account) {
            AccountType::Clock { unix_timestamp, slot, epoch } => {
                debug!("Clock update: slot={}, epoch={}, timestamp={}", slot, epoch, unix_timestamp);
                
                // Update executor clock
                self.executor_logic.update_clock(slot, epoch, unix_timestamp).await;
                
                // Check timestamp-based triggers
                let processor_fn = self.create_processor_fn();
                self.thread_queue.check_and_execute_single_trigger(
                    TriggerType::Time,
                    unix_timestamp as u64,
                    processor_fn,
                ).await;
            }
            AccountType::Thread(thread) => {
                debug!("Thread update: {}", account_update.pubkey);
                
                // Schedule thread for processing
                if let Err(e) = self.thread_queue.schedule_thread(
                    account_update.pubkey,
                    thread,
                ).await {
                    warn!("Failed to schedule thread {}: {}", account_update.pubkey, e);
                }
            }
            AccountType::Other => {
                // Other accounts are already cached if needed
                debug!("Other account update: {}", account_update.pubkey);
            }
        }
        
        Ok(())
    }
    
    /// Create the processor function for thread execution
    fn create_processor_fn(&self) -> impl Fn(Pubkey, antegen_thread_program::state::Thread) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<solana_sdk::signature::Signature>> + Send>> + Send + Sync + Clone {
        let submission_service = self.submission_service.clone();
        let executor = self.executor_logic.clone();
        let executor_keypair = self.executor_keypair.clone();
        let max_retries = self.config.tpu_max_retries as usize;
        
        move |thread_pubkey, thread| {
            let submission_service = submission_service.clone();
            let executor = executor.clone();
            let executor_keypair = executor_keypair.clone();
            
            Box::pin(async move {
                info!("Processing thread {}", thread_pubkey);
                
                // Build instructions
                let executable = ExecutableThread {
                    thread_pubkey,
                    thread: thread.clone(),
                    slot: 0, // Current slot, could be passed in if needed
                };
                let instructions = executor.build_execute_transaction(
                    &executable,
                    None, // fiber state
                    None, // compute units (let submitter optimize)
                ).await?;
                
                // Submit with retries and automatic simulation
                let mut attempts = 0;
                while attempts < max_retries {
                    match submission_service.submit_with_options(
                        instructions.clone(),
                        &executor.pubkey(),
                        &[&*executor_keypair],
                        true,  // Enable simulation
                        Some(&thread_pubkey),
                    ).await {
                        Ok(sig) => {
                            info!("Transaction {} submitted for thread {}", sig, thread_pubkey);
                            
                            // For TPU submissions, wait and check if thread was updated
                            if submission_service.get_mode().await == antegen_submitter::SubmissionMode::Tpu {
                                tokio::time::sleep(Duration::from_millis(1500)).await;
                                
                                // Check if thread's exec_count increased
                                if let Ok(account) = submission_service.cached_rpc().bypass().get_account(&thread_pubkey).await {
                                    if let Ok(updated_thread) = antegen_thread_program::state::Thread::try_deserialize(&mut account.data.as_slice()) {
                                        if updated_thread.exec_count > thread.exec_count {
                                            info!("Thread {} confirmed executed (exec_count: {} -> {})", 
                                                thread_pubkey, thread.exec_count, updated_thread.exec_count);
                                            return Ok(sig);
                                        }
                                    }
                                }
                                
                                attempts += 1;
                                if attempts < max_retries {
                                    warn!("Thread {} not confirmed, retrying ({}/{})", 
                                        thread_pubkey, attempts, max_retries);
                                }
                            } else {
                                // RPC submission already waits for confirmation
                                return Ok(sig);
                            }
                        }
                        Err(e) => {
                            error!("Failed to submit transaction for thread {}: {}", thread_pubkey, e);
                            attempts += 1;
                            
                            if attempts < max_retries {
                                tokio::time::sleep(Duration::from_millis(1000 * attempts as u64)).await;
                            }
                        }
                    }
                }
                
                
                Err(anyhow!("Failed to execute thread {} after {} attempts", thread_pubkey, max_retries))
            })
        }
    }
    
    /// Publish a durable transaction to NATS for replay
    #[allow(dead_code)]
    async fn publish_durable_transaction(&self, message: DurableTransactionMessage) -> Result<()> {
        self.submission_service.publish_durable_transaction(message).await
    }
}