use anchor_lang::{
    prelude::Pubkey,
    solana_program::{
        system_program,
        instruction::Instruction
    },
    InstructionData, ToAccountMetas
};
use antegen_network_program::{state::*, ANTEGEN_SQUADS};

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

    let network_fee_pubkey: Pubkey = ANTEGEN_SQUADS;
    let fee_data: Vec<u8> = client
        .get_account_data(&network_fee_pubkey)
        .map_err(|_err| CliError::AccountNotFound(network_fee_pubkey.to_string()))?;
    let fee_min_rent: u64 = client
        .get_minimum_balance_for_rent_exemption(fee_data.len())
        .unwrap();
    let fee_balance: u64 = client.get_balance(&network_fee_pubkey).unwrap();
    let registry_balance: u64 = fee_balance.saturating_sub(fee_min_rent);

    println!("{}\n{:#?}", registry_pubkey, registry);
    println!("Balance: {}\n", registry_balance);
    println!("{}\n{:#?}", snapshot_pubkey, snapshot);
    Ok(())
}

pub fn reset(client: &Client) -> Result<(), CliError> {
    let payer: Pubkey = client.payer_pubkey();
    let admin: Pubkey = if cfg!(feature = "mainnet") {
        ANTEGEN_SQUADS
    } else {
        payer
    };

    let registry: Pubkey = Registry::pubkey();
    let ix: Instruction = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::RegistryReset {
            payer: admin,
            admin,
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
