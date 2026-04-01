//! Antegen CLI Core — shared library for antegen and antegenctl binaries

use clap::{Subcommand, ValueEnum};
use std::path::PathBuf;

pub mod commands;
pub mod download;

#[derive(Clone, Debug, ValueEnum)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Off,
}

impl LogLevel {
    pub fn to_level_filter(&self) -> log::LevelFilter {
        match self {
            LogLevel::Trace => log::LevelFilter::Trace,
            LogLevel::Debug => log::LevelFilter::Debug,
            LogLevel::Info => log::LevelFilter::Info,
            LogLevel::Warn => log::LevelFilter::Warn,
            LogLevel::Error => log::LevelFilter::Error,
            LogLevel::Off => log::LevelFilter::Off,
        }
    }
}

// =============================================================================
// Node config commands (shared between antegenctl and backward-compat aliases)
// =============================================================================

#[derive(Subcommand)]
pub enum NodeConfigCommands {
    /// Display the current executor node configuration
    Get {
        /// Path to config file (defaults to ~/.config/antegen/antegen.toml)
        #[arg(short, long)]
        config: Option<PathBuf>,
    },

    /// Update configuration values
    #[command(after_long_help = "\
EXAMPLES:
    antegenctl config set --max-threads 20
    antegenctl config set --commitment finalized --tpu-enabled false
    antegenctl config set --keypair-path ~/.antegen/my-keypair.json
    antegenctl config set --grace-period 15 --eviction-buffer 30
")]
    Set {
        /// Path to config file (defaults to ~/.config/antegen/antegen.toml)
        #[arg(short, long)]
        config: Option<PathBuf>,

        // -- executor --
        /// Path to executor keypair file
        #[arg(long)]
        keypair_path: Option<String>,

        /// Skip taking executor commission fee
        #[arg(long)]
        forgo_commission: Option<bool>,

        // -- datasources --
        /// Commitment level (processed, confirmed, finalized)
        #[arg(long)]
        commitment: Option<String>,

        // -- processor --
        /// Maximum number of concurrent threads to process
        #[arg(long)]
        max_threads: Option<usize>,

        // -- cache --
        /// Maximum number of accounts to cache
        #[arg(long)]
        cache_max_capacity: Option<u64>,

        // -- load_balancer --
        /// Grace period in seconds for fee decay calculations
        #[arg(long)]
        grace_period: Option<u64>,

        /// Eviction buffer in seconds (extra cache TTL after grace period)
        #[arg(long)]
        eviction_buffer: Option<u64>,

        /// Delay in seconds before claiming new threads
        #[arg(long)]
        thread_process_delay: Option<u64>,

        // -- observability --
        /// Enable/disable observability agent
        #[arg(long)]
        observability_enabled: Option<bool>,

        /// Storage path for observability data
        #[arg(long)]
        observability_storage_path: Option<String>,

        // -- tpu --
        /// Enable/disable TPU client for transaction submission
        #[arg(long)]
        tpu_enabled: Option<bool>,

        /// Number of QUIC connections per leader
        #[arg(long)]
        tpu_num_connections: Option<usize>,

        /// Number of leaders to fan out transactions to
        #[arg(long)]
        tpu_leaders_fanout: Option<usize>,
    },

    /// Generate default config file
    Init {
        /// Output path for config file (defaults to ~/.config/antegen/antegen.toml)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// RPC endpoint URL
        #[arg(long)]
        rpc: Option<String>,

        /// Path to executor keypair file
        #[arg(long)]
        keypair_path: Option<String>,

        /// Path to observability storage
        #[arg(long)]
        storage_path: Option<String>,

        /// Overwrite existing config file
        #[arg(long)]
        force: bool,
    },

    /// Validate config file
    Validate {
        /// Path to config file
        #[arg(short, long, default_value = "antegen.toml")]
        config: PathBuf,
    },
}

/// Dispatch a NodeConfigCommands variant to the appropriate handler
pub fn dispatch_config(config_cmd: NodeConfigCommands, global_rpc: Option<String>) -> anyhow::Result<()> {
    match config_cmd {
        NodeConfigCommands::Get { config } => {
            let path = config
                .map(Ok)
                .unwrap_or_else(commands::default_config_path)?;
            commands::config::get(path)
        }
        NodeConfigCommands::Set {
            config,
            keypair_path,
            forgo_commission,
            commitment,
            max_threads,
            cache_max_capacity,
            grace_period,
            eviction_buffer,
            thread_process_delay,
            observability_enabled,
            observability_storage_path,
            tpu_enabled,
            tpu_num_connections,
            tpu_leaders_fanout,
        } => {
            let path = config
                .map(Ok)
                .unwrap_or_else(commands::default_config_path)?;
            commands::config::set(
                path,
                global_rpc,
                keypair_path,
                forgo_commission,
                commitment,
                max_threads,
                cache_max_capacity,
                grace_period,
                eviction_buffer,
                thread_process_delay,
                observability_enabled,
                observability_storage_path,
                tpu_enabled,
                tpu_num_connections,
                tpu_leaders_fanout,
            )
        }
        NodeConfigCommands::Init {
            output,
            rpc,
            keypair_path,
            storage_path,
            force,
        } => {
            let path = output
                .map(Ok)
                .unwrap_or_else(commands::default_config_path)?;
            commands::config::init(path, rpc, keypair_path, storage_path, force)
        }
        NodeConfigCommands::Validate { config } => commands::config::validate(config),
    }
}
