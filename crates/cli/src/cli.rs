use antegen_sdk::state::Trigger;
use clap::{Arg, ArgAction, ArgGroup, Command};
use solana_sdk::pubkey::Pubkey;

use crate::parser::ProgramInfo;

#[derive(Debug, PartialEq)]
pub enum CliCommand {
    // Crontab
    Crontab {
        schedule: String,
    },
    Localnet {
        force_init: bool,
        clone_addresses: Vec<Pubkey>,
        program_infos: Vec<ProgramInfo>,
        solana_archive: Option<String>,
        antegen_archive: Option<String>,
        dev: bool,
        enable_replay: bool,
        nats_url: Option<String>,
        replay_delay_ms: u64,
        forgo_commission: bool,
        trailing_args: Vec<String>,
    },
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
                    Arg::new("enable_replay")
                        .long("enable-replay")
                        .action(ArgAction::SetTrue)
                        .default_value("false")
                        .help("Enable transaction replay via NATS")
                )
                .arg(
                    Arg::new("nats_url")
                        .long("nats-url")
                        .value_name("NATS_URL")
                        .num_args(1)
                        .help("NATS server URL for transaction replay (e.g., nats://localhost:4222)")
                )
                .arg(
                    Arg::new("replay_delay_ms")
                        .long("replay-delay-ms")
                        .value_name("MILLISECONDS")
                        .num_args(1)
                        .default_value("30000")
                        .help("Delay in milliseconds before replaying transactions")
                )
                .arg(
                    Arg::new("forgo_commission")
                        .long("forgo-commission")
                        .action(ArgAction::SetTrue)
                        .default_value("false")
                        .help("Executor forgoes commission fees")
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
                ),
        )
}
