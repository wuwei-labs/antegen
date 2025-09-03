use antegen_sdk::rpc::{CacheConfig, CachedRpcClient};
use anyhow::{anyhow, Result};
use crossbeam::channel::{Receiver, Sender};
use log::{debug, error, info, warn};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    pubkey::Pubkey,
    signature::{read_keypair_file, Keypair},
    signer::Signer,
};
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::clock::SharedClock;
use crate::executor::ExecutorLogic;
use crate::load_balancer::{LoadBalancer, ProcessDecision};
use crate::metrics::ProcessorMetrics;
use crate::parser::{classify_account, AccountType};
use crate::queue::{ThreadQueue, TriggerType};
use crate::types::{AccountUpdate, ExecutableThread, ProcessorConfig};
use antegen_sdk::ProcessorMessage;

/// Main processor service that handles thread processing
pub struct ProcessorService {
    /// Configuration
    #[allow(dead_code)]
    config: ProcessorConfig,
    /// RPC client for reading blockchain state
    #[allow(dead_code)]
    rpc_client: Arc<RpcClient>,
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
    /// Clock broadcaster for retry timing
    clock_broadcaster: broadcast::Sender<solana_sdk::clock::Clock>,
    /// Cached RPC client
    cached_rpc: Arc<CachedRpcClient>,
    /// Load balancer for distributed thread processing
    load_balancer: Arc<LoadBalancer>,
}

impl ProcessorService {
    /// Create a new processor service
    pub async fn new(
        config: ProcessorConfig,
        account_receiver: Receiver<AccountUpdate>,
        _transaction_sender: Sender<ProcessorMessage>, // Keep for compatibility, but unused
    ) -> Result<Self> {
        // Load executor keypair
        let executor_keypair = Arc::new(
            read_keypair_file(&config.executor_keypair_path)
                .map_err(|e| anyhow!("Failed to read executor keypair: {}", e))?,
        );

        // Create metrics
        let processor_metrics = Arc::new(ProcessorMetrics::default());

        // Create RPC client for reading state
        let rpc_client = Arc::new(RpcClient::new(config.rpc_url.clone()));

        // Create shared blockchain clock
        let clock = SharedClock::new();

        // Create a cached RPC client wrapper for the executor
        let cached_rpc = Arc::new(CachedRpcClient::new(
            RpcClient::new(config.rpc_url.clone()),
            CacheConfig::default(),
        ));

        // Create executor logic with shared clock
        let executor_logic = Arc::new(ExecutorLogic::new(
            executor_keypair.clone(),
            cached_rpc.clone(),
            clock.clone(),
            config.forgo_executor_commission,
            processor_metrics.clone(),
        ));

        // Create thread queue with metrics and shared clock
        let thread_queue = Arc::new(ThreadQueue::with_metrics(
            config.max_concurrent_threads,
            clock.clone(),
            processor_metrics.clone(),
        )?);

        // Create clock broadcaster for retry timing
        let (clock_broadcaster, _) = broadcast::channel(100);
        
        // Create load balancer with config settings
        let load_balancer = Arc::new(LoadBalancer::new(
            executor_keypair.pubkey(),
            config.load_balancer.clone(),
        ));

        Ok(Self {
            config,
            rpc_client,
            executor_logic,
            thread_queue,
            metrics: processor_metrics,
            executor_keypair,
            account_receiver,
            clock_broadcaster,
            cached_rpc,
            load_balancer,
        })
    }

    /// Start a task to continuously process threads as they become ready (event-driven)
    fn start_thread_processing<F, Fut>(&self, processor_fn: F)
    where
        F: Fn(Pubkey, antegen_thread_program::state::Thread) -> Fut + Send + Sync + 'static + Clone,
        Fut:
            std::future::Future<Output = Result<solana_sdk::signature::Signature>> + Send + 'static,
    {
        let queue = self.thread_queue.clone();
        tokio::spawn(async move {
            // Event-driven: blocks waiting for threads to execute
            queue.spawn_execution_tasks_continuous(processor_fn).await;
        });
    }

