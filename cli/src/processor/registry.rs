use anchor_lang::prelude::Pubkey;
use antegen_network_program::{state::*, ANTEGEN_SQUADS};

use crate::{client::Client, errors::CliError};

pub fn get(client: &Client) -> Result<(), CliError> {
    let registry_pubkey: Pubkey = antegen_network_program::state::Registry::pubkey();
    let registry: Registry = client
        .get::<Registry>(&registry_pubkey)
        .map_err(|_err| CliError::AccountDataNotParsable(registry_pubkey.to_string()))?;

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
    Ok(())
}


