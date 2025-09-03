use std::{convert::TryFrom, fs, str::FromStr};

use antegen_sdk::state::{Trigger, SerializableAccountMeta, SerializableInstruction};
use clap::ArgMatches;
use serde::{Deserialize as JsonDeserialize, Serialize as JsonSerialize};
use solana_sdk::pubkey::Pubkey;

use crate::{cli::CliCommand, errors::CliError};

impl TryFrom<&ArgMatches> for CliCommand {
    type Error = CliError;

    fn try_from(matches: &ArgMatches) -> Result<Self, Self::Error> {
        match matches.subcommand() {
            Some(("crontab", matches)) => parse_crontab_command(matches),
            Some(("localnet", matches)) => parse_bpf_command(matches),
            Some(("thread", matches)) => parse_thread_command(matches),
            _ => Err(CliError::CommandNotRecognized(
                matches.subcommand().unwrap().0.into(),
            )),
        }
    }
}

// Command parsers
fn parse_bpf_command(matches: &ArgMatches) -> Result<CliCommand, CliError> {
    match matches.subcommand() {
        Some(("start", matches)) => {
            let config = parse_string("config", matches).ok();
            let validator = parse_string("validator", matches).ok();
            let clients = matches
                .get_many::<String>("client")
                .map(|values| values.cloned().collect())
                .unwrap_or_default();
            let release = matches.get_flag("release");
            
            Ok(CliCommand::LocalnetStart {
                config,
                validator,
                clients,
                release,
            })
        }
        Some(("stop", _)) => Ok(CliCommand::LocalnetStop),
        Some(("status", _)) => Ok(CliCommand::LocalnetStatus),
        Some(("client", matches)) => parse_localnet_client_command(matches),
        _ => Err(CliError::CommandNotRecognized(
            matches.subcommand().unwrap().0.into(),
        )),
    }
}

fn parse_localnet_client_command(matches: &ArgMatches) -> Result<CliCommand, CliError> {
    match matches.subcommand() {
        Some(("add", matches)) => {
            Ok(CliCommand::LocalnetClientAdd {
                client_type: parse_string("type", matches)?,
                name: matches.get_one::<String>("name").cloned(),
                rpc_url: matches.get_one::<String>("rpc-url").cloned(),
                keypair: matches.get_one::<String>("keypair").cloned(),
            })
        }
        Some(("remove", matches)) => {
            Ok(CliCommand::LocalnetClientRemove {
                name: parse_string("name", matches)?,
            })
        }
        Some(("list", _)) => Ok(CliCommand::LocalnetClientList),
        _ => Err(CliError::CommandNotRecognized(
            matches.subcommand().unwrap().0.into(),
        )),
    }
}

fn parse_crontab_command(matches: &ArgMatches) -> Result<CliCommand, CliError> {
    Ok(CliCommand::Crontab {
        schedule: parse_string("schedule", matches)?,
    })
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
        Some(("stress-test", matches)) => {
            let durable_ratio = parse_string("durable-ratio", matches)?
                .parse::<u8>()
                .map_err(|_| CliError::BadParameter("durable-ratio must be a number".into()))?;
            
            if durable_ratio > 100 {
                return Err(CliError::BadParameter("durable-ratio must be between 0 and 100".into()));
            }
            
            let fiber_count = parse_string("fiber-count", matches)?
                .parse::<u8>()
                .map_err(|_| CliError::BadParameter("fiber-count must be a number".into()))?;
            
            if fiber_count == 0 || fiber_count > 50 {
                return Err(CliError::BadParameter("fiber-count must be between 1 and 50".into()));
            }
            
            Ok(CliCommand::ThreadStressTest {
                count: parse_string("count", matches)?
                    .parse::<u32>()
                    .map_err(|_| CliError::BadParameter("count must be a number".into()))?,
                interval: parse_string("interval", matches)?
                    .parse::<u64>()
                    .map_err(|_| CliError::BadParameter("interval must be a number".into()))?,
                jitter: parse_string("jitter", matches)?
                    .parse::<u64>()
                    .map_err(|_| CliError::BadParameter("jitter must be a number".into()))?,
                prefix: parse_string("prefix", matches)?,
                with_fibers: matches.get_flag("with-fibers"),
                batch_size: parse_string("batch-size", matches)?
                    .parse::<u32>()
                    .map_err(|_| CliError::BadParameter("batch-size must be a number".into()))?,
                durable_ratio,
                fiber_count,
            })
        }
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

