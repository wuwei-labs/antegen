use std::sync::Arc;
use std::time::Duration;
use solana_client::{
    nonblocking::rpc_client::RpcClient,
    rpc_config::RpcSimulateTransactionConfig,
};
use solana_sdk::{
    commitment_config::CommitmentConfig,
    signature::Signature,
    transaction::{Transaction, VersionedTransaction},
};
use anyhow::{Result, anyhow};
use log::{info, warn};
use bincode::serialize;
use crate::tpu_client::TpuClientManager;
use crate::transaction_monitor::{TransactionMonitor, TransactionStatus};

/// Handles transaction submission and monitoring
pub struct TransactionSubmitter {
    tpu_manager: TpuClientManager,
    rpc_client: Arc<RpcClient>,
    monitor: TransactionMonitor,
    retry_strategy: RetryStrategy,
}

impl TransactionSubmitter {
    pub fn new(rpc_url: String, websocket_url: String) -> Self {
        let rpc_client = Arc::new(RpcClient::new_with_commitment(
            rpc_url.clone(),
            CommitmentConfig::processed(),
        ));
        
        let monitor = TransactionMonitor::new(rpc_client.clone());
        
        Self {
            tpu_manager: TpuClientManager::new(rpc_url, websocket_url, 24),
            rpc_client,
            monitor,
            retry_strategy: RetryStrategy::default(),
        }
    }
    
    /// Simulate a transaction
    pub async fn simulate_tx(&self, tx: &Transaction) -> Result<()> {
        let response = self.rpc_client
            .simulate_transaction_with_config(
                tx,
                RpcSimulateTransactionConfig {
                    replace_recent_blockhash: false,
                    commitment: Some(CommitmentConfig::processed()),
                    ..RpcSimulateTransactionConfig::default()
                },
            )
            .await
            .map_err(|e| anyhow!("Transaction simulation failed: {}", e))?;
        
        if let Some(err) = response.value.err {
            return Err(anyhow!(
                "Transaction simulation failed: {} Logs: {:#?}",
                err,
                response.value.logs
            ));
        }
        
        Ok(())
    }
    
    /// Submit a transaction via TPU
    pub async fn submit_tx(&self, tx: &Transaction) -> Result<Signature> {
        let tpu_client = self.tpu_manager.get_tpu_client().await?;
        
        if !tpu_client.send_transaction(tx).await {
            return Err(anyhow!("Failed to send transaction"));
        }
        
        info!("Transaction submitted: {}", tx.signatures[0]);
        Ok(tx.signatures[0])
    }
    
    /// Submit a versioned transaction via TPU
    pub async fn submit_versioned_tx(&self, tx: &VersionedTransaction) -> Result<Signature> {
        let tpu_client = self.tpu_manager.get_tpu_client().await?;
        let wire_tx = serialize(tx)
            .map_err(|e| anyhow!("Failed to serialize transaction: {}", e))?;
        
        tpu_client.try_send_wire_transaction(wire_tx)
            .await
            .map_err(|e| anyhow!("Failed to send transaction: {}", e))?;
        
        info!("Versioned transaction submitted: {}", tx.signatures[0]);
        Ok(tx.signatures[0])
    }
    
    /// Submit multiple transactions as a batch
    pub async fn submit_batch(&self, txs: Vec<&VersionedTransaction>) -> Result<Vec<Signature>> {
        let tpu_client = self.tpu_manager.get_tpu_client().await?;
        
        let wire_txs: Result<Vec<Vec<u8>>> = txs.iter()
            .map(|tx| serialize(tx).map_err(|e| anyhow!("Failed to serialize: {}", e)))
            .collect();
        
        let wire_txs = wire_txs?;
        let signatures: Vec<Signature> = txs.iter().map(|tx| tx.signatures[0]).collect();
        
        tpu_client.try_send_wire_transaction_batch(wire_txs)
            .await
            .map_err(|e| anyhow!("Failed to send batch: {}", e))?;
        
        info!("Submitted batch of {} transactions", signatures.len());
        Ok(signatures)
    }
    
    /// Check transaction status
    pub async fn check_status(&self, signature: &Signature) -> Result<Option<bool>> {
        match self.rpc_client
            .get_signature_status_with_commitment(signature, CommitmentConfig::processed())
            .await
        {
            Ok(status) => match status {
                None => Ok(None), // Transaction not found
                Some(result) => match result {
                    Ok(()) => Ok(Some(true)), // Success
                    Err(_) => Ok(Some(false)), // Failed
                }
            },
            Err(e) => Err(anyhow!("Failed to check status: {}", e)),
        }
    }
    
    /// Submit transaction and wait for confirmation
    pub async fn submit_and_confirm(
        &self,
        tx: &VersionedTransaction,
    ) -> Result<SubmissionResult> {
        let signature = tx.signatures[0];
        
        // First, check if already processed (idempotency)
        if self.monitor.is_already_confirmed(&signature).await? {
            info!("Transaction already confirmed: {}", signature);
            return Ok(SubmissionResult::AlreadyProcessed(signature));
        }
        
        // Submit via TPU
        self.submit_versioned_tx(tx).await?;
        
        // Wait for confirmation
        match self.monitor.wait_for_confirmation(&signature).await? {
            TransactionStatus::Confirmed => {
                Ok(SubmissionResult::Success(signature))
            }
            TransactionStatus::Failed(err) => {
                Ok(SubmissionResult::Failed(err))
            }
            TransactionStatus::Expired => {
                Ok(SubmissionResult::Expired(signature))
            }
            TransactionStatus::Pending => {
                unreachable!("wait_for_confirmation should not return Pending")
            }
        }
    }
    
    /// Submit with retry logic
    pub async fn submit_with_retry(
        &self,
        tx: &VersionedTransaction,
    ) -> Result<SubmissionResult> {
        let mut attempts = 0;
        
        loop {
            match self.submit_and_confirm(tx).await? {
                SubmissionResult::Success(sig) => {
                    return Ok(SubmissionResult::Success(sig));
                }
                SubmissionResult::AlreadyProcessed(sig) => {
                    return Ok(SubmissionResult::AlreadyProcessed(sig));
                }
                SubmissionResult::Failed(err) if attempts < self.retry_strategy.max_retries => {
                    warn!("Attempt {} failed: {}", attempts + 1, err);
                    attempts += 1;
                    tokio::time::sleep(self.retry_strategy.retry_delay).await;
                }
                result => return Ok(result),
            }
        }
    }
}

#[derive(Debug)]
pub enum SubmissionResult {
    Success(Signature),
    AlreadyProcessed(Signature),
    Failed(String),
    Expired(Signature),
}

#[derive(Clone)]
pub struct RetryStrategy {
    pub max_retries: usize,
    pub retry_delay: Duration,
}

impl Default for RetryStrategy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            retry_delay: Duration::from_secs(1),
        }
    }
}