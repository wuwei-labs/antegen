//! The `antegen` binary: developer tooling, node operations, and the executor
//! daemon itself.
//!
//! One binary does all three. `antegen node run` is the process the service
//! supervises — the daemon is not a separate program — so the CLI you type and
//! the node you operate are the same artifact and the same version.
#![warn(unreachable_pub)]

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use commands::config::{dispatch_config, NodeConfigCommands};
use commands::node::NodeCommands;
use std::path::PathBuf;

mod commands;
mod download;

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
// Node config commands
// =============================================================================

/// `<cli> (client <client>)` — the daemon ships inside this binary, so the
/// runtime version is no longer visible from the package version alone.
///
/// This is also why `antegen-cli` has to be released whenever the libraries it
/// links move: the version an operator sees, and the code they actually run,
/// both change even when nothing under `cli/antegen` does. release-please
/// attributes by path and cannot see that, so a client-only fix has to be
/// released with a commit scoped to this crate or it never reaches a binary.
/// This binary's version, `v`-prefixed to match release tags.
pub(crate) fn current_version() -> &'static str {
    concat!("v", env!("CARGO_PKG_VERSION"))
}

/// The target triple this binary was built for, used to pick the right release
/// asset — for the node binary and for the geyser plugin alike.
///
/// `self_update::get_target()` reports the actual build target. The alternative,
/// matching on `target_os`/`target_arch` by hand, was maintained separately for
/// the plugin download and could disagree with this one about the same host.
pub(crate) fn get_platform_target() -> &'static str {
    self_update::get_target()
}

fn version_string() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION.get_or_init(|| {
        format!(
            "{} (client {})",
            env!("CARGO_PKG_VERSION"),
            antegen_client::VERSION
        )
    })
}

#[derive(Parser)]
#[command(name = "antegen")]
#[command(about = "Antegen automation client", version = version_string())]
#[command(long_about = "
Antegen automation client for Solana thread execution.

Supports two deployment modes:
  1. Standalone: Run as a separate process using RPC subscriptions
  2. Plugin: Run as a Geyser plugin inside the validator

For node service control and version management, see `antegen node`.
For more information, visit: https://antegen.xyz
")]
struct Cli {
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
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Thread program management
    #[command(subcommand)]
    Program(ProgramCommands),

    /// Thread inspection operations
    #[command(subcommand)]
    Thread(ThreadCommands),

    /// Geyser plugin operations (downloads plugin from GitHub releases)
    #[command(subcommand)]
    Geyser(GeyserCommands),

    /// Executor node — service control and version management
    #[command(subcommand)]
    Node(NodeCommands),

    /// Config file operations
    #[command(subcommand)]
    Config(NodeConfigCommands),

    /// Initialize antegen — creates config
    Init {
        /// RPC endpoint URL (prompts if not provided)
        #[arg(long)]
        rpc: Option<String>,

        /// Overwrite existing config
        #[arg(long)]
        force: bool,
    },

    /// Show info (versions, executor, balance)
    Info {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

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

    // =========================================================================
    // Hidden deprecated aliases — these moved under `antegen node`.
    // Kept for one release so existing runbooks keep working.
    // =========================================================================
    /// Install and start the antegen service (init if needed)
    #[command(hide = true)]
    Start {
        #[arg(long)]
        rpc: Option<String>,
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
    },

    /// Show service status
    #[command(hide = true)]
    Status,

    /// View service logs
    #[command(hide = true)]
    Logs {
        #[arg(short, long)]
        follow: bool,
    },

    /// Stop the antegen service
    #[command(hide = true)]
    Stop,

    /// Restart the antegen service
    #[command(hide = true)]
    Restart,

    /// Uninstall the antegen service
    #[command(hide = true)]
    Uninstall,
}

// =============================================================================
// Node commands
// =============================================================================

// =============================================================================
// Program commands
// =============================================================================

#[derive(Subcommand)]
enum ProgramCommands {
    /// Deploy the program binary to a Solana cluster
    Deploy {
        /// Path to a single .so file (omit to deploy both fiber + thread)
        program_binary: Option<PathBuf>,

        /// Program ID or keypair path (single-program mode only)
        #[arg(long)]
        program_id: Option<String>,

        /// Directory containing program keypair files named {program_id}.json
        #[arg(long)]
        keys_dir: Option<PathBuf>,

        /// Skip `config init` after deploy
        #[arg(long)]
        skip_init: bool,

        /// Skip on-chain verification after deploy
        #[arg(long)]
        skip_verify: bool,
    },

    /// Program configuration management
    #[command(subcommand)]
    Config(ProgramConfigCommands),
}

#[derive(Subcommand)]
enum ProgramConfigCommands {
    /// Initialize the ThreadConfig account (required before threads can execute)
    Init,

