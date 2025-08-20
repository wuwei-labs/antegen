use std::sync::Arc;
use std::time::Duration;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    signature::Signature,
    transaction::Transaction,
};
use anyhow::{Result, anyhow};
use log::{info, warn};

/// Handles transaction submission
pub struct TransactionSubmitter {
    rpc_client: Arc<RpcClient>,
    #[allow(dead_code)]
    use_tpu: bool,
}

impl TransactionSubmitter {
    pub async fn new(
        rpc_client: Arc<RpcClient>,
        _tpu_config: Option<String>, // For future TPU support
    ) -> Result<Self> {
        Ok(Self {
            rpc_client,
            use_tpu: false, // Start with RPC, can add TPU later
        })
    }
    
    /// Submit a transaction
    pub async fn submit(&self, tx: &Transaction) -> Result<Signature> {
        info!("Submitting transaction with {} instruction(s)", tx.message.instructions.len());
        
        // For now, just use RPC send_and_confirm
        // Can add TPU submission later
        let signature = self.rpc_client.send_and_confirm_transaction(tx).await?;
        
        info!("Transaction submitted successfully: {}", signature);
        Ok(signature)
    }
    
    /// Submit with retries
    pub async fn submit_with_retries(&self, tx: &Transaction, max_retries: u32) -> Result<Signature> {
        let mut attempts = 0;
        let mut last_error = None;
        
        while attempts < max_retries {
            match self.submit(tx).await {
                Ok(sig) => return Ok(sig),
                Err(e) => {
                    attempts += 1;
                    warn!("Submission attempt {} failed: {}", attempts, e);
                    last_error = Some(e);
                    
                    if attempts < max_retries {
                        tokio::time::sleep(Duration::from_millis(500 * attempts as u64)).await;
                    }
                }
            }
        }
        
        Err(last_error.unwrap_or_else(|| anyhow!("Failed to submit transaction")))
    }
}