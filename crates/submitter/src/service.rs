use anyhow::{anyhow, Result};
use log::{debug, error, info, warn};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    pubkey::Pubkey,
    signature::{read_keypair_file, Signature},
    signer::Signer,
    transaction::Transaction,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinHandle;

use antegen_thread_program::state::Thread;
use crate::executor::ExecutorLogic;
use crate::queue::ThreadQueue;
use crate::replay::ReplayConsumer;
use crate::{
    ClockUpdate, DurableTransactionMessage, ExecutableThread, SubmitterConfig, SubmitterMode,
    TransactionSubmitter,
};

/// Main submitter service that handles both local submission and NATS replay
pub struct SubmitterService {
    /// Operational mode
    mode: SubmitterMode,
    /// Core transaction submitter
    submitter: Arc<TransactionSubmitter>,
    /// RPC client for status checks
    rpc_client: Arc<RpcClient>,
    /// NATS client for publishing (optional)
    nats_client: Option<async_nats::Client>,
    /// Configuration
    config: SubmitterConfig,
    /// Replay consumer handle (when enabled)
    replay_handle: Option<JoinHandle<Result<()>>>,
    /// Executor logic (only in full mode)
    executor_logic: Option<ExecutorLogic>,
    /// Thread queue (only in full mode)
    thread_queue: Option<ThreadQueue>,
    /// Clock update receiver (for full mode)
    clock_receiver: Option<Receiver<ClockUpdate>>,
    /// Track last processed values to avoid redundant processing
    last_processed_timestamp: i64,
    last_processed_slot: u64,
    last_processed_epoch: u64,
}

impl SubmitterService {
    /// Create a new submitter service (auto-detects mode from config)
    pub async fn new(config: SubmitterConfig) -> Result<Self> {
        // Create RPC client
        let rpc_client = Arc::new(RpcClient::new(config.rpc_url.clone()));
        // Determine mode based on config
        let (mode, executor_logic) = if let Some(keypair_path) = &config.executor_keypair_path {
            // Full mode - load executor keypair
            info!("Initializing in FULL mode with executor functionality");
            let keypair = Arc::new(
                read_keypair_file(keypair_path)
                    .map_err(|e| anyhow!("Failed to read keypair file: {}", e))?,
            );
            info!("Loaded executor keypair: {}", keypair.pubkey());

            let executor_logic = ExecutorLogic::new(
                keypair.clone(),
                rpc_client.clone(),
                config.forgo_executor_commission,
            );

            let mode = SubmitterMode::Full {
                executor_keypair: keypair,
                thread_receiver: None, // Will be set later via set_thread_receiver
            };

            (mode, Some(executor_logic))
        } else {
            // Replay-only mode
            info!("Initializing in REPLAY-ONLY mode");
            (SubmitterMode::ReplayOnly, None)
        };

        // Create the core transaction submitter with TPU config
        let submitter = Arc::new(
            TransactionSubmitter::new(rpc_client.clone(), config.tpu_config.clone()).await?,
        );

        // Connect to NATS if configured
        let nats_client = if let Some(nats_url) = &config.nats_url {
            info!("Connecting to NATS server: {}", nats_url);
            Some(async_nats::connect(nats_url).await?)
        } else {
            info!("No NATS URL configured, skipping NATS connection");
            None
        };

        // Create thread queue if in full mode
        let thread_queue = if matches!(mode, SubmitterMode::Full { .. }) {
            info!("Initializing ephemeral thread queue with max {} concurrent threads", 
                  config.max_concurrent_threads);
            Some(ThreadQueue::new(config.max_concurrent_threads)?)
        } else {
            None
        };

        Ok(Self {
            mode,
            submitter,
            rpc_client,
            nats_client,
            config,
            replay_handle: None,
            executor_logic,
            thread_queue,
            clock_receiver: None,
            last_processed_timestamp: 0,
            last_processed_slot: 0,
            last_processed_epoch: 0,
        })
    }

