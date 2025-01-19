use clap::{Arg, ArgAction, ArgGroup, Command};
use antegen_thread_program::state::{SerializableInstruction, Trigger};
use solana_sdk::{pubkey::Pubkey, signature::Keypair};

use crate::parser::ProgramInfo;

#[derive(Debug, PartialEq)]
pub enum CliCommand {
    // Config commands
    ConfigView,
    ConfigSet {
        admin: Option<Pubkey>,
        epoch_thread: Option<Pubkey>,
        hasher_thread: Option<Pubkey>,
    },

    // Crontab
    Crontab {
        schedule: String,
    },

    // Delegation
    DelegationCreate {
        worker_id: u64,
    },
    DelegationDeposit {
        amount: u64,
        delegation_id: u64,
        worker_id: u64,
    },
    DelegationInfo {
        delegation_id: u64,
        worker_id: u64,
    },
    DelegationWithdraw {
        amount: u64,
        delegation_id: u64,
        worker_id: u64,
    },
    Initialize {
        mint: Pubkey,
    },
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
    ThreadDelete {
        id: String,
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
    RegistryUnlock,

    // Worker commands
    WorkerCreate {
        signatory: Keypair,
    },
    WorkerFind {
        id: u64,
    },
    WorkerUpdate {
        id: u64,
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
            Command::new("delegation")
                .about("Manage a stake delegation to a Antegen worker")
                .subcommand(
                    Command::new("create")
                        .about("Create a new delegation")
                        .arg_required_else_help(true)
                        .arg(
                            Arg::new("worker_id")
                                .long("worker_id")
                                .short('w')
                                .value_name("WORKER_ID")
                                .num_args(1)
                                .required(false)
                                .help("The ID of the worker to create a delegation with"),
                        ),
                )
                .subcommand(
                    Command::new("deposit")
                        .about("Deposit CLOCK to a delegation account")
                        .arg_required_else_help(true)
                        .arg(
                            Arg::new("amount")
                                .long("amount")
                                .short('a')
                                .value_name("AMOUNT")
                                .num_args(1)
                                .required(false)
                                .help("The number of tokens to deposit"),
                        )
                        .arg(
                            Arg::new("delegation_id")
                                .long("delegation_id")
                                .short('i')
                                .value_name("DELEGATION_ID")
                                .num_args(1)
                                .required(false)
                                .help("The ID of the delegation to deposit into"),
                        )
                        .arg(
                            Arg::new("worker_id")
                                .long("worker_id")
                                .short('w')
                                .value_name("WORKER_ID")
                                .num_args(1)
                                .required(false)
                                .help("The ID of the worker"),
                        ),
                )
                .subcommand(
                    Command::new("info")
                        .about("Get a delegation")
                        .arg_required_else_help(true)
                        .arg(
                            Arg::new("delegation_id")
                                .long("delegation_id")
                                .short('i')
                                .value_name("DELEGATION_ID")
                                .num_args(1)
                                .required(false)
                                .help("The ID of the delegation"),
                        )
                        .arg(
                            Arg::new("worker_id")
                                .long("worker_id")
                                .short('w')
                                .value_name("WORKER_ID")
                                .num_args(1)
                                .required(false)
                                .help("The ID of the worker"),
                        ),
                )
                .subcommand(
                    Command::new("withdraw")
                        .about("Withdraw CLOCK from a delegation account")
                        .arg_required_else_help(true)
                        .arg(
                            Arg::new("amount")
                                .long("amount")
                                .short('a')
                                .value_name("AMOUNT")
                                .num_args(1)
                                .required(false)
                                .help("The number of tokens to withdraw"),
                        )
                        .arg(
                            Arg::new("delegation_id")
                                .long("delegation_id")
                                .short('i')
                                .value_name("DELEGATION_ID")
                                .num_args(1)
                                .required(false)
                                .help("The ID of the delegation to withdraw from"),
                        )
                        .arg(
                            Arg::new("worker_id")
                                .long("worker_id")
                                .short('w')
                                .value_name("WORKER_ID")
                                .num_args(1)
                                .required(false)
                                .help("The ID of the worker"),
                        ),
                ),
        )
        .subcommand(
            Command::new("initialize")
                .about("Initialize the Antegen network program")
                .arg(
                    Arg::new("mint")
                        .long("mint")
                        .short('m')
                        .value_name("MINT")
                        .num_args(1)
                        .required(true)
                        .help("Mint address of network token"),
                ),
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
                    Command::new("delete")
                        .about("Delete a thread")
                        .arg_required_else_help(true)
                        .arg(
                            Arg::new("id")
                                .index(1)
                                .value_name("ID")
                                .num_args(1)
                                .required(false)
                                .help("The id of the thread to delete"),
                        ),
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
                    Command::new("find")
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
                        .about("Update a new worker")
                        .arg(
                            Arg::new("id")
                                .index(1)
                                .value_name("ID")
                                .num_args(1)
                                .required(true)
                                .help("The ID of the worker to edit"),
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
