use anyhow::Result;
use crossbeam::channel::Receiver;
use std::sync::Arc;

use crate::{
    ReplayConfig, SubmissionConfig, SubmissionMode, SubmissionService, SubmitterMetrics, TpuConfig,
    TransactionWorkerPool, WorkerPoolConfig,
};
use antegen_sdk::types::TransactionMessage;

/// Builder for SubmissionService
pub struct SubmitterBuilder {
    rpc_url: String,
    submission_mode: SubmissionMode,
    tpu_config: Option<TpuConfig>,
    replay_config: ReplayConfig,
    metrics: Option<Arc<SubmitterMetrics>>,
    transaction_receiver: Option<Receiver<TransactionMessage>>,
    executor_keypair: Option<Arc<solana_sdk::signature::Keypair>>,
    worker_pool_config: Option<WorkerPoolConfig>,
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
            worker_pool_config: None,
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
    pub fn transaction_receiver(mut self, receiver: Receiver<TransactionMessage>) -> Self {
        self.transaction_receiver = Some(receiver);
        self
    }

    /// Set executor keypair for signing transactions
    pub fn executor_keypair(mut self, keypair: Arc<solana_sdk::signature::Keypair>) -> Self {
        self.executor_keypair = Some(keypair);
        self
    }
    
    /// Enable worker pool with default configuration
    pub fn with_worker_pool(mut self) -> Self {
        self.worker_pool_config = Some(WorkerPoolConfig::default());
        self
    }
    
    /// Set custom worker pool configuration
    pub fn worker_pool_config(mut self, config: WorkerPoolConfig) -> Self {
        self.worker_pool_config = Some(config);
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
            worker_pool_config: None,
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

        // Initialize the service
        service.initialize().await?;

        // If transaction receiver is provided, start processing task
        if let Some(receiver) = self.transaction_receiver {
            let executor_keypair = self.executor_keypair.ok_or_else(|| {
                anyhow::anyhow!("Executor keypair required when using transaction receiver")
            })?;

            // Use worker pool if configured
            if let Some(pool_config) = self.worker_pool_config {
                let pool_metrics = metrics
                    .unwrap_or_else(|| Arc::new(SubmitterMetrics::default()));
                
                let worker_pool = Arc::new(TransactionWorkerPool::new(
                    Arc::new(service.clone()),
                    pool_config,
                    pool_metrics,
                ));
                
                tokio::spawn(async move {
                    if let Err(e) = worker_pool
                        .process_with_batching(receiver, executor_keypair)
                        .await
                    {
                        log::error!("Worker pool processor error: {}", e);
                    }
                });
            } else {
                // Use simple serial processing
                let service_clone = service.clone();
                tokio::spawn(async move {
                    if let Err(e) = service_clone
                        .process_transaction_messages(receiver, executor_keypair)
                        .await
                    {
                        log::error!("Transaction processor error: {}", e);
                    }
                });
            }
        }

        Ok(service)
    }
}
