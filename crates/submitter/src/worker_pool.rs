use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use crossbeam::channel::{Receiver, Sender, bounded};
use log::{info, warn, error, debug};
use solana_sdk::{
    signature::{Keypair, Signer},
    transaction::VersionedTransaction,
};
use tokio::time::interval;

use crate::{
    TransactionMessage,
    SubmissionService,
    SubmitterMetrics,
};

/// Configuration for the worker pool
#[derive(Clone, Debug)]
pub struct WorkerPoolConfig {
    /// Number of concurrent workers
    pub num_workers: usize,
    /// Max batch size for batch submission
    pub batch_size: usize,
    /// Batch accumulation timeout
    pub batch_timeout_ms: u64,
    /// Channel buffer size
    pub channel_size: usize,
}

impl Default for WorkerPoolConfig {
    fn default() -> Self {
        Self {
            num_workers: 4,
            batch_size: 25,
            batch_timeout_ms: 100,
            channel_size: 1000,
        }
    }
}

/// Internal work item for the pool
enum WorkItem {
    Single(TransactionMessage, Arc<Keypair>),
    Batch(Vec<TransactionMessage>, Arc<Keypair>),
    Shutdown,
}

/// Worker pool for concurrent transaction submission
pub struct TransactionWorkerPool {
    #[allow(dead_code)]
    service: Arc<SubmissionService>,
    config: WorkerPoolConfig,
    work_sender: Sender<WorkItem>,
    worker_handles: Vec<tokio::task::JoinHandle<()>>,
    #[allow(dead_code)]
    metrics: Arc<SubmitterMetrics>,
}

impl TransactionWorkerPool {
    /// Create a new worker pool
    pub fn new(
        service: Arc<SubmissionService>,
        config: WorkerPoolConfig,
        metrics: Arc<SubmitterMetrics>,
    ) -> Self {
        let (work_sender, work_receiver) = bounded(config.channel_size);
        let work_receiver = Arc::new(tokio::sync::Mutex::new(work_receiver));
        
        let mut worker_handles = Vec::with_capacity(config.num_workers);
        
        // Spawn worker tasks
        for i in 0..config.num_workers {
            let service = service.clone();
            let receiver = work_receiver.clone();
            let metrics = metrics.clone();
            
            let handle = tokio::spawn(async move {
                Self::worker_task(i, service, receiver, metrics).await;
            });
            
            worker_handles.push(handle);
        }
        
        Self {
            service,
            config,
            work_sender,
            worker_handles,
            metrics,
        }
    }
    
    /// Submit a single transaction message
    pub fn submit(&self, msg: TransactionMessage, executor_keypair: Arc<Keypair>) -> Result<()> {
        self.work_sender
            .send(WorkItem::Single(msg, executor_keypair))
            .map_err(|e| anyhow::anyhow!("Failed to send work item: {}", e))
    }
    
    /// Submit a batch of transaction messages
    pub fn submit_batch(
        &self,
        messages: Vec<TransactionMessage>,
        executor_keypair: Arc<Keypair>,
    ) -> Result<()> {
        self.work_sender
            .send(WorkItem::Batch(messages, executor_keypair))
            .map_err(|e| anyhow::anyhow!("Failed to send batch work item: {}", e))
    }
    
