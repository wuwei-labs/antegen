use anyhow::Result;
use log::info;
use std::sync::Arc;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::signature::{Keypair, read_keypair_file};

use crate::events::EventSource;
use crate::service::{BuilderService, BuilderConfig};

/// Builder mode: Observes blockchain events and builds transactions, publishes to NATS
pub struct BuilderMode {
    service: BuilderService,
}

impl BuilderMode {
    pub async fn new(
        config: BuilderConfig,
        event_source: Box<dyn EventSource>,
    ) -> Result<Self> {
        info!(
            "Initializing Builder mode - builder_id: {}, NATS: {:?}",
            config.builder_id,
            config.nats_url
        );
        
        let rpc_client = Arc::new(RpcClient::new(config.rpc_url));
        
        // Load keypair
        let keypair = read_keypair_file(&config.keypair_path)
            .map_err(|e| anyhow::anyhow!("Failed to load keypair: {}", e))?;
        let keypair = Arc::new(keypair);
        
        // Create builder service with NATS publisher
        let service = BuilderService::new_builder(
            event_source,
            config.builder_id,
            rpc_client,
            keypair,
            config.nats_url.as_ref().unwrap_or(&"nats://localhost:4222".to_string()),
        ).await?;
        
        Ok(Self { service })
    }
    
    pub async fn run(mut self) -> Result<()> {
        info!("Starting Builder mode - building and publishing transactions");
        
        // Run the builder service
        self.service.run().await
    }
}