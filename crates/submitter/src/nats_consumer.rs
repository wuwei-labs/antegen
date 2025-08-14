use async_trait::async_trait;
use anyhow::Result;
use async_nats::jetstream::{self, Message};
use async_nats::jetstream::consumer::{pull::Config as PullConsumerConfig, AckPolicy};
use async_nats::jetstream::stream::{Config as StreamConfig, RetentionPolicy};
use futures::StreamExt;
use std::collections::HashMap;
use std::time::Duration;
use log::{info, debug, error};

use crate::source::TransactionSource;
use crate::types::BuiltTransaction;

/// NATS JetStream consumer for receiving transactions
pub struct NatsConsumer {
    _client: async_nats::Client,
    jetstream: jetstream::Context,
    stream_name: String,
    consumer_name: String,
    pending_messages: HashMap<String, Message>,
    batch_size: usize,
    // Store the message stream for reuse
    message_stream: Option<jetstream::consumer::pull::Stream>,
}

impl NatsConsumer {
    pub async fn new(
        url: &str, 
        consumer_name: &str,
        stream_name: Option<&str>,
    ) -> Result<Self> {
        info!("Connecting to NATS at {}", url);
        let client = async_nats::connect(url).await?;
        let jetstream = jetstream::new(client.clone());
        
        let stream_name = stream_name.unwrap_or("ANTEGEN_TRANSACTIONS");
        
        // Ensure stream exists
        let _stream = jetstream
            .get_or_create_stream(StreamConfig {
                name: stream_name.to_string(),
                subjects: vec!["antegen.tx.>".to_string()],
                retention: RetentionPolicy::WorkQueue,
                max_age: Duration::from_secs(30),
                ..Default::default()
            })
            .await?;
        
        info!("Connected to stream: {}", stream_name);
        
        // Get the stream first
        let stream = jetstream.get_stream(stream_name).await?;
        
        // Create or get durable pull consumer with pull config
        let consumer = stream
            .get_or_create_consumer(
                consumer_name,
                PullConsumerConfig {
                    durable_name: Some(consumer_name.to_string()),
                    ack_policy: AckPolicy::Explicit,
                    max_deliver: 3,
                    ack_wait: Duration::from_secs(30),
                    ..Default::default()
                },
            )
            .await?;
        
        info!("Created/retrieved consumer: {}", consumer_name);
        
        // Get the message stream
        let message_stream = consumer
            .stream()
            .max_messages_per_batch(1)
            .messages()
            .await?;
        
        Ok(Self {
            _client: client,
            jetstream,
            stream_name: stream_name.to_string(),
            consumer_name: consumer_name.to_string(),
            pending_messages: HashMap::new(),
            batch_size: 10,
            message_stream: Some(message_stream),
        })
    }
    
    pub fn set_batch_size(&mut self, size: usize) {
        self.batch_size = size;
    }
    
    /// Recreate the message stream if needed
    async fn ensure_message_stream(&mut self) -> Result<()> {
        if self.message_stream.is_none() {
            let stream = self.jetstream
                .get_stream(&self.stream_name)
                .await?;
            
            let consumer = stream
                .get_consumer(&self.consumer_name)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to get consumer: {}", e))?;
            
            let message_stream = consumer
                .stream()
                .max_messages_per_batch(1)
                .messages()
                .await?;
            
            self.message_stream = Some(message_stream);
        }
        Ok(())
    }
}

#[async_trait]
impl TransactionSource for NatsConsumer {
    async fn receive(&mut self) -> Result<Option<BuiltTransaction>> {
        // Ensure we have a message stream
        self.ensure_message_stream().await?;
        
        if let Some(ref mut stream) = self.message_stream {
            // Try to get the next message with a timeout
            match tokio::time::timeout(Duration::from_millis(100), stream.next()).await {
                Ok(Some(msg_result)) => {
                    match msg_result {
                        Ok(msg) => {
                            // Parse transaction from message
                            match serde_json::from_slice::<BuiltTransaction>(&msg.payload) {
                                Ok(tx) => {
                                    let tx_id = tx.id.clone();
                                    debug!("Received transaction from NATS: {}", tx_id);
                                    
                                    // Store message for later ack/nack
                                    self.pending_messages.insert(tx_id.clone(), msg);
                                    
                                    Ok(Some(tx))
                                }
                                Err(e) => {
                                    error!("Failed to parse transaction from NATS: {}", e);
                                    // Acknowledge bad message so it doesn't get redelivered
                                    msg.ack().await
                                        .map_err(|e| anyhow::anyhow!("Failed to ack bad message: {}", e))?;
                                    Ok(None)
                                }
                            }
                        }
                        Err(e) => {
                            error!("Error receiving message from NATS: {}", e);
                            // Reset the stream on error
                            self.message_stream = None;
                            Err(anyhow::anyhow!("NATS stream error: {}", e))
                        }
                    }
                }
                Ok(None) => {
                    // Stream ended, recreate it
                    self.message_stream = None;
                    Ok(None)
                }
                Err(_) => {
                    // Timeout - no messages available
                    Ok(None)
                }
            }
        } else {
            Ok(None)
        }
    }
    
    async fn ack(&mut self, tx_id: &str) -> Result<()> {
        if let Some(msg) = self.pending_messages.remove(tx_id) {
            debug!("Acknowledging transaction: {}", tx_id);
            msg.ack().await
                .map_err(|e| anyhow::anyhow!("Failed to ack message: {}", e))?;
        }
        Ok(())
    }
    
    async fn nack(&mut self, tx_id: &str) -> Result<()> {
        // Just remove from pending - message will be redelivered automatically
        if self.pending_messages.remove(tx_id).is_some() {
            debug!("Not acknowledging transaction (will be redelivered): {}", tx_id);
        }
        Ok(())
    }
    
    fn name(&self) -> &str {
        "NatsConsumer"
    }
}