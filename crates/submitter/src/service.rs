use anyhow::Result;
use log::{info, debug, error, warn};
use solana_sdk::{
    signature::{Keypair, Signature},
    transaction::VersionedTransaction,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::Sender;

use crate::source::TransactionSource;
use crate::local_queue::LocalQueue;
use crate::nats_consumer::NatsConsumer;
use crate::hybrid_source::HybridSource;
use crate::transaction_submitter::{TransactionSubmitter, SubmissionResult};
use crate::types::BuiltTransaction;

/// Submitter service that can use different transaction sources
pub struct SubmitterService {
    /// Transaction submitter for TPU submission
    submitter: TransactionSubmitter,
    /// Source of transactions
    source: Box<dyn TransactionSource>,
    /// Submitter keypair
    _submitter_keypair: Arc<Keypair>,
    /// Pending confirmations
    pending_confirmations: HashMap<String, PendingTransaction>,
    /// Maximum time to track pending transactions
    max_pending_time: Duration,
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
            pending_confirmations: HashMap::new(),
            max_pending_time: Duration::from_secs(60),
        };
        
        (service, sender)
    }
    
    /// Create submitter with NATS queue (Submitter mode)
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
            pending_confirmations: HashMap::new(),
            max_pending_time: Duration::from_secs(60),
        })
    }
    
    /// Create submitter with both queues (hybrid mode - local queue has priority)
    pub async fn with_hybrid_queue(
        rpc_url: String,
        ws_url: String,
        submitter_keypair: Arc<Keypair>,
        buffer_size: usize,
        nats_url: &str,
        consumer_name: &str,
    ) -> Result<(Self, Sender<BuiltTransaction>)> {
        let hybrid_source = HybridSource::new(buffer_size, nats_url, consumer_name).await?;
        let sender = hybrid_source.local_sender();
        
        Ok((Self {
            submitter: TransactionSubmitter::new(rpc_url, ws_url),
            source: Box::new(hybrid_source),
            _submitter_keypair: submitter_keypair,
            pending_confirmations: HashMap::new(),
            max_pending_time: Duration::from_secs(60),
        }, sender))
    }
    
    /// Main execution loop
    pub async fn run(&mut self) -> Result<()> {
        info!("Submitter service started (source: {})", self.source.name());
        
        loop {
            // Check pending confirmations
            self.check_pending_confirmations().await?;
            
            // Process new transactions
            match tokio::time::timeout(
                Duration::from_millis(100),
                self.source.receive()
            ).await {
                Ok(Ok(Some(built_tx))) => {
                    self.process_new_transaction(built_tx).await?;
                }
                Ok(Ok(None)) | Err(_) => {
                    // No new transactions or timeout
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Ok(Err(e)) => {
                    error!("Error receiving transaction: {}", e);
                }
            }
        }
    }
    
    /// Process a new transaction
    async fn process_new_transaction(&mut self, built_tx: BuiltTransaction) -> Result<()> {
        let tx_id = built_tx.id.clone();
        info!(
            "Processing transaction: {} (thread: {}, builder: {})",
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
        // Deserialize the partial transaction
        let partial_tx: VersionedTransaction = bincode::deserialize(&built_tx.partial_tx)?;
        
        // In a real implementation, we would wrap this with thread_submit
        // For now, just return the partial transaction
        // TODO: Implement thread_submit wrapping
        
        Ok(partial_tx)
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
    /// Worker mode - uses local queue
    Worker,
    /// Submitter mode - uses NATS
    Submitter {
        nats_url: String,
        consumer_name: String,
    },
    /// Hybrid mode - both local and NATS
    Hybrid {
        nats_url: String,
        consumer_name: String,
    },
}