    /// Display the current ThreadConfig settings
    Get,
}

// =============================================================================
// Geyser commands
// =============================================================================

#[derive(Subcommand)]
enum GeyserCommands {
    /// Initialize plugin for validator
    Init {
        /// Output path for validator plugin config
        #[arg(short, long, default_value = "validator-plugin-config.json")]
        output: PathBuf,

        /// Path to antegen.toml config file
        #[arg(short, long, default_value = "antegen.toml")]
        config: PathBuf,
    },

    /// Extract plugin .so to custom location
    Extract {
        /// Output path for the .so file
        #[arg(short, long)]
        output: PathBuf,
    },
}

// =============================================================================
// Thread commands
// =============================================================================

#[derive(Subcommand)]
enum ThreadCommands {
    /// Fetch and display a thread account
    Get {
        /// Thread public key
        address: String,
    },

    /// Admin: force delete a thread (skips all checks)
    #[cfg(feature = "dev")]
    Delete {
        /// Thread public key to delete
        address: String,
    },

    /// Test thread operations (create, list, delete)
    #[cfg(feature = "dev")]
    #[command(subcommand)]
    Test(TestCommands),
}

#[cfg(feature = "dev")]
#[derive(Subcommand)]
pub enum TestCommands {
    /// Create a test thread (auto-generates ID like test-1, test-2, ...)
    #[command(after_long_help = "\
EXAMPLES:
    antegen thread test create
    antegen thread test create --trigger immediate
    antegen thread test create --trigger \"interval:30\"
    antegen thread test create --trigger \"interval:30\" --signal repeat

    # Multi-fiber with signals (fiber count inferred from signals)
    antegen thread test create --trigger \"interval:30\" --signal 0:chain:1 --signal 1:chain:2

    # Explicit fiber count override
    antegen thread test create --trigger \"interval:30\" --fibers 5 --signal 0:chain:1

    # Advanced test types (use fixed IDs)
    antegen thread test create --test-type account   # Creates paired threads
    antegen thread test create --test-type chain     # Creates 3-fiber chain test
")]
    Create {
        /// Trigger type: immediate, cron:<schedule>, interval:<secs>, timestamp:<unix>, slot:<num>, epoch:<num>, account:<pubkey>
        #[arg(long, default_value = "immediate")]
        trigger: String,

        /// Signal configuration (can be repeated). Simple: repeat, close.
        /// Per-fiber: F:chain:T or F:next:T (e.g., 0:chain:1, 1:next:0)
        #[arg(long)]
        signal: Vec<String>,

        /// Override fiber count. If omitted, inferred from signals (or 1 if no signals).
        #[arg(long)]
        fibers: Option<u8>,

        /// Advanced test type: account (paired threads), chain (3-fiber chaining)
        #[arg(long)]
        test_type: Option<String>,
    },

    /// Create many threads at once for load testing and benchmarking
    #[command(after_long_help = "\
Creates threads with staggered interval triggers so their deadlines spread out
rather than all firing at once. IDs are deterministic (load-00000, load-00001,
...) so `load-clean` can remove them without a registry.

EXAMPLES:
    antegen thread test load --count 200
    antegen thread test load --count 500 --min-interval 5 --max-interval 120
    antegen thread test load-clean --count 500
")]
    Load {
        /// Number of threads to create
        #[arg(long, default_value_t = 100)]
        count: u32,

        /// Shortest interval trigger, in seconds
        #[arg(long, default_value_t = 10)]
        min_interval: u64,

        /// Longest interval trigger, in seconds
        #[arg(long, default_value_t = 60)]
        max_interval: u64,

        /// Transactions submitted concurrently
        #[arg(long, default_value_t = 8)]
        concurrency: usize,

        /// Lamports funded into each thread
        #[arg(long, default_value_t = 100_000_000)]
        fund: u64,
    },

    /// Delete threads created by `load`
    LoadClean {
        /// Number of threads to remove (must cover the range used to create)
        #[arg(long, default_value_t = 100)]
        count: u32,

        /// Deletions submitted concurrently
        #[arg(long, default_value_t = 8)]
        concurrency: usize,
    },

    /// List all managed test threads
    List,

    /// Delete test thread(s)
    #[command(after_long_help = "\
EXAMPLES:
    antegen thread test delete --id test-1
    antegen thread test delete --all

    # Advanced test types
    antegen thread test delete --test-type account
    antegen thread test delete --test-type chain
")]
    Delete {
        /// Thread ID to delete
        #[arg(long)]
        id: Option<String>,

        /// Delete all test threads
        #[arg(long)]
        all: bool,

        /// Advanced test type to delete: account, chain
        #[arg(long)]
        test_type: Option<String>,
    },