    /// Main processing loop (event-driven)
    pub async fn run(self) -> Result<()> {
        // Start the continuous event-driven thread processor
        let processor_fn = self.create_processor_fn();
        self.start_thread_processing(processor_fn);

        // Event-driven main loop - wrap blocking recv() in spawn_blocking
        loop {
            let receiver = self.account_receiver.clone();
            let account_update = tokio::task::spawn_blocking(move || receiver.recv()).await?;

            match account_update {
                Ok(account_update) => {
                    if let Err(e) = self.process_account_update(account_update).await {
                        error!("Error processing account update: {}", e);
                    }
                }
                Err(_) => {
                    warn!("Account receiver disconnected, shutting down");
                    break;
                }
            }
        }

        // Shutdown
        info!("Processor service shutting down");
        // ThreadQueue will be dropped and cleaned up automatically

        Ok(())
    }

    /// Process a single account update
    async fn process_account_update(&self, account_update: AccountUpdate) -> Result<()> {
        // Classify and process the account
        match classify_account(&account_update.pubkey, &account_update.account) {
            AccountType::Clock {
                unix_timestamp,
                slot,
                epoch,
            } => {
                info!(
                    "PROCESSOR: Clock update - slot: {}, epoch: {}, time: {}",
                    slot, epoch, unix_timestamp
                );

                // Always update cache with Clock
                self.cached_rpc
                    .update_account_selectively(
                        &account_update.pubkey,
                        account_update.account.clone(),
                    )
                    .await;

                // Update executor clock
                self.executor_logic
                    .update_clock(slot, epoch, unix_timestamp)
                    .await;

                // Broadcast Clock update for retry timing
                let clock = solana_sdk::clock::Clock {
                    slot,
                    epoch,
                    unix_timestamp,
                    epoch_start_timestamp: unix_timestamp - ((slot % 432000) as i64 * 400 / 1000),
                    leader_schedule_epoch: epoch,
                };
                let _ = self.clock_broadcaster.send(clock);

                // Check timestamp-based triggers (execution happens automatically)
                self.thread_queue
                    .check_single_trigger(TriggerType::Time, unix_timestamp as u64)
                    .await;
            }
            AccountType::Thread(thread) => {
                // Check if we should process this Thread update based on exec_count
                if !self
                    .cached_rpc
                    .should_process_thread(&account_update.pubkey, &thread)
                    .await
                {
                    // Already processed this exec_count - skip
                    debug!(
                        "Skipping duplicate Thread update for {} (exec_count: {})",
                        account_update.pubkey, thread.exec_count
                    );
                    return Ok(());
                }

                // Update cache with the new Thread account
                self.cached_rpc
                    .update_account_selectively(
                        &account_update.pubkey,
                        account_update.account.clone(),
                    )
                    .await;
                
                // Check with load balancer if we should process this thread
                let current_time = self.executor_logic.current_timestamp().await;
                let (is_overdue, overdue_seconds) = self.calculate_overdue(&thread, current_time);
                
                let decision = self.load_balancer.should_process(
                    &account_update.pubkey,
                    &thread.last_executor,
                    is_overdue,
                    overdue_seconds,
                ).await?;
                
                match decision {
                    ProcessDecision::Process => {
                        // Schedule thread for processing
                        if let Err(e) = self
                            .thread_queue
                            .schedule_thread(account_update.pubkey, thread)
                            .await
                        {
                            warn!("Failed to schedule thread {}: {}", account_update.pubkey, e);
                        }
                    }
                    ProcessDecision::Skip => {
                        debug!("Load balancer: skipping thread {} (owned by another processor)", account_update.pubkey);
                    }
                    ProcessDecision::AtCapacity => {
                        debug!("Load balancer: at capacity, skipping non-critical thread {}", account_update.pubkey);
                    }
                }
            }
            AccountType::Other => {
                // Update cache for other accounts (if already cached)
                self.cached_rpc
                    .update_account_selectively(
                        &account_update.pubkey,
                        account_update.account.clone(),
                    )
                    .await;
            }
        }

        Ok(())
    }

