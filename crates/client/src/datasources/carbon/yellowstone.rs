use anyhow::Result;
use async_trait::async_trait;
use carbon_core::pipeline::Pipeline;
use carbon_log_metrics::LogMetrics;
use carbon_yellowstone_grpc_datasource::{YellowstoneGrpcGeyserClient, BlockFilters};
use tokio::sync::mpsc;
use log::info;
use solana_sdk::sysvar;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use yellowstone_grpc_proto::geyser::{SubscribeRequestFilterAccounts, CommitmentLevel};

use crate::builder::DatasourceBuilder;
use antegen_processor::types::AccountUpdate;

use super::{CarbonConfig, processor::{BasicAccountDecoder, ThreadAccountProcessor}};

/// Carbon Yellowstone datasource configuration
pub struct CarbonYellowstoneConfig {
    /// Base Carbon configuration
    pub carbon_config: CarbonConfig,
    /// Yellowstone gRPC endpoint
    pub endpoint: String,
    /// Authentication token
    pub token: String,
}

/// Carbon Yellowstone datasource for monitoring thread accounts
pub struct CarbonYellowstoneDatasource {
    config: CarbonYellowstoneConfig,
}

impl CarbonYellowstoneDatasource {
    /// Create a new Carbon Yellowstone datasource
    pub fn new(carbon_config: CarbonConfig, endpoint: String, token: String) -> Self {
        Self {
            config: CarbonYellowstoneConfig {
                carbon_config,
                endpoint,
                token,
            },
        }
    }
}

#[async_trait]
impl DatasourceBuilder for CarbonYellowstoneDatasource {
    async fn run(&self, sender: mpsc::Sender<AccountUpdate>) -> Result<()> {
        info!("Starting Carbon Yellowstone gRPC datasource");
        info!("Thread program ID: {}", self.config.carbon_config.thread_program_id);
        info!("Clock sysvar ID: {}", sysvar::clock::ID);
        info!("Endpoint: {}", self.config.endpoint);

        // Create the processor with the sender channel
        let processor = ThreadAccountProcessor::new(
            sender, 
            self.config.carbon_config.thread_program_id
        );
        
        // Create metrics
        let metrics = Arc::new(LogMetrics::new());
        
        // Create account filters for the thread program AND Clock sysvar
        let mut account_filters = HashMap::new();
        
        // Filter for thread program accounts
        account_filters.insert(
            "thread_program".to_string(),
            SubscribeRequestFilterAccounts {
                account: vec![],
                owner: vec![self.config.carbon_config.thread_program_id.to_string()],
                filters: vec![],
                nonempty_txn_signature: None,
            }
        );
        
        // Filter for Clock sysvar specifically
        account_filters.insert(
            "clock_sysvar".to_string(),
            SubscribeRequestFilterAccounts {
                account: vec![sysvar::clock::ID.to_string()],
                owner: vec![],
                filters: vec![],
                nonempty_txn_signature: None,
            }
        );
        
        let datasource = YellowstoneGrpcGeyserClient::new(
            self.config.endpoint.clone(),
            Some(self.config.token.clone()),
            Some(CommitmentLevel::Confirmed),
            account_filters,
            HashMap::new(), // No transaction filters
            BlockFilters::default(),
            Arc::new(RwLock::new(HashSet::new()))
        );
        
        // Build and run pipeline
        Pipeline::builder()
            .datasource(datasource)
            .metrics(metrics)
            .account(BasicAccountDecoder, processor)
            .build()?
            .run()
            .await?;
            
        Ok(())
    }
}