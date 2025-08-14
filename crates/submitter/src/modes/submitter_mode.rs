use anyhow::Result;
use log::info;
use std::sync::Arc;
use solana_sdk::signer::keypair::Keypair;

use crate::service::{SubmitterService, SubmitterConfig};

/// Submitter mode: Only submits transactions (listens to NATS and submits)
pub struct SubmitterMode {
    service: SubmitterService,
}

impl SubmitterMode {
    pub async fn new(
        config: SubmitterConfig,
        submitter_keypair: Arc<Keypair>,
    ) -> Result<Self> {
        info!(
            "Initializing Submitter mode - NATS: {}",
            match &config.mode {
                crate::service::SubmitterMode::Submitter { nats_url, .. } => nats_url,
                _ => "not configured",
            }
        );
        
        // Extract NATS config from mode
        let (nats_url, consumer_name) = match &config.mode {
            crate::service::SubmitterMode::Submitter { nats_url, consumer_name } => {
                (nats_url.as_str(), consumer_name.as_str())
            }
            _ => {
                return Err(anyhow::anyhow!("Submitter mode requires NATS configuration"));
            }
        };
        
        // Create submitter service with NATS consumer
        let service = SubmitterService::with_nats_queue(
            config.rpc_url,
            config.ws_url,
            submitter_keypair,
            nats_url,
            consumer_name,
        ).await?;
        
        Ok(Self { service })
    }
    
    pub async fn run(mut self) -> Result<()> {
        info!("Starting Submitter mode - listening for transactions to submit");
        
        // Run the submitter service
        self.service.run().await
    }
}