    /// Process incoming transactions with batching
    pub async fn process_with_batching(
        &self,
        receiver: Receiver<TransactionMessage>,
        executor_keypair: Arc<Keypair>,
    ) -> Result<()> {
        info!("Starting transaction processor with batching (batch_size: {}, timeout: {}ms)",
              self.config.batch_size, self.config.batch_timeout_ms);
        
        let mut batch = Vec::with_capacity(self.config.batch_size);
        let mut batch_timer = interval(Duration::from_millis(self.config.batch_timeout_ms));
        
        loop {
            tokio::select! {
                // Check for new messages
                _ = tokio::task::yield_now() => {
                    // Try to receive messages non-blocking
                    while let Ok(msg) = receiver.try_recv() {
                        batch.push(msg);
                        
                        // Submit batch if full
                        if batch.len() >= self.config.batch_size {
                            let to_submit = std::mem::replace(
                                &mut batch,
                                Vec::with_capacity(self.config.batch_size)
                            );
                            
                            if !to_submit.is_empty() {
                                debug!("Submitting full batch of {} transactions", to_submit.len());
                                self.submit_batch(to_submit, executor_keypair.clone())?;
                            }
                        }
                    }
                    
                    // Check if channel is closed
                    if receiver.is_empty() && receiver.try_recv().is_err() {
                        // Submit any remaining batch
                        if !batch.is_empty() {
                            debug!("Submitting final batch of {} transactions", batch.len());
                            self.submit_batch(batch, executor_keypair)?;
                        }
                        info!("Transaction receiver closed, shutting down");
                        break;
                    }
                }
                
                // Batch timeout - submit partial batch
                _ = batch_timer.tick() => {
                    if !batch.is_empty() {
                        let to_submit = std::mem::replace(
                            &mut batch,
                            Vec::with_capacity(self.config.batch_size)
                        );
                        
                        debug!("Submitting timeout batch of {} transactions", to_submit.len());
                        self.submit_batch(to_submit, executor_keypair.clone())?;
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// Worker task that processes work items
    async fn worker_task(
        id: usize,
        service: Arc<SubmissionService>,
        receiver: Arc<tokio::sync::Mutex<Receiver<WorkItem>>>,
        metrics: Arc<SubmitterMetrics>,
    ) {
        debug!("Worker {} started", id);
        
        loop {
            // Get work item from queue
            let work_item = {
                let receiver = receiver.lock().await;
                receiver.recv()
            };
            
            match work_item {
                Ok(WorkItem::Single(msg, executor_keypair)) => {
                    Self::process_single(&service, msg, executor_keypair, &metrics).await;
                }
                Ok(WorkItem::Batch(messages, executor_keypair)) => {
                    Self::process_batch(&service, messages, executor_keypair, &metrics).await;
                }
                Ok(WorkItem::Shutdown) | Err(_) => {
                    debug!("Worker {} shutting down", id);
                    break;
                }
            }
        }
    }
    
    /// Process a single transaction message
    async fn process_single(
        service: &Arc<SubmissionService>,
        msg: TransactionMessage,
        executor_keypair: Arc<Keypair>,
        metrics: &Arc<SubmitterMetrics>,
    ) {
        // Get blockhash
        let blockhash = match service.rpc_client().get_latest_blockhash().await {
            Ok(bh) => bh,
            Err(e) => {
                error!("Failed to get blockhash: {}", e);
                metrics.transaction_failed();
                return;
            }
        };
        
        // Build and sign transaction
        let tx = service.build_transaction_with_compute_budget(
            msg.instructions,
            &executor_keypair.pubkey(),
            blockhash,
            msg.compute_units,
        );
        
        let mut signed_tx = tx;
        signed_tx.sign(&[executor_keypair.as_ref()], blockhash);
        let versioned_tx = VersionedTransaction::from(signed_tx);
        
        // Submit transaction
        match service.submit(&versioned_tx).await {
            Ok(sig) => {
                info!("Worker submitted transaction {} for thread {}", sig, msg.thread_pubkey);
                metrics.transaction_submitted("worker_pool");
            }
            Err(e) => {
                warn!("Worker failed to submit transaction for thread {}: {}", msg.thread_pubkey, e);
                metrics.transaction_failed();
            }
        }
    }
    
    /// Process a batch of transaction messages
    async fn process_batch(
        service: &Arc<SubmissionService>,
        messages: Vec<TransactionMessage>,
        executor_keypair: Arc<Keypair>,
        metrics: &Arc<SubmitterMetrics>,
    ) {
        if messages.is_empty() {
            return;
        }
        
        debug!("Processing batch of {} transactions", messages.len());
        
        // Get blockhash for batch
        let blockhash = match service.rpc_client().get_latest_blockhash().await {
            Ok(bh) => bh,
            Err(e) => {
                error!("Failed to get blockhash for batch: {}", e);
                for _ in &messages {
                    metrics.transaction_failed();
                }
                return;
            }
        };
        
        // Build and sign all transactions
        let mut versioned_txs = Vec::with_capacity(messages.len());
        let mut thread_pubkeys = Vec::with_capacity(messages.len());
        
        for msg in messages {
            let tx = service.build_transaction_with_compute_budget(
                msg.instructions,
                &executor_keypair.pubkey(),
                blockhash,
                msg.compute_units,
            );
            
            let mut signed_tx = tx;
            signed_tx.sign(&[executor_keypair.as_ref()], blockhash);
            
            versioned_txs.push(VersionedTransaction::from(signed_tx));
            thread_pubkeys.push(msg.thread_pubkey);
        }
        
        // Submit batch
        match service.submit_batch(&versioned_txs).await {
            Ok(results) => {
                for (result, thread_pubkey) in results.into_iter().zip(thread_pubkeys.iter()) {
                    match result {
                        Ok(sig) => {
                            info!("Batch submitted transaction {} for thread {}", sig, thread_pubkey);
                            metrics.transaction_submitted("worker_pool_batch");
                        }
                        Err(e) => {
                            warn!("Batch failed transaction for thread {}: {}", thread_pubkey, e);
                            metrics.transaction_failed();
                        }
                    }
                }
            }
            Err(e) => {
                error!("Failed to submit batch: {}", e);
                for _ in &versioned_txs {
                    metrics.transaction_failed();
                }
            }
        }
    }
    
    /// Shutdown the worker pool
    pub async fn shutdown(self) {
        info!("Shutting down worker pool");
        
        // Send shutdown signal to all workers
        for _ in 0..self.config.num_workers {
            let _ = self.work_sender.send(WorkItem::Shutdown);
        }
        
        // Wait for workers to finish
        for handle in self.worker_handles {
            let _ = handle.await;
        }
        
        info!("Worker pool shutdown complete");
    }
}