use anyhow::{Result, anyhow};
use log::{info, debug, warn};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    signature::Signature,
    commitment_config::{CommitmentConfig, CommitmentLevel},
};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Status of a transaction
#[derive(Debug, Clone, PartialEq)]
pub enum TransactionStatus {
    /// Transaction is pending confirmation
    Pending,
    /// Transaction was confirmed at the given commitment level
    Confirmed(CommitmentLevel),
    /// Transaction failed with an error
    Failed(String),
    /// Transaction was not found (may have expired)
    NotFound,
}

/// Monitor for tracking transaction confirmations
pub struct TransactionMonitor {
    rpc_client: Arc<RpcClient>,
    max_confirmation_time: Duration,
}

impl TransactionMonitor {
    pub fn new(rpc_client: Arc<RpcClient>) -> Self {
        Self {
            rpc_client,
            max_confirmation_time: Duration::from_secs(60), // 60 seconds max
        }
    }
    
    /// Monitor a transaction until it's confirmed or times out
    pub async fn monitor_transaction(
        &self,
        signature: &Signature,
        target_commitment: CommitmentConfig,
    ) -> Result<TransactionStatus> {
        let start = Instant::now();
        let mut last_status = TransactionStatus::Pending;
        
        info!("MONITOR: Starting to monitor transaction {}", signature);
        
        loop {
            // Check if we've exceeded max confirmation time
            if start.elapsed() > self.max_confirmation_time {
                warn!("MONITOR: Transaction {} timed out after {:?}", 
                      signature, self.max_confirmation_time);
                return Ok(TransactionStatus::NotFound);
            }
            
            // Check transaction status
            match self.check_transaction_status(signature).await {
                Ok(status) => {
                    if status != last_status {
                        debug!("MONITOR: Transaction {} status changed to {:?}", 
                               signature, status);
                        last_status = status.clone();
                    }
                    
                    match &status {
                        TransactionStatus::Confirmed(level) => {
                            // Check if we've reached target commitment
                            if self.meets_commitment_target(*level, target_commitment) {
                                info!("MONITOR: Transaction {} confirmed at {:?}", 
                                      signature, level);
                                return Ok(status);
                            }
                        }
                        TransactionStatus::Failed(err) => {
                            warn!("MONITOR: Transaction {} failed: {}", signature, err);
                            return Ok(status);
                        }
                        TransactionStatus::NotFound => {
                            // Keep trying until timeout
                            debug!("MONITOR: Transaction {} not found yet", signature);
                        }
                        TransactionStatus::Pending => {
                            // Still pending
                            debug!("MONITOR: Transaction {} still pending", signature);
                        }
                    }
                }
                Err(e) => {
                    debug!("MONITOR: Error checking transaction {}: {}", signature, e);
                }
            }
            
            // Wait before next check
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    
    /// Check the current status of a transaction
    async fn check_transaction_status(&self, signature: &Signature) -> Result<TransactionStatus> {
        // First check if transaction exists
        match self.rpc_client.get_signature_status(signature).await? {
            Some(status) => {
                match status {
                    Ok(()) => {
                        // Transaction succeeded, check confirmation level
                        // We need to get the actual confirmation status
                        // For now, assume confirmed
                        Ok(TransactionStatus::Confirmed(CommitmentLevel::Confirmed))
                    }
                    Err(err) => {
                        // Transaction failed with an error
                        Ok(TransactionStatus::Failed(err.to_string()))
                    }
                }
            }
            None => Ok(TransactionStatus::NotFound),
        }
    }
    
    /// Check if a commitment level meets the target
    fn meets_commitment_target(
        &self,
        achieved: CommitmentLevel,
        target: CommitmentConfig,
    ) -> bool {
        match target.commitment {
            CommitmentLevel::Processed => true, // Any level meets processed
            CommitmentLevel::Confirmed => {
                matches!(achieved, CommitmentLevel::Confirmed | CommitmentLevel::Finalized)
            }
            CommitmentLevel::Finalized => {
                matches!(achieved, CommitmentLevel::Finalized)
            }
        }
    }
    
    /// Monitor a transaction and retry if it fails or times out
    pub async fn monitor_with_retry(
        &self,
        signature: &Signature,
        target_commitment: CommitmentConfig,
        max_retries: u32,
    ) -> Result<bool> {
        for attempt in 0..max_retries {
            if attempt > 0 {
                info!("MONITOR: Retry attempt {} for transaction {}", attempt, signature);
            }
            
            match self.monitor_transaction(signature, target_commitment).await? {
                TransactionStatus::Confirmed(_) => {
                    return Ok(true);
                }
                TransactionStatus::Failed(err) => {
                    // Check if error is retryable
                    if err.contains("blockhash not found") || err.contains("already processed") {
                        debug!("MONITOR: Retryable error for {}: {}", signature, err);
                        continue;
                    } else {
                        return Err(anyhow!("Transaction failed: {}", err));
                    }
                }
                TransactionStatus::NotFound | TransactionStatus::Pending => {
                    // Transaction expired or still pending after timeout
                    warn!("MONITOR: Transaction {} not confirmed, may retry", signature);
                    continue;
                }
            }
        }
        
        Ok(false)
    }
}