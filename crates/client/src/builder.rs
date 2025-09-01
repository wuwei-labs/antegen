use anyhow::{anyhow, Result};
use crossbeam::channel::{bounded, Receiver, Sender};
use log::info;
use std::sync::Arc;
use tokio::task::JoinHandle;

use antegen_adapter::events::ObservedEvent;
use antegen_sdk::ProcessorMessage;
use antegen_submitter::SubmissionService;

/// Main Antegen client that orchestrates all components
pub struct AntegenClient {
    /// Handles for datasource services
    datasource_handles: Vec<JoinHandle<()>>,
    /// Handle for adapter service
    adapter_handle: Option<JoinHandle<Result<()>>>,
    /// Handle for processor service
    processor_handle: Option<JoinHandle<Result<()>>>,
    /// Reference to submission service (shared with processor)
    submitter: Option<Arc<SubmissionService>>,
}

impl AntegenClient {
    /// Create a new builder
    pub fn builder() -> AntegenClientBuilder {
        AntegenClientBuilder::default()
    }

    /// Run the client and wait for completion
    pub async fn run(self) -> Result<()> {
        info!("Starting AntegenClient");

        // Wait for all components
        let mut handles = vec![];

        // Add datasource handles
        for handle in self.datasource_handles {
            handles.push(tokio::spawn(async move {
                handle.await.ok();
                Ok(())
            }));
        }

        // Add adapter handle
        if let Some(handle) = self.adapter_handle {
            handles.push(handle);
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
    adapter_builder: Option<antegen_adapter::builder::AdapterBuilder>,
    processor_builder: Option<antegen_processor::builder::ProcessorBuilder>,
    submitter_builder: Option<antegen_submitter::builder::SubmitterBuilder>,
    global_metrics: Option<opentelemetry::metrics::Meter>,
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

    /// Configure the adapter
    pub fn adapter(mut self, adapter: antegen_adapter::builder::AdapterBuilder) -> Self {
        self.adapter_builder = Some(adapter);
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

    /// Build the client with automatic wiring
    pub async fn build(self) -> Result<AntegenClient> {
        let mut client = AntegenClient {
            datasource_handles: vec![],
            adapter_handle: None,
            processor_handle: None,
            submitter: None,
        };

        // Create shared event channel if we have datasources
        let (event_tx, event_rx): (
            Option<Sender<ObservedEvent>>,
            Option<Receiver<ObservedEvent>>,
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

        // Build adapter if configured or if we have datasources
        let account_rx = if let Some(rx) = event_rx {
            let mut adapter_builder = self
                .adapter_builder
                .unwrap_or_else(|| antegen_adapter::builder::AdapterBuilder::default());

            // Apply global metrics if available
            if let Some(meter) = &self.global_metrics {
                adapter_builder = adapter_builder.metrics(meter.clone());
            }

            let (mut adapter_service, account_rx) = adapter_builder.event_receiver(rx).build()?;

            client.adapter_handle = Some(tokio::spawn(async move { adapter_service.run().await }));

            info!("Started adapter service");
            Some(account_rx)
        } else {
            None
        };

        // Create transaction channel if both processor and submitter are configured
        let (transaction_tx, transaction_rx): (
            Option<Sender<ProcessorMessage>>,
            Option<Receiver<ProcessorMessage>>,
        ) = if self.processor_builder.is_some() && self.submitter_builder.is_some() {
            let (tx, rx) = bounded(100);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

        // Build processor if configured
        if let Some(mut processor_builder) = self.processor_builder {
            let account_rx = account_rx
                .ok_or_else(|| anyhow!("Processor requires adapter or custom event source"))?;

            let transaction_tx = transaction_tx.ok_or_else(|| {
                anyhow!("Processor requires transaction sender (submitter must be configured)")
            })?;

            // Apply global metrics if available
            if let Some(meter) = &self.global_metrics {
                processor_builder = processor_builder.metrics(meter.clone());
            }

            let processor_service = processor_builder
                .account_receiver(account_rx)
                .transaction_sender(transaction_tx)
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

            // Add transaction receiver if processor is configured
            if let Some(rx) = transaction_rx {
                submitter_builder = submitter_builder.transaction_receiver(rx);
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
    async fn run(&self, sender: Sender<ObservedEvent>) -> Result<()>;
}
