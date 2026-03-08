//! Antegen CLI - Unified command-line interface
//!
//! Single binary that supports both standalone mode and Geyser plugin initialization.

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

#[derive(Parser)]
#[command(name = "antegen")]
#[command(about = "Antegen automation client", version)]
#[command(long_about = "
Antegen automation client for Solana thread execution.

Supports two deployment modes:
  1. Standalone: Run as a separate process using RPC subscriptions
  2. Plugin: Run as a Geyser plugin inside the validator

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
    /// Executor node management
    #[command(subcommand)]
    Node(NodeCommands),

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
    // Hidden backwards-compatibility aliases (deprecated)
    // =========================================================================

    /// Run the executor directly (no service, blocking)
    #[command(hide = true)]
    Run {
        #[arg(short, long)]
        config: Option<PathBuf>,
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
    },

    /// Initialize config only (no service)
    #[command(hide = true)]
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

    /// Update antegen to the latest version
    #[command(hide = true)]
    Update {
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
        #[arg(long)]
        manual_restart: bool,
    },

    /// Install antegen binary to ~/.local/bin (used by install script)
    #[command(hide = true)]
    Install {
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
    },

    /// Show antegen configuration and status
    #[command(hide = true)]
    Info {
        #[arg(long)]
        json: bool,
    },

    /// Config file operations
    #[command(hide = true, subcommand)]
    Config(NodeConfigCommands),
}

// =============================================================================
// Node commands
// =============================================================================

#[derive(Subcommand)]
enum NodeCommands {
    /// Run the executor directly (no service, blocking)
    Run {
        /// Path to configuration file (defaults to ~/.config/antegen/antegen.toml, will init if needed)
        #[arg(short, long)]
        config: Option<PathBuf>,

        /// Run a specific version (e.g., v4.4.0)
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
    },

    /// Initialize config only (no service)
    Init {
        /// RPC endpoint URL (prompts if not provided)
        #[arg(long)]
        rpc: Option<String>,

        /// Overwrite existing config
        #[arg(long)]
        force: bool,
    },

    /// Install and start the antegen service (init if needed)
    Start {
        /// RPC endpoint URL (prompts if not provided and interactive)
        #[arg(long)]
        rpc: Option<String>,

        /// Start a specific version (e.g., v4.4.0)
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
    },

    /// Show service status
    Status,

    /// View service logs
    Logs {
        /// Follow log output (like tail -f)
        #[arg(short, long)]
        follow: bool,
    },

    /// Stop the antegen service
    Stop,

    /// Restart the antegen service
    Restart,

    /// Uninstall the antegen service
    Uninstall,

    /// Update antegen to the latest version
    Update {
        /// Update to a specific version (e.g., v4.4.0)
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,

        /// Don't automatically restart the service after updating
        #[arg(long)]
        manual_restart: bool,
    },

    /// Install antegen binary to ~/.local/bin (used by install script)
    Install {
        /// Install a specific version (e.g., v4.4.0)
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
    },

    /// Show antegen configuration and status
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

    /// Config file operations
    #[command(subcommand)]
    Config(NodeConfigCommands),
}

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
    antegen node config set --max-threads 20
    antegen node config set --commitment finalized --tpu-enabled false
    antegen node config set --keypair-path ~/.antegen/my-keypair.json
    antegen node config set --grace-period 15 --eviction-buffer 30
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
        /// Output path for config file
        #[arg(short, long, default_value = "antegen.toml")]
        output: PathBuf,

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
// Geyser commands (unchanged)
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
// Thread commands (unchanged)
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
    eprintln!("Warning: `antegen {}` is deprecated. Use `antegen {}` instead.", old, new);
    eprintln!();
}