    /// Fiber management for test threads
    #[command(subcommand)]
    Fiber(TestFiberCommands),
}

#[cfg(feature = "dev")]
#[derive(Subcommand)]
pub enum TestFiberCommands {
    /// Add a fiber to a test thread
    #[command(after_long_help = "\
EXAMPLES:
    antegen thread test fiber add test-1
    antegen thread test fiber add test-1 --signal chain:2
    antegen thread test fiber add test-1 --signal next:0
    antegen thread test fiber add test-1 --signal repeat
")]
    Add {
        /// Test thread ID (e.g., \"test-1\")
        id: String,

        /// Signal for the fiber: chain:T, next:T, repeat, close
        #[arg(long)]
        signal: Option<String>,
    },

    /// List fibers on a test thread
    List {
        /// Test thread ID
        id: String,
    },

    /// Delete a fiber from a test thread
    Delete {
        /// Test thread ID
        id: String,

        /// Fiber index to delete
        #[arg(long)]
        index: u8,
    },
}

// =============================================================================
// Deprecation warning helper
// =============================================================================

fn deprecation_warning(old: &str, new: &str) {
    eprintln!(
        "Warning: `antegen {}` is deprecated. Use `antegen node {}` instead.",
        old, new
    );
    eprintln!();
}

// =============================================================================
// Main dispatch
// =============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    run_antegen().await
}

async fn run_antegen() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        // =================================================================
        // Program commands
        // =================================================================
        Commands::Program(program_cmd) => match program_cmd {
            ProgramCommands::Deploy {
                program_binary,
                program_id,
                keys_dir,
                skip_init,
                skip_verify,
            } => {
                commands::program::deploy(
                    program_binary,
                    cli.rpc,
                    cli.keypair,
                    program_id,
                    keys_dir,
                    skip_init,
                    skip_verify,
                )
                .await
            }
            ProgramCommands::Config(config_cmd) => match config_cmd {
                ProgramConfigCommands::Init => {
                    commands::program::config_init(cli.rpc, cli.keypair).await
                }
                ProgramConfigCommands::Get => commands::program::config_get(cli.rpc).await,
            },
        },

        // =================================================================
        // Thread commands
        // =================================================================
        Commands::Thread(thread_cmd) => match thread_cmd {
            ThreadCommands::Get { address } => commands::thread::get(address, cli.rpc).await,
            #[cfg(feature = "dev")]
            ThreadCommands::Delete { address } => {
                commands::thread::admin_delete(address, cli.rpc, cli.keypair).await
            }
            #[cfg(feature = "dev")]
            ThreadCommands::Test(test_cmd) => {
                commands::thread::test(cli.rpc, cli.keypair, test_cmd).await
            }
        },

        // =================================================================
        // Geyser commands
        // =================================================================
        Commands::Geyser(geyser_cmd) => match geyser_cmd {
            GeyserCommands::Init { output, config } => commands::geyser::init(output, config).await,
            GeyserCommands::Extract { output } => commands::geyser::extract(output).await,
        },

        // =================================================================
        // Node commands
        // =================================================================
        Commands::Node(node_cmd) => {
            commands::node::dispatch(node_cmd, cli.rpc, cli.log_level).await
        }

        // =================================================================
        // Top-level operator commands
        // =================================================================
        Commands::Init { rpc, force } => crate::commands::node::service::init(rpc, force),
        Commands::Info { json } => crate::commands::info::info(json).await,
        Commands::Fund { amount } => {
            let config = crate::commands::default_config_path()?;
            crate::commands::wallet::fund(config, amount, cli.keypair, cli.rpc).await
        }
        Commands::Withdraw { amount } => {
            let config = crate::commands::default_config_path()?;
            crate::commands::wallet::withdraw(config, amount, cli.rpc).await
        }
        Commands::Config(config_cmd) => dispatch_config(config_cmd, cli.rpc),

        // =================================================================
        // Hidden deprecated aliases — these moved under `antegen node`
        // =================================================================
        Commands::Start { rpc, version } => {
            deprecation_warning("start", "start");
            crate::commands::node::service::start(rpc, version).await
        }
        Commands::Status => {
            deprecation_warning("status", "status");
            crate::commands::node::service::status()
        }
        Commands::Logs { follow } => {
            deprecation_warning("logs", "logs");
            crate::commands::node::service::logs(follow)
        }
        Commands::Stop => {
            deprecation_warning("stop", "stop");
            crate::commands::node::service::stop()
        }
        Commands::Restart => {
            deprecation_warning("restart", "restart");
            crate::commands::node::service::restart()
        }
        Commands::Uninstall => {
            deprecation_warning("uninstall", "uninstall");
            crate::commands::node::service::uninstall()
        }
    }
}
