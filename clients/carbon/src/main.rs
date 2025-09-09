use anyhow::Result;
use clap::Parser;
use log::info;
use std::path::PathBuf;
use std::sync::Arc;
use solana_sdk::signature::{read_keypair_file, Signer};

use antegen_client::{
    AntegenClient, 
    CarbonConfig,
    CarbonRpcDatasource,
    CarbonHeliusDatasource,
    CarbonYellowstoneDatasource,
};
use antegen_processor::builder::ProcessorBuilder;
use antegen_submitter::builder::SubmitterBuilder;

mod config;
use config::{Config, DatasourceType};

#[derive(Parser)]
#[command(name = "antegen-carbon")]
#[command(about = "Standalone Antegen worker using Carbon framework")]
#[command(version)]
struct Args {
    /// Config file path
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Datasource type: rpc, helius, or yellowstone
    #[arg(long)]
    datasource: Option<String>,

    /// RPC URL for blockchain connection
    #[arg(long)]
    rpc_url: Option<String>,

    /// Path to executor keypair
    #[arg(long)]
    keypair: Option<PathBuf>,

    /// Thread program ID to monitor
    #[arg(long)]
    thread_program_id: Option<String>,

    /// Helius WebSocket URL
    #[arg(long)]
    helius_ws_url: Option<String>,

    /// Helius API key
    #[arg(long)]
    helius_api_key: Option<String>,

    /// Yellowstone gRPC endpoint
    #[arg(long)]
    yellowstone_endpoint: Option<String>,

    /// Yellowstone authentication token
    #[arg(long)]
    yellowstone_token: Option<String>,

    /// Enable verbose/debug logging
    #[arg(long, short = 'v', alias = "verbose")]
    debug: bool,

    /// Forgo executor commission
    #[arg(long)]
    forgo_commission: bool,

    /// Enable transaction replay via NATS
    #[arg(long)]
    enable_replay: bool,

    /// NATS server URL for replay
    #[arg(long)]
    nats_url: Option<String>,

    /// Replay delay in milliseconds
    #[arg(long, default_value = "30000")]
    replay_delay_ms: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Load config from file if provided, otherwise from args/env
    let config = if let Some(config_path) = args.config {
        Config::from_file(config_path)?
    } else {
        Config::from_args(args)?
    };

    // Initialize logging
    init_logging(config.debug);

    info!("Starting Antegen Carbon worker");
    info!("Datasource: {:?}", config.datasource);
    info!("Thread program ID: {}", config.thread_program_id);

    // Run the Carbon client with pre-built datasources
    run_carbon_client(config).await
}

fn init_logging(debug: bool) {
    use env_logger::{Builder, Target};
    
    let level = if debug {
        "debug,antegen_client=debug,antegen_processor=debug,antegen_submitter=debug,carbon_core=debug"
    } else {
        "info,antegen_client=info,antegen_processor=info,antegen_submitter=info,carbon_core=info"
    };

    let mut builder = Builder::from_env(env_logger::Env::default().default_filter_or(level));
    builder.target(Target::Stdout);
    builder.init();
}

/// Run the Carbon client using pre-built datasources
async fn run_carbon_client(config: Config) -> Result<()> {
    info!("Initializing Carbon client with pre-built datasources");

    // Validate keypair
    let keypair = Arc::new(
        read_keypair_file(&config.keypair_path)
            .map_err(|e| anyhow::anyhow!("Failed to read keypair: {}", e))?,
    );
    let executor_pubkey = keypair.pubkey();
    info!("Executor pubkey: {}", executor_pubkey);

    // Create Carbon configuration
    let carbon_config = CarbonConfig::new(
        config.thread_program_id,
        config.rpc_url.clone(),
    );

    // Create appropriate datasource based on configuration
    let datasource: Box<dyn antegen_client::DatasourceBuilder> = match config.datasource {
        DatasourceType::Rpc => {
            info!("Using Carbon RPC datasource");
            Box::new(CarbonRpcDatasource::new(carbon_config))
        }
        DatasourceType::Helius => {
            let helius_config = config.helius.as_ref()
                .ok_or_else(|| anyhow::anyhow!("Helius configuration required"))?;
            info!("Using Carbon Helius datasource");
            Box::new(CarbonHeliusDatasource::new(
                carbon_config,
                helius_config.ws_url.clone(),
            ))
        }
        DatasourceType::Yellowstone => {
            let yellowstone_config = config.yellowstone.as_ref()
                .ok_or_else(|| anyhow::anyhow!("Yellowstone configuration required"))?;
            info!("Using Carbon Yellowstone datasource");
            Box::new(CarbonYellowstoneDatasource::new(
                carbon_config,
                yellowstone_config.endpoint.clone(),
                yellowstone_config.token.clone(),
            ))
        }
    };

    // Build the client using the selected datasource
    let client = AntegenClient::builder()
        .rpc_url(config.rpc_url.clone())
        .datasource(datasource)
        .processor(
            ProcessorBuilder::new()
                .keypair(config.keypair_path.to_string_lossy())
                .rpc_url(config.rpc_url.clone())
                .forgo_commission(config.forgo_commission),
        )
        .submitter(
            SubmitterBuilder::new()
                .rpc_url(config.rpc_url.clone())
                .executor_keypair(keypair.clone())
                .replay_config(build_replay_config(&config))
                .tpu_enabled(),
        )
        .build()
        .await?;

    info!("Starting Carbon client");

    // Run the client
    match client.run().await {
        Ok(()) => info!("Carbon client completed normally"),
        Err(e) => log::error!("Carbon client error: {}", e),
    }

    info!("Carbon client shutting down");
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