    /// Create the processor function for thread execution
    fn create_processor_fn(
        &self,
    ) -> impl Fn(
        Pubkey,
        antegen_thread_program::state::Thread,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<solana_sdk::signature::Signature>> + Send>,
    > + Send
           + Sync
           + Clone {
        let executor = self.executor_logic.clone();
        let rpc_url = self.config.rpc_url.clone();
        let executor_keypair = self.executor_keypair.clone();
        let clock_broadcaster = self.clock_broadcaster.clone();
        let load_balancer = self.load_balancer.clone();

        move |thread_pubkey, thread| {
            let executor = executor.clone();
            let rpc_url = rpc_url.clone();
            let executor_keypair = executor_keypair.clone();
            let clock_rx = clock_broadcaster.subscribe();
            let load_balancer = load_balancer.clone();

            Box::pin(async move {
                let blockchain_time = executor.current_timestamp().await;
                info!(
                    "PROCESSOR: Building transaction for thread {} at blockchain time {}",
                    thread_pubkey, blockchain_time
                );

                // Build instructions
                let executable = ExecutableThread {
                    thread_pubkey,
                    thread: thread.clone(),
                    slot: 0, // Current slot, could be passed in if needed
                };
                let (instructions, priority_fee) = executor
                    .build_execute_transaction(
                        &executable,
                        None, // fiber state
                        None, // compute units (let submitter optimize)
                    )
                    .await?;

                // Create TransactionSubmitter for this task
                let submitter = antegen_submitter::TransactionSubmitter::new(
                    rpc_url,
                    None, // TPU config - could add to ProcessorConfig if needed
                    Arc::new(antegen_submitter::SubmitterMetrics::default()),
                    clock_rx,
                )?;

                // Initialize TPU if configured
                if let Err(e) = submitter.initialize_tpu().await {
                    debug!("Failed to initialize TPU: {}", e);
                    // Continue with RPC submission
                }

                // Submit with honeybadger retry (blocks until timeout)
                info!(
                    "Starting honeybadger submission for thread {}",
                    thread_pubkey
                );
                
                // Attempt submission and track result
                let current_time = executor.current_timestamp().await;
                submitter.submit(instructions, executor_keypair, Some(priority_fee)).await.map_err(|e| {
                    // Record failed execution (someone else likely beat us)
                    let balancer = load_balancer.clone();
                    let thread_pk = thread_pubkey.clone();
                    tokio::spawn(async move {
                        let _ = balancer.record_execution_result(
                            &thread_pk,
                            false,
                            current_time,
                        ).await;
                    });
                    e
                })?;
                
                // Record successful execution
                let _ = load_balancer.record_execution_result(
                    &thread_pubkey,
                    true,
                    current_time,
                ).await;
                
                // This returns on timeout (error) - submitter doesn't return signature
                Err(anyhow!("Submission timed out for thread {}", thread_pubkey))
            })
        }
    }
    
    /// Calculate if a thread is overdue and by how much
    fn calculate_overdue(&self, thread: &antegen_thread_program::state::Thread, current_timestamp: i64) -> (bool, i64) {
        use antegen_thread_program::state::TriggerContext;
        
        match &thread.trigger_context {
            TriggerContext::Timestamp { next, .. } => {
                let overdue = current_timestamp > *next;
                let overdue_seconds = if overdue {
                    current_timestamp - *next
                } else {
                    0
                };
                (overdue, overdue_seconds)
            }
            TriggerContext::Block { .. } => {
                // For block-based triggers, we'd need current slot/epoch
                // For now, consider not overdue
                (false, 0)
            }
            TriggerContext::Account { .. } => {
                // Account triggers are event-driven, not time-based
                (false, 0)
            }
        }
    }
}
