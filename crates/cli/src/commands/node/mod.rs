//! `antegen node` — everything an operator does with the executor.
//!
//! The command group is large enough to be its own module: the daemon itself
//! (`run`), the service that supervises it, and the versioned binaries it runs
//! from. Every other command group is a single file named after it; this one
//! was three files named after implementation concerns, so finding the code
//! behind `antegen node update` meant grepping.

use crate::LogLevel;
use anyhow::Result;
use clap::Subcommand;
use std::path::PathBuf;

pub(crate) mod run;
pub(crate) mod service;
pub(crate) mod version;

#[derive(Subcommand)]
pub(crate) enum NodeCommands {
    /// Run the executor in the foreground (no service, blocking)
    Run {
        /// Path to configuration file
        #[arg(short, long)]
        config: Option<PathBuf>,
    },

    /// Install and start the antegen service
    Start {
        /// RPC endpoint URL (prompts if not provided and interactive)
        #[arg(long)]
        rpc: Option<String>,

        /// Start a specific version (e.g., v6.0.0)
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

    /// Update the node to the latest version
    Update {
        /// Update to a specific version (e.g., v6.0.0)
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,

        /// Build and install from the local workspace instead of downloading
        #[arg(long)]
        local: bool,
    },

    /// List installed and available node versions
    List,

    /// Switch the node to a specific version (reinstalls the service)
    Use {
        /// Version to switch to (e.g., v6.0.0)
        version: String,
    },

    /// Download a specific node version (doesn't switch)
    Install {
        /// Version to install (e.g., v6.0.0)
        #[arg(required_unless_present = "local")]
        version: Option<String>,

        /// Build and install from the local workspace instead of downloading
        #[arg(long)]
        local: bool,
    },
}

/// Route an `antegen node` subcommand to its implementation.
pub(crate) async fn dispatch(
    cmd: NodeCommands,
    rpc: Option<String>,
    log_level: Option<LogLevel>,
) -> Result<()> {
    match cmd {
        NodeCommands::Run { config } => {
            let cfg = match config {
                Some(p) => p,
                None => service::ensure_config()?,
            };
            run::run(cfg, rpc, log_level).await
        }
        NodeCommands::Start { rpc, version } => service::start(rpc, version).await,
        NodeCommands::Stop => service::stop(),
        NodeCommands::Restart => service::restart(),
        NodeCommands::Status => service::status(),
        NodeCommands::Logs { follow } => service::logs(follow),
        NodeCommands::Uninstall => service::uninstall(),
        NodeCommands::Update { version, local } => version::update_node(version, local).await,
        NodeCommands::List => version::list_node().await,
        NodeCommands::Use { version } => version::use_node_version(version).await,
        NodeCommands::Install { version, local } => {
            version::install_node_version(version, local).await
        }
    }
}
