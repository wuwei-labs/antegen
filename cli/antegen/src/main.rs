//! Antegen CLI — developer-facing: program, thread, geyser commands

use antegen_cli_core::{dispatch_config, LogLevel, NodeConfigCommands};
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod commands;

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

For node management, use `antegenctl` (Antegen Node Manager).
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
    // Hidden backwards-compatibility aliases (deprecated — use `antegenctl` instead)
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
    Fund { amount: Option<f64> },

    /// Withdraw SOL from executor
    #[command(hide = true)]
    Withdraw { amount: Option<f64> },

    /// Config file operations
    #[command(hide = true, subcommand)]
    Config(NodeConfigCommands),
}

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
        "Warning: `antegen {}` is deprecated. Use `antegenctl {}` instead.",
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
        // Hidden: executor runtime (service entry point, no deprecation warning)
        // =================================================================
        Commands::Run { config, version } => {
            let cfg = match config {
                Some(p) => p,
                None => antegen_cli_core::commands::service::ensure_config()?,
            };
            antegen_cli_core::commands::run::execute(cfg, cli.rpc, cli.log_level, version).await
        }

        // =================================================================
        // Hidden backwards-compatibility aliases (deprecated — use `antegenctl`)
        // =================================================================
        Commands::Verify => {
            antegen_cli_core::commands::update::import_current_binary()?;
            antegen_cli_core::commands::update::import_node_binary()?;
            // Validate config if it exists
            if let Ok(config_path) = antegen_cli_core::commands::default_config_path() {
                if config_path.exists() {
                    println!();
                    antegen_cli_core::commands::config::validate(config_path)?;
                }
            }
            Ok(())
        }
        Commands::Init { rpc, force } => antegen_cli_core::commands::service::init(rpc, force),
        Commands::Start { rpc, version } => {
            deprecation_warning("start", "start");
            antegen_cli_core::commands::service::start(rpc, version).await
        }
        Commands::Status => {
            deprecation_warning("status", "status");
            antegen_cli_core::commands::service::status()
        }
        Commands::Logs { follow } => {
            deprecation_warning("logs", "logs");
            antegen_cli_core::commands::service::logs(follow)
        }
        Commands::Stop => {
            deprecation_warning("stop", "stop");
            antegen_cli_core::commands::service::stop()
        }
        Commands::Restart => {
            deprecation_warning("restart", "restart");
            antegen_cli_core::commands::service::restart()
        }
        Commands::Uninstall => {
            deprecation_warning("uninstall", "uninstall");
            antegen_cli_core::commands::service::uninstall()
        }
        Commands::Update { version } => antegen_cli_core::commands::update::update(version).await,
        Commands::List { remote } => antegen_cli_core::commands::update::list_cli(remote).await,
        Commands::Use { version } => {
            antegen_cli_core::commands::update::use_cli_version(version).await
        }
        Commands::Install { version } => {
            // Used by install script — no deprecation warning
            antegen_cli_core::commands::update::install(version).await
        }
        Commands::Info { json } => {
            deprecation_warning("info", "info");
            antegen_cli_core::commands::info::info(json).await
        }
        Commands::Fund { amount } => {
            deprecation_warning("fund", "fund");
            let config = antegen_cli_core::commands::default_config_path()?;
            antegen_cli_core::commands::client::fund(config, amount, cli.keypair, cli.rpc).await
        }
        Commands::Withdraw { amount } => {
            deprecation_warning("withdraw", "withdraw");
            let config = antegen_cli_core::commands::default_config_path()?;
            antegen_cli_core::commands::client::withdraw(config, amount, cli.rpc).await
        }
        Commands::Config(config_cmd) => {
            deprecation_warning("config", "config");
            dispatch_config(config_cmd, cli.rpc)
        }
    }
}
