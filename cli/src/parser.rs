use std::{convert::TryFrom, fs, path::PathBuf, str::FromStr};

use antegen_thread_program::state::Trigger;
use antegen_utils::thread::{SerializableAccountMeta, SerializableInstruction};
use clap::ArgMatches;
use serde::{Deserialize as JsonDeserialize, Serialize as JsonSerialize};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{read_keypair_file, Keypair},
    signer::Signer,
};

use crate::{cli::CliCommand, errors::CliError};

impl TryFrom<&ArgMatches> for CliCommand {
    type Error = CliError;

    fn try_from(matches: &ArgMatches) -> Result<Self, Self::Error> {
        match matches.subcommand() {
            Some(("crontab", matches)) => parse_crontab_command(matches),
            Some(("network", matches)) => parse_network_command(matches),
            Some(("localnet", matches)) => parse_bpf_command(matches),
            Some(("thread", matches)) => parse_thread_command(matches),
            Some(("registry", matches)) => parse_registry_command(matches),
            Some(("builder", matches)) => parse_builder_command(matches),
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
        trailing_args: matches
            .get_many::<String>("test_validator_args")
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
        Some(("initialize", _)) => Ok(CliCommand::NetworkInitialize {}),
        _ => Err(CliError::CommandNotRecognized(
            matches.subcommand().unwrap().0.into(),
        )),
    }
}


fn parse_thread_command(matches: &ArgMatches) -> Result<CliCommand, CliError> {
    match matches.subcommand() {
        Some(("create", matches)) => Ok(CliCommand::ThreadCreate {
            id: parse_string("id", matches)?,
            trigger: parse_trigger(matches)?,
        }),
        Some(("delete", matches)) => Ok(CliCommand::ThreadDelete {
            id: parse_string("id", matches).ok(),
            address: parse_pubkey("address", matches).ok(),
        }),
        Some(("get", matches)) => Ok(CliCommand::ThreadGet {
            id: parse_string("id", matches).ok(),
            address: parse_pubkey("address", matches).ok(),
        }),
        Some(("toggle", matches)) => Ok(CliCommand::ThreadToggle {
            id: parse_string("id", matches)?,
        }),
        Some(("update", matches)) => Ok(CliCommand::ThreadUpdate {
            id: parse_string("id", matches)?,
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

fn parse_builder_command(matches: &ArgMatches) -> Result<CliCommand, CliError> {
    match matches.subcommand() {
        Some(("create", matches)) => Ok(CliCommand::BuilderCreate {
            signatory: parse_keypair_file("signatory_keypair", matches)?,
        }),
        Some(("get", matches)) => Ok(CliCommand::BuilderGet {
            id: parse_u32("id", matches)?,
        }),
        Some(("update", matches)) => Ok(CliCommand::BuilderUpdate {
            id: parse_u32("id", matches)?,
            commission_rate: parse_u64("commission_rate", matches).ok(),
            signatory: parse_keypair_file("signatory_keypair", matches).ok(),
        }),
        Some(("activate", matches)) => Ok(CliCommand::BuilderActivate {
            id: parse_u32("id", matches)?,
        }),
        Some(("deactivate", matches)) => Ok(CliCommand::BuilderDeactivate {
            id: parse_u32("id", matches)?,
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
    } else if matches.contains_id("immediate") {
        return Ok(Trigger::Now);
    }

    Err(CliError::BadParameter("trigger".into()))
}

// Removed - no longer needed for thread creation
fn _parse_instruction_file(
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


pub fn parse_u32(arg: &str, matches: &ArgMatches) -> Result<u32, CliError> {
    Ok(parse_string(arg, matches)?
        .parse::<u32>()
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
                .map(|acc| SerializableAccountMeta::try_from(acc).unwrap())
                .collect::<Vec<SerializableAccountMeta>>(),
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

impl TryFrom<&JsonAccountMetaData> for SerializableAccountMeta {
    type Error = CliError;

    fn try_from(value: &JsonAccountMetaData) -> Result<Self, Self::Error> {
        Ok(SerializableAccountMeta {
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
