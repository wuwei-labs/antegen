use antegen_sdk::rpc::{CacheConfig, CachedRpcClient};
use anyhow::{anyhow, Result};
use log::{debug, error, info, warn};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    signature::{read_keypair_file, Keypair},
    signer::Signer,
};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Semaphore};

use crate::clock::SharedClock;
use crate::executor::ExecutorLogic;
use crate::load_balancer::{LoadBalancer, ProcessDecision};
use crate::metrics::ProcessorMetrics;
use crate::parser::{classify_account, AccountType};
use crate::queue::ThreadQueue;
use crate::types::{AccountUpdate, ExecutableThread, ProcessorConfig};

/// Main processor service that handles thread processing
pub struct ProcessorService {
    /// Executor logic for building transactions
    executor_logic: Arc<ExecutorLogic>,
    /// Thread queue for scheduling
    thread_queue: Arc<ThreadQueue>,
    /// Executor keypair
    executor_keypair: Arc<Keypair>,
    /// Account update receiver from adapter
    account_receiver: mpsc::Receiver<AccountUpdate>,
    /// Clock broadcaster for retry timing
    clock_broadcaster: broadcast::Sender<solana_sdk::clock::Clock>,
    /// Cached RPC client
    cached_rpc: Arc<CachedRpcClient>,
    /// Load balancer for distributed thread processing
    load_balancer: Arc<LoadBalancer>,
    /// Shared transaction submitter
    submitter: Arc<antegen_submitter::TransactionSubmitter>,
    /// Semaphore to limit concurrent thread executions
    task_semaphore: Arc<Semaphore>,
}

