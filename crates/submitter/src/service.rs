use anyhow::Result;
use log::{info, debug, error, warn};
use solana_sdk::{
    signature::{Keypair, Signature},
    transaction::VersionedTransaction,
};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::str::FromStr;
use tokio::sync::mpsc::{Sender, Receiver};

use crate::sources::{TransactionSource, LocalQueue, NatsConsumer, HybridSource};
use crate::transaction::{TransactionSubmitter, SubmissionResult};
use crate::types::BuiltTransaction;
use antegen_thread_program::state::Trigger;
use solana_cron::Schedule;
use chrono::{DateTime, Utc};

/// Transaction status for monitoring
#[derive(Debug, Clone, PartialEq)]
enum TransactionStatus {
    NotFound,
    Processed,
    Confirmed,
}

/// A transaction being monitored for confirmation
#[derive(Clone)]
struct MonitoredTransaction {
    tx_id: String,
    signature: Signature,
    transaction: VersionedTransaction,
    last_submission_block: u64,
    submission_count: u32,
    built_tx: BuiltTransaction,
}

/// Submitter service that can use different transaction sources
pub struct SubmitterService {
    /// Transaction submitter for TPU submission
    submitter: TransactionSubmitter,
    /// Source of transactions
    source: Box<dyn TransactionSource>,
    /// Submitter keypair
    _submitter_keypair: Arc<Keypair>,
    
    // Monitoring
    /// Transactions being monitored for confirmation
    monitoring: HashMap<String, MonitoredTransaction>,
    /// Maximum transactions to monitor concurrently
    max_monitored: usize,
    /// Blocks before resubmission
    resubmit_after_blocks: u64,
    /// Maximum submission attempts
    max_submissions: u32,
    
    // Clock tracking
    /// Current slot
    current_slot: u64,
    /// Current confirmed block height
    current_block: u64,
    /// Current timestamp
    current_timestamp: i64,
    /// Current epoch
    current_epoch: u64,
    /// Clock event receiver (for Worker mode)
    clock_receiver: Option<Receiver<ClockEvent>>,
    
    // Legacy fields (to be removed)
    pending_confirmations: HashMap<String, PendingTransaction>,
    max_pending_time: Duration,
    pending_transactions: VecDeque<BuiltTransaction>,
}

/// Clock event for time tracking
#[derive(Debug, Clone)]
pub struct ClockEvent {
    pub slot: u64,
    pub epoch: u64,
    pub timestamp: i64,
    pub block_height: u64,  // Confirmed block count
}

#[derive(Clone)]
struct PendingTransaction {
    _tx_id: String,
    signature: Signature,
    submitted_at: Instant,
    _built_tx: BuiltTransaction,
}

impl SubmitterService {
    /// Create submitter with local queue (Worker mode)
    pub fn with_local_queue(
        rpc_url: String,
        ws_url: String,
        submitter_keypair: Arc<Keypair>,
        buffer_size: usize,
    ) -> (Self, Sender<BuiltTransaction>) {
        let local_queue = LocalQueue::new(buffer_size);
        let sender = local_queue.sender();
        
        let service = Self {
            submitter: TransactionSubmitter::new(rpc_url, ws_url),
            source: Box::new(local_queue),
            _submitter_keypair: submitter_keypair,
            
            // Monitoring configuration
            monitoring: HashMap::new(),
            max_monitored: 100,
            resubmit_after_blocks: 150,
            max_submissions: 5,
            
            // Clock tracking
            current_slot: 0,
            current_block: 0,
            current_timestamp: 0,
            current_epoch: 0,
            clock_receiver: None, // To be set by caller if needed
            
            // Legacy fields
            pending_confirmations: HashMap::new(),
            max_pending_time: Duration::from_secs(60),
            pending_transactions: VecDeque::new(),
        };
        
        (service, sender)
    }
    
    /// Create submitter with external channel (Worker mode with external sender)
    pub fn with_channel(
        rpc_url: String,
        ws_url: String,
        submitter_keypair: Arc<Keypair>,
        sender: Sender<BuiltTransaction>,
        receiver: Receiver<BuiltTransaction>,
    ) -> Self {
        let local_queue = LocalQueue::from_receiver(receiver, sender);
        
        Self {
            submitter: TransactionSubmitter::new(rpc_url, ws_url),
            source: Box::new(local_queue),
            _submitter_keypair: submitter_keypair,
            
            // Monitoring configuration
            monitoring: HashMap::new(),
            max_monitored: 100,
            resubmit_after_blocks: 150,
            max_submissions: 5,
            
            // Clock tracking
            current_slot: 0,
            current_block: 0,
            current_timestamp: 0,
            current_epoch: 0,
            clock_receiver: None, // To be set by caller if needed
            
            // Legacy fields
            pending_confirmations: HashMap::new(),
            max_pending_time: Duration::from_secs(60),
            pending_transactions: VecDeque::new(),
        }
    }
    
