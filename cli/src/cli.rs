use antegen_network_program::state::MAX_COMMISSION_RATE;
use clap::{value_parser, Arg, ArgAction, ArgGroup, Command};
use antegen_thread_program::state::{SerializableInstruction, Trigger};
use solana_sdk::{pubkey::Pubkey, signature::Keypair};

use crate::parser::ProgramInfo;

#[derive(Debug, PartialEq)]
pub enum CliCommand {
    // Crontab
    Crontab {
        schedule: String,
    },
    NetworkInitialize {},
    NetworkThreadCreate { amount: u64 },
    NetworkConfigSet {
        admin: Option<Pubkey>,
        epoch_thread: Option<Pubkey>,
        hasher_thread: Option<Pubkey>,
    },
    NetworkConfigGet,
    Localnet {
        force_init: bool,
        clone_addresses: Vec<Pubkey>,
        program_infos: Vec<ProgramInfo>,
        solana_archive: Option<String>,
        antegen_archive: Option<String>,
        dev: bool,
        trailing_args: Vec<String>,
    },
    PoolGet {
        id: u64,
    },
    PoolList {},
    PoolUpdate {
        id: u64,
        size: u64,
    },
    PoolRotate {
        id: u64,
    },
    // TODO Rename to Version. Use flags to filter by program.
    //      Default to listing all deployed program versions on the user's configured cluster.
    ThreadCrateInfo,
    ThreadCreate {
        id: String,
        kickoff_instruction: SerializableInstruction,
        trigger: Trigger,
    },
    ThreadMemoTest {  // New command for testing
        id: Option<String>,
        schedule: Option<String>,
        skippable: Option<bool>,
    },
    ThreadCloseTest {},
    ThreadDelete {
        id: Option<String>,
        address: Option<Pubkey>,
    },
    ThreadGet {
        id: Option<String>,
        address: Option<Pubkey>,
    },
    ThreadPause {
        id: String,
    },
    ThreadResume {
        id: String,
    },
    ThreadReset {
        id: String,
    },
    ThreadUpdate {
        id: String,
        rate_limit: Option<u64>,
        schedule: Option<String>,
    },

    // Registry
    RegistryGet,
    RegistryReset,
    RegistryUnlock,

    // Worker commands
    WorkerCreate {
        signatory: Keypair,
    },
    WorkerGet {
        id: u64,
    },
    WorkerUpdate {
        id: u64,
        commission_rate: Option<u64>,
        signatory: Option<Keypair>,
    },
}

