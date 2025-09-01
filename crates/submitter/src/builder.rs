use anyhow::Result;
use crossbeam::channel::Receiver;
use std::sync::Arc;

use crate::{
    ReplayConfig, SubmissionConfig, SubmissionMode, SubmissionService, SubmitterMetrics, TpuConfig,
};
use antegen_sdk::ProcessorMessage;

/// Builder for SubmissionService
pub struct SubmitterBuilder {
    rpc_url: String,
    submission_mode: SubmissionMode,
    tpu_config: Option<TpuConfig>,
    replay_config: ReplayConfig,
    metrics: Option<Arc<SubmitterMetrics>>,
    transaction_receiver: Option<Receiver<ProcessorMessage>>,
    executor_keypair: Option<Arc<solana_sdk::signature::Keypair>>,
}

impl Default for SubmitterBuilder {
    fn default() -> Self {
        Self {
            rpc_url: "http://localhost:8899".to_string(),
            submission_mode: SubmissionMode::default(),
            tpu_config: Some(TpuConfig::default()),
            replay_config: ReplayConfig::default(),
            metrics: None,
            transaction_receiver: None,
            executor_keypair: None,
        }
    }
}

impl SubmitterBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a replay-only submitter
    pub fn replay_only(nats_url: impl Into<String>) -> Self {
        let mut replay_config = ReplayConfig::default();
        replay_config.enable_replay = true;
        replay_config.nats_url = Some(nats_url.into());

        Self::default()
            .submission_mode(SubmissionMode::Rpc)
            .replay_config(replay_config)
    }

    /// Set RPC URL
    pub fn rpc_url(mut self, url: impl Into<String>) -> Self {
        self.rpc_url = url.into();
        self
    }

    /// Set submission mode
    pub fn submission_mode(mut self, mode: SubmissionMode) -> Self {
        self.submission_mode = mode;
        self
    }

    /// Enable TPU submission
    pub fn tpu_enabled(mut self) -> Self {
        self.submission_mode = SubmissionMode::TpuWithFallback;
        self.tpu_config = Some(TpuConfig::default());
        self
    }

    /// Disable TPU (RPC only)
    pub fn rpc_only(mut self) -> Self {
        self.submission_mode = SubmissionMode::Rpc;
        self.tpu_config = None;
        self
    }

    /// Set TPU configuration
    pub fn tpu_config(mut self, config: TpuConfig) -> Self {
        self.tpu_config = Some(config);
        self
    }

    /// Set replay configuration
    pub fn replay(mut self, config: ReplayConfig) -> Self {
        self.replay_config = config;
        self
    }

    /// Enable replay with optional NATS URL
    pub fn replay_if(mut self, enable: bool, nats_url: Option<String>) -> Self {
        if enable {
            self.replay_config.enable_replay = true;
            self.replay_config.nats_url = nats_url;
        }
        self
    }

    /// Set replay configuration from existing config
    pub fn replay_config(mut self, config: ReplayConfig) -> Self {
        self.replay_config = config;
        self
    }

    /// Set metrics
    pub fn metrics(mut self, meter: opentelemetry::metrics::Meter) -> Self {
        self.metrics = Some(SubmitterMetrics::new(&meter));
        self
    }

    /// Set transaction receiver from processor
    pub fn transaction_receiver(mut self, receiver: Receiver<ProcessorMessage>) -> Self {
        self.transaction_receiver = Some(receiver);
        self
    }

    /// Set executor keypair for signing transactions
    pub fn executor_keypair(mut self, keypair: Arc<solana_sdk::signature::Keypair>) -> Self {
        self.executor_keypair = Some(keypair);
        self
    }
    /// Build from existing config (for compatibility)
    pub fn from_config(config: SubmissionConfig) -> Self {
        Self {
            rpc_url: "http://localhost:8899".to_string(), // Will be overridden
            submission_mode: SubmissionMode::default(),
            tpu_config: config.tpu_config,
            replay_config: config.replay_config,
            metrics: None,
            transaction_receiver: None,
            executor_keypair: None,
        }
    }

    /// Build the submission service
    pub async fn build(self) -> Result<SubmissionService> {
        // Create submission configuration
        let config = SubmissionConfig {
            tpu_config: self.tpu_config,
            replay_config: self.replay_config,
        };

        // Store metrics before moving to service
        let metrics = self.metrics.clone();

        // Create submission service
        let service = SubmissionService::new(self.rpc_url, config, metrics.clone()).await?;

        // NOTE: Don't initialize here - let it happen lazily when the service starts
        // Otherwise we block the Geyser plugin load which prevents the validator from starting

        // If transaction receiver is provided, start processing task
        if let Some(receiver) = self.transaction_receiver {
            let executor_keypair = self.executor_keypair.ok_or_else(|| {
                anyhow::anyhow!("Executor keypair required when using transaction receiver")
            })?;

            // Start the message processor directly (no worker pool)
            let service_arc = Arc::new(service.clone());
            tokio::spawn(async move {
                if let Err(e) = service_arc
                    .process_transaction_messages(receiver, executor_keypair)
                    .await
                {
                    log::error!("Transaction processor error: {}", e);
                }
            });
        }

        Ok(service)
    }
}
