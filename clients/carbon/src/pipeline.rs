use anyhow::Result;
use log::{error, info};
use solana_sdk::signature::{read_keypair_file, Signer};
use std::sync::Arc;

use antegen_client::AntegenClient;
use antegen_adapter::builder::AdapterBuilder;
use antegen_processor::builder::ProcessorBuilder;
use antegen_submitter::builder::SubmitterBuilder;

use crate::config::Config;
use crate::datasources::create_datasource;
use crate::datasource_builder::CarbonDatasourceBuilder;

/// Run the Carbon pipeline using the builder pattern
pub async fn run_carbon_pipeline(config: Config) -> Result<()> {
    info!("Initializing Carbon pipeline with builder pattern");
    
    // Validate keypair
    let keypair = Arc::new(
        read_keypair_file(&config.keypair_path)
            .map_err(|e| anyhow::anyhow!("Failed to read keypair: {}", e))?
    );
    let executor_pubkey = keypair.pubkey();
    info!("Executor pubkey: {}", executor_pubkey);
    
    // Create Carbon datasource
    let datasource = create_datasource(&config).await?;
    let datasource_builder = Box::new(
        CarbonDatasourceBuilder::new(datasource, config.thread_program_id)
    );
    
    // Build the client using the builder pattern
    let client = AntegenClient::builder()
        .datasource(datasource_builder)
        .adapter(
            AdapterBuilder::carbon()
                .adapter_pubkey(executor_pubkey)
                .buffer_size(1000)
        )
        .processor(
            ProcessorBuilder::new()
                .keypair(config.keypair_path.to_string_lossy())
                .rpc_url(config.rpc_url.clone())
                .forgo_commission(config.forgo_commission)
        )
        .submitter(
            SubmitterBuilder::new()
                .rpc_url(config.rpc_url.clone())
                .executor_keypair(keypair.clone())
                .replay_config(build_replay_config(&config))
        )
        .build()
        .await?;
    
    info!("Starting Carbon pipeline");
    
    // Run the client
    match client.run().await {
        Ok(()) => info!("Carbon pipeline completed normally"),
        Err(e) => error!("Carbon pipeline error: {}", e),
    }
    
    info!("Carbon worker shutting down");
    Ok(())
}

fn build_replay_config(config: &Config) -> antegen_submitter::ReplayConfig {
    let mut replay_config = antegen_submitter::ReplayConfig::default();
    
    if config.replay.enabled {
        replay_config.enable_replay = true;
        replay_config.nats_url = Some(config.replay.nats_url.clone());
    }
    
    replay_config
}