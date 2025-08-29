use anyhow::Result;
use clap::Parser;
use log::info;
use std::path::PathBuf;

mod builder;
mod config;
mod pipeline;
mod processor;

use config::Config;
use pipeline::run_carbon_pipeline;

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

    /// Enable debug logging
    #[arg(long)]
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

    // Run the Carbon pipeline
    run_carbon_pipeline(config).await
}

fn init_logging(debug: bool) {
    let level = if debug {
        "debug,antegen_adapter=debug,antegen_processor=debug,antegen_submitter=debug"
    } else {
        "info,antegen_adapter=info,antegen_processor=info,antegen_submitter=info"
    };

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(level)).init();
}
