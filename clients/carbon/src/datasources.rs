use anyhow::{anyhow, Result};
use log::info;

use crate::config::{Config, DatasourceType};

pub enum CarbonDatasource {
    Rpc(carbon_rpc_program_subscribe_datasource::RpcProgramSubscribe),
    Helius(carbon_rpc_program_subscribe_datasource::RpcProgramSubscribe), // Using RPC as fallback
    Yellowstone(carbon_yellowstone_grpc_datasource::YellowstoneGrpcGeyserClient),
}

/// Create a datasource based on configuration
pub async fn create_datasource(config: &Config) -> Result<CarbonDatasource> {
    match &config.datasource {
        DatasourceType::Rpc => create_rpc_datasource(config).await,
        DatasourceType::Helius => create_helius_datasource(config).await,
        DatasourceType::Yellowstone => create_yellowstone_datasource(config).await,
    }
}

/// Create RPC Program Subscribe datasource
async fn create_rpc_datasource(config: &Config) -> Result<CarbonDatasource> {
    info!("Creating RPC Program Subscribe datasource");
    info!("RPC URL: {}", config.rpc_url);
    info!("Program ID: {}", config.thread_program_id);

    use carbon_rpc_program_subscribe_datasource::{Filters, RpcProgramSubscribe};

    // Create filters for the thread program
    let filters = Filters::new(config.thread_program_id, None);

    // Create the datasource
    let datasource = RpcProgramSubscribe::new(config.rpc_url.clone(), filters);

    Ok(CarbonDatasource::Rpc(datasource))
}

/// Create Helius Atlas WebSocket datasource
async fn create_helius_datasource(config: &Config) -> Result<CarbonDatasource> {
    let helius_config = config
        .helius
        .as_ref()
        .ok_or_else(|| anyhow!("Helius configuration required"))?;

    info!("Creating Helius Atlas WebSocket datasource");
    info!("WebSocket URL: {}", helius_config.ws_url);
    info!("Program ID: {}", config.thread_program_id);

    // For Helius, we can't use the HeliusWebsocket directly due to incompatible helius version
    // Instead, we'll need to use the RPC datasource as a fallback for now
    info!("Note: Using RPC datasource as fallback for Helius integration");
    info!("Full Helius WebSocket support requires matching helius crate versions");
    
    use carbon_rpc_program_subscribe_datasource::{Filters, RpcProgramSubscribe};

    // Create filters for the thread program
    let filters = Filters::new(config.thread_program_id, None);

    // Create the datasource using Helius RPC endpoint
    let datasource = RpcProgramSubscribe::new(helius_config.ws_url.clone(), filters);

    Ok(CarbonDatasource::Helius(datasource))
}

/// Create Yellowstone gRPC datasource
async fn create_yellowstone_datasource(config: &Config) -> Result<CarbonDatasource> {
    let yellowstone_config = config
        .yellowstone
        .as_ref()
        .ok_or_else(|| anyhow!("Yellowstone configuration required"))?;

    info!("Creating Yellowstone gRPC datasource");
    info!("Endpoint: {}", yellowstone_config.endpoint);
    info!("Program ID: {}", config.thread_program_id);

    use carbon_yellowstone_grpc_datasource::{YellowstoneGrpcGeyserClient, BlockFilters};
    use yellowstone_grpc_proto::geyser::{SubscribeRequestFilterAccounts, CommitmentLevel};
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;
    use tokio::sync::RwLock;

    // Create account filters for the thread program
    let mut account_filters = HashMap::new();
    account_filters.insert(
        "thread_program".to_string(),
        SubscribeRequestFilterAccounts {
            account: vec![],
            owner: vec![config.thread_program_id.to_string()],
            filters: vec![],
            nonempty_txn_signature: None,
        }
    );
    
    // Create empty transaction filters
    let transaction_filters = HashMap::new();
    
    // Create block filters
    let block_filters = BlockFilters::default();
    
    // Create account deletions tracker
    let account_deletions_tracked = Arc::new(RwLock::new(HashSet::new()));

    // Create the datasource
    let datasource = YellowstoneGrpcGeyserClient::new(
        yellowstone_config.endpoint.clone(),
        Some(yellowstone_config.token.clone()),
        Some(CommitmentLevel::Confirmed),
        account_filters,
        transaction_filters,
        block_filters,
        account_deletions_tracked
    );

    Ok(CarbonDatasource::Yellowstone(datasource))
}