impl ProcessorService {
    /// Create a new processor service
    pub async fn new(
        config: ProcessorConfig,
        account_receiver: mpsc::Receiver<AccountUpdate>,
    ) -> Result<Self> {
        // Load executor keypair
        let executor_keypair = Arc::new(
            read_keypair_file(&config.executor_keypair_path)
                .map_err(|e| anyhow!("Failed to read executor keypair: {}", e))?,
        );

        // Create metrics
        let processor_metrics = Arc::new(ProcessorMetrics::default());

        // Create shared blockchain clock
        let clock = SharedClock::new();

        // Create a cached RPC client wrapper for the executor
        let cached_rpc = Arc::new(CachedRpcClient::new(
            RpcClient::new_with_commitment(config.rpc_url.clone(), CommitmentConfig::confirmed()),
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

        // Create thread queue with metrics
        let thread_queue = Arc::new(ThreadQueue::with_metrics(
            processor_metrics.clone(),
        ));

        // Create clock broadcaster for retry timing
        let (clock_broadcaster, clock_rx) = broadcast::channel(100);
        
        // Create load balancer with config settings
        let load_balancer = Arc::new(LoadBalancer::new(
            executor_keypair.pubkey(),
            config.load_balancer.clone(),
        ));
        
        // Create shared transaction submitter
        let submitter = Arc::new(antegen_submitter::TransactionSubmitter::new(
            config.rpc_url.clone(),
            None, // TPU config could be added to ProcessorConfig
            Arc::new(antegen_submitter::SubmitterMetrics::default()),
            clock_rx,
        )?);
        
        // Initialize TPU if available
        if let Err(e) = submitter.initialize_tpu().await {
            debug!("TPU initialization failed (will use RPC): {}", e);
        }
        
        // Create semaphore for limiting concurrent thread executions
        let task_semaphore = Arc::new(Semaphore::new(config.max_concurrent_threads));
        info!("Initialized with max {} concurrent thread executions", config.max_concurrent_threads);

        Ok(Self {
            executor_logic,
            thread_queue,
            executor_keypair,
            account_receiver,
            clock_broadcaster,
            cached_rpc,
            load_balancer,
            submitter,
            task_semaphore,
        })
    }

    /// Main processing loop (event-driven)
    pub async fn run(mut self) -> Result<()> {
        info!("Starting processor service");
        
        // Event-driven main loop - now fully async
        loop {
            match self.account_receiver.recv().await {
                Some(account_update) => {
                    if let Err(e) = self.process_account_update(account_update).await {
                        error!("Error processing account update: {}", e);
                    }
                }
                None => {
                    info!("Account receiver disconnected, shutting down");
                    break;
                }
            }
        }

        // Shutdown
        info!("Processor service shutting down");
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
                debug!(
                    "Clock update - slot: {}, epoch: {}, time: {}",
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

                // Check for ready threads and execute them
                debug!("Checking for ready threads at time {}", unix_timestamp);
                let ready_threads = self.thread_queue
                    .get_ready_threads(unix_timestamp, slot, epoch)
                    .await;
                
                if ready_threads.is_empty() {
                    debug!("0 threads ready for execution");
                } else {
                    debug!("{} threads ready for execution", ready_threads.len());
                }
                
                // Spawn execution tasks for ready threads
                for thread in ready_threads {
                    self.spawn_thread_execution(thread).await;
                }
            }
            AccountType::Thread(thread) => {
                info!("Thread update for {} - fibers: {}, exec_count: {}", 
                     account_update.pubkey, thread.fibers.len(), thread.exec_count);
                
                // Always check if there's an active task for this thread and abort it
                // This handles cases where the thread was already executed elsewhere
                self.thread_queue.abort_task_if_active(&account_update.pubkey);
                
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
                
                debug!("Load balancer decision for thread {}: {:?}", 
                     account_update.pubkey, decision);
                
                match decision {
                    ProcessDecision::Process => {
                        // Check if thread is paused
                        if thread.paused {
                            debug!("Thread {} is paused, skipping scheduling", account_update.pubkey);
                            return Ok(());
                        }
                        
                        // Check if thread has fibers before scheduling
                        if thread.fibers.is_empty() {
                            debug!("Thread {} has no fibers, waiting for update with fibers", account_update.pubkey);
                            return Ok(());
                        }
                        
                        // Schedule thread for processing
                        debug!("Scheduling thread {} for processing (has {} fibers)", 
                             account_update.pubkey, thread.fibers.len());
                        if let Err(e) = self
                            .thread_queue
                            .schedule_thread(account_update.pubkey, thread)
                            .await
                        {
                            warn!("Failed to schedule thread {}: {}", account_update.pubkey, e);
                        } else {
                            debug!("Successfully scheduled thread {}", account_update.pubkey);
                        }
                    }
                    ProcessDecision::Skip => {
                        debug!("Load balancer skipping thread {} (owned by another processor)", account_update.pubkey);
                    }
                    ProcessDecision::AtCapacity => {
                        debug!("Load balancer at capacity, skipping thread {}", account_update.pubkey);
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

    /// Spawn an atomic task to execute a thread
    async fn spawn_thread_execution(&self, executable: ExecutableThread) {
        let thread_pubkey = executable.thread_pubkey;
        let thread = executable.thread;
        
        // Check if thread has fibers before spawning task
        if thread.fibers.is_empty() {
            debug!("Thread {} has no fibers yet, skipping execution", thread_pubkey);
            return;
        }
        
        // Clone resources for the task
        let executor = self.executor_logic.clone();
        let submitter = self.submitter.clone();
        let executor_keypair = self.executor_keypair.clone();
        let load_balancer = self.load_balancer.clone();
        let queue = self.thread_queue.clone();
        let semaphore = self.task_semaphore.clone();
        
        // Spawn atomic execution task
        let handle = tokio::spawn(async move {
            // Acquire permit before executing
            let _permit = match semaphore.acquire().await {
                Ok(permit) => permit,
                Err(_) => {
                    error!("Failed to acquire semaphore permit for thread {}", thread_pubkey);
                    queue.task_completed(&thread_pubkey);
                    return;
                }
            };
            
            debug!("Acquired execution permit for thread {}", thread_pubkey);
            info!("Starting execution task for thread {}", thread_pubkey);
            
            // Build transaction
            let blockchain_time = executor.current_timestamp().await;
            debug!("Building transaction for thread {} at time {}", 
                 thread_pubkey, blockchain_time);
            
            let executable = ExecutableThread {
                thread_pubkey,
                thread: thread.clone(),
                slot: 0, // Could be passed if needed
            };
            
            match executor.build_execute_transaction(&executable, None, None).await {
                Ok((instructions, priority_fee)) => {
                    debug!("Built {} instructions with priority fee {} for thread {}",
                          instructions.len(), priority_fee, thread_pubkey);
                    
                    // Submit with honeybadger retry
                    debug!("Starting submission for thread {}", thread_pubkey);
                    match submitter.submit(
                        instructions,
                        executor_keypair,
                        Some(priority_fee)
                    ).await {
                        Ok(_) => {
                            info!("Successfully submitted thread {}", thread_pubkey);
                            let _ = load_balancer.record_execution_result(
                                &thread_pubkey,
                                true,
                                blockchain_time,
                            ).await;
                        }
                        Err(e) => {
                            error!("Failed to submit thread {}: {}", thread_pubkey, e);
                            let _ = load_balancer.record_execution_result(
                                &thread_pubkey,
                                false,
                                blockchain_time,
                            ).await;
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to build transaction for thread {}: {}", 
                           thread_pubkey, e);
                }
            }
            
            // Remove from active tasks
            queue.task_completed(&thread_pubkey);
            info!("Completed execution task for thread {}", thread_pubkey);
        });
        
        // Track the active task
        self.thread_queue.track_task(thread_pubkey, handle);
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