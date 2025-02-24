use anchor_lang::{
    solana_program::{
        hash::Hash,
        instruction::Instruction,
        pubkey::Pubkey,
    },
    InstructionData, ToAccountMetas
};
use antegen_network_program::state::{Config, ConfigSettings};
use solana_sdk::{message::Message, transaction::Transaction};

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
    hasher_thread: Option<Pubkey>,
    output_format: Option<String>,
) -> Result<(), CliError> {
    // Get the current config.
    let config = client
        .get::<Config>(&Config::pubkey())
        .map_err(|_err| CliError::AccountNotFound(Config::pubkey().to_string()))?;

    // Build new config settings
    let settings: ConfigSettings = ConfigSettings {
        admin: admin.unwrap_or(config.admin),
        epoch_thread: epoch_thread.unwrap_or(config.epoch_thread),
        hasher_thread: hasher_thread.unwrap_or(config.hasher_thread)
    };

    // Create instruction
    let ix: Instruction = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::ConfigUpdate {
            admin: settings.admin,
            config: Config::pubkey(),
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::ConfigUpdate {
            settings: settings.clone()
        }.data(),
    };
    
    // Check if base58 output is requested
    if let Some(format) = output_format {
        if format == "base58" {
            // Create unsigned transaction
            let blockhash: Hash = client.get_latest_blockhash().unwrap();
            let message: Message = Message::new(&[ix], Some(&settings.admin));
            let mut tx: Transaction = Transaction::new_unsigned(message);
            tx.message.recent_blockhash = blockhash;
            
            // Serialize and base58 encode the transaction
            let serialized_tx: Vec<u8> = bincode::serialize(&tx).unwrap();
            let base58_tx = bs58::encode(serialized_tx).into_string();
            
            // Print the base58 encoded transaction
            println!("{}", base58_tx);
            return Ok(());
        }
    }

    // Default behavior: submit tx
    client.send_and_confirm(&[ix], &[client.payer()]).unwrap();
    get(client)?;
    Ok(())
}
