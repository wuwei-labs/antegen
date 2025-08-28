use anyhow::{anyhow, Result};
use crossbeam::channel::{Receiver, Sender};
use solana_client::nonblocking::rpc_client::RpcClient;
use log::{debug, error, info, warn};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{read_keypair_file, Keypair},
};
use std::sync::Arc;
use antegen_sdk::rpc::{CachedRpcClient, CacheConfig};

use crate::clock::SharedClock;
use crate::parser::{classify_account, AccountType};
use crate::executor::ExecutorLogic;
use crate::queue::{ThreadQueue, TriggerType};
use crate::types::{ProcessorConfig, AccountUpdate, ExecutableThread};
use crate::metrics::ProcessorMetrics;
use antegen_sdk::types::TransactionMessage;

/// Main processor service that handles thread processing
pub struct ProcessorService {
    /// Configuration
    #[allow(dead_code)]
    config: ProcessorConfig,
    /// Shared blockchain clock
    clock: SharedClock,
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
    /// Transaction sender to submitter
    transaction_sender: Sender<TransactionMessage>,
    /// Cached RPC client
    cached_rpc: Arc<CachedRpcClient>,
}

impl ProcessorService {
    /// Create a new processor service
    pub async fn new(
        config: ProcessorConfig,
        account_receiver: Receiver<AccountUpdate>,
        transaction_sender: Sender<TransactionMessage>,
    ) -> Result<Self> {
        // Load executor keypair
        let executor_keypair = Arc::new(
            read_keypair_file(&config.executor_keypair_path)
                .map_err(|e| anyhow!("Failed to read executor keypair: {}", e))?
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
            CacheConfig::default()
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
        let thread_queue = Arc::new(
            ThreadQueue::with_metrics(
                config.max_concurrent_threads,
                clock.clone(),
                processor_metrics.clone(),
            )?
        );
        
        Ok(Self {
            config,
            clock,
            rpc_client,
            executor_logic,
            thread_queue,
            metrics: processor_metrics,
            executor_keypair,
            account_receiver,
            transaction_sender,
            cached_rpc,
        })
    }
    
    /// Start a task to continuously process threads as they become ready (event-driven)
    fn start_thread_processing<F, Fut>(&self, processor_fn: F)
    where
        F: Fn(Pubkey, antegen_thread_program::state::Thread) -> Fut + Send + Sync + 'static + Clone,
        Fut: std::future::Future<Output = Result<solana_sdk::signature::Signature>> + Send + 'static,
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
            let account_update = tokio::task::spawn_blocking(move || {
                receiver.recv()
            }).await?;
            
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
            AccountType::Clock { unix_timestamp, slot, epoch } => {
                info!("PROCESSOR: Clock update - slot: {}, epoch: {}, time: {}", slot, epoch, unix_timestamp);
                
                // Always update cache with Clock
                self.cached_rpc.update_account_selectively(
                    &account_update.pubkey,
                    account_update.account.clone(),
                ).await;
                
                // Update executor clock
                self.executor_logic.update_clock(slot, epoch, unix_timestamp).await;
                
                // Check timestamp-based triggers (execution happens automatically)
                self.thread_queue.check_single_trigger(
                    TriggerType::Time,
                    unix_timestamp as u64,
                ).await;
            }
            AccountType::Thread(thread) => {
                // Check if we should process this Thread update based on exec_count
                if !self.cached_rpc
                    .should_process_thread(&account_update.pubkey, &thread)
                    .await 
                {
                    // Already processed this exec_count - skip
                    debug!("Skipping duplicate Thread update for {} (exec_count: {})", 
                        account_update.pubkey, thread.exec_count);
                    return Ok(());
                }
                
                // Update cache with the new Thread account
                self.cached_rpc.update_account_selectively(
                    &account_update.pubkey,
                    account_update.account.clone(),
                ).await;
                
                // Schedule thread for processing
                if let Err(e) = self.thread_queue.schedule_thread(
                    account_update.pubkey,
                    thread,
                ).await {
                    warn!("Failed to schedule thread {}: {}", account_update.pubkey, e);
                }
            }
            AccountType::Other => {
                // Update cache for other accounts (if already cached)
                self.cached_rpc.update_account_selectively(
                    &account_update.pubkey,
                    account_update.account.clone(),
                ).await;
            }
        }
        
        Ok(())
    }
    
    /// Create the processor function for thread execution
    fn create_processor_fn(&self) -> impl Fn(Pubkey, antegen_thread_program::state::Thread) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<solana_sdk::signature::Signature>> + Send>> + Send + Sync + Clone {
        let executor = self.executor_logic.clone();
        let transaction_sender = self.transaction_sender.clone();
        
        move |thread_pubkey, thread| {
            let executor = executor.clone();
            let transaction_sender = transaction_sender.clone();
            
            Box::pin(async move {
                let blockchain_time = executor.current_timestamp().await;
                info!("PROCESSOR: Building transaction for thread {} at blockchain time {}",
                    thread_pubkey, blockchain_time);
                
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
                
                // Create transaction message and send to submitter
                let transaction_msg = TransactionMessage {
                    instructions,
                    thread_pubkey,
                    executor_pubkey: executor.pubkey(),
                    priority_fee: None, // Let submitter decide
                    compute_units: None, // Let submitter optimize
                };
                
                transaction_sender.send(transaction_msg)
                    .map_err(|e| anyhow!("Failed to send transaction to submitter: {}", e))?;
                
                info!("Transaction for thread {} sent to submitter", thread_pubkey);
                
                // Return a dummy signature since we're not actually submitting
                // The real signature will be created by the submitter
                Ok(solana_sdk::signature::Signature::default())
            })
        }
    }
    
}