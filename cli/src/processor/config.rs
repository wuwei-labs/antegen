use anchor_lang::{
    solana_program::{
        instruction::Instruction, pubkey::Pubkey,
    }, InstructionData, ToAccountMetas
};
use antegen_network_program::state::{Config, ConfigSettings};

use crate::{client::Client, errors::CliError};

pub fn _get(client: &Client) -> Result<Config, CliError> {
    let config = client
        .get::<Config>(&Config::pubkey())
        .map_err(|_err| CliError::AccountNotFound(Config::pubkey().to_string()))?;
    Ok(config)
}

pub fn get(client: &Client) -> Result<(), CliError> {
    let config: Result<Config, CliError> = _get(client);
    println!("Address: {}", Config::pubkey());
    println!("{:#?}", config?);
    Ok(())
}

pub fn set(
    client: &Client,
    admin: Option<Pubkey>,
    epoch_thread: Option<Pubkey>,
    hasher_thread: Option<Pubkey>
) -> Result<(), CliError> {
    // Get the current config.
    let config = client
        .get::<Config>(&Config::pubkey())
        .map_err(|_err| CliError::AccountNotFound(Config::pubkey().to_string()))?;

    // Build new config settings
    let settings = ConfigSettings {
        admin: admin.unwrap_or(config.admin),
        epoch_thread: epoch_thread.unwrap_or(config.epoch_thread),
        hasher_thread: hasher_thread.unwrap_or(config.hasher_thread)
    };

    // Submit tx
    let ix_update = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::ConfigUpdate {
            admin: settings.admin,
            config: Config::pubkey(),
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::ConfigUpdate {
            settings: settings.clone()
        }.data(),
    };

    client.send_and_confirm(&[ix_update], &[client.payer()]).unwrap();
    get(client)?;
    Ok(())
}
