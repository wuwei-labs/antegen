use std::sync::Arc;
use std::time::Duration;
use anyhow::{Result, anyhow};
use log::{info, warn, error, debug};
use futures::StreamExt;
use async_nats::jetstream::consumer::PullConsumer;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{signature::Signature, transaction::Transaction};

use crate::{TransactionSubmitter, SubmitterConfig, DurableTransactionMessage};

/// Consumes durable transactions from NATS and replays them if needed
pub struct ReplayConsumer {
    nats_client: async_nats::Client,
    consumer: PullConsumer,
    submitter: Arc<TransactionSubmitter>,
    rpc_client: Arc<RpcClient>,
    config: SubmitterConfig,
}

impl ReplayConsumer {
    /// Create a new replay consumer
    pub async fn new(
        nats_client: async_nats::Client,
        submitter: Arc<TransactionSubmitter>,
        rpc_client: Arc<RpcClient>,
        config: SubmitterConfig,
    ) -> Result<Self> {
        let jetstream = async_nats::jetstream::new(nats_client.clone());
        
        // Create or get the stream for durable transactions
        let stream = jetstream
            .get_or_create_stream(async_nats::jetstream::stream::Config {
                name: "ANTEGEN_DURABLE_TXS".to_string(),
                subjects: vec!["antegen.durable_txs".to_string()],
                retention: async_nats::jetstream::stream::RetentionPolicy::WorkQueue,
                max_age: std::time::Duration::from_secs(600), // 10 minutes
                storage: async_nats::jetstream::stream::StorageType::Memory,
                ..Default::default()
            })
            .await?;
        
        // Create consumer for this service instance
        let consumer_name = format!("replay_consumer_{}", 
                                  std::time::SystemTime::now()
                                      .duration_since(std::time::UNIX_EPOCH)
                                      .unwrap_or_default()
                                      .as_nanos());
        
        let consumer = stream
            .create_consumer(async_nats::jetstream::consumer::pull::Config {
                name: Some(consumer_name),
                durable_name: None, // Ephemeral consumer
                ack_policy: async_nats::jetstream::consumer::AckPolicy::Explicit,
                max_deliver: 3,
                ..Default::default()
            })
            .await?;
        
        Ok(Self {
            nats_client,
            consumer,
            submitter,
            rpc_client,
            config,
        })
    }
    
    /// Start consuming and replaying transactions
    pub async fn run(&mut self) -> Result<()> {
        info!("Starting replay consumer");
        
        let mut messages = self.consumer.messages().await?;
        
        while let Some(msg) = messages.next().await {
            let msg = match msg {
                Ok(msg) => msg,
                Err(e) => {
                    error!("Error receiving message: {}", e);
                    // Brief pause before continuing
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };
            
            if let Err(e) = self.process_message(msg).await {
                error!("Failed to process replay message: {}", e);
            }
        }
        
        Ok(())
    }
    
    /// Process a single NATS message
    async fn process_message(&self, msg: async_nats::jetstream::Message) -> Result<()> {
        // Parse the message
        let tx_msg: DurableTransactionMessage = serde_json::from_slice(&msg.payload)?;
        
        debug!("Received durable transaction: {} (age: {}ms)", 
               tx_msg.original_signature, tx_msg.age_ms());
        
        // Check if transaction is expired
        if tx_msg.is_expired(self.config.replay_max_age_ms) {
            info!("Transaction {} is expired, skipping", tx_msg.original_signature);
            if let Err(e) = msg.ack().await {
                error!("Failed to ack expired message: {}", e);
            }
            return Ok(());
        }
        
        // Check if we've exceeded max attempts
        if tx_msg.retry_count >= self.config.replay_max_attempts {
            warn!("Transaction {} exceeded max retry attempts", tx_msg.original_signature);
            if let Err(e) = msg.ack().await {
                error!("Failed to ack max retry message: {}", e);
            }
            return Ok(());
        }
        
        // Wait for replay delay if this is the first retry
        if tx_msg.retry_count == 0 && tx_msg.age_ms() < self.config.replay_delay_ms {
            let remaining_delay = self.config.replay_delay_ms - tx_msg.age_ms();
            debug!("Waiting {}ms before replaying transaction {}", 
                   remaining_delay, tx_msg.original_signature);
            tokio::time::sleep(Duration::from_millis(remaining_delay)).await;
        }
        
        // Check if transaction is already confirmed
        match self.check_transaction_status(&tx_msg.original_signature).await {
            Ok(true) => {
                info!("Transaction {} already confirmed, skipping replay", tx_msg.original_signature);
                if let Err(e) = msg.ack().await {
                    error!("Failed to ack confirmed message: {}", e);
                }
                return Ok(());
            }
            Ok(false) => {
                debug!("Transaction {} not confirmed, proceeding with replay", tx_msg.original_signature);
            }
            Err(e) => {
                warn!("Could not check status for transaction {}: {}", tx_msg.original_signature, e);
                // Proceed with replay attempt anyway
            }
        }
        
        // Attempt to replay the transaction
        match self.replay_transaction(&tx_msg).await {
            Ok(signature) => {
                info!("Successfully replayed transaction: {}", signature);
                if let Err(e) = msg.ack().await {
                    error!("Failed to ack successful replay message: {}", e);
                }
            }
            Err(e) => {
                warn!("Failed to replay transaction {}: {}", tx_msg.original_signature, e);
                
                // Increment retry count and republish if under limit
                if tx_msg.retry_count < self.config.replay_max_attempts - 1 {
                    let mut retry_msg = tx_msg.clone();
                    retry_msg.retry_count += 1;
                    
                    if let Err(publish_err) = self.republish_for_retry(retry_msg).await {
                        error!("Failed to republish transaction for retry: {}", publish_err);
                    }
                }
                
                if let Err(e) = msg.ack().await {
                    error!("Failed to ack failed replay message: {}", e);
                }
            }
        }
        
        Ok(())
    }
    
    /// Check if transaction is already confirmed on-chain
    async fn check_transaction_status(&self, signature_str: &str) -> Result<bool> {
        let signature = signature_str.parse::<Signature>()?;
        
        match self.rpc_client.get_signature_status(&signature).await? {
            Some(status) => Ok(status.is_ok()),
            None => Ok(false), // Transaction not found
        }
    }
    
    /// Replay a transaction
    async fn replay_transaction(&self, tx_msg: &DurableTransactionMessage) -> Result<Signature> {
        // Decode the base64 transaction
        use base64::Engine;
        let tx_bytes = base64::engine::general_purpose::STANDARD.decode(&tx_msg.base64_transaction)?;
        let tx: Transaction = bincode::deserialize(&tx_bytes)?;
        
        info!("Replaying transaction {} for thread {}", 
              tx_msg.original_signature, tx_msg.thread_pubkey);
        
        // Submit the transaction
        self.submitter.submit(&tx).await
    }
    
    /// Republish transaction for retry
    async fn republish_for_retry(&self, tx_msg: DurableTransactionMessage) -> Result<()> {
        let subject = "antegen.durable_txs";
        let payload = serde_json::to_vec(&tx_msg)?;
        
        if let Err(e) = self.nats_client.publish(subject, payload.into()).await {
            return Err(anyhow!("Failed to publish to NATS: {}", e));
        }
        
        debug!("Republished transaction {} for retry (attempt {})", 
               tx_msg.original_signature, tx_msg.retry_count);
        
        Ok(())
    }
}