    /// Create submitter with NATS queue (Standalone mode)
    pub async fn with_nats_queue(
        rpc_url: String,
        ws_url: String,
        submitter_keypair: Arc<Keypair>,
        nats_url: &str,
        consumer_name: &str,
    ) -> Result<Self> {
        let nats_consumer = NatsConsumer::new(nats_url, consumer_name, None).await?;
        
        Ok(Self {
            submitter: TransactionSubmitter::new(rpc_url, ws_url),
            source: Box::new(nats_consumer),
            _submitter_keypair: submitter_keypair,
            
            // Monitoring configuration
            monitoring: HashMap::new(),
            max_monitored: 100,
            resubmit_after_blocks: 150,
            max_submissions: 5,
            
            // Clock tracking
            current_slot: 0,
            current_block: 0,
            current_timestamp: 0,
            current_epoch: 0,
            clock_receiver: None, // Standalone mode uses RPC subscriptions
            
            // Legacy fields
            pending_confirmations: HashMap::new(),
            max_pending_time: Duration::from_secs(60),
            pending_transactions: VecDeque::new(),
        })
    }
    
    /// Create submitter in worker mode (local queue with optional NATS)
    pub async fn with_worker_mode(
        rpc_url: String,
        ws_url: String,
        submitter_keypair: Arc<Keypair>,
        buffer_size: usize,
        nats_config: Option<NatsConfig>,
    ) -> Result<(Self, Sender<BuiltTransaction>)> {
        if let Some(config) = nats_config {
            // Worker mode with NATS - use hybrid source
            let hybrid_source = HybridSource::new(buffer_size, &config.nats_url, &config.consumer_name).await?;
            let sender = hybrid_source.local_sender();
            Ok((Self {
                submitter: TransactionSubmitter::new(rpc_url, ws_url),
                source: Box::new(hybrid_source),
                _submitter_keypair: submitter_keypair,
                
                // Monitoring configuration
                monitoring: HashMap::new(),
                max_monitored: 100,
                resubmit_after_blocks: 150,
                max_submissions: 5,
                
                // Clock tracking
                current_slot: 0,
                current_block: 0,
                current_timestamp: 0,
                current_epoch: 0,
                clock_receiver: None, // To be set by caller
                
                // Legacy fields
                pending_confirmations: HashMap::new(),
                max_pending_time: Duration::from_secs(60),
                pending_transactions: VecDeque::new(),
            }, sender))
        } else {
            // Worker mode without NATS - local queue only
            Ok(Self::with_local_queue(rpc_url, ws_url, submitter_keypair, buffer_size))
        }
    }
    
    /// Set clock receiver for Worker mode
    pub fn set_clock_receiver(&mut self, receiver: Receiver<ClockEvent>) {
        self.clock_receiver = Some(receiver);
    }
    
    /// Submit transaction to the network
    async fn submit_transaction_to_network(&self, tx: &MonitoredTransaction) -> Result<()> {
        info!("SUBMITTER: Submitting tx {} to network (signature: {:?})", 
              tx.tx_id, tx.signature);
        
        match self.submitter.submit_versioned_tx(&tx.transaction).await {
            Ok(result) => {
                info!("SUBMITTER: ✅ Successfully submitted tx {} with result: {:?}", 
                      tx.tx_id, result);
                Ok(())
            }
            Err(e) => {
                error!("SUBMITTER: ❌ Submission FAILED for tx {}: {}", tx.tx_id, e);
                Err(anyhow::anyhow!("Submission failed: {}", e))
            }
        }
    }
    
    /// Check transaction status via RPC
    async fn check_transaction_status(&self, tx: &MonitoredTransaction) -> TransactionStatus {
        match self.submitter.check_status(&tx.signature).await {
            Ok(Some(true)) => TransactionStatus::Confirmed,
            Ok(Some(false)) => TransactionStatus::Processed,
            Ok(None) | Err(_) => TransactionStatus::NotFound,
        }
    }
    