pub fn app() -> Command {
    Command::new("Antegen")
        .bin_name("antegen")
        .about("An automation engine for the Solana blockchain")
        .version(env!("CARGO_PKG_VERSION")) // Use the crate version
        .arg_required_else_help(true)
        .subcommand(
            Command::new("config")
                .about("Manage the Antegen network config")
                .arg_required_else_help(true)
                .subcommand(Command::new("view").about("Get a config value"))
                .subcommand(
                    Command::new("set")
                        .about("Set a config value")
                        .arg(
                            Arg::new("admin")
                                .long("admin")
                                .value_name("ADMIN")
                                .num_args(1)
                        )
                        .arg(
                            Arg::new("epoch_thread")
                                .long("epoch_thread")
                                .value_name("EPOCH_THREAD")
                                .num_args(1)
                        )
                        .arg(
                            Arg::new("hasher_thread")
                                .long("hasher_thread")
                                .value_name("HASHER_THREAD")
                                .num_args(1)
                        )
                        .group(
                            ArgGroup::new("config_settings")
                                .args(&["admin", "epoch_thread", "hasher_thread"])
                                .multiple(true),
                        ),
                ),
        )
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
            Command::new("network")
                .about("Manage the Antegen Network Program")
                .subcommand(
                    Command::new("config")
                    .about("Antegen Network Configuration")
                    .arg_required_else_help(true)
                    .subcommand(Command::new("get")
                        .about("Get current config settings")
                    )
                    .subcommand(
                        Command::new("set")
                            .about("Set a config setting")
                            .arg(
                                Arg::new("admin")
                                    .long("admin")
                                    .value_name("ADMIN")
                                    .num_args(1)
                            )
                            .arg(
                                Arg::new("epoch_thread")
                                    .long("epoch_thread")
                                    .value_name("EPOCH_THREAD")
                                    .num_args(1)
                            )
                            .arg(
                                Arg::new("hasher_thread")
                                    .long("hasher_thread")
                                    .value_name("HASHER_THREAD")
                                    .num_args(1)
                            )
                            .group(
                                ArgGroup::new("config_settings")
                                    .args(&["admin", "epoch_thread", "hasher_thread"])
                                    .multiple(true),
                            ),
                    )
                )
                .subcommand(
                    Command::new("initialize")
                        .about("Initialize the Network Program")
                )
                .subcommand(
                    Command::new("threads")
                        .about("Manage Network threads")
                        .subcommand(
                            Command::new("create")
                                .about("Create Epoch and Hasher threads")
                                .arg(
                                    Arg::new("amount")
                                        .long("amount")
                                        .help("Amount in SOL to deposit")
                                        .value_parser(value_parser!(f64))
                                        .default_value("1.0")
                                )
                        )
                )
        )
        .subcommand(
            Command::new("localnet")
                .about("Launch a local Antegen worker for app development and testing")
                .arg(
                    Arg::new("bpf_program")
                        .long("bpf-program")
                        .value_names(&["ADDRESS_OR_KEYPAIR", "BPF_PROGRAM.SO"])
                        .value_name("BPF_PROGRAM")
                        .num_args(2)
                        .action(ArgAction::Append)
                        .help(
                            "Add a BPF program to the genesis configuration. \
                       If the ledger already exists then this parameter is silently ignored. \
                       First argument can be a pubkey string or path to a keypair",
                        ),
                )
                .arg(
                    Arg::new("clone")
                        .long("clone")
                        .short('c')
                        .value_names(&["ADDRESS"])
                        .value_name("CLONE")
                        .num_args(1)
                        .action(ArgAction::Append)
                        .help("Copy an account from the cluster referenced by the --url argument the genesis configuration. If the ledger already exists then this parameter is silently ignored")
                )
                .arg(
                    Arg::new("url")
                        .long("url")
                        .short('u')
                        .value_names(&["URL_OR_MONIKER"])
                        .value_name("URL")
                        .num_args(1)
                        .help("URL for Solana's JSON RPC or moniker (or their first letter): [mainnet-beta, testnet, devnet, localhost]")
                )
                .arg(Arg::new("force_init")
                    .long("force-init")
                    .action(ArgAction::SetTrue)
                    .default_value("false")
                    .help("Initializes and downloads localnet dependencies")
                )
                .arg(
                    Arg::new("solana_archive")
                        .long("solana-archive")
                        .help("url or local path to the solana archive containing the necessary \
                     dependencies such as solana-test-validator. \
                     Can be useful for debugging or testing different versions of solana-test-validator
                     ")
                    .value_name("SOLANA_ARCHIVE")
                        .num_args(1),
                )
                .arg(
                    Arg::new("antegen_archive")
                        .long("antegen-archive")
                        .help("url or local path to the solana archive containing the necessary \
                     dependencies such as clocwkork-thread-program, etc. \
                     Can be useful for debugging or testing different versions of antegen releases
                     ")
                        .value_name("ANTEGEN_ARCHIVE")
                        .num_args(1)

                )
                .arg(
                    Arg::new("dev")
                        .long("dev")
                        .action(ArgAction::SetTrue)
                        .default_value("false")
                        .help("Use development versions of antegen programs")
                )
                .arg(
                    Arg::new("test_validator_args")
                        .num_args(0..)
                        .allow_hyphen_values(true)
                        .trailing_var_arg(true)
                        .help("Arguments to pass to solana-test-validator")
                )
        )
        .subcommand(
            Command::new("pool")
                .about("Manage the Antegen network worker pools")
                .subcommand(
                    Command::new("get")
                        .about("Get a pool")
                        .arg_required_else_help(true)
                        .arg(
                            Arg::new("id")
                                .index(1)
                                .value_name("ID")
                                .num_args(1)
                                .required(false)
                                .help("The ID of the pool to lookup"),
                        ),
                )
                .subcommand(Command::new("list").about("List the pools"))
                .subcommand(
                    Command::new("update")
                        .about("Update a pool")
                        .arg_required_else_help(true)
                        .arg(
                            Arg::new("id")
                                .index(1)
                                .value_name("ID")
                                .num_args(1)
                                .required(false)
                                .help("The ID of the pool to update"),
                        )
                        .arg(
                            Arg::new("size")
                                .long("size")
                                .short('s')
                                .value_name("SIZE")
                                .num_args(1)
                                .required(false)
                                .help("The size of the pool"),
                        ),
                )
                .subcommand(
                    Command::new("rotate")
                        .about("Rotate worker into pool if space is available")
                        .arg_required_else_help(true)
                        .arg(
                            Arg::new("id")
                                .index(1)
                                .value_name("ID")
                                .num_args(1)
                                .required(false)
                                .help("The ID of the worker to rotate in"),
                        )
                ),
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
                            Arg::new("kickoff_instruction")
                                .long("kickoff_instruction")
                                .short('k')
                                .value_name("FILEPATH")
                                .num_args(1)
                                .required(true)
                                .help("Filepath to a description of the kickoff instruction"),
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
                    Command::new("memo-test")
                        .about("Create a test thread (localnet only)")
                        .arg(
                            Arg::new("id")
                                .short('i')
                                .long("id")
                                .required(false)
                                .help("Thread identifier, required to do multiples (default: memo-test")
                        )
                        .arg(
                            Arg::new("schedule")
                                .long("schedule")
                                .required(false)
                                .help("Cron schedule (default: */10 * * * * * *)")
                        )
                        .arg(
                            Arg::new("skippable")
                                .long("skippable")
                                .required(false)
                                .action(ArgAction::SetTrue)
                                .default_value("false")
                                .help("Whether to skip missed triggers")
                        )
                )
                .subcommand(
                    Command::new("close-test")
                        .about("Create a test thread (localnet only)")
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
                    Command::new("pause")
                        .about("Pause a thread")
                        .arg_required_else_help(true)
                        .arg(
                            Arg::new("id")
                                .index(1)
                                .value_name("ID")
                                .num_args(1)
                                .required(false)
                                .help("The id of the thread to pause"),
                        ),
                )
                .subcommand(
                    Command::new("resume").about("Resume a thread").arg(
                        Arg::new("id")
                            .index(1)
                            .value_name("ID")
                            .num_args(1)
                            .required(false)
                            .help("The id of the thread to resume"),
                    ),
                )
                .subcommand(
                    Command::new("reset").about("Reset a thread").arg(
                        Arg::new("id")
                            .index(1)
                            .required(false)
                            .value_name("ID")
                            .num_args(1)
                            .help("The id of the thread to stop"),
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
                            Arg::new("rate_limit")
                                .long("rate_limit")
                                .short('r')
                                .value_name("RATE_LIMIT")
                                .num_args(1)
                                .required(false)
                                .help(
                                    "The maximum number of instructions this thread can execute per slot",
                                ),
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
                ),
        )
        .subcommand(
            Command::new("registry")
                .about("Manage the Antegen network registry")
                .arg_required_else_help(true)
                .subcommand(Command::new("get").about("Lookup the registry"))
                .subcommand(Command::new("reset").about("Manually reset the registry"))
                .subcommand(Command::new("unlock").about("Manually unlock the registry")),
        )
        .subcommand(
            Command::new("snapshot")
                .about("Lookup the current Antegen network registry")
        )
        .subcommand(
            Command::new("worker")
                .about("Manage your workers")
                .arg_required_else_help(true)
                .subcommand(
                    Command::new("create")
                        .about("Register a new worker with the Antegen network")
                        .arg(
                            Arg::new("signatory_keypair")
                                .index(1)
                                .value_name("SIGNATORY_KEYPAIR")
                                .num_args(1)
                                .required(true)
                                .help("Filepath to the worker's signatory keypair"),
                        ),
                )
                .subcommand(
                    Command::new("get")
                        .about("Lookup a worker on the Antegen network")
                        .arg(
                            Arg::new("id")
                                .index(1)
                                .value_name("ID")
                                .num_args(1)
                                .required(true)
                                .help("The ID of the worker to lookup"),
                        ),
                )
                .subcommand(
                    Command::new("update")
                        .about("Update a worker")
                        .arg(
                            Arg::new("id")
                                .index(1)
                                .value_name("ID")
                                .num_args(1)
                                .required(true)
                                .help("The ID of the worker to edit"),
                        )
                        .arg(
                            Arg::new("commission_rate")
                                .value_name("COMMISSION_RATE")
                                .num_args(1)
                                .required(false)
                                .value_parser(value_parser!(u64))
                                .value_parser(value_parser!(u64).range(0..=MAX_COMMISSION_RATE))
                                .help("The commission rate (0-90)"),
                        )
                        .arg(
                            Arg::new("signatory_keypair")
                                .long("signatory_keypair")
                                .short('k')
                                .value_name("SIGNATORY_KEYPAIR")
                                .num_args(1)
                                .required(false)
                                .help("Filepath to the worker's new signatory keypair"),
                        ),
                ),
        )
}
