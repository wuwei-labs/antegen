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
    /// Start the executor in standalone mode (RPC datasource)
    Start {
        /// Path to configuration file
        #[arg(short, long, default_value = "antegen.toml")]
        config: PathBuf,
    },

    /// Geyser plugin operations (downloads plugin from GitHub releases)
    #[command(subcommand)]
    Geyser(GeyserCommands),

    /// Config file operations
    #[command(subcommand)]
    Config(ConfigCommands),

    /// Thread inspection operations
    #[command(subcommand)]
    Thread(ThreadCommands),

    /// Client identity and utility operations
    #[command(subcommand)]
    Client(ClientCommands),

    /// Thread program management
    #[command(subcommand)]
    Program(ProgramCommands),
}

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

#[derive(Subcommand)]
enum ProgramCommands {
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

#[derive(Subcommand)]
enum ConfigCommands {
    /// Generate default config file
    Init {
        /// Output path for config file
        #[arg(short, long, default_value = "antegen.toml")]
        output: PathBuf,
    },

    /// Validate config file
    Validate {
        /// Path to config file
        #[arg(short, long, default_value = "antegen.toml")]
        config: PathBuf,
    },
}

#[derive(Subcommand)]
enum ClientCommands {
    /// Show executor public key (address)
    Address {
        /// Path to configuration file
        #[arg(short, long, default_value = "antegen.toml")]
        config: PathBuf,
    },

    /// Show executor SOL balance
    Balance {
        /// Path to configuration file
        #[arg(short, long, default_value = "antegen.toml")]
        config: PathBuf,

        /// RPC endpoint (overrides config)
        #[arg(long)]
        rpc: Option<String>,
    },

    /// Transfer SOL to an address (fund executor or any wallet)
    Refill {
        /// Destination address (executor pubkey or any wallet)
        #[arg(long)]
        address: String,

        /// Amount of SOL to transfer
        #[arg(long)]
        amount: f64,

        /// Funding keypair (defaults to Solana CLI keypair)
        #[arg(short, long)]
        keypair: Option<PathBuf>,

        /// RPC endpoint (defaults to Solana CLI config)
        #[arg(long)]
        rpc: Option<String>,
    },

    /// Export client identity for backup/migration
    Export {
        /// Path to configuration file
        #[arg(short, long, default_value = "antegen.toml")]
        config: PathBuf,

        /// Output archive path
        #[arg(short, long, default_value = "antegen-backup.tar.gz")]
        output: PathBuf,
    },

    /// Import client identity from backup
    Import {
        /// Input archive path
        #[arg(short, long)]
        input: PathBuf,

        /// Use existing keypair instead of generating new one
        #[arg(long)]
        keypair: Option<PathBuf>,

        /// Overwrite existing files
        #[arg(long)]
        force: bool,
    },

    /// Withdraw SOL from executor to Solana CLI keypair
    Withdraw {
        /// Path to configuration file
        #[arg(short, long, default_value = "antegen.toml")]
        config: PathBuf,

        /// Amount of SOL to withdraw
        #[arg(long)]
        amount: Option<f64>,

        /// Withdraw all SOL (minus transaction fee)
        #[arg(long)]
        all: bool,

        /// RPC endpoint (overrides config)
        #[arg(long)]
        rpc: Option<String>,
    },
}

#[derive(Subcommand)]
enum ThreadCommands {
    /// Fetch and display a thread account
    Get {
        /// Thread public key
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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Start { config } => commands::run::execute(config, cli.log_level).await,
        Commands::Geyser(geyser_cmd) => match geyser_cmd {
            GeyserCommands::Init { output, config } => {
                commands::geyser::init(output, config).await
            }
            GeyserCommands::Extract { output } => commands::geyser::extract(output).await,
        },
        Commands::Config(config_cmd) => match config_cmd {
            ConfigCommands::Init { output } => commands::config::init(output),
            ConfigCommands::Validate { config } => commands::config::validate(config),
        },
        Commands::Thread(thread_cmd) => match thread_cmd {
            ThreadCommands::Get { address } => commands::thread::get(address, cli.rpc).await,
            #[cfg(feature = "dev")]
            ThreadCommands::Test(test_cmd) => {
                commands::thread::test(cli.rpc, cli.keypair, test_cmd).await
            }
        },
        Commands::Client(client_cmd) => match client_cmd {
            ClientCommands::Address { config } => commands::client::address(config),
            ClientCommands::Balance { config, rpc } => {
                commands::client::balance(config, rpc).await
            }
            ClientCommands::Refill {
                address,
                amount,
                keypair,
                rpc,
            } => commands::client::refill(address, amount, keypair, rpc).await,
            ClientCommands::Export { config, output } => commands::client::export(config, output),
            ClientCommands::Import {
                input,
                keypair,
                force,
            } => commands::client::import(input, keypair, force),
            ClientCommands::Withdraw {
                config,
                amount,
                all,
                rpc,
            } => commands::client::withdraw(config, amount, all, rpc).await,
        },
        Commands::Program(program_cmd) => match program_cmd {
            ProgramCommands::Config(config_cmd) => match config_cmd {
                ProgramConfigCommands::Init => {
                    commands::program::config_init(cli.rpc, cli.keypair).await
                }
                ProgramConfigCommands::Get => commands::program::config_get(cli.rpc).await,
            },
        },
    }
}
