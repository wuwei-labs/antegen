use anyhow::Result;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::{CommitmentConfig, CommitmentLevel},
    signature::Signature,
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use log::{info, warn, error, debug};

/// Monitors transaction status on-chain
pub struct TransactionMonitor {
    rpc_client: Arc<RpcClient>,
    max_wait_time: Duration,
    confirmation_level: CommitmentLevel,
    check_interval: Duration,
}

impl TransactionMonitor {
    pub fn new(rpc_client: Arc<RpcClient>) -> Self {
        Self {
            rpc_client,
            max_wait_time: Duration::from_secs(30),
            confirmation_level: CommitmentLevel::Processed,
            check_interval: Duration::from_millis(500),
        }
    }
    
    pub fn with_confirmation_level(mut self, level: CommitmentLevel) -> Self {
        self.confirmation_level = level;
        self
    }
    
    pub fn with_max_wait_time(mut self, duration: Duration) -> Self {
        self.max_wait_time = duration;
        self
    }
    
    /// Wait for transaction to be confirmed on-chain
    pub async fn wait_for_confirmation(
        &self,
        signature: &Signature,
    ) -> Result<TransactionStatus> {
        let start = Instant::now();
        info!("Waiting for confirmation of transaction: {}", signature);
        
        loop {
            // Check if transaction landed
            match self.check_status_internal(signature).await? {
                TransactionStatus::Confirmed => {
                    let elapsed = start.elapsed();
                    info!(
                        "Transaction confirmed: {} (took {:.2}s)",
                        signature,
                        elapsed.as_secs_f64()
                    );
                    return Ok(TransactionStatus::Confirmed);
                }
                TransactionStatus::Failed(err) => {
                    error!("Transaction failed: {} - {}", signature, err);
                    return Ok(TransactionStatus::Failed(err));
                }
                TransactionStatus::Pending => {
                    // Check timeout
                    if start.elapsed() > self.max_wait_time {
                        warn!(
                            "Transaction timed out after {:.2}s: {}",
                            self.max_wait_time.as_secs_f64(),
                            signature
                        );
                        return Ok(TransactionStatus::Expired);
                    }
                    
                    debug!("Transaction still pending: {}", signature);
                    // Continue waiting
                    tokio::time::sleep(self.check_interval).await;
                }
                TransactionStatus::Expired => {
                    // Should not happen from check_status_internal
                    return Ok(TransactionStatus::Expired);
                }
            }
        }
    }
    
    /// Check transaction status without waiting
    pub async fn check_status(&self, signature: &Signature) -> Result<TransactionStatus> {
        self.check_status_internal(signature).await
    }
    
    /// Internal status check
    async fn check_status_internal(&self, signature: &Signature) -> Result<TransactionStatus> {
        let commitment = CommitmentConfig {
            commitment: self.confirmation_level,
        };
        
        match self.rpc_client
            .get_signature_status_with_commitment(signature, commitment)
            .await
        {
            Ok(Some(Ok(()))) => Ok(TransactionStatus::Confirmed),
            Ok(Some(Err(err))) => Ok(TransactionStatus::Failed(err.to_string())),
            Ok(None) => Ok(TransactionStatus::Pending),
            Err(e) => {
                // RPC error - treat as pending
                debug!("RPC error checking status: {}", e);
                Ok(TransactionStatus::Pending)
            }
        }
    }
    
    /// Check if transaction is already on-chain (for idempotency)
    pub async fn is_already_confirmed(&self, signature: &Signature) -> Result<bool> {
        Ok(matches!(
            self.check_status(signature).await?,
            TransactionStatus::Confirmed
        ))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransactionStatus {
    /// Transaction is not yet known to the network
    Pending,
    /// Transaction has been confirmed at the desired commitment level
    Confirmed,
    /// Transaction failed with an error
    Failed(String),
    /// Transaction was not confirmed within timeout period
    Expired,
}

impl TransactionStatus {
    pub fn is_final(&self) -> bool {
        matches!(
            self,
            TransactionStatus::Confirmed | TransactionStatus::Failed(_)
        )
    }
    
    pub fn is_success(&self) -> bool {
        matches!(self, TransactionStatus::Confirmed)
    }
}