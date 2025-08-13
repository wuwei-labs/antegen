use {
    crate::{client::Client, errors::CliError},
    anchor_lang::{
        prelude::Pubkey,
        solana_program::{instruction::Instruction, system_program},
        InstructionData, ToAccountMetas,
    },
    antegen_network_program::state::Registry,
};

pub fn initialize(client: &Client) -> Result<(), CliError> {
    let payer: Pubkey = client.payer_pubkey();
    let registry: Pubkey = Registry::pubkey();
    let ix: Instruction = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::Initialize {
            payer,
            registry,
            system_program: system_program::ID,
        }
        .to_account_metas(Some(false)),
        data: antegen_network_program::instruction::Initialize {}.data(),
    };

    // Submit tx
    client
        .send_and_confirm(&[ix], &[client.payer()])
        .unwrap();
    Ok(())
}

// Test thread creation has been moved to test suite
