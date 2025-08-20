use std::sync::Arc;
use anyhow::{Result, anyhow};
use log::{info, warn, error, debug};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{signature::Signature, transaction::Transaction};
use tokio::task::JoinHandle;

use crate::{TransactionSubmitter, SubmitterConfig, DurableTransactionMessage};
use crate::replay::ReplayConsumer;

/// Main submitter service that handles both local submission and NATS replay
pub struct SubmitterService {
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
}

impl SubmitterService {
    /// Create a new submitter service
    pub async fn new(
        rpc_client: Arc<RpcClient>,
        config: SubmitterConfig,
    ) -> Result<Self> {
        // Create the core transaction submitter with TPU config
        let submitter = Arc::new(
            TransactionSubmitter::new(rpc_client.clone(), config.tpu_config.clone()).await?
        );

        // Connect to NATS if configured
        let nats_client = if let Some(nats_url) = &config.nats_url {
            info!("Connecting to NATS server: {}", nats_url);
            Some(async_nats::connect(nats_url).await?)
        } else {
            info!("No NATS URL configured, skipping NATS connection");
            None
        };

        Ok(Self {
            submitter,
            rpc_client,
            nats_client,
            config,
            replay_handle: None,
        })
    }

    /// Start the service (including optional replay consumer)
    pub async fn start(&mut self) -> Result<()> {
        info!("Starting submitter service (replay: {})", self.config.enable_replay);

        // Start replay consumer if enabled and NATS is available
        if self.config.enable_replay {
            if let Some(nats_client) = &self.nats_client {
                info!("Starting replay consumer");
                let mut replay_consumer = ReplayConsumer::new(
                    nats_client.clone(),
                    self.submitter.clone(),
                    self.rpc_client.clone(),
                    self.config.clone(),
                ).await?;

                let handle = tokio::spawn(async move {
                    replay_consumer.run().await
                });

                self.replay_handle = Some(handle);
                info!("Replay consumer started");
            } else {
                warn!("Replay enabled but no NATS client available");
            }
        }

        Ok(())
    }
    
    /// Start replay consumer without requiring mutable self (for Arc usage)
    pub async fn start_replay_consumer(&self) -> Result<Option<tokio::task::JoinHandle<Result<()>>>> {
        info!("Starting submitter service (replay: {})", self.config.enable_replay);

        // Start replay consumer if enabled and NATS is available
        if self.config.enable_replay {
            if let Some(nats_client) = &self.nats_client {
                info!("Starting replay consumer");
                let mut replay_consumer = ReplayConsumer::new(
                    nats_client.clone(),
                    self.submitter.clone(),
                    self.rpc_client.clone(),
                    self.config.clone(),
                ).await?;

                let handle = tokio::spawn(async move {
                    replay_consumer.run().await
                });

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
    pub async fn submit_with_retries(&self, tx: &Transaction, max_retries: u32) -> Result<Signature> {
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
        let nats_client = self.nats_client.as_ref().ok_or_else(|| anyhow!("No NATS client"))?;

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
            tx.message.account_keys.first()
                .map(|k| k.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
        );

        // Publish to NATS
        let subject = "antegen.durable_txs";
        let payload = serde_json::to_vec(&message)?;
        
        nats_client.publish(subject, payload.into()).await
            .map_err(|e| anyhow!("Failed to publish to NATS: {}", e))?;

        debug!("Published durable transaction {} to NATS for replay", signature);
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

    /// Get service status
    pub fn is_replay_enabled(&self) -> bool {
        self.config.enable_replay
    }

    pub fn has_nats_connection(&self) -> bool {
        self.nats_client.is_some()
    }
}