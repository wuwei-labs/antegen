use anyhow::{anyhow, Result};
use crossbeam::channel::{Receiver, Sender};
use std::sync::Arc;

use crate::{
    types::{ProcessorConfig, AccountUpdate},
    service::ProcessorService,
    metrics::ProcessorMetrics,
};
use antegen_sdk::types::TransactionMessage;

/// Builder for ProcessorService
pub struct ProcessorBuilder {
    keypair_path: Option<String>,
    rpc_url: String,
    forgo_commission: bool,
    max_concurrent_threads: usize,
    simulate_before_submit: bool,
    compute_unit_multiplier: f64,
    max_compute_units: u32,
    account_receiver: Option<Receiver<AccountUpdate>>,
    transaction_sender: Option<Sender<TransactionMessage>>,
    metrics: Option<Arc<ProcessorMetrics>>,
}

impl Default for ProcessorBuilder {
    fn default() -> Self {
        let config = ProcessorConfig::default();
        Self {
            keypair_path: None,
            rpc_url: config.rpc_url,
            forgo_commission: config.forgo_executor_commission,
            max_concurrent_threads: config.max_concurrent_threads,
            simulate_before_submit: config.simulate_before_submit,
            compute_unit_multiplier: config.compute_unit_multiplier,
            max_compute_units: config.max_compute_units,
            account_receiver: None,
            transaction_sender: None,
            metrics: None,
        }
    }
}

impl ProcessorBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Create a CLI-specific processor
    pub fn cli() -> Self {
        Self::default()
            .simulate_transactions(true)
            .max_concurrent_threads(10)
    }
    
    /// Set keypair path
    pub fn keypair(mut self, path: impl Into<String>) -> Self {
        self.keypair_path = Some(path.into());
        self
    }
    
    /// Set keypair path (alias for backward compatibility)
    pub fn keypair_path(self, path: impl Into<String>) -> Self {
        self.keypair(path)
    }
    
    /// Set RPC URL
    pub fn rpc_url(mut self, url: impl Into<String>) -> Self {
        self.rpc_url = url.into();
        self
    }
    
    /// Set whether to forgo executor commission
    pub fn forgo_commission(mut self, forgo: bool) -> Self {
        self.forgo_commission = forgo;
        self
    }
    
    /// Set maximum concurrent threads
    pub fn max_concurrent_threads(mut self, max: usize) -> Self {
        self.max_concurrent_threads = max;
        self
    }
    
    /// Set whether to simulate before submit
    pub fn simulate_transactions(mut self, simulate: bool) -> Self {
        self.simulate_before_submit = simulate;
        self
    }
    
    /// Set compute unit multiplier
    pub fn compute_unit_multiplier(mut self, multiplier: f64) -> Self {
        self.compute_unit_multiplier = multiplier;
        self
    }
    
    /// Set maximum compute units
    pub fn max_compute_units(mut self, max: u32) -> Self {
        self.max_compute_units = max;
        self
    }
    
    /// Set account receiver from adapter
    pub fn account_receiver(mut self, receiver: Receiver<AccountUpdate>) -> Self {
        self.account_receiver = Some(receiver);
        self
    }
    
    /// Set transaction sender to submitter
    pub fn transaction_sender(mut self, sender: Sender<TransactionMessage>) -> Self {
        self.transaction_sender = Some(sender);
        self
    }
    
    /// Set metrics
    pub fn metrics(mut self, meter: opentelemetry::metrics::Meter) -> Self {
        self.metrics = Some(Arc::new(ProcessorMetrics::new(&meter)));
        self
    }
    
    /// Build from existing config (for compatibility)
    pub fn from_config(config: ProcessorConfig) -> Self {
        Self {
            keypair_path: Some(config.executor_keypair_path),
            rpc_url: config.rpc_url,
            forgo_commission: config.forgo_executor_commission,
            max_concurrent_threads: config.max_concurrent_threads,
            simulate_before_submit: config.simulate_before_submit,
            compute_unit_multiplier: config.compute_unit_multiplier,
            max_compute_units: config.max_compute_units,
            account_receiver: None,
            transaction_sender: None,
            metrics: None,
        }
    }
    
    /// Build the processor service
    pub async fn build(self) -> Result<ProcessorService> {
        // Validate required fields
        let keypair_path = self.keypair_path
            .ok_or_else(|| anyhow!("Keypair path is required"))?;
        let account_receiver = self.account_receiver
            .ok_or_else(|| anyhow!("Account receiver is required"))?;
        let transaction_sender = self.transaction_sender
            .ok_or_else(|| anyhow!("Transaction sender is required"))?;
        
        // Create config
        let config = ProcessorConfig {
            executor_keypair_path: keypair_path,
            rpc_url: self.rpc_url,
            forgo_executor_commission: self.forgo_commission,
            max_concurrent_threads: self.max_concurrent_threads,
            simulate_before_submit: self.simulate_before_submit,
            compute_unit_multiplier: self.compute_unit_multiplier,
            max_compute_units: self.max_compute_units,
        };
        
        // Create service
        ProcessorService::new(config, account_receiver, transaction_sender).await
    }
}