use std::{convert::TryFrom, fs, path::PathBuf, str::FromStr};

use clap::ArgMatches;
use antegen_thread_program::state::{SerializableAccount, SerializableInstruction, Trigger};
use serde::{Deserialize as JsonDeserialize, Serialize as JsonSerialize};
use solana_sdk::{
    native_token::LAMPORTS_PER_SOL, pubkey::Pubkey, signature::{read_keypair_file, Keypair}, signer::Signer
};

use crate::{cli::CliCommand, errors::CliError};

impl TryFrom<&ArgMatches> for CliCommand {
    type Error = CliError;

    fn try_from(matches: &ArgMatches) -> Result<Self, Self::Error> {
        match matches.subcommand() {
            Some(("crontab", matches)) => parse_crontab_command(matches),
            Some(("network", matches)) => parse_network_command(matches),
            Some(("localnet", matches)) => parse_bpf_command(matches),
            Some(("pool", matches)) => parse_pool_command(matches),
            Some(("thread", matches)) => parse_thread_command(matches),
            Some(("registry", matches)) => parse_registry_command(matches),
            Some(("worker", matches)) => parse_worker_command(matches),
            _ => Err(CliError::CommandNotRecognized(
                matches.subcommand().unwrap().0.into(),
            )),
        }
    }
}

// Command parsers
fn parse_bpf_command(matches: &ArgMatches) -> Result<CliCommand, CliError> {
    let mut program_infos = Vec::<ProgramInfo>::new();
    let mut clone_addresses = Vec::<Pubkey>::new();

    if let Some(values) = matches.get_many::<String>("bpf_program") {
        let values: Vec<String> = values.cloned().collect();
        for address_program in values.chunks(2) {
            match address_program {
                [address, program] => {
                    let address = address
                        .parse::<Pubkey>()
                        .or_else(|_| read_keypair_file(address).map(|keypair| keypair.pubkey()));

                    if address.is_err() {
                        return Err(CliError::InvalidAddress);
                    }

                    let program_path = PathBuf::from(program);

                    if !program_path.exists() {
                        return Err(CliError::InvalidProgramFile);
                    }

                    program_infos.push(ProgramInfo {
                        program_id: address.unwrap(),
                        program_path,
                    });
                }
                _ => unreachable!(),
            }
        }
    }

    if let Some(values) = matches.get_many::<String>("clone") {
        let values: Vec<String> = values.cloned().collect();
        for value in values {
            let address = value
                .parse::<Pubkey>()
                .map_err(|_| CliError::InvalidAddress)
                .unwrap();
            clone_addresses.push(address);
        }
    }

    Ok(CliCommand::Localnet {
        clone_addresses,
        program_infos,
        force_init: matches.get_flag("force_init"),
        solana_archive: parse_string("solana_archive", matches).ok(),
        antegen_archive: parse_string("antegen_archive", matches).ok(),
        dev: matches.get_flag("dev"),
        trailing_args: matches.get_many::<String>("test_validator_args")
            .unwrap_or_default()
            .map(|s| s.to_string())
            .collect(),
    })
}

fn parse_crontab_command(matches: &ArgMatches) -> Result<CliCommand, CliError> {
    Ok(CliCommand::Crontab {
        schedule: parse_string("schedule", matches)?,
    })
}

fn parse_network_command(matches: &ArgMatches) -> Result<CliCommand, CliError> {
    match matches.subcommand() {
        Some(("config", config_matches)) => match config_matches.subcommand() {
            Some(("set", matches)) => Ok(CliCommand::NetworkConfigSet {
                admin: parse_pubkey("admin", matches).ok(),
                epoch_thread: parse_pubkey("epoch_thread", matches).ok(),
                hasher_thread: parse_pubkey("hasher_thread", matches).ok(),
            }),
            Some(("get", _)) => Ok(CliCommand::NetworkConfigGet {}),
            _ => Err(CliError::CommandNotRecognized(
                matches.subcommand().unwrap().0.into(),
            )),
        },
        Some(("initialize", _)) => Ok(CliCommand::NetworkInitialize {}),
        Some(("threads", thread_matches)) => match thread_matches.subcommand() {
            Some(("create", create_matches)) => {
                let sol_amount = create_matches
                    .get_one::<f64>("amount")
                    .copied()
                    .unwrap_or(1.0);
                let amount = (sol_amount * LAMPORTS_PER_SOL as f64) as u64;
                Ok(CliCommand::NetworkThreadCreate { amount })
            },
            _ => Err(CliError::CommandNotRecognized(
                matches.subcommand().unwrap().0.into(),
            )),
        },
        _ => Err(CliError::CommandNotRecognized(
            matches.subcommand().unwrap().0.into(),
        )),
    }
}

