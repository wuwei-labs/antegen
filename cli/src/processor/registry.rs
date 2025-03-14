use anchor_lang::{
    prelude::Pubkey,
    solana_program::{
        system_program,
        instruction::Instruction
    },
    InstructionData, ToAccountMetas
};
use antegen_network_program::state::*;

use crate::{client::Client, errors::CliError};

pub fn get(client: &Client) -> Result<(), CliError> {
    let registry_pubkey: Pubkey = antegen_network_program::state::Registry::pubkey();
    let registry: Registry = client
        .get::<Registry>(&registry_pubkey)
        .map_err(|_err| CliError::AccountDataNotParsable(registry_pubkey.to_string()))?;

    let snapshot_pubkey: Pubkey = Snapshot::pubkey(registry.current_epoch);
    let snapshot: Snapshot = client
        .get::<Snapshot>(&snapshot_pubkey)
        .map_err(|_err| CliError::AccountDataNotParsable(snapshot_pubkey.to_string()))?;

    println!("{}\n{:#?}", registry_pubkey, registry);
    println!("{}\n{:#?}", snapshot_pubkey, snapshot);
    Ok(())
}

pub fn reset(client: &Client) -> Result<(), CliError> {
    let payer: Pubkey = client.payer_pubkey();

    let registry: Pubkey = Registry::pubkey();
    let ix: Instruction = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::RegistryReset {
            admin: payer,
            config: Config::pubkey(),
            registry,
            snapshot: Snapshot::pubkey(0),
            system_program: system_program::ID,
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::RegistryReset {}.data(),
    };

    client.send_and_confirm(&[ix], &[client.payer()]).unwrap();
    get(client)?;
    Ok(())
}

pub fn unlock(client: &Client) -> Result<(), CliError> {
    let ix: Instruction = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::RegistryUnlock {
            admin: client.payer_pubkey(),
            config: Config::pubkey(),
            registry: Registry::pubkey()
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::RegistryUnlock {}.data(),
    };
    client.send_and_confirm(&[ix], &[client.payer()]).unwrap();
    get(client)?;
    Ok(())
}