    /// Start the service (including optional replay consumer)
    pub async fn start(&mut self) -> Result<()> {
        info!(
            "Starting submitter service (replay: {})",
            self.config.enable_replay
        );

        // Start replay consumer if enabled and NATS is available
        if self.config.enable_replay {
            if let Some(nats_client) = &self.nats_client {
                info!("Starting replay consumer");
                let mut replay_consumer = ReplayConsumer::new(
                    nats_client.clone(),
                    self.submitter.clone(),
                    self.rpc_client.clone(),
                    self.config.clone(),
                )
                .await?;

                let handle = tokio::spawn(async move { replay_consumer.run().await });

                self.replay_handle = Some(handle);
                info!("Replay consumer started");
            } else {
                warn!("Replay enabled but no NATS client available");
            }
        }

        Ok(())
    }

    /// Start replay consumer without requiring mutable self (for Arc usage)
    pub async fn start_replay_consumer(
        &self,
    ) -> Result<Option<tokio::task::JoinHandle<Result<()>>>> {
        info!(
            "Starting submitter service (replay: {})",
            self.config.enable_replay
        );

        // Start replay consumer if enabled and NATS is available
        if self.config.enable_replay {
            if let Some(nats_client) = &self.nats_client {
                info!("Starting replay consumer");
                let mut replay_consumer = ReplayConsumer::new(
                    nats_client.clone(),
                    self.submitter.clone(),
                    self.rpc_client.clone(),
                    self.config.clone(),
                )
                .await?;

                let handle = tokio::spawn(async move { replay_consumer.run().await });

                info!("Replay consumer started");
                return Ok(Some(handle));
            } else {
                warn!("Replay enabled but no NATS client available");
            }
        }

        Ok(None)
    }

    /// Submit a transaction (primary interface used by executor)
    pub async fn submit(&self, tx: &Transaction) -> Result<Signature> {
        // Submit the transaction
        let signature = self.submitter.submit(tx).await?;

        // If this is a durable transaction and NATS is available, publish for replay
        if self.submitter.is_durable_transaction(tx) {
            if self.nats_client.is_some() {
                if let Err(e) = self.publish_for_replay(tx, &signature).await {
                    // Log error but don't fail the submission
                    error!("Failed to publish durable transaction to NATS: {}", e);
                }
            }
        }

        Ok(signature)
    }

    /// Submit with retries
    pub async fn submit_with_retries(
        &self,
        tx: &Transaction,
        max_retries: u32,
    ) -> Result<Signature> {
        // Submit the transaction with retries
        let signature = self.submitter.submit_with_retries(tx, max_retries).await?;

        // If this is a durable transaction and NATS is available, publish for replay
        if self.submitter.is_durable_transaction(tx) {
            if self.nats_client.is_some() {
                if let Err(e) = self.publish_for_replay(tx, &signature).await {
                    // Log error but don't fail the submission
                    error!("Failed to publish durable transaction to NATS: {}", e);
                }
            }
        }

        Ok(signature)
    }

