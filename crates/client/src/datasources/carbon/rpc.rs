use anyhow::Result;
use async_trait::async_trait;
use carbon_core::pipeline::Pipeline;
use carbon_log_metrics::LogMetrics;
use carbon_rpc_program_subscribe_datasource::{Filters, RpcProgramSubscribe};
use tokio::sync::mpsc;
use futures::StreamExt;
use log::info;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::sysvar;
use std::sync::Arc;

use crate::builder::DatasourceBuilder;
use crate::utils::wait_for_validator;
use antegen_processor::types::AccountUpdate;

use super::{
    processor::{BasicAccountDecoder, ThreadAccountProcessor},
    CarbonConfig,
};

/// Carbon RPC datasource for monitoring thread accounts
pub struct CarbonRpcDatasource {
    config: CarbonConfig,
}

impl CarbonRpcDatasource {
    /// Create a new Carbon RPC datasource
    pub fn new(config: CarbonConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl DatasourceBuilder for CarbonRpcDatasource {
    async fn run(&self, sender: mpsc::Sender<AccountUpdate>) -> Result<()> {
        info!("Starting Carbon RPC datasource");
        info!("Thread program ID: {}", self.config.thread_program_id);
        info!("Clock sysvar ID: {}", sysvar::clock::ID);
        info!("RPC URL: {}", self.config.rpc_url);

        // Convert RPC URL to WebSocket URL if needed
        let ws_url = if self.config.rpc_url.starts_with("ws://")
            || self.config.rpc_url.starts_with("wss://")
        {
            self.config.rpc_url.clone()
        } else {
            self.config
                .rpc_url
                .replace("http://", "ws://")
                .replace("https://", "wss://")
                .replace(":8899", ":8900")
        };

        // Wait for validator to be ready before initializing connections
        wait_for_validator(&self.config.rpc_url, &ws_url).await?;

        // Clone sender for both pipelines
        let sender_thread = sender.clone();
        let sender_clock = sender;

        // Create processor for thread accounts
        let processor_thread =
            ThreadAccountProcessor::new(sender_thread, self.config.thread_program_id);

        // Create metrics
        let metrics = Arc::new(LogMetrics::new());

        // Task 1: Subscribe to thread program accounts
        let thread_program_id = self.config.thread_program_id;
        let ws_url_thread = ws_url.clone();
        let metrics_thread = metrics.clone();
        let thread_task = tokio::spawn(async move {
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
        let rpc_url = self
            .config
            .rpc_url
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

/// Track Clock sysvar updates
async fn track_clock(
    ws_url: String,
    sender: mpsc::Sender<AccountUpdate>,
    rpc_client: Arc<RpcClient>,
) -> Result<()> {
    use solana_client::nonblocking::pubsub_client::PubsubClient;
    use solana_client::rpc_config::RpcAccountInfoConfig;
    use solana_sdk::commitment_config::CommitmentConfig;

    // First, fetch the current Clock account
    let clock_account = rpc_client.get_account(&sysvar::clock::ID).await?;

    let update = AccountUpdate {
        pubkey: sysvar::clock::ID,
        account: clock_account,
    };

    if let Err(e) = sender.send(update).await {
        log::error!("Failed to send initial Clock update: {}", e);
        return Ok(());
    }

    // Subscribe to Clock updates
    let pubsub_client = PubsubClient::new(&ws_url).await?;

    let (mut stream, _unsub) = pubsub_client
        .account_subscribe(
            &sysvar::clock::ID,
            Some(RpcAccountInfoConfig {
                encoding: Some(solana_account_decoder::UiAccountEncoding::Base64),
                commitment: Some(CommitmentConfig::confirmed()),
                ..Default::default()
            }),
        )
        .await?;

    info!("Subscribed to Clock sysvar updates");

    while let Some(response) = stream.next().await {
        let account: solana_sdk::account::Account =
            response
                .value
                .decode()
                .unwrap_or_else(|| solana_sdk::account::Account {
                    lamports: 0,
                    data: vec![],
                    owner: solana_program::pubkey::Pubkey::default(),
                    executable: false,
                    rent_epoch: 0,
                });

        let update = AccountUpdate {
            pubkey: sysvar::clock::ID,
            account,
        };

        if let Err(e) = sender.send(update).await {
            log::error!("Failed to send Clock update: {}", e);
            break;
        }
    }

    Ok(())
}
