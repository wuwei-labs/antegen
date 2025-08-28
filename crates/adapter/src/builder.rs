use anyhow::Result;
use crossbeam::channel::Receiver;
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;

use crate::{
    events::{CarbonEventSource, GeyserEventSource, ObservedEvent, EventSource},
    metrics::AdapterMetrics,
    service::AdapterService,
    AccountUpdate,
};

/// Builder for AdapterService
pub struct AdapterBuilder {
    event_receiver: Option<Receiver<ObservedEvent>>,
    event_source: Option<Box<dyn EventSource>>,
    adapter_pubkey: Option<Pubkey>,
    buffer_size: usize,
    metrics: Option<Arc<AdapterMetrics>>,
}

impl Default for AdapterBuilder {
    fn default() -> Self {
        Self {
            event_receiver: None,
            event_source: None,
            adapter_pubkey: None,
            buffer_size: 1000,
            metrics: None,
        }
    }
}

impl AdapterBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Create a Carbon-specific adapter
    pub fn carbon() -> Self {
        Self::default().buffer_size(1000)
    }
    
    /// Create a Geyser-specific adapter
    pub fn geyser() -> Self {
        Self::default().buffer_size(2000)
    }
    
    /// Set the event receiver (for Carbon/Geyser sources)
    pub fn event_receiver(mut self, receiver: Receiver<ObservedEvent>) -> Self {
        self.event_receiver = Some(receiver);
        self
    }
    
    /// Set a custom event source
    pub fn event_source(mut self, source: Box<dyn EventSource>) -> Self {
        self.event_source = Some(source);
        self
    }
    
    /// Set the adapter pubkey
    pub fn adapter_pubkey(mut self, pubkey: Pubkey) -> Self {
        self.adapter_pubkey = Some(pubkey);
        self
    }
    
    /// Set the buffer size
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }
    
    /// Set metrics
    pub fn metrics(mut self, meter: opentelemetry::metrics::Meter) -> Self {
        self.metrics = Some(Arc::new(AdapterMetrics::new(&meter)));
        self
    }
    
    /// Build the adapter service
    pub fn build(self) -> Result<(AdapterService, Receiver<AccountUpdate>)> {
        // Create event source based on what's provided
        let event_source: Box<dyn EventSource> = if let Some(source) = self.event_source {
            source
        } else if let Some(receiver) = self.event_receiver {
            // Auto-detect source type (default to Carbon)
            Box::new(CarbonEventSource::new(receiver))
        } else {
            return Err(anyhow::anyhow!("Either event_receiver or event_source must be provided"));
        };
        
        // Use default pubkey if not provided
        let adapter_pubkey = self.adapter_pubkey.unwrap_or_default();
        
        // Create adapter with configured buffer size
        let (tx, rx) = crossbeam::channel::bounded(self.buffer_size);
        
        let mut service = AdapterService::new_with_source(event_source, adapter_pubkey, tx);
        
        // Set metrics if provided
        if let Some(metrics) = self.metrics {
            service.set_metrics(metrics);
        }
        
        Ok((service, rx))
    }
    
    /// Create a Carbon event source adapter
    pub fn carbon_source(receiver: Receiver<ObservedEvent>) -> Box<dyn EventSource> {
        Box::new(CarbonEventSource::new(receiver))
    }
    
    /// Create a Geyser event source adapter  
    pub fn geyser_source(receiver: Receiver<ObservedEvent>) -> Box<dyn EventSource> {
        Box::new(GeyserEventSource::new(receiver))
    }
}