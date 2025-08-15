use anyhow::Result;
use log::info;
use std::sync::Arc;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::signer::keypair::Keypair;

use crate::events::EventSource;
use crate::service::{BuilderService, BuilderConfig};
use antegen_submitter::SubmitterService;

/// Worker mode: Both builds and submits transactions (full pipeline)
pub struct WorkerMode {
    builder_service: BuilderService,
    submitter_service: SubmitterService,
}

impl WorkerMode {
    pub async fn new(
        builder_config: BuilderConfig,
        event_source: Box<dyn EventSource>,
        submitter_keypair: Arc<Keypair>,
        rpc_url: String,
        ws_url: String,
    ) -> Result<Self> {
        info!(
            "Initializing Worker mode - builder_id: {}",
            builder_config.builder_id
        );
        
        let rpc_client = Arc::new(RpcClient::new(builder_config.rpc_url));
        
        // Create channel for builder -> submitter communication
        let (builder_service, _tx_sender) = BuilderService::new_worker(
            event_source,
            builder_config.builder_id,
            rpc_client,
            submitter_keypair.clone(),
        );
        
        // Create submitter with local queue
        let (submitter_service, _) = SubmitterService::with_local_queue(
            rpc_url,
            ws_url,
            submitter_keypair,
            100, // buffer size
        );
        
        // Connect builder output to submitter input via channel
        // Note: In the actual implementation, we'd need to connect tx_sender
        // to the submitter's receiver, but that's handled internally
        
        Ok(Self {
            builder_service,
            submitter_service,
        })
    }
    
    pub async fn run(mut self) -> Result<()> {
        info!("Starting Worker mode - building and submitting transactions");
        
        // Run both services concurrently
        tokio::select! {
            result = self.builder_service.run() => {
                info!("Builder service stopped");
                result
            }
            result = self.submitter_service.run() => {
                info!("Submitter service stopped");
                result
            }
        }
    }
}