use anyhow::{anyhow, Result};
use crossbeam::channel::{bounded, Receiver, Sender};
use log::info;
use std::sync::Arc;
use tokio::task::JoinHandle;

use antegen_processor::types::AccountUpdate;
use antegen_submitter::SubmissionService;

/// Main Antegen client that orchestrates all components
pub struct AntegenClient {
    /// Handles for datasource services
    datasource_handles: Vec<JoinHandle<()>>,
    /// Handle for processor service
    processor_handle: Option<JoinHandle<Result<()>>>,
    /// Reference to submission service (shared with processor)
    submitter: Option<Arc<SubmissionService>>,
    /// Global RPC URL for all components
    rpc_url: Option<String>,
}

impl AntegenClient {
    /// Create a new builder
    pub fn builder() -> AntegenClientBuilder {
        AntegenClientBuilder::default()
    }

    /// Run the client and wait for completion
    pub async fn run(self) -> Result<()> {
        info!("Starting AntegenClient");

        // Wait for RPC connection globally before starting any services
        if let Some(rpc_url) = &self.rpc_url {
            let ws_url = rpc_url.replace("http://", "ws://").replace(":8899", ":8900");
            info!("Waiting for validator to be ready before starting services...");
            
            // Add a small delay to let validator finish initialization
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            
            // Wait for validator with better error handling
            match crate::utils::wait_for_validator(rpc_url, &ws_url).await {
                Ok(()) => {
                    info!("Validator ready, starting all services");
                }
                Err(e) => {
                    log::error!("Failed to connect to validator: {}", e);
                    return Err(e);
                }
            }
        }

        // Wait for all components
        let mut handles = vec![];

        // Add datasource handles
        for handle in self.datasource_handles {
            handles.push(tokio::spawn(async move {
                handle.await.ok();
                Ok(())
            }));
        }


        // Add processor handle
        if let Some(handle) = self.processor_handle {
            handles.push(handle);
        }

        // Wait for any handle to complete (usually due to error or shutdown)
        tokio::select! {
            _ = futures::future::join_all(handles) => {
                info!("AntegenClient shutting down");
            }
        }

        Ok(())
    }

    /// Run on a specific runtime (for Geyser plugin)
    pub fn run_on(self, runtime: &tokio::runtime::Handle) -> Result<()> {
        runtime.block_on(self.run())
    }
}

/// Builder for AntegenClient
#[derive(Default)]
pub struct AntegenClientBuilder {
    datasource_builders: Vec<Box<dyn DatasourceBuilder>>,
    processor_builder: Option<antegen_processor::builder::ProcessorBuilder>,
    submitter_builder: Option<antegen_submitter::builder::SubmitterBuilder>,
    global_metrics: Option<opentelemetry::metrics::Meter>,
    rpc_url: Option<String>,
}

impl AntegenClientBuilder {
    /// Add multiple datasources that will feed into a single adapter
    pub fn datasources<I>(mut self, sources: I) -> Self
    where
        I: IntoIterator<Item = Box<dyn DatasourceBuilder>>,
    {
        self.datasource_builders.extend(sources);
        self
    }

    /// Add a single datasource
    pub fn datasource(mut self, source: Box<dyn DatasourceBuilder>) -> Self {
        self.datasource_builders.push(source);
        self
    }


    /// Configure the processor
    pub fn processor(mut self, processor: antegen_processor::builder::ProcessorBuilder) -> Self {
        self.processor_builder = Some(processor);
        self
    }

    /// Configure the submitter
    pub fn submitter(mut self, submitter: antegen_submitter::builder::SubmitterBuilder) -> Self {
        self.submitter_builder = Some(submitter);
        self
    }

    /// Set global metrics for all components
    pub fn metrics(mut self, meter: opentelemetry::metrics::Meter) -> Self {
        self.global_metrics = Some(meter);
        self
    }

    /// Set RPC URL for all components
    pub fn rpc_url<S: Into<String>>(mut self, url: S) -> Self {
        self.rpc_url = Some(url.into());
        self
    }

    /// Build the client with automatic wiring
    pub async fn build(self) -> Result<AntegenClient> {
        let mut client = AntegenClient {
            datasource_handles: vec![],
            processor_handle: None,
            submitter: None,
            rpc_url: self.rpc_url.clone(),
        };

        // Create shared event channel if we have datasources
        let (event_tx, event_rx): (
            Option<Sender<AccountUpdate>>,
            Option<Receiver<AccountUpdate>>,
        ) = if !self.datasource_builders.is_empty() {
            let (tx, rx) = bounded(1000);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

        // Spawn datasource services (all share the same sender)
        if let Some(tx) = event_tx {
            for datasource_builder in self.datasource_builders {
                let tx_clone = tx.clone();
                let handle = tokio::spawn(async move {
                    if let Err(e) = datasource_builder.run(tx_clone).await {
                        log::error!("Datasource error: {}", e);
                    }
                });
                client.datasource_handles.push(handle);
            }
            info!(
                "Started {} datasource services",
                client.datasource_handles.len()
            );
        }

        // Use event_rx directly as account_rx since datasources now emit AccountUpdate
        let account_rx = event_rx;

        // Build processor if configured
        if let Some(mut processor_builder) = self.processor_builder {
            let account_rx = account_rx
                .ok_or_else(|| anyhow!("Processor requires datasource or custom event source"))?;

            // Apply global metrics if available
            if let Some(meter) = &self.global_metrics {
                processor_builder = processor_builder.metrics(meter.clone());
            }

            let processor_service = processor_builder
                .account_receiver(account_rx)
                .build()
                .await?;

            client.processor_handle =
                Some(tokio::spawn(async move { processor_service.run().await }));

            info!("Started processor service");
        }

        // Build submitter if configured (can be standalone for replay)
        if let Some(mut submitter_builder) = self.submitter_builder {
            // Apply global metrics if available
            if let Some(meter) = &self.global_metrics {
                submitter_builder = submitter_builder.metrics(meter.clone());
            }

            let submission_service = Arc::new(submitter_builder.build().await?);
            client.submitter = Some(submission_service);

            info!("Created submission service");
        }

        Ok(client)
    }
}

/// Trait for datasource builders that can run with a sender
#[async_trait::async_trait]
pub trait DatasourceBuilder: Send + Sync {
    /// Run the datasource, sending events to the provided channel
    async fn run(&self, sender: Sender<AccountUpdate>) -> Result<()>;
}
