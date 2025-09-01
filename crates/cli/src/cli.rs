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
    },
    LocalnetStop,
    LocalnetStatus,
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
                )
                .subcommand(
                    Command::new("stop")
                        .about("Stop the running localnet")
                )
                .subcommand(
                    Command::new("status")
                        .about("Get status of the running localnet")
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
