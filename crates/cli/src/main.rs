//! Antegen CLI - Unified command-line interface
//!
//! Single binary that supports two personalities via argv[0]:
//!   - `antegen` — developer-facing: program, thread, geyser commands
//!   - `anm` — operator-facing: node version management, service control, config

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
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
// Antegen CLI (developer-facing: program, thread, geyser)
// =============================================================================

#[derive(Parser)]
#[command(name = "antegen")]
#[command(about = "Antegen automation client", version)]
#[command(long_about = "
Antegen automation client for Solana thread execution.

Supports two deployment modes:
  1. Standalone: Run as a separate process using RPC subscriptions
  2. Plugin: Run as a Geyser plugin inside the validator

For node management, use `anm` (Antegen Node Manager).
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

    // =========================================================================
    // Hidden: executor runtime (service invokes versioned binary with `run`)
    // =========================================================================

    /// Run the executor directly (no service, blocking)
    #[command(hide = true)]
    Run {
        #[arg(short, long)]
        config: Option<PathBuf>,
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
    },

    // =========================================================================
    // Hidden backwards-compatibility aliases (deprecated — use `anm` instead)
    // =========================================================================

    /// Register locally-built binaries and verify configuration
    Verify,

    /// Initialize antegen — creates config
    Init {
        #[arg(long)]
        rpc: Option<String>,
        #[arg(long)]
        force: bool,
    },

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

    /// Update CLI to the latest version
    Update {
        /// Update to a specific version (e.g., v5.0.0)
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
    },

    /// List installed and available versions
    List {
        /// Also show available versions from GitHub
        #[arg(long)]
        remote: bool,
    },

    /// Switch CLI to a specific version
    Use {
        /// Version to switch to (e.g., v5.0.0)
        version: String,
    },

    /// Download a specific CLI version (doesn't switch)
    Install {
        /// Version to install (e.g., v5.0.0)
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
    },

    /// Show antegen configuration and status
    #[command(hide = true)]
    Info {
        #[arg(long)]
        json: bool,
    },

    /// Fund the executor with SOL
    #[command(hide = true)]
    Fund {
        amount: Option<f64>,
    },

    /// Withdraw SOL from executor
    #[command(hide = true)]
    Withdraw {
        amount: Option<f64>,
    },

    /// Config file operations
    #[command(hide = true, subcommand)]
    Config(NodeConfigCommands),
}

// =============================================================================
// ANM CLI (operator-facing: node version management, service control, config)
// =============================================================================

#[derive(Parser)]
#[command(name = "anm")]
#[command(about = "Antegen Node Manager — node version management and service control", version)]
struct AnmCli {
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
    command: AnmCommands,
}

#[derive(Subcommand)]
enum AnmCommands {
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
        version: String,
    },
}

// =============================================================================
// Node config commands (shared between anm and backward-compat aliases)
// =============================================================================

#[derive(Subcommand)]
enum NodeConfigCommands {
    /// Display the current executor node configuration
    Get {
        /// Path to config file (defaults to ~/.config/antegen/antegen.toml)
        #[arg(short, long)]
        config: Option<PathBuf>,
    },