    /// Main execution loop
    pub async fn run(&mut self) -> Result<()> {
        info!("SUBMITTER: Service started (source: {})", self.source.name());
        info!("SUBMITTER: Configuration: max_monitored={}, resubmit_after={} blocks", 
              self.max_monitored, self.resubmit_after_blocks);
        
        let mut tx_count = 0;
        
        loop {
            // Check for clock updates (non-blocking)
            let clock_events: Vec<ClockEvent> = if let Some(clock_rx) = &mut self.clock_receiver {
                let mut events = Vec::new();
                while let Ok(clock_event) = clock_rx.try_recv() {
                    events.push(clock_event);
                }
                events
            } else {
                Vec::new()
            };
            
            // Process clock events after releasing the borrow
            for clock_event in clock_events {
                let prev_block = self.current_block;
                self.current_slot = clock_event.slot;
                self.current_timestamp = clock_event.timestamp;
                self.current_epoch = clock_event.epoch;
                self.current_block = clock_event.block_height;
                
                // Only check transactions when we get a new block
                if clock_event.block_height > prev_block {
                    debug!("SUBMITTER: New block {} (slot {})", clock_event.block_height, clock_event.slot);
                    
                    // Check pending transactions for readiness
                    self.check_pending_transactions_for_readiness().await?;
                    
                    // Check monitored transactions for staleness
                    self.check_monitored_transactions().await?;
                }
            }
            
            // Process new transactions
            match tokio::time::timeout(
                Duration::from_millis(100),
                self.source.receive()
            ).await {
                Ok(Ok(Some(built_tx))) => {
                    tx_count += 1;
                    info!("SUBMITTER: Received transaction #{} (id={})", tx_count, built_tx.id);
                    self.process_new_transaction(built_tx).await?;
                }
                Ok(Ok(None)) | Err(_) => {
                    // No new transactions or timeout
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Ok(Err(e)) => {
                    error!("SUBMITTER: Error receiving transaction: {}", e);
                }
            }
        }
    }
    
    /// Process clock update and submit ready transactions
    async fn process_clock_update(&mut self, clock: ClockEvent) -> Result<()> {
        debug!("Clock update: slot={}, epoch={}, timestamp={}", 
               clock.slot, clock.epoch, clock.timestamp);
        
        // Update current time
        self.current_slot = clock.slot;
        self.current_epoch = clock.epoch;
        self.current_timestamp = clock.timestamp;
        
        // Check all pending transactions (FIFO order preserved)
        let mut ready_txs = Vec::new();
        let mut still_pending = VecDeque::new();
        
        while let Some(tx) = self.pending_transactions.pop_front() {
            if self.is_ready_to_submit(&tx) {
                ready_txs.push(tx);
            } else {
                still_pending.push_back(tx);
            }
        }
        
        // Put back the not-ready transactions
        self.pending_transactions = still_pending;
        
        // Submit all ready transactions
        for tx in ready_txs {
            info!("Clock triggered submission for transaction {}", tx.id);
            self.submit_transaction(tx).await?;
        }
        
        Ok(())
    }
    
    /// Check pending transactions to see if they're ready to submit
    async fn check_pending_transactions_for_readiness(&mut self) -> Result<()> {
        if self.pending_transactions.is_empty() {
            return Ok(());
        }
        
        debug!("SUBMITTER: Checking {} pending transactions for readiness", 
               self.pending_transactions.len());
        
        let mut ready_transactions = Vec::new();
        let mut still_pending = VecDeque::new();
        
        // Check each pending transaction
        while let Some(tx) = self.pending_transactions.pop_front() {
            if self.is_ready_to_submit(&tx) {
                info!("SUBMITTER: ⏰ → ✅ Transaction {} is now ready! (trigger: {:?})", 
                      tx.id, tx.trigger);
                ready_transactions.push(tx);
            } else {
                // Log why it's not ready (every 10 blocks to avoid spam)
                if self.current_block % 10 == 0 {
                    debug!("SUBMITTER: Tx {} still waiting (trigger: {:?}, current time: {}, slot: {})", 
                           tx.id, tx.trigger, self.current_timestamp, self.current_slot);
                }
                still_pending.push_back(tx);
            }
        }
        
        self.pending_transactions = still_pending;
        
        // Process ready transactions
        for built_tx in ready_transactions {
            // Move to monitoring queue
            info!("SUBMITTER: Moving tx {} from pending to monitoring for submission", built_tx.id);
            
            // Create versioned transaction
            let versioned_tx = self.build_versioned_transaction(built_tx.clone()).await?;
            let signature = versioned_tx.signatures[0];
            
            // Create monitored transaction
            let mut monitored = MonitoredTransaction {
                tx_id: built_tx.id.clone(),
                signature,
                transaction: versioned_tx,
                last_submission_block: self.current_block,
                submission_count: 0,
                built_tx: built_tx.clone(),
            };
            
            // Try to submit
            info!("SUBMITTER: Submitting ready transaction {}", built_tx.id);
            match self.submit_transaction_to_network(&monitored).await {
                Ok(()) => {
                    monitored.submission_count = 1;
                    monitored.last_submission_block = self.current_block;
                }
                Err(e) => {
                    error!("SUBMITTER: Submission failed: {}", e);
                }
            }
            
            self.monitoring.insert(built_tx.id.clone(), monitored);
        }
        
        if !self.pending_transactions.is_empty() {
            debug!("SUBMITTER: {} transactions still pending", self.pending_transactions.len());
        }
        
        Ok(())
    }
    
    /// Check monitored transactions for staleness and confirmation
    async fn check_monitored_transactions(&mut self) -> Result<()> {
        if self.monitoring.is_empty() {
            return Ok(());
        }
        
        info!("SUBMITTER: Block {} - Checking {} monitored transactions", 
              self.current_block, self.monitoring.len());
        
        let mut to_remove = Vec::new();
        let mut to_resubmit = Vec::new();
        
        for (tx_id, tx) in &self.monitoring {
            let blocks_since_submission = self.current_block.saturating_sub(tx.last_submission_block);
            let status = self.check_transaction_status(tx).await;
            
            info!("  Tx {}: submitted at block {}, age: {} blocks, status: {:?}", 
                  tx_id, tx.last_submission_block, blocks_since_submission, status);
            
            match status {
                TransactionStatus::Confirmed => {
                    info!("  🎉 Tx {} CONFIRMED! Removing from monitoring", tx_id);
                    to_remove.push(tx_id.clone());
                }
                TransactionStatus::Processed => {
                    debug!("  Tx {} processed but not confirmed, continuing to monitor", tx_id);
                }
                TransactionStatus::NotFound => {
                    if blocks_since_submission >= self.resubmit_after_blocks {
                        info!("  ⏰ Tx {} is stale ({} blocks), marking for resubmission", 
                              tx_id, blocks_since_submission);
                        to_resubmit.push(tx_id.clone());
                    }
                }
            }
        }
        
        // Remove confirmed transactions
        for tx_id in to_remove {
            self.monitoring.remove(&tx_id);
            info!("SUBMITTER: Monitoring queue now has {} transactions", self.monitoring.len());
        }
        
        // Resubmit stale transactions
        for tx_id in to_resubmit {
            // First check if we should remove due to max submissions
            let should_remove = {
                if let Some(tx) = self.monitoring.get(&tx_id) {
                    tx.submission_count >= self.max_submissions
                } else {
                    false
                }
            };
            
            if should_remove {
                error!("  Tx {} exceeded max submissions ({}), removing", tx_id, self.max_submissions);
                self.monitoring.remove(&tx_id);
            } else {
                // Get tx, update count, and prepare for submission
                let tx_clone = {
                    if let Some(tx) = self.monitoring.get_mut(&tx_id) {
                        tx.submission_count += 1;
                        info!("  Resubmitting tx {} (attempt #{})", tx_id, tx.submission_count);
                        Some(tx.clone())
                    } else {
                        None
                    }
                };
                
                // Submit outside of the mutable borrow
                if let Some(tx_for_submit) = tx_clone {
                    match self.submit_transaction_to_network(&tx_for_submit).await {
                        Ok(()) => {
                            // Update submission block after successful submit
                            if let Some(tx) = self.monitoring.get_mut(&tx_id) {
                                tx.last_submission_block = self.current_block;
                            }
                        }
                        Err(e) => {
                            error!("  Resubmission failed: {}", e);
                        }
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// Process a new transaction
    async fn process_new_transaction(&mut self, built_tx: BuiltTransaction) -> Result<()> {
        let tx_id = built_tx.id.clone();
        
        info!("SUBMITTER: Processing new transaction {}", tx_id);
        info!("SUBMITTER: Trigger: {:?}, Last started: {}", 
             built_tx.trigger, built_tx.last_started_at);
        
        // Check if we're at capacity
        if self.monitoring.len() >= self.max_monitored {
            error!("SUBMITTER: Monitoring queue full ({}/{}), dropping transaction", 
                   self.monitoring.len(), self.max_monitored);
            return Ok(());
        }
        
        // Check if it's ready to submit based on trigger
        let ready_to_submit = self.is_ready_to_submit(&built_tx);
        
        if !ready_to_submit {
            // Calculate when it will be ready
            let ready_info = match &built_tx.trigger {
                Trigger::Cron { schedule, .. } => {
                    if let Some(next_time) = self.calculate_next_cron(built_tx.last_started_at, schedule) {
                        format!("at timestamp {} (cron: {})", next_time, schedule)
                    } else {
                        "unknown cron time".to_string()
                    }
                }
                Trigger::Interval { seconds, .. } => {
                    let next_time = built_tx.last_started_at + seconds;
                    format!("at timestamp {} (interval: {}s)", next_time, seconds)
                }
                Trigger::Timestamp { unix_ts } => {
                    format!("at timestamp {}", unix_ts)
                }
                Trigger::Slot { slot } => {
                    format!("at slot {}", slot)
                }
                Trigger::Epoch { epoch } => {
                    format!("at epoch {}", epoch)
                }
                _ => "immediately".to_string()
            };
            
            info!("SUBMITTER: Tx {} NOT ready yet, will be ready {}", tx_id, ready_info);
            info!("SUBMITTER: Current time: {}, slot: {}, epoch: {}", 
                  self.current_timestamp, self.current_slot, self.current_epoch);
            
            // Add to pending queue without submitting
            self.pending_transactions.push_back(built_tx);
            info!("SUBMITTER: Added to pending queue ({} pending)", self.pending_transactions.len());
            return Ok(());
        }
        
        // Create versioned transaction
        let versioned_tx = self.build_versioned_transaction(built_tx.clone()).await?;
        let signature = versioned_tx.signatures[0];
        
        // Create monitored transaction
        let monitored = MonitoredTransaction {
            tx_id: tx_id.clone(),
            signature,
            transaction: versioned_tx,
            last_submission_block: self.current_block,
            submission_count: 0,
            built_tx,
        };
        
        // Check initial status
        let status = self.check_transaction_status(&monitored).await;
        info!("SUBMITTER: Initial status check: {:?}", status);
        
        match status {
            TransactionStatus::Confirmed => {
                info!("SUBMITTER: Tx {} already confirmed, not monitoring", tx_id);
                return Ok(());
            }
            TransactionStatus::Processed => {
                info!("SUBMITTER: Tx {} already processed, adding to monitoring", tx_id);
                self.monitoring.insert(tx_id, monitored);
            }
            TransactionStatus::NotFound => {
                info!("SUBMITTER: Tx {} not found, submitting now", tx_id);
                
                // Try to submit
                let mut monitored = monitored;
                match self.submit_transaction_to_network(&monitored).await {
                    Ok(()) => {
                        monitored.submission_count = 1;
                        monitored.last_submission_block = self.current_block;
                    }
                    Err(e) => {
                        error!("SUBMITTER: Initial submission failed: {}", e);
                        // Add to monitoring anyway to retry later
                    }
                }
                
                self.monitoring.insert(tx_id.clone(), monitored);
                info!("SUBMITTER: Added tx {} to monitoring ({} total)", 
                      tx_id, self.monitoring.len());
            }
        }
        
        Ok(())
    }
    
    /// Check if a transaction is ready to submit based on its trigger
    fn is_ready_to_submit(&self, tx: &BuiltTransaction) -> bool {
        match &tx.trigger {
            Trigger::Now => true,
            
            // Time-based triggers
            Trigger::Cron { schedule, .. } => {
                if let Some(next_time) = self.calculate_next_cron(tx.last_started_at, schedule) {
                    self.current_timestamp >= next_time
                } else {
                    false
                }
            }
            Trigger::Interval { seconds, .. } => {
                self.current_timestamp >= tx.last_started_at + *seconds
            }
            Trigger::Timestamp { unix_ts } => {
                self.current_timestamp >= *unix_ts
            }
            
            // Block-based triggers
            Trigger::Slot { slot } => self.current_slot >= *slot,
            Trigger::Epoch { epoch } => self.current_epoch >= *epoch,
            
            // Account trigger - already triggered
            Trigger::Account { .. } => true,
        }
    }
    
    /// Calculate next cron execution time
    fn calculate_next_cron(&self, after: i64, schedule: &str) -> Option<i64> {
        let sched = Schedule::from_str(schedule).ok()?;
        let after_dt = DateTime::<Utc>::from_timestamp(after, 0)?;
        sched.next_after(&after_dt)
            .take()
            .map(|dt| dt.timestamp())
    }
    
    /// Submit a transaction
    async fn submit_transaction(&mut self, built_tx: BuiltTransaction) -> Result<()> {
        let tx_id = built_tx.id.clone();
        info!(
            "Submitting transaction: {} (thread: {}, builder: {})",
            tx_id, built_tx.thread_pubkey, built_tx.builder_id
        );
        
        // Build versioned transaction with thread_submit wrapper
        let versioned_tx = self.build_versioned_transaction(built_tx.clone()).await?;
        let signature = versioned_tx.signatures[0];
        
        // Submit and monitor
        match self.submitter.submit_and_confirm(&versioned_tx).await {
            Ok(SubmissionResult::Success(sig)) => {
                info!("Transaction confirmed: {}", sig);
                self.source.ack(&tx_id).await?;
            }
            Ok(SubmissionResult::AlreadyProcessed(sig)) => {
                info!("Transaction already processed: {}", sig);
                self.source.ack(&tx_id).await?;
            }
            Ok(SubmissionResult::Expired(_)) => {
                // Add to pending for continued monitoring
                warn!("Transaction expired, adding to pending: {}", signature);
                self.pending_confirmations.insert(
                    tx_id.clone(),
                    PendingTransaction {
                        _tx_id: tx_id,
                        signature,
                        submitted_at: Instant::now(),
                        _built_tx: built_tx,
                    },
                );
            }
            Ok(SubmissionResult::Failed(err)) => {
                error!("Transaction failed: {}", err);
                self.source.nack(&tx_id).await?;
            }
            Err(e) => {
                error!("Submission error: {}", e);
                self.source.nack(&tx_id).await?;
            }
        }
        
        Ok(())
    }
    
    /// Check status of pending transactions
    async fn check_pending_confirmations(&mut self) -> Result<()> {
        let mut confirmed = Vec::new();
        let mut failed = Vec::new();
        
        for (tx_id, pending) in &self.pending_confirmations {
            // Check if transaction has landed
            match self.submitter.check_status(&pending.signature).await? {
                Some(true) => {
                    info!("Pending transaction confirmed: {}", pending.signature);
                    confirmed.push(tx_id.clone());
                }
                Some(false) => {
                    error!("Pending transaction failed: {}", pending.signature);
                    failed.push(tx_id.clone());
                }
                None => {
                    // Still pending, check timeout
                    if pending.submitted_at.elapsed() > self.max_pending_time {
                        warn!("Pending transaction timed out: {}", pending.signature);
                        failed.push(tx_id.clone());
                    }
                }
            }
        }
        
        // Process confirmed transactions
        for tx_id in confirmed {
            self.pending_confirmations.remove(&tx_id);
            self.source.ack(&tx_id).await?;
        }
        
        // Process failed transactions
        for tx_id in failed {
            self.pending_confirmations.remove(&tx_id);
            self.source.nack(&tx_id).await?;
        }
        
        if !self.pending_confirmations.is_empty() {
            debug!("Still tracking {} pending transactions", self.pending_confirmations.len());
        }
        
        Ok(())
    }
    
    /// Build versioned transaction with thread_submit wrapper
    async fn build_versioned_transaction(
        &self,
        built_tx: BuiltTransaction,
    ) -> Result<VersionedTransaction> {
        // Deserialize the transaction bytes
        let versioned_tx: VersionedTransaction = bincode::deserialize(&built_tx.partial_tx)?;
        
        // In a real implementation, we would wrap this with thread_submit
        // For now, just return the deserialized transaction
        // TODO: Implement thread_submit wrapping if needed
        
        Ok(versioned_tx)
    }
}

/// Configuration for submitter service
#[derive(Debug, Clone)]
pub struct SubmitterConfig {
    pub rpc_url: String,
    pub ws_url: String,
    pub keypair_path: String,
    pub mode: SubmitterMode,
}

#[derive(Debug, Clone)]
pub enum SubmitterMode {
    /// Worker mode - uses local queue with optional NATS
    Worker {
        /// Optional NATS configuration for additional transaction sources
        nats_config: Option<NatsConfig>,
    },
    /// Standalone mode - independent submitter using NATS
    Standalone {
        nats_url: String,
        consumer_name: String,
    },
}

#[derive(Debug, Clone)]
pub struct NatsConfig {
    pub nats_url: String,
    pub consumer_name: String,
}