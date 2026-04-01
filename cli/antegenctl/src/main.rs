//! antegenctl — Antegen system controller: node version management and service control

use anyhow::Result;
use antegen_cli_core::{LogLevel, NodeConfigCommands, dispatch_config};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

// =============================================================================
// antegenctl CLI (operator-facing: node version management, service control, config)
// =============================================================================

#[derive(Parser)]
#[command(name = "antegenctl")]
#[command(about = "Antegen system controller — node version management and service control", version)]
struct AntegenctlCli {
    /// Set the logging level (overrides RUST_LOG environment variable)
    #[arg(long, global = true, value_name = "LEVEL")]
    log_level: Option<LogLevel>,

    /// RPC endpoint URL (defaults to Solana CLI config)
    #[arg(long, global = true)]
    rpc: Option<String>,

    /// Path to keypair file (defaults to Solana CLI config)
    #[arg(long, global = true)]
    keypair: Option<PathBuf>,

    #[command(subcommand)]
    command: AntegenctlCommands,
}

#[derive(Subcommand)]
enum AntegenctlCommands {
    /// Run the executor directly (no service, blocking)
    #[command(hide = true)]
    Run {
        /// Path to configuration file
        #[arg(short, long)]
        config: Option<PathBuf>,

        /// Run a specific version (e.g., v4.4.0)
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
    },

    /// Initialize config and keypair
    Init {
        /// RPC endpoint URL (prompts if not provided)
        #[arg(long)]
        rpc: Option<String>,

        /// Overwrite existing config
        #[arg(long)]
        force: bool,
    },

    /// Show info (CLI version, node version, executor, balance)
    Info {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Config file operations
    #[command(subcommand)]
    Config(NodeConfigCommands),

    /// Install and start the antegen service
    Start {
        /// RPC endpoint URL (prompts if not provided and interactive)
        #[arg(long)]
        rpc: Option<String>,

        /// Start a specific version (e.g., v4.4.0)
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
    },

    /// Stop the antegen service
    Stop,

    /// Restart the antegen service
    Restart,

    /// Show service status
    Status,

    /// View service logs
    Logs {
        /// Follow log output (like tail -f)
        #[arg(short, long)]
        follow: bool,
    },

    /// Uninstall the antegen service
    Uninstall,

    /// Fund the executor with SOL from your Solana CLI wallet
    Fund {
        /// Amount of SOL to transfer (defaults to minimum required)
        amount: Option<f64>,
    },

    /// Withdraw SOL from executor to Solana CLI keypair
    Withdraw {
        /// Amount of SOL to withdraw (defaults to everything above minimum)
        amount: Option<f64>,
    },

    /// Update node to latest version
    Update {
        /// Update to a specific version (e.g., v4.1.1)
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,

        /// Build and install from local workspace instead of downloading
        #[arg(long)]
        local: bool,
    },

    /// List installed and available node versions
    List,

    /// Switch node to a specific version (reinstalls service)
    Use {
        /// Version to switch to (e.g., v4.1.1)
        version: String,
    },

    /// Download a specific node version (doesn't switch)
    Install {
        /// Version to install (e.g., v4.1.1)
        #[arg(required_unless_present = "local")]
        version: Option<String>,

        /// Build and install from local workspace instead of downloading
        #[arg(long)]
        local: bool,
    },
}

// =============================================================================
// Main dispatch
// =============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    run_antegenctl().await
}

async fn run_antegenctl() -> Result<()> {
    let cli = AntegenctlCli::parse();

    match cli.command {
        AntegenctlCommands::Run { config, version } => {
            let cfg = match config {
                Some(p) => p,
                None => antegen_cli_core::commands::service::ensure_config()?,
            };
            antegen_cli_core::commands::run::execute(cfg, cli.rpc, cli.log_level, version).await
        }
        AntegenctlCommands::Init { rpc, force } => antegen_cli_core::commands::service::init(rpc, force),
        AntegenctlCommands::Start { rpc, version } => antegen_cli_core::commands::service::start(rpc, version).await,
        AntegenctlCommands::Stop => antegen_cli_core::commands::service::stop(),
        AntegenctlCommands::Restart => antegen_cli_core::commands::service::restart(),
        AntegenctlCommands::Status => antegen_cli_core::commands::service::status(),
        AntegenctlCommands::Logs { follow } => antegen_cli_core::commands::service::logs(follow),
        AntegenctlCommands::Uninstall => antegen_cli_core::commands::service::uninstall(),
        AntegenctlCommands::Info { json } => antegen_cli_core::commands::info::info(json).await,
        AntegenctlCommands::Fund { amount } => {
            let config = antegen_cli_core::commands::default_config_path()?;
            antegen_cli_core::commands::client::fund(config, amount, cli.keypair, cli.rpc).await
        }
        AntegenctlCommands::Withdraw { amount } => {
            let config = antegen_cli_core::commands::default_config_path()?;
            antegen_cli_core::commands::client::withdraw(config, amount, cli.rpc).await
        }
        AntegenctlCommands::Update { version, local } => antegen_cli_core::commands::update::update_node(version, local).await,
        AntegenctlCommands::List => antegen_cli_core::commands::update::list_node().await,
        AntegenctlCommands::Use { version } => antegen_cli_core::commands::update::use_node_version(version).await,
        AntegenctlCommands::Install { version, local } => antegen_cli_core::commands::update::install_node_version(version, local).await,
        AntegenctlCommands::Config(config_cmd) => dispatch_config(config_cmd, cli.rpc),
    }
}
