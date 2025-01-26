use anchor_lang::{
    solana_program::{
        instruction::Instruction,
        system_program,
    },
    InstructionData, ToAccountMetas,
};
use antegen_network_program::{state::{Config, Pool, Registry, RegistryFee, Snapshot}, ANTEGEN_SQUADS};
use crate::{client::Client, errors::CliError};

pub fn initialize(client: &Client) -> Result<(), CliError> {
    // Initialize the programs
    let payer = client.payer_pubkey();
    let admin = if cfg!(feature = "mainnet") {
        ANTEGEN_SQUADS
    } else {
        payer
    };

    let registry = Registry::pubkey();
    let ix_a = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::Initialize {
            payer: admin,
            admin,
            config: Config::pubkey(),
            registry,
            registry_fee: RegistryFee::pubkey(registry),
            snapshot: Snapshot::pubkey(0),
            system_program: system_program::ID,
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::Initialize {}.data(),
    };
    let ix_b = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::PoolCreate {
            payer: admin,
            admin,
            config: Config::pubkey(),
            pool: Pool::pubkey(0),
            registry: Registry::pubkey(),
            system_program: system_program::ID,
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::PoolCreate {}.data(),
    };

    // Submit tx
    client
        .send_and_confirm(&[ix_a, ix_b], &[client.payer()])
        .unwrap();
    Ok(())
}
