use anyhow::Result;
use async_trait::async_trait;
use crossbeam::channel::Sender;
use futures::StreamExt;
use log::{error, info};
use solana_client::nonblocking::pubsub_client::PubsubClient;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig};
use solana_client::rpc_filter::RpcFilterType;
use solana_program::pubkey::Pubkey;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::sysvar;
use std::str::FromStr;
use std::sync::Arc;

use crate::builder::DatasourceBuilder;
use crate::utils::wait_for_validator;
use antegen_processor::types::AccountUpdate;

/// Configuration for RPC datasource
#[derive(Clone, Debug)]
pub struct RpcConfig {
    /// RPC URL for blockchain connection
    pub rpc_url: String,
    /// WebSocket URL (will be derived from RPC URL if not provided)
    pub ws_url: Option<String>,
    /// Program ID to monitor for account updates
    pub program_id: Pubkey,
    /// Whether to monitor Clock sysvar updates
    pub monitor_clock: bool,
    /// Commitment level for subscriptions
    pub commitment: CommitmentConfig,
    /// Optional account filters
    pub filters: Vec<RpcFilterType>,
}

impl RpcConfig {
    /// Create a new RPC configuration with defaults
    pub fn new(rpc_url: String, program_id: Pubkey) -> Self {
        Self {
            rpc_url,
            ws_url: None,
            program_id,
            monitor_clock: true,
            commitment: CommitmentConfig::confirmed(),
            filters: vec![],
        }
    }

    /// Set a custom WebSocket URL
    pub fn with_ws_url(mut self, ws_url: String) -> Self {
        self.ws_url = Some(ws_url);
        self
    }

    /// Disable Clock sysvar monitoring
    pub fn without_clock(mut self) -> Self {
        self.monitor_clock = false;
        self
    }

    /// Set commitment level
    pub fn with_commitment(mut self, commitment: CommitmentConfig) -> Self {
        self.commitment = commitment;
        self
    }

    /// Add account filters
    pub fn with_filters(mut self, filters: Vec<RpcFilterType>) -> Self {
        self.filters = filters;
        self
    }

    /// Get the WebSocket URL, deriving from RPC URL if needed
    fn get_ws_url(&self) -> String {
        self.ws_url.clone().unwrap_or_else(|| {
            self.rpc_url
                .replace("http://", "ws://")
                .replace("https://", "wss://")
                .replace(":8899", ":8900")
        })
    }
}

/// Pre-built RPC datasource for monitoring Solana accounts
pub struct RpcDatasource {
    config: RpcConfig,
}

impl RpcDatasource {
    /// Create a new RPC datasource
    pub fn new(config: RpcConfig) -> Self {
        Self { config }
    }

    /// Create with simple configuration
    pub fn simple(rpc_url: String, program_id: Pubkey) -> Self {
        Self::new(RpcConfig::new(rpc_url, program_id))
    }
}

#[async_trait]
impl DatasourceBuilder for RpcDatasource {
    async fn run(&self, sender: Sender<AccountUpdate>) -> Result<()> {
        info!("Starting RPC datasource");
        info!("Program ID: {}", self.config.program_id);
        info!("RPC URL: {}", self.config.rpc_url);
        info!("Monitor Clock: {}", self.config.monitor_clock);

        let ws_url = self.config.get_ws_url();
        info!("WebSocket URL: {}", ws_url);

        // Wait for validator to be ready
        wait_for_validator(&self.config.rpc_url, &ws_url).await?;

        // Clone sender for both tasks
        let sender_program = sender.clone();
        let sender_clock = sender;

        // Task 1: Subscribe to program accounts
        let program_id = self.config.program_id;
        let ws_url_program = ws_url.clone();
        let commitment = self.config.commitment;
        let filters = self.config.filters.clone();

        let program_task = tokio::spawn(async move {
            info!("Starting program account subscription");

            loop {
                match subscribe_to_program_accounts(
                    ws_url_program.clone(),
                    program_id,
                    sender_program.clone(),
                    commitment,
                    filters.clone(),
                )
                .await
                {
                    Ok(_) => {
                        info!("Program account subscription ended");
                        break;
                    }
                    Err(e) => {
                        error!("Program account subscription error: {}. Reconnecting...", e);
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    }
                }
            }
        });

        // Task 2: Optionally track Clock sysvar updates
        let clock_task = if self.config.monitor_clock {
            let rpc_client = Arc::new(RpcClient::new(self.config.rpc_url.clone()));
            Some(tokio::spawn(async move {
                info!("Starting Clock subscription");
                track_clock(ws_url, sender_clock, rpc_client).await
            }))
        } else {
            None
        };

        // Wait for tasks to complete
        if let Some(clock_task) = clock_task {
            let (program_result, clock_result) = tokio::join!(program_task, clock_task);
            program_result?;
            clock_result??;
        } else {
            program_task.await?;
        }

        Ok(())
    }
}

/// Subscribe to program accounts via WebSocket
async fn subscribe_to_program_accounts(
    ws_url: String,
    program_id: Pubkey,
    sender: Sender<AccountUpdate>,
    commitment: CommitmentConfig,
    filters: Vec<RpcFilterType>,
) -> Result<()> {
    let pubsub_client = PubsubClient::new(&ws_url).await?;

    let config = RpcProgramAccountsConfig {
        filters: Some(filters),
        account_config: RpcAccountInfoConfig {
            encoding: Some(solana_account_decoder::UiAccountEncoding::Base64),
            commitment: Some(commitment),
            ..Default::default()
        },
        with_context: Some(false),
        sort_results: Some(false),
    };

    let (mut stream, _unsub) = pubsub_client
        .program_subscribe(&program_id, Some(config))
        .await?;

    info!("Subscribed to program accounts for {}", program_id);

    while let Some(response) = stream.next().await {
        let pubkey = Pubkey::from_str(&response.value.pubkey)?;
        let account: solana_sdk::account::Account =
            response
                .value
                .account
                .decode()
                .unwrap_or_else(|| solana_sdk::account::Account {
                    lamports: 0,
                    data: vec![],
                    owner: Pubkey::default(),
                    executable: false,
                    rent_epoch: 0,
                });

        let update = AccountUpdate { pubkey, account };

        if let Err(e) = sender.send(update) {
            error!("Failed to send account update: {}", e);
            break;
        }
    }

    Ok(())
}

/// Track Clock sysvar updates
async fn track_clock(
    ws_url: String,
    sender: Sender<AccountUpdate>,
    rpc_client: Arc<RpcClient>,
) -> Result<()> {
    // First, fetch the current Clock account
    let clock_account = rpc_client.get_account(&sysvar::clock::ID).await?;

    let update = AccountUpdate {
        pubkey: sysvar::clock::ID,
        account: clock_account,
    };

    if let Err(e) = sender.send(update) {
        error!("Failed to send initial Clock update: {}", e);
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
                    owner: Pubkey::default(),
                    executable: false,
                    rent_epoch: 0,
                });

        let update = AccountUpdate {
            pubkey: sysvar::clock::ID,
            account,
        };

        if let Err(e) = sender.send(update) {
            error!("Failed to send Clock update: {}", e);
            break;
        }
    }

    Ok(())
}
