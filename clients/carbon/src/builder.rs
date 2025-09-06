use anyhow::Result;
use async_trait::async_trait;
use carbon_core::pipeline::Pipeline;
use carbon_log_metrics::LogMetrics;
use crossbeam::channel::Sender;
use log::info;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{pubkey::Pubkey, sysvar};
use std::sync::Arc;

use crate::clock_tracker::track_clock;
use crate::processor::{BasicAccountDecoder, ThreadAccountProcessor};
use antegen_adapter::events::ObservedEvent;

/// Configuration for Carbon datasource builders
#[derive(Clone)]
pub struct CarbonConfig {
    pub thread_program_id: Pubkey,
    pub rpc_url: String,
}

/// RPC datasource builder for Carbon
pub struct RpcDatasourceBuilder {
    config: CarbonConfig,
}

impl RpcDatasourceBuilder {
    pub fn new(config: CarbonConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl antegen_client::builder::DatasourceBuilder for RpcDatasourceBuilder {
    async fn run(&self, sender: Sender<ObservedEvent>) -> Result<()> {
        info!("Starting Carbon RPC datasource");
        info!("Thread program ID: {}", self.config.thread_program_id);
        info!("Clock sysvar ID: {}", sysvar::clock::ID);
        info!("RPC URL: {}", self.config.rpc_url);

        // Convert RPC URL to WebSocket URL if needed
        let ws_url = if self.config.rpc_url.starts_with("ws://") || self.config.rpc_url.starts_with("wss://") {
            self.config.rpc_url.clone()
        } else {
            self.config.rpc_url
                .replace("http://", "ws://")
                .replace("https://", "wss://")
                .replace(":8899", ":8900")
        };
        
        // Wait for validator to be ready before initializing connections
        crate::connection_waiter::wait_for_validator(&self.config.rpc_url, &ws_url).await?;

        // Clone sender for both pipelines
        let sender_thread = sender.clone();
        let sender_clock = sender;
        
        // Create processor for thread accounts
        let processor_thread = ThreadAccountProcessor::new(sender_thread, self.config.thread_program_id);
        
        // Create metrics
        let metrics = Arc::new(LogMetrics::new());
        
        // Task 1: Subscribe to thread program accounts
        let thread_program_id = self.config.thread_program_id;
        let ws_url_thread = ws_url.clone();
        let metrics_thread = metrics.clone();
        let thread_task = tokio::spawn(async move {
            use carbon_rpc_program_subscribe_datasource::{Filters, RpcProgramSubscribe};
            let filters = Filters::new(thread_program_id, None);
            let datasource = RpcProgramSubscribe::new(ws_url_thread, filters);
            
            info!("Starting thread program subscription");
            Pipeline::builder()
                .datasource(datasource)
                .metrics(metrics_thread)
                .account(BasicAccountDecoder, processor_thread)
                .build()?
                .run()
                .await
        });
        
        // Task 2: Track Clock sysvar updates via WebSocket
        
        // Use HTTP URL for RPC client
        let rpc_url = self.config.rpc_url
            .replace("ws://", "http://")
            .replace("wss://", "https://")
            .replace(":8900", ":8899");
        
        let rpc_client = Arc::new(RpcClient::new(rpc_url));
        let clock_task = tokio::spawn(async move {
            info!("Starting Clock subscription");
            track_clock(ws_url, sender_clock, rpc_client).await
        });
        
        // Wait for both tasks
        let (thread_result, clock_result) = tokio::join!(thread_task, clock_task);
        
        // Return first error if any
        thread_result??;
        clock_result??;
            
        Ok(())
    }
}

/// Helius datasource builder for Carbon
pub struct HeliusDatasourceBuilder {
    config: CarbonConfig,
    ws_url: String,
}

impl HeliusDatasourceBuilder {
    pub fn new(config: CarbonConfig, ws_url: String) -> Self {
        Self { config, ws_url }
    }
}

#[async_trait]
impl antegen_client::builder::DatasourceBuilder for HeliusDatasourceBuilder {
    async fn run(&self, sender: Sender<ObservedEvent>) -> Result<()> {
        info!("Starting Carbon Helius datasource (using RPC fallback)");
        info!("Thread program ID: {}", self.config.thread_program_id);
        info!("Clock sysvar ID: {}", sysvar::clock::ID);
        info!("WebSocket URL: {}", self.ws_url);

        // Clone sender for both pipelines
        let sender_thread = sender.clone();
        let sender_clock = sender;
        
        // Create processor for thread accounts
        let processor_thread = ThreadAccountProcessor::new(sender_thread, self.config.thread_program_id);
        
        // Create metrics
        let metrics = Arc::new(LogMetrics::new());
        
        // Task 1: Subscribe to thread program accounts
        let thread_program_id = self.config.thread_program_id;
        let ws_url_thread = self.ws_url.clone();
        let metrics_thread = metrics.clone();
        let thread_task = tokio::spawn(async move {
            use carbon_rpc_program_subscribe_datasource::{Filters, RpcProgramSubscribe};
            let filters = Filters::new(thread_program_id, None);
            let datasource = RpcProgramSubscribe::new(ws_url_thread, filters);
            
            Pipeline::builder()
                .datasource(datasource)
                .metrics(metrics_thread)
                .account(BasicAccountDecoder, processor_thread)
                .build()?
                .run()
                .await
        });
        
        // Task 2: Track Clock sysvar updates via WebSocket
        let ws_url = self.ws_url.clone();
        // Use HTTP URL for RPC client (convert WebSocket URL if necessary)
        let rpc_url = self.config.rpc_url.clone();
        let rpc_client = Arc::new(RpcClient::new(rpc_url));
        let clock_task = tokio::spawn(async move {
            info!("Starting Clock subscription");
            track_clock(ws_url, sender_clock, rpc_client).await
        });
        
        // Wait for both tasks
        let (thread_result, clock_result) = tokio::join!(thread_task, clock_task);
        
        // Return first error if any
        thread_result??;
        clock_result??;
            
        Ok(())
    }
}

/// Yellowstone datasource builder for Carbon
pub struct YellowstoneDatasourceBuilder {
    config: CarbonConfig,
    endpoint: String,
    token: String,
}

impl YellowstoneDatasourceBuilder {
    pub fn new(config: CarbonConfig, endpoint: String, token: String) -> Self {
        Self { config, endpoint, token }
    }
}

#[async_trait]
impl antegen_client::builder::DatasourceBuilder for YellowstoneDatasourceBuilder {
    async fn run(&self, sender: Sender<ObservedEvent>) -> Result<()> {
        info!("Starting Carbon Yellowstone gRPC datasource");
        info!("Thread program ID: {}", self.config.thread_program_id);
        info!("Clock sysvar ID: {}", sysvar::clock::ID);
        info!("Endpoint: {}", self.endpoint);

        // Create the processor with the sender channel
        let processor = ThreadAccountProcessor::new(sender, self.config.thread_program_id);
        
        // Create metrics
        let metrics = Arc::new(LogMetrics::new());
        
        // Create Yellowstone datasource
        use carbon_yellowstone_grpc_datasource::{YellowstoneGrpcGeyserClient, BlockFilters};
        use yellowstone_grpc_proto::geyser::{SubscribeRequestFilterAccounts, CommitmentLevel};
        use std::collections::{HashMap, HashSet};
        use tokio::sync::RwLock;
        
        // Create account filters for the thread program AND Clock sysvar
        let mut account_filters = HashMap::new();
        
        // Filter for thread program accounts
        account_filters.insert(
            "thread_program".to_string(),
            SubscribeRequestFilterAccounts {
                account: vec![],
                owner: vec![self.config.thread_program_id.to_string()],
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
            self.endpoint.clone(),
            Some(self.token.clone()),
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

/// Factory function to create appropriate datasource builder based on config
pub fn create_datasource_builder(
    datasource_type: &crate::config::DatasourceType,
    config: &crate::config::Config,
) -> Result<Box<dyn antegen_client::builder::DatasourceBuilder>> {
    let carbon_config = CarbonConfig {
        thread_program_id: config.thread_program_id,
        rpc_url: config.rpc_url.clone(),
    };
    
    match datasource_type {
        crate::config::DatasourceType::Rpc => {
            Ok(Box::new(RpcDatasourceBuilder::new(carbon_config)))
        }
        crate::config::DatasourceType::Helius => {
            let helius_config = config.helius.as_ref()
                .ok_or_else(|| anyhow::anyhow!("Helius configuration required"))?;
            Ok(Box::new(HeliusDatasourceBuilder::new(
                carbon_config,
                helius_config.ws_url.clone(),
            )))
        }
        crate::config::DatasourceType::Yellowstone => {
            let yellowstone_config = config.yellowstone.as_ref()
                .ok_or_else(|| anyhow::anyhow!("Yellowstone configuration required"))?;
            Ok(Box::new(YellowstoneDatasourceBuilder::new(
                carbon_config,
                yellowstone_config.endpoint.clone(),
                yellowstone_config.token.clone(),
            )))
        }
    }
}