fn parse_pool_command(matches: &ArgMatches) -> Result<CliCommand, CliError> {
    match matches.subcommand() {
        Some(("get", matches)) => Ok(CliCommand::PoolGet {
            id: parse_u64("id", matches)?,
        }),
        Some(("update", matches)) => Ok(CliCommand::PoolUpdate {
            id: parse_u64("id", matches)?,
            size: parse_u64("size", matches)?,
        }),
        Some(("rotate", matches)) => Ok(CliCommand::PoolRotate {
            id: parse_u64("id", matches)?,
        }),
        Some(("list", _)) => Ok(CliCommand::PoolList {}),
        _ => Err(CliError::CommandNotRecognized(
            matches.subcommand().unwrap().0.into(),
        )),
    }
}

fn parse_thread_command(matches: &ArgMatches) -> Result<CliCommand, CliError> {
    match matches.subcommand() {
        Some(("crate-info", _)) => Ok(CliCommand::ThreadCrateInfo {}),
        Some(("create", matches)) => Ok(CliCommand::ThreadCreate {
            id: parse_string("id", matches)?,
            kickoff_instruction: parse_instruction_file("kickoff_instruction", matches)?,
            trigger: parse_trigger(matches)?,
        }),
        Some(("memo-test", matches)) => Ok(CliCommand::ThreadMemoTest {
            id: matches.get_one::<String>("id").map(|s| s.to_string()),
            schedule: matches.get_one::<String>("schedule").map(|s| s.to_string()),
            skippable: matches.get_one::<bool>("skippable").copied(),
        }),
        Some(("close-test", _)) => Ok(CliCommand::ThreadCloseTest {}),
        Some(("delete", matches)) => Ok(CliCommand::ThreadDelete {
            id: parse_string("id", matches).ok(),
            address: parse_pubkey("address", matches).ok(),
        }),
        Some(("get", matches)) => Ok(CliCommand::ThreadGet {
            id: parse_string("id", matches).ok(),
            address: parse_pubkey("address", matches).ok(),
        }),
        Some(("pause", matches)) => Ok(CliCommand::ThreadPause {
            id: parse_string("id", matches)?,
        }),
        Some(("resume", matches)) => Ok(CliCommand::ThreadResume {
            id: parse_string("id", matches)?,
        }),
        Some(("reset", matches)) => Ok(CliCommand::ThreadReset {
            id: parse_string("id", matches)?,
        }),
        Some(("update", matches)) => Ok(CliCommand::ThreadUpdate {
            id: parse_string("id", matches)?,
            rate_limit: parse_u64("rate_limit", matches).ok(),
            schedule: parse_string("schedule", matches).ok(),
        }),
        _ => Err(CliError::CommandNotRecognized(
            matches.subcommand().unwrap().0.into(),
        )),
    }
}

fn parse_registry_command(matches: &ArgMatches) -> Result<CliCommand, CliError> {
    match matches.subcommand() {
        Some(("get", _)) => Ok(CliCommand::RegistryGet {}),
        Some(("reset", _)) => Ok(CliCommand::RegistryReset {}),
        Some(("unlock", _)) => Ok(CliCommand::RegistryUnlock {}),
        _ => Err(CliError::CommandNotRecognized(
            matches.subcommand().unwrap().0.into(),
        )),
    }
}

fn parse_worker_command(matches: &ArgMatches) -> Result<CliCommand, CliError> {
    match matches.subcommand() {
        Some(("create", matches)) => Ok(CliCommand::WorkerCreate {
            signatory: parse_keypair_file("signatory_keypair", matches)?,
        }),
        Some(("get", matches)) => Ok(CliCommand::WorkerGet {
            id: parse_u64("id", matches)?,
        }),
        Some(("update", matches)) => Ok(CliCommand::WorkerUpdate {
            id: parse_u64("id", matches)?,
            commission_rate: parse_u64("commission_rate", matches).ok(),
            signatory: parse_keypair_file("signatory_keypair", matches).ok(),
        }),
        _ => Err(CliError::CommandNotRecognized(
            matches.subcommand().unwrap().0.into(),
        )),
    }
}