// =============================================================================
// Main dispatch
// =============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        // =================================================================
        // Node commands
        // =================================================================
        Commands::Node(node_cmd) => match node_cmd {
            NodeCommands::Run { config, version } => {
                let cfg = match config {
                    Some(p) => p,
                    None => commands::service::ensure_config()?,
                };
                commands::run::execute(cfg, cli.rpc, cli.log_level, version).await
            }
            NodeCommands::Init { rpc, force } => commands::service::init(rpc, force),
            NodeCommands::Start { rpc, version } => commands::service::start(rpc, version).await,
            NodeCommands::Status => commands::service::status(),
            NodeCommands::Logs { follow } => commands::service::logs(follow),
            NodeCommands::Stop => commands::service::stop(),
            NodeCommands::Restart => commands::service::restart(),
            NodeCommands::Uninstall => commands::service::uninstall(),
            NodeCommands::Update { version, manual_restart } => {
                commands::update::update(version, manual_restart).await
            }
            NodeCommands::Install { version } => commands::update::install(version).await,
            NodeCommands::Info { json } => commands::info::info(json).await,
            NodeCommands::Fund { amount } => {
                let config = commands::default_config_path()?;
                commands::client::fund(config, amount, cli.keypair, cli.rpc).await
            }
            NodeCommands::Withdraw { amount } => {
                let config = commands::default_config_path()?;
                commands::client::withdraw(config, amount, cli.rpc).await
            }
            NodeCommands::Config(config_cmd) => match config_cmd {
                NodeConfigCommands::Get { config } => {
                    let path = config.map(Ok).unwrap_or_else(commands::default_config_path)?;
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
                    let path = config.map(Ok).unwrap_or_else(commands::default_config_path)?;
                    commands::config::set(
                        path,
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
                NodeConfigCommands::Init { output, rpc, keypair_path, storage_path, force } => {
                    commands::config::init(output, rpc, keypair_path, storage_path, force)
                }
                NodeConfigCommands::Validate { config } => commands::config::validate(config),
            },
        },

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
                commands::program::deploy(program_binary, cli.rpc, cli.keypair, program_id, skip_init, skip_verify).await
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
        // Hidden backwards-compatibility aliases (deprecated)
        // =================================================================
        Commands::Run { config, version } => {
            deprecation_warning("run", "node run");
            let cfg = match config {
                Some(p) => p,
                None => commands::service::ensure_config()?,
            };
            commands::run::execute(cfg, cli.rpc, cli.log_level, version).await
        }
        Commands::Init { rpc, force } => {
            deprecation_warning("init", "node init");
            commands::service::init(rpc, force)
        }
        Commands::Start { rpc, version } => {
            deprecation_warning("start", "node start");
            commands::service::start(rpc, version).await
        }
        Commands::Status => {
            deprecation_warning("status", "node status");
            commands::service::status()
        }
        Commands::Logs { follow } => {
            deprecation_warning("logs", "node logs");
            commands::service::logs(follow)
        }
        Commands::Stop => {
            deprecation_warning("stop", "node stop");
            commands::service::stop()
        }
        Commands::Restart => {
            deprecation_warning("restart", "node restart");
            commands::service::restart()
        }
        Commands::Uninstall => {
            deprecation_warning("uninstall", "node uninstall");
            commands::service::uninstall()
        }
        Commands::Update { version, manual_restart } => {
            deprecation_warning("update", "node update");
            commands::update::update(version, manual_restart).await
        }
        Commands::Install { version } => {
            deprecation_warning("install", "node install");
            commands::update::install(version).await
        }
        Commands::Info { json } => {
            deprecation_warning("info", "node info");
            commands::info::info(json).await
        }
        Commands::Config(config_cmd) => {
            deprecation_warning("config", "node config");
            match config_cmd {
                NodeConfigCommands::Get { config } => {
                    let path = config.map(Ok).unwrap_or_else(commands::default_config_path)?;
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
                    let path = config.map(Ok).unwrap_or_else(commands::default_config_path)?;
                    commands::config::set(
                        path,
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
                NodeConfigCommands::Init { output, rpc, keypair_path, storage_path, force } => {
                    commands::config::init(output, rpc, keypair_path, storage_path, force)
                }
                NodeConfigCommands::Validate { config } => commands::config::validate(config),
            }
        }
    }
}
