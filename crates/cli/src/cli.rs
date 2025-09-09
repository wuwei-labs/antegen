use antegen_sdk::state::Trigger;
use clap::{Arg, ArgAction, ArgGroup, Command};
use solana_sdk::pubkey::Pubkey;

#[derive(Debug, PartialEq)]
pub enum CliCommand {
    // Crontab
    Crontab {
        schedule: String,
    },
    LocalnetStart {
        config: Option<String>,
        validator: Option<String>,
        clients: Vec<String>,
        release: bool,
        verbose: bool,
    },
    LocalnetStartRpc {
        release: bool,
        verbose: bool,
    },
    LocalnetStartCarbon {
        release: bool,
        verbose: bool,
    },
    LocalnetStartGeyser {
        release: bool,
        verbose: bool,
    },
    LocalnetStop,
    LocalnetStatus,
    LocalnetClientAdd {
        client_type: String,
        name: Option<String>,
        rpc_url: Option<String>,
        keypair: Option<String>,
    },
    LocalnetClientRemove {
        name: Option<String>,
    },
    LocalnetClientList,
    ThreadCreate {
        id: String,
        trigger: Trigger,
    },
    ThreadDelete {
        id: Option<String>,
        address: Option<Pubkey>,
    },
    ThreadGet {
        id: Option<String>,
        address: Option<Pubkey>,
    },
    ThreadToggle {
        id: String,
    },
    ThreadUpdate {
        id: String,
        schedule: Option<String>,
    },
    ThreadStressTest {
        count: u32,
        interval: u64,
        jitter: u64,
        prefix: String,
        with_fibers: bool,
        batch_size: u32,
        durable_ratio: u8,
        min_fiber_count: u8,
        max_fiber_count: u8,
    },
    Config {
        subcommand: ConfigSubcommand,
    },
}

#[derive(Debug, PartialEq)]
pub enum ConfigSubcommand {
    Init {
        admin: Option<Pubkey>,
    },
    Show,
    Update {
        commission_fee: Option<u64>,
        executor_fee_bps: Option<u16>,
        core_team_bps: Option<u16>,
        grace_period: Option<i64>,
        fee_decay: Option<i64>,
        pause: bool,
        unpause: bool,
        multisig: bool,
    },
}