// Arg parsers

fn parse_trigger(matches: &ArgMatches) -> Result<Trigger, CliError> {
    if matches.contains_id("account") {
        return Ok(Trigger::Account {
            address: parse_pubkey("account", matches)?,
            offset: 0, // TODO
            size: 32,  // TODO
        });
    } else if matches.contains_id("cron") {
        return Ok(Trigger::Cron {
            schedule: parse_string("cron", matches)?,
            skippable: true,
        });
    } else if matches.contains_id("now") {
        return Ok(Trigger::Now);
    }

    Err(CliError::BadParameter("trigger".into()))
}

fn parse_instruction_file(
    arg: &str,
    matches: &ArgMatches,
) -> Result<SerializableInstruction, CliError> {
    let filepath = parse_string(arg, matches)?;
    let text = fs::read_to_string(filepath).map_err(|_err| CliError::BadParameter(arg.into()))?;
    let ix: JsonInstructionData =
        serde_json::from_str(text.as_str()).expect("JSON was not well-formatted");
    SerializableInstruction::try_from(&ix)
}

fn parse_keypair_file(arg: &str, matches: &ArgMatches) -> Result<Keypair, CliError> {
    Ok(read_keypair_file(parse_string(arg, matches)?)
        .map_err(|_err| CliError::BadParameter(arg.into()))?)
}

fn parse_pubkey(arg: &str, matches: &ArgMatches) -> Result<Pubkey, CliError> {
    Ok(Pubkey::from_str(parse_string(arg, matches)?.as_str())
        .map_err(|_err| CliError::BadParameter(arg.into()))?)
}

fn parse_string(arg: &str, matches: &ArgMatches) -> Result<String, CliError> {
    Ok(matches
        .get_one::<String>(arg)
        .ok_or_else(|| CliError::BadParameter(arg.into()))?
        .to_string())
}

pub fn _parse_i64(arg: &str, matches: &ArgMatches) -> Result<i64, CliError> {
    Ok(parse_string(arg, matches)?
        .parse::<i64>()
        .map_err(|_err| CliError::BadParameter(arg.into()))
        .unwrap())
}

pub fn parse_u64(arg: &str, matches: &ArgMatches) -> Result<u64, CliError> {
    Ok(parse_string(arg, matches)?
        .parse::<u64>()
        .map_err(|_err| CliError::BadParameter(arg.into()))
        .unwrap())
}

// Json parsers

#[derive(Debug, JsonDeserialize, JsonSerialize)]
pub struct JsonInstructionData {
    pub program_id: String,
    pub accounts: Vec<JsonAccountMetaData>,
    pub data: Vec<u8>,
}

impl TryFrom<&JsonInstructionData> for SerializableInstruction {
    type Error = CliError;

    fn try_from(value: &JsonInstructionData) -> Result<Self, Self::Error> {
        Ok(SerializableInstruction {
            program_id: Pubkey::from_str(value.program_id.as_str())
                .map_err(|_err| CliError::BadParameter("Could not parse pubkey".into()))?,
            accounts: value
                .accounts
                .iter()
                .map(|acc| SerializableAccount::try_from(acc).unwrap())
                .collect::<Vec<SerializableAccount>>(),
            data: value.data.clone(),
        })
    }
}

// pub fn _parse_instruction(filepath: &String) -> Result<Instruction, CliError> {
//     let text =
//         fs::read_to_string(filepath).map_err(|_err| CliError::BadParameter("filepath".into()))?;
//     let ix: JsonInstructionData =
//         serde_json::from_str(text.as_str()).expect("JSON was not well-formatted");
//     Instruction::try_from(&ix)
// }

#[derive(Debug, JsonDeserialize, JsonSerialize, PartialEq)]
pub struct JsonAccountMetaData {
    pub pubkey: String,
    pub is_signer: bool,
    pub is_writable: bool,
}

impl TryFrom<&JsonAccountMetaData> for SerializableAccount {
    type Error = CliError;

    fn try_from(value: &JsonAccountMetaData) -> Result<Self, Self::Error> {
        Ok(SerializableAccount {
            pubkey: Pubkey::from_str(value.pubkey.as_str())
                .map_err(|_err| CliError::BadParameter("Could not parse pubkey".into()))?,
            is_signer: value.is_signer,
            is_writable: value.is_writable,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProgramInfo {
    pub program_id: Pubkey,
    pub program_path: PathBuf,
}
