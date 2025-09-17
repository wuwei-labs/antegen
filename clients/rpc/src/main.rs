use anyhow::Result;
use clap::Parser;
use log::info;
use std::path::PathBuf;
use std::sync::Arc;
use solana_sdk::signature::{read_keypair_file, Signer};

use antegen_client::{AntegenClient, RpcDatasource, RpcConfig};
use antegen_processor::builder::ProcessorBuilder;
use antegen_submitter::builder::SubmitterBuilder;

mod config;
use config::Config;

#[derive(Parser)]
#[command(name = "antegen-rpc")]
#[command(about = "Standalone Antegen worker using direct RPC pubsub")]
#[command(version)]
pub struct Args {
    /// Config file path
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// RPC URL for blockchain connection
    #[arg(long)]
    rpc_url: Option<String>,

    /// Path to executor keypair
    #[arg(long)]
    keypair: Option<PathBuf>,

    /// Thread program ID to monitor
    #[arg(long)]
    thread_program_id: Option<String>,

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

    info!("Starting Antegen RPC worker");
    info!("Thread program ID: {}", config.thread_program_id);

    // Run the simplified RPC client
    run_rpc_client(config).await
}

fn init_logging(debug: bool) {
    use env_logger::{Builder, Target};
    
    let level = if debug {
        "debug,antegen_client=debug,antegen_processor=debug,antegen_submitter=debug"
    } else {
        "info,antegen_client=info,antegen_processor=info,antegen_submitter=info"
    };

    let mut builder = Builder::from_env(env_logger::Env::default().default_filter_or(level));
    builder.target(Target::Stdout);
    builder.init();
}

/// Run the RPC client using pre-built datasource
async fn run_rpc_client(config: Config) -> Result<()> {
    info!("Initializing RPC client with pre-built datasource");

    // Validate keypair
    let keypair = Arc::new(
        read_keypair_file(&config.keypair_path)
            .map_err(|e| anyhow::anyhow!("Failed to read keypair: {}", e))?,
    );
    let executor_pubkey = keypair.pubkey();
    info!("Executor pubkey: {}", executor_pubkey);

    // Create RPC datasource configuration
    let rpc_config = RpcConfig::new(config.rpc_url.clone(), config.thread_program_id)
        .with_commitment(solana_sdk::commitment_config::CommitmentConfig::confirmed());

    // Build the client using the pre-built RPC datasource
    let client = AntegenClient::builder()
        .rpc_url(config.rpc_url.clone())
        .datasource(Box::new(RpcDatasource::new(rpc_config)))
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
                // TODO: Add replay config when implemented
                // .replay_config(build_replay_config(&config))
                .tpu_enabled(),
        )
        .build()
        .await?;

    info!("Starting RPC client");

    // Run the client
    match client.run().await {
        Ok(()) => info!("RPC client completed normally"),
        Err(e) => log::error!("RPC client error: {}", e),
    }

    info!("RPC client shutting down");
    Ok(())
}

// TODO: Implement replay configuration when message queue is added
// fn build_replay_config(config: &Config) -> ReplayConfig {
//     let mut replay_config = ReplayConfig::default();
//
//     if config.replay.enabled {
//         replay_config.enable_replay = true;
//         replay_config.queue_url = Some(config.replay.queue_url.clone());
//     }
//
//     replay_config
// }