    /// Publish a durable transaction to NATS for potential replay
    async fn publish_for_replay(&self, tx: &Transaction, signature: &Signature) -> Result<()> {
        let nats_client = self
            .nats_client
            .as_ref()
            .ok_or_else(|| anyhow!("No NATS client"))?;

        // Serialize transaction to base64
        use base64::Engine;
        let tx_bytes = bincode::serialize(tx)?;
        let base64_tx = base64::engine::general_purpose::STANDARD.encode(&tx_bytes);

        // Extract thread pubkey from transaction (this is a simplification)
        // In reality, we'd need to parse the thread execution instruction
        let thread_pubkey = if let Some(account) = tx.message.account_keys.get(2) {
            account.to_string()
        } else {
            "unknown".to_string()
        };

        // Create message
        let message = DurableTransactionMessage::new(
            base64_tx,
            thread_pubkey,
            signature.to_string(),
            tx.message
                .account_keys
                .first()
                .map(|k| k.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
        );

        // Publish to NATS
        let subject = "antegen.durable_txs";
        let payload = serde_json::to_vec(&message)?;

        nats_client
            .publish(subject, payload.into())
            .await
            .map_err(|e| anyhow!("Failed to publish to NATS: {}", e))?;

        debug!(
            "Published durable transaction {} to NATS for replay",
            signature
        );
        Ok(())
    }

    /// Shutdown the service gracefully
    pub async fn shutdown(&mut self) -> Result<()> {
        info!("Shutting down submitter service");

        // Stop replay consumer if running
        if let Some(handle) = self.replay_handle.take() {
            info!("Stopping replay consumer");
            handle.abort();

            // Wait a bit for cleanup
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        info!("Submitter service shutdown complete");
        Ok(())
    }

    /// Set the thread receiver (for full mode)
    pub fn set_thread_receiver(&mut self, receiver: Receiver<ExecutableThread>) -> Result<()> {
        match &mut self.mode {
            SubmitterMode::Full {
                thread_receiver, ..
            } => {
                *thread_receiver = Some(receiver);
                Ok(())
            }
            SubmitterMode::ReplayOnly => {
                Err(anyhow!("Cannot set thread receiver in replay-only mode"))
            }
        }
    }
    
    /// Set the clock receiver (for full mode)
    pub fn set_clock_receiver(&mut self, receiver: Receiver<ClockUpdate>) -> Result<()> {
        match &self.mode {
            SubmitterMode::Full { .. } => {
                self.clock_receiver = Some(receiver);
                Ok(())
            }
            SubmitterMode::ReplayOnly => {
                Err(anyhow!("Cannot set clock receiver in replay-only mode"))
            }
        }
    }

    /// Main run loop
    pub async fn run(&mut self) -> Result<()> {
        match &self.mode {
            SubmitterMode::Full { .. } => {
                info!("SUBMITTER: Starting in FULL mode with executor functionality");
                self.run_full_mode().await
            }
            SubmitterMode::ReplayOnly => {
                info!("SUBMITTER: Starting in REPLAY-ONLY mode");
                self.run_replay_only().await
            }
        }
    }

    /// Run in full mode with executor functionality
    async fn run_full_mode(&mut self) -> Result<()> {
        // Initialize the submitter (wait for RPC and setup TPU)
        info!("SUBMITTER: Initializing connection to RPC server...");
        self.submitter.initialize().await?;
        info!("SUBMITTER: Initialization complete, RPC and TPU connections established");

        // Start replay consumer if enabled
        if self.config.enable_replay {
            self.start().await?;
        }

        // Get thread receiver
        let mut thread_rx = match &mut self.mode {
            SubmitterMode::Full {
                thread_receiver, ..
            } => thread_receiver
                .take()
                .ok_or_else(|| anyhow!("No thread receiver set for full mode"))?,
            _ => unreachable!(),
        };
        
        // Get clock receiver
        let mut clock_rx = self.clock_receiver
            .take()
            .ok_or_else(|| anyhow!("No clock receiver set for full mode"))?;

        info!("SUBMITTER: Entering main loop, waiting for events...");

        loop {
            tokio::select! {
                // Handle thread events
                Some(executable) = thread_rx.recv() => {
                    info!(
                        "SUBMITTER: Received executable thread {} from observer",
                        executable.thread_pubkey
                    );

                    // Schedule thread in queue
                    if let Some(queue) = &self.thread_queue {
                        if let Err(e) = queue.schedule_thread(executable.thread_pubkey, executable.thread) {
                            error!("SUBMITTER: Failed to schedule thread: {}", e);
                        }
                    }
                }
                
                // Handle clock updates
                Some(clock) = clock_rx.recv() => {
                    // Check what has changed since last processing
                    let timestamp_changed = clock.unix_timestamp != self.last_processed_timestamp;
                    let slot_changed = clock.slot != self.last_processed_slot;
                    let epoch_changed = clock.epoch != self.last_processed_epoch;
                    
                    // Skip if nothing has changed
                    if !timestamp_changed && !slot_changed && !epoch_changed {
                        debug!(
                            "SUBMITTER: Skipping duplicate clock update - slot: {}, epoch: {}, timestamp: {}",
                            clock.slot, clock.epoch, clock.unix_timestamp
                        );
                        continue;
                    }
                    
                    info!(
                        "SUBMITTER: Processing clock update - slot: {} ({}), epoch: {} ({}), timestamp: {} ({})",
                        clock.slot,
                        if slot_changed { "changed" } else { "unchanged" },
                        clock.epoch,
                        if epoch_changed { "changed" } else { "unchanged" },
                        clock.unix_timestamp,
                        if timestamp_changed { "changed" } else { "unchanged" }
                    );
                    
                    // Update last processed values
                    self.last_processed_timestamp = clock.unix_timestamp;
                    self.last_processed_slot = clock.slot;
                    self.last_processed_epoch = clock.epoch;
                    
                    // Process only the queues for values that changed
                    if let Err(e) = self.process_ready_threads_selective(
                        clock.slot,
                        clock.epoch, 
                        clock.unix_timestamp,
                        timestamp_changed,
                        slot_changed,
                        epoch_changed
                    ).await {
                        error!("SUBMITTER: Failed to process ready threads on clock update: {}", e);
                    }
                }
                
                // Both channels closed
                else => {
                    error!("SUBMITTER: All channels disconnected");
                    return Err(anyhow!("Observer channels disconnected"));
                }
            }
        }
    }

    /// Run in replay-only mode
    async fn run_replay_only(&mut self) -> Result<()> {
        if !self.config.enable_replay {
            return Err(anyhow!("Replay not enabled in configuration"));
        }

        // Initialize the submitter (wait for RPC, even in replay mode we need RPC to submit)
        info!("SUBMITTER: Initializing connection to RPC server...");
        self.submitter.initialize().await?;
        info!("SUBMITTER: Initialization complete, RPC connection established");

        // Start replay consumer
        self.start().await?;

        info!("SUBMITTER: Running in replay-only mode, processing NATS messages...");

        // Keep the service running
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;

            // Could add health checks or status reporting here
            debug!("SUBMITTER: Replay service running...");
        }
    }

    
    /// Process threads that are ready with specific clock values
    async fn process_ready_threads_with_clock(&self, slot: u64, epoch: u64, timestamp: i64) -> Result<()> {
        let queue = self.thread_queue.as_ref()
            .ok_or_else(|| anyhow!("No thread queue in full mode"))?;
        
        // Clone what we need for the closure
        let executor = self.executor_logic.as_ref()
            .ok_or_else(|| anyhow!("No executor logic in full mode"))?
            .clone();
        let submitter = self.submitter.clone();
        let simulate_before_submit = self.config.simulate_before_submit;
        let compute_unit_multiplier = self.config.compute_unit_multiplier;
        let max_compute_units = self.config.max_compute_units;
        
        // Create the processing function that will be called for each ready thread
        let process_fn = move |thread_pubkey: Pubkey, thread: Thread| {
            let executor = executor.clone();
            let submitter = submitter.clone();
            
            async move {
                info!("SUBMITTER: Processing thread {}", thread_pubkey);
                
                // Create ExecutableThread for compatibility
                let executable = ExecutableThread {
                    thread_pubkey,
                    thread: thread.clone(),
                    slot,
                };
                
                // Build transaction
                let tx = if simulate_before_submit {
                    info!("SUBMITTER: Simulating transaction for thread {}", thread_pubkey);
                    executor.simulate_and_optimize_transaction(
                        &executable,
                        compute_unit_multiplier,
                        max_compute_units,
                    ).await?
                } else {
                    debug!("SUBMITTER: Building transaction for thread {} without simulation", thread_pubkey);
                    executor.build_execute_transaction(&executable).await?
                };
                
                // Submit transaction
                let signature = submitter.submit(&tx).await?;
                
                info!("SUBMITTER: Successfully submitted thread {} with signature {}", 
                    thread_pubkey, signature);
                
                Ok(signature)
            }
        };
        
        // Process all ready threads (spawns async tasks)
        queue.process_threads(slot, epoch, timestamp, process_fn).await;
        
        Ok(())
    }

    /// Process threads selectively based on what clock values changed
    async fn process_ready_threads_selective(
        &self,
        slot: u64,
        epoch: u64,
        timestamp: i64,
        timestamp_changed: bool,
        slot_changed: bool,
        epoch_changed: bool,
    ) -> Result<()> {
        let queue = self.thread_queue.as_ref()
            .ok_or_else(|| anyhow!("No thread queue in full mode"))?;
        
        // Clone what we need for the closure
        let executor = self.executor_logic.as_ref()
            .ok_or_else(|| anyhow!("No executor logic in full mode"))?
            .clone();
        let submitter = self.submitter.clone();
        let simulate_before_submit = self.config.simulate_before_submit;
        let compute_unit_multiplier = self.config.compute_unit_multiplier;
        let max_compute_units = self.config.max_compute_units;
        
        // Create the processing function that will be called for each ready thread
        let process_fn = move |thread_pubkey: Pubkey, thread: Thread| {
            let executor = executor.clone();
            let submitter = submitter.clone();
            
            async move {
                info!("SUBMITTER: Processing thread {}", thread_pubkey);
                
                // Create ExecutableThread for compatibility
                let executable = ExecutableThread {
                    thread_pubkey,
                    thread: thread.clone(),
                    slot,
                };
                
                // Build transaction
                let tx = if simulate_before_submit {
                    info!("SUBMITTER: Simulating transaction for thread {}", thread_pubkey);
                    executor.simulate_and_optimize_transaction(
                        &executable,
                        compute_unit_multiplier,
                        max_compute_units,
                    ).await?
                } else {
                    debug!("SUBMITTER: Building transaction for thread {} without simulation", thread_pubkey);
                    executor.build_execute_transaction(&executable).await?
                };
                
                // Submit transaction
                let signature = submitter.submit(&tx).await?;
                
                info!("SUBMITTER: Successfully submitted thread {} with signature {}", 
                    thread_pubkey, signature);
                
                Ok(signature)
            }
        };
        
        // Only process the queues for values that actually changed
        if timestamp_changed {
            info!("SUBMITTER: Checking time-triggered threads (timestamp: {})", timestamp);
            queue.process_time_queue(timestamp, process_fn.clone()).await;
        }
        
        if slot_changed {
            info!("SUBMITTER: Checking slot-triggered threads (slot: {})", slot);
            queue.process_slot_queue(slot, process_fn.clone()).await;
        }
        
        if epoch_changed {
            info!("SUBMITTER: Checking epoch-triggered threads (epoch: {})", epoch);
            queue.process_epoch_queue(epoch, process_fn).await;
        }
        
        Ok(())
    }
    
    /// Update clock state (called by Observer via channel)
    pub async fn update_clock(&self, slot: u64, epoch: u64, unix_timestamp: i64) {
        // Note: We can't update executor's clock state with immutable self
        // Instead, we'll pass the clock state directly to process_ready_threads
        debug!("SUBMITTER: Clock update - slot: {}, epoch: {}, timestamp: {}", slot, epoch, unix_timestamp);
        
        if self.executor_logic.is_some() {
            // Process any threads that are now ready
            if let Err(e) = self.process_ready_threads_with_clock(slot, epoch, unix_timestamp).await {
                error!("SUBMITTER: Failed to process ready threads on clock update: {}", e);
            }
        }
    }

    /// Get service status
    pub fn is_replay_enabled(&self) -> bool {
        self.config.enable_replay
    }

    pub fn has_nats_connection(&self) -> bool {
        self.nats_client.is_some()
    }

    pub fn mode_name(&self) -> &str {
        match &self.mode {
            SubmitterMode::Full { .. } => "Full",
            SubmitterMode::ReplayOnly => "ReplayOnly",
        }
    }
}
