use anyhow::Result;
use async_nats::jetstream::stream::{Config as StreamConfig, RetentionPolicy};
use async_nats::jetstream::{self, Context as JetStreamContext};
use log::{debug, info};
use std::time::Duration;
use anchor_lang::AnchorSerialize;

use antegen_submitter::BuiltTransaction;

/// NATS JetStream publisher for broadcasting built transactions
pub struct NatsPublisher {
    jetstream: JetStreamContext,
    stream_name: String,
    subject_prefix: String,
}

impl NatsPublisher {
    pub async fn new(
        url: &str,
        stream_name: Option<&str>,
        subject_prefix: Option<&str>,
    ) -> Result<Self> {
        info!("Connecting to NATS publisher at {}", url);

        let client = async_nats::connect(url).await?;
        let jetstream = jetstream::new(client);

        let stream_name = stream_name.unwrap_or("ANTEGEN_TRANSACTIONS").to_string();
        let subject_prefix = subject_prefix.unwrap_or("antegen.tx").to_string();

        // Create or get stream for publishing transactions
        let _stream = jetstream
            .get_or_create_stream(StreamConfig {
                name: stream_name.clone(),
                subjects: vec![format!("{}.>", subject_prefix)],
                retention: RetentionPolicy::WorkQueue,
                max_age: Duration::from_secs(30),
                storage: jetstream::stream::StorageType::Memory,
                ..Default::default()
            })
            .await?;

        info!(
            "Created/retrieved stream '{}' with subject prefix '{}'",
            stream_name, subject_prefix
        );

        Ok(Self {
            jetstream,
            stream_name,
            subject_prefix,
        })
    }

    /// Publish a built transaction to the stream
    pub async fn publish(&self, tx: &BuiltTransaction) -> Result<()> {
        // Create subject based on thread and builder
        let subject = format!(
            "{}.{}.{}",
            self.subject_prefix,
            hex::encode(&tx.thread_pubkey.to_bytes()[0..8]), // First 8 bytes of thread pubkey
            tx.builder_id
        );

        // Serialize transaction using Anchor serialization
        let payload = tx.try_to_vec()?;

        // Publish to JetStream
        let ack = self
            .jetstream
            .publish(subject.clone(), payload.into())
            .await?
            .await?;

        debug!(
            "Published transaction {} to NATS (stream: {}, seq: {})",
            tx.id, ack.stream, ack.sequence
        );

        Ok(())
    }

    /// Publish with custom subject
    pub async fn publish_with_subject(&self, subject: &str, tx: &BuiltTransaction) -> Result<()> {
        let full_subject = format!("{}.{}", self.subject_prefix, subject);
        let payload = tx.try_to_vec()?;

        let ack = self
            .jetstream
            .publish(full_subject, payload.into())
            .await?
            .await?;

        debug!(
            "Published transaction {} to custom subject (seq: {})",
            tx.id, ack.sequence
        );

        Ok(())
    }

    /// Get stream info for monitoring
    pub async fn get_stream_info(&self) -> Result<StreamInfo> {
        let mut stream = self.jetstream.get_stream(&self.stream_name).await?;
        let info = stream.info().await?;

        Ok(StreamInfo {
            messages: info.state.messages,
            bytes: info.state.bytes,
            consumer_count: info.state.consumer_count,
        })
    }
}

#[derive(Debug)]
pub struct StreamInfo {
    pub messages: u64,
    pub bytes: u64,
    pub consumer_count: usize,
}
