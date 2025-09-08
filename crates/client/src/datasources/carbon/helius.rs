use anyhow::Result;
use async_trait::async_trait;
use carbon_core::pipeline::Pipeline;
use carbon_log_metrics::LogMetrics;
use carbon_rpc_program_subscribe_datasource::{Filters, RpcProgramSubscribe};
use futures::StreamExt;
use tokio::sync::mpsc;
use log::info;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::sysvar;
use std::sync::Arc;

use crate::builder::DatasourceBuilder;
use antegen_processor::types::AccountUpdate;

use super::{
    processor::{BasicAccountDecoder, ThreadAccountProcessor},
    CarbonConfig,
};

/// Carbon Helius datasource configuration
pub struct CarbonHeliusConfig {
    /// Base Carbon configuration
    pub carbon_config: CarbonConfig,
    /// Helius WebSocket URL
    pub ws_url: String,
}

/// Carbon Helius datasource for monitoring thread accounts
pub struct CarbonHeliusDatasource {
    config: CarbonHeliusConfig,
}

impl CarbonHeliusDatasource {
    /// Create a new Carbon Helius datasource
    pub fn new(carbon_config: CarbonConfig, ws_url: String) -> Self {
        Self {
            config: CarbonHeliusConfig {
                carbon_config,
                ws_url,
            },
        }
    }
}

#[async_trait]
impl DatasourceBuilder for CarbonHeliusDatasource {
    async fn run(&self, sender: mpsc::Sender<AccountUpdate>) -> Result<()> {
        info!("Starting Carbon Helius datasource (using RPC fallback)");
        info!(
            "Thread program ID: {}",
            self.config.carbon_config.thread_program_id
        );
        info!("Clock sysvar ID: {}", sysvar::clock::ID);
        info!("WebSocket URL: {}", self.config.ws_url);

        // Clone sender for both pipelines
        let sender_thread = sender.clone();
        let sender_clock = sender;

        // Create processor for thread accounts
        let processor_thread =
            ThreadAccountProcessor::new(sender_thread, self.config.carbon_config.thread_program_id);

        // Create metrics
        let metrics = Arc::new(LogMetrics::new());

        // Task 1: Subscribe to thread program accounts
        let thread_program_id = self.config.carbon_config.thread_program_id;
        let ws_url_thread = self.config.ws_url.clone();
        let metrics_thread = metrics.clone();
        let thread_task = tokio::spawn(async move {
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
        let ws_url = self.config.ws_url.clone();
        // Use HTTP URL for RPC client (from carbon config)
        let rpc_url = self.config.carbon_config.rpc_url.clone();
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

/// Track Clock sysvar updates (same as RPC version)
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