    /// Update configuration values
    #[command(after_long_help = "\
EXAMPLES:
    anm config set --max-threads 20
    anm config set --commitment finalized --tpu-enabled false
    anm config set --keypair-path ~/.antegen/my-keypair.json
    anm config set --grace-period 15 --eviction-buffer 30
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

// =============================================================================
// Program commands
// =============================================================================

#[derive(Subcommand)]
enum ProgramCommands {
    /// Deploy the program binary to a Solana cluster
    Deploy {
        /// Path to the compiled program .so file
        program_binary: PathBuf,

        /// Program ID or path to program keypair (defaults to declared program ID)
        #[arg(long)]
        program_id: Option<String>,

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
        "Warning: `antegen {}` is deprecated. Use `anm {}` instead.",
        old, new
    );
    eprintln!();
}

// =============================================================================
// Config command dispatch (shared between anm and backward-compat aliases)
// =============================================================================

fn dispatch_config(config_cmd: NodeConfigCommands, global_rpc: Option<String>) -> Result<()> {
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

// =============================================================================
// Main dispatch
// =============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    let argv0 = std::env::args().next().unwrap_or_default();
    let bin_name = std::path::Path::new(&argv0)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("antegen");

    if bin_name == "anm" {
        run_anm().await
    } else {
        run_antegen().await
    }
}

// =============================================================================
// ANM dispatch (operator-facing)
// =============================================================================

async fn run_anm() -> Result<()> {
    let cli = AnmCli::parse();

    match cli.command {
        AnmCommands::Run { config, version } => {
            let cfg = match config {
                Some(p) => p,
                None => commands::service::ensure_config()?,
            };
            commands::run::execute(cfg, cli.rpc, cli.log_level, version).await
        }
        AnmCommands::Init { rpc, force } => commands::service::init(rpc, force),
        AnmCommands::Start { rpc, version } => commands::service::start(rpc, version).await,
        AnmCommands::Stop => commands::service::stop(),
        AnmCommands::Restart => commands::service::restart(),
        AnmCommands::Status => commands::service::status(),
        AnmCommands::Logs { follow } => commands::service::logs(follow),
        AnmCommands::Uninstall => commands::service::uninstall(),
        AnmCommands::Info { json } => commands::info::info(json).await,
        AnmCommands::Fund { amount } => {
            let config = commands::default_config_path()?;
            commands::client::fund(config, amount, cli.keypair, cli.rpc).await
        }
        AnmCommands::Withdraw { amount } => {
            let config = commands::default_config_path()?;
            commands::client::withdraw(config, amount, cli.rpc).await
        }
        AnmCommands::Update { version } => commands::update::update_node(version).await,
        AnmCommands::List => commands::update::list_node().await,
        AnmCommands::Use { version } => commands::update::use_node_version(version).await,
        AnmCommands::Install { version } => commands::update::install_node_version(version).await,
        AnmCommands::Config(config_cmd) => dispatch_config(config_cmd, cli.rpc),
    }
}

// =============================================================================
// Antegen dispatch (developer-facing)
// =============================================================================

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
                skip_init,
                skip_verify,
            } => {
                commands::program::deploy(
                    program_binary,
                    cli.rpc,
                    cli.keypair,
                    program_id,
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
            GeyserCommands::Init { output, config } => {
                commands::geyser::init(output, config).await
            }
            GeyserCommands::Extract { output } => commands::geyser::extract(output).await,
        },

        // =================================================================
        // Hidden: executor runtime (service entry point, no deprecation warning)
        // =================================================================
        Commands::Run { config, version } => {
            let cfg = match config {
                Some(p) => p,
                None => commands::service::ensure_config()?,
            };
            commands::run::execute(cfg, cli.rpc, cli.log_level, version).await
        }

        // =================================================================
        // Hidden backwards-compatibility aliases (deprecated — use `anm`)
        // =================================================================
        Commands::Verify => {
            commands::update::import_current_binary()?;
            commands::update::import_node_binary()?;
            // Validate config if it exists
            if let Ok(config_path) = commands::default_config_path() {
                if config_path.exists() {
                    println!();
                    commands::config::validate(config_path)?;
                }
            }
            Ok(())
        }
        Commands::Init { rpc, force } => {
            commands::service::init(rpc, force)
        }
        Commands::Start { rpc, version } => {
            deprecation_warning("start", "start");
            commands::service::start(rpc, version).await
        }
        Commands::Status => {
            deprecation_warning("status", "status");
            commands::service::status()
        }
        Commands::Logs { follow } => {
            deprecation_warning("logs", "logs");
            commands::service::logs(follow)
        }
        Commands::Stop => {
            deprecation_warning("stop", "stop");
            commands::service::stop()
        }
        Commands::Restart => {
            deprecation_warning("restart", "restart");
            commands::service::restart()
        }
        Commands::Uninstall => {
            deprecation_warning("uninstall", "uninstall");
            commands::service::uninstall()
        }
        Commands::Update { version } => {
            commands::update::update(version).await
        }
        Commands::List { remote } => {
            commands::update::list_cli(remote).await
        }
        Commands::Use { version } => {
            commands::update::use_cli_version(version).await
        }
        Commands::Install { version } => {
            // Used by install script — no deprecation warning
            commands::update::install(version).await
        }
        Commands::Info { json } => {
            deprecation_warning("info", "info");
            commands::info::info(json).await
        }
        Commands::Fund { amount } => {
            deprecation_warning("fund", "fund");
            let config = commands::default_config_path()?;
            commands::client::fund(config, amount, cli.keypair, cli.rpc).await
        }
        Commands::Withdraw { amount } => {
            deprecation_warning("withdraw", "withdraw");
            let config = commands::default_config_path()?;
            commands::client::withdraw(config, amount, cli.rpc).await
        }
        Commands::Config(config_cmd) => {
            deprecation_warning("config", "config");
            dispatch_config(config_cmd, cli.rpc)
        }
    }
}