pub fn app() -> Command {
    Command::new("Antegen")
        .bin_name("antegen")
        .about("An automation engine for the Solana blockchain")
        .version(env!("CARGO_PKG_VERSION")) // Use the crate version
        .arg_required_else_help(true)
        .subcommand(
            Command::new("crontab")
                .about("Generate a cron firing table from schedule")
                .arg_required_else_help(true)
                .arg(
                    Arg::new("schedule")
                        .index(1)
                        .value_name("SCHEDULE")
                        .num_args(1)
                        .required(true)
                        .help("The schedule to generate a cron table for"),
                ),
        )
        .subcommand(
            Command::new("localnet")
                .about("Manage local Antegen development environment")
                .arg_required_else_help(true)
                .subcommand(
                    Command::new("start")
                        .about("Start a local Antegen environment")
                        .arg(
                            Arg::new("config")
                                .long("config")
                                .short('c')
                                .value_name("PATH")
                                .num_args(1)
                                .help("Path to a custom configuration file")
                        )
                        .arg(
                            Arg::new("validator")
                                .long("validator")
                                .short('v')
                                .value_name("TYPE")
                                .num_args(1)
                                .help("Validator type to use (default: solana)")
                        )
                        .arg(
                            Arg::new("client")
                                .long("client")
                                .value_name("TYPE")
                                .num_args(1)
                                .action(ArgAction::Append)
                                .help("Add a client to run (can be specified multiple times). Options: geyser, carbon")
                        )
                        .arg(
                            Arg::new("release")
                                .long("release")
                                .action(ArgAction::SetTrue)
                                .help("Use release binaries from ~/.config/antegen instead of dev builds")
                        )
                        .arg(
                            Arg::new("verbose")
                                .long("verbose")
                                .short('V')
                                .action(ArgAction::SetTrue)
                                .help("Enable verbose/debug logging for all services")
                        )
                )
                .subcommand(
                    Command::new("start:rpc")
                        .about("Start localnet with RPC client")
                        .arg(
                            Arg::new("release")
                                .long("release")
                                .action(ArgAction::SetTrue)
                                .help("Use release binaries from ~/.config/antegen instead of dev builds")
                        )
                        .arg(
                            Arg::new("verbose")
                                .long("verbose")
                                .short('V')
                                .action(ArgAction::SetTrue)
                                .help("Enable verbose/debug logging for all services")
                        )
                )
                .subcommand(
                    Command::new("start:carbon")
                        .about("Start localnet with Carbon client")
                        .arg(
                            Arg::new("release")
                                .long("release")
                                .action(ArgAction::SetTrue)
                                .help("Use release binaries from ~/.config/antegen instead of dev builds")
                        )
                        .arg(
                            Arg::new("verbose")
                                .long("verbose")
                                .short('V')
                                .action(ArgAction::SetTrue)
                                .help("Enable verbose/debug logging for all services")
                        )
                )
                .subcommand(
                    Command::new("start:geyser")
                        .about("Start localnet with Geyser plugin enabled")
                        .arg(
                            Arg::new("release")
                                .long("release")
                                .action(ArgAction::SetTrue)
                                .help("Use release binaries from ~/.config/antegen instead of dev builds")
                        )
                        .arg(
                            Arg::new("verbose")
                                .long("verbose")
                                .short('V')
                                .action(ArgAction::SetTrue)
                                .help("Enable verbose/debug logging for all services")
                        )
                )
                .subcommand(
                    Command::new("stop")
                        .about("Stop the running localnet")
                )
                .subcommand(
                    Command::new("status")
                        .about("Get status of the running localnet")
                )
                .subcommand(
                    Command::new("client")
                        .about("Manage localnet clients")
                        .arg_required_else_help(true)
                        .subcommand(
                            Command::new("add")
                                .about("Add a new client to the running localnet")
                                .arg(
                                    Arg::new("type")
                                        .long("type")
                                        .short('t')
                                        .value_name("TYPE")
                                        .num_args(1)
                                        .required(true)
                                        .help("Client type (rpc, carbon)")
                                )
                                .arg(
                                    Arg::new("name")
                                        .long("name")
                                        .short('n')
                                        .value_name("NAME")
                                        .num_args(1)
                                        .help("Client name (defaults to type-N)")
                                )
                                .arg(
                                    Arg::new("rpc-url")
                                        .long("rpc-url")
                                        .value_name("URL")
                                        .num_args(1)
                                        .help("RPC URL (default: http://localhost:8899)")
                                )
                                .arg(
                                    Arg::new("keypair")
                                        .long("keypair")
                                        .short('k')
                                        .value_name("PATH")
                                        .num_args(1)
                                        .help("Path to keypair file")
                                )
                        )
                        .subcommand(
                            Command::new("remove")
                                .about("Remove a client from the running localnet")
                                .arg(
                                    Arg::new("name")
                                        .index(1)
                                        .value_name("NAME")
                                        .required(false)
                                        .help("Client name to remove (interactive selection if not provided)")
                                )
                        )
                        .subcommand(
                            Command::new("list")
                                .about("List all running clients")
                        )
                )
        )
        .subcommand(
            Command::new("thread")
                .about("Manage your transaction threads")
                .arg_required_else_help(true)
                .subcommand(
                    Command::new("crate-info")
                        .about("Crate Information")
                )
                .subcommand(
                    Command::new("create")
                        .about("Create an new thread")
                        .arg_required_else_help(true)
                        .arg(
                            Arg::new("id")
                                .long("id")
                                .short('i')
                                .value_name("ID")
                                .num_args(1)
                                .required(true)
                                .help("The ID of the thread to be created"),
                        )
                        .arg(
                            Arg::new("account")
                                .long("account")
                                .short('a')
                                .value_name("ADDRESS")
                                .num_args(1)
                                .help("An account-based trigger"),
                        )
                        .arg(
                            Arg::new("cron")
                                .long("cron")
                                .short('c')
                                .value_name("SCHEDULE")
                                .num_args(1)
                                .help("A cron-based trigger"),
                        )
                        .arg(
                            Arg::new("immediate")
                                .long("immediate")
                                .short('m')
                                .help("An immediate trigger"),
                        )
                        .group(
                            ArgGroup::new("trigger")
                                .args(&["account", "cron", "immediate"])
                                .required(true),
                        ),
                )
                .subcommand(
                    Command::new("delete")
                        .about("Delete a thread")
                        .arg_required_else_help(true)
                        .arg(
                            Arg::new("id")
                                .index(1)
                                .required(false)
                                .value_name("ID")
                                .num_args(1)
                                .help("The ID of the thread to delete (must have authority)")
                        )
                        .arg(
                            Arg::new("address")
                                .short('k')
                                .long("address")
                                .value_name("ADDRESS")
                                .num_args(1)
                                .help("The address of the thread to delete"),
                        )
                )
                .subcommand(
                    Command::new("get")
                        .about("Lookup a thread")
                        .arg_required_else_help(true)
                        .arg(
                            Arg::new("id")
                                .index(1)
                                .required(false)
                                .value_name("ID")
                                .num_args(1)
                                .help("The label of the thread to lookup (only works if you \
                                are the signer of that thread)")
                        )
                        .arg(
                            Arg::new("address")
                                .short('k')
                                .long("address")
                                .value_name("ADDRESS")
                                .num_args(1)
                                .help("The address of the thread to lookup"),
                        )
                )
                .subcommand(
                    Command::new("toggle")
                        .about("Toggle a thread's pause state")
                        .arg_required_else_help(true)
                        .arg(
                            Arg::new("id")
                                .index(1)
                                .value_name("ID")
                                .num_args(1)
                                .required(true)
                                .help("The id of the thread to toggle"),
                        ),
                )
                .subcommand(
                    Command::new("update")
                        .about("Update a property of a thread")
                        .arg_required_else_help(true)
                        .arg(
                            Arg::new("id")
                                .index(1)
                                .value_name("ID")
                                .num_args(1)
                                .required(false)
                                .help("The id of the thread to lookup"),
                        )
                        .arg(
                            Arg::new("schedule")
                                .long("schedule")
                                .short('s')
                                .value_name("SCHEDULE")
                                .num_args(1)
                                .required(false)
                                .help("The cron schedule of the thread"),
                        ),
                )
                .subcommand(
                    Command::new("stress-test")
                        .about("Create multiple test threads for stress testing")
                        .arg(
                            Arg::new("count")
                                .long("count")
                                .short('c')
                                .value_name("COUNT")
                                .default_value("100")
                                .help("Number of threads to create")
                        )
                        .arg(
                            Arg::new("interval")
                                .long("interval")
                                .short('i')
                                .value_name("SECONDS")
                                .default_value("30")
                                .help("Base interval in seconds for thread triggers")
                        )
                        .arg(
                            Arg::new("jitter")
                                .long("jitter")
                                .short('j')
                                .value_name("SECONDS")
                                .default_value("5")
                                .help("Random jitter in seconds to add/subtract from base interval")
                        )
                        .arg(
                            Arg::new("prefix")
                                .long("prefix")
                                .short('p')
                                .value_name("PREFIX")
                                .default_value("stress-test")
                                .help("Prefix for thread IDs")
                        )
                        .arg(
                            Arg::new("with-fibers")
                                .long("with-fibers")
                                .action(clap::ArgAction::SetTrue)
                                .help("Create fibers with memo instructions for each thread")
                        )
                        .arg(
                            Arg::new("batch-size")
                                .long("batch-size")
                                .short('b')
                                .value_name("SIZE")
                                .default_value("10")
                                .help("Number of threads to create per batch (to avoid rate limiting)")
                        )
                        .arg(
                            Arg::new("durable-ratio")
                                .long("durable-ratio")
                                .short('d')
                                .value_name("PERCENTAGE")
                                .default_value("50")
                                .help("Percentage of threads to create as durable (with nonce accounts), 0-100")
                        )
                        .arg(
                            Arg::new("min-fibers")
                                .long("min-fibers")
                                .value_name("MIN")
                                .default_value("1")
                                .help("Minimum number of fibers per thread (1-50)")
                        )
                        .arg(
                            Arg::new("max-fibers")
                                .long("max-fibers")
                                .value_name("MAX")
                                .help("Maximum number of fibers per thread (defaults to min-fibers value)")
                        ),
                ),
        )
        .subcommand(
            Command::new("config")
                .about("Manage thread program configuration")
                .arg_required_else_help(true)
                .subcommand(
                    Command::new("init")
                        .about("Initialize thread program config")
                        .arg(
                            Arg::new("admin")
                                .long("admin")
                                .value_name("PUBKEY")
                                .help("Admin pubkey (defaults to keypair)")
                        )
                )
                .subcommand(
                    Command::new("show")
                        .about("Display current thread config")
                )
                .subcommand(
                    Command::new("update")
                        .about("Update thread config parameters")
                        .arg(
                            Arg::new("commission-fee")
                                .long("commission-fee")
                                .value_name("LAMPORTS")
                                .help("Base commission fee in lamports")
                        )
                        .arg(
                            Arg::new("executor-fee-bps")
                                .long("executor-fee-bps")
                                .value_name("BPS")
                                .help("Executor fee percentage in basis points (100 = 1%)")
                        )
                        .arg(
                            Arg::new("core-team-bps")
                                .long("core-team-bps")
                                .value_name("BPS")
                                .help("Core team fee percentage in basis points")
                        )
                        .arg(
                            Arg::new("grace-period")
                                .long("grace-period")
                                .value_name("SECONDS")
                                .help("Grace period in seconds")
                        )
                        .arg(
                            Arg::new("fee-decay")
                                .long("fee-decay")
                                .value_name("SECONDS")
                                .help("Fee decay period in seconds")
                        )
                        .arg(
                            Arg::new("pause")
                                .long("pause")
                                .action(ArgAction::SetTrue)
                                .help("Pause the thread program")
                        )
                        .arg(
                            Arg::new("unpause")
                                .long("unpause")
                                .action(ArgAction::SetTrue)
                                .help("Unpause the thread program")
                        )
                        .arg(
                            Arg::new("multisig")
                                .long("multisig")
                                .action(ArgAction::SetTrue)
                                .help("Use SQUADs multisig for the update")
                        )
                )
        )
}
