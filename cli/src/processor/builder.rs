use {
    crate::{client::Client, errors::CliError},
    anchor_lang::{
        solana_program::{instruction::Instruction, system_program},
        AccountDeserialize, InstructionData, ToAccountMetas,
    },
    antegen_network_program::state::{Builder, BuilderSettings, Registry},
    antegen_utils::explorer::Explorer,
    solana_sdk::{
        pubkey::Pubkey,
        signature::{Keypair, Signer},
    },
};

#[derive(Debug)]
pub struct BuilderInfo {
    pub builder: Builder,
    pub pubkey: Pubkey,
    pub explorer: Explorer,
}

impl BuilderInfo {
    pub fn print_status(&self) {
        println!(
            "Address: {}\nStatus: {}\n{:#?}",
            self.explorer.account(&self.pubkey),
            "Active",
            self.builder
        );
    }
}

pub fn _get(client: &Client, id: u32) -> Result<BuilderInfo, CliError> {
    let builder_pubkey = Builder::pubkey(id);
    let builder = client
        .get::<Builder>(&builder_pubkey)
        .map_err(|_err| CliError::AccountDataNotParsable(builder_pubkey.to_string()))?;

    let builder_info = BuilderInfo {
        builder,
        pubkey: builder_pubkey,
        explorer: Explorer::from(client.client.url().clone()),
    };

    Ok(builder_info)
}

pub fn get(client: &Client, id: u32) -> Result<(), CliError> {
    let builder_info: Result<BuilderInfo, CliError> = _get(client, id);
    builder_info?.print_status();
    Ok(())
}

pub fn create(client: &Client, signatory: Keypair, silent: bool) -> Result<(), CliError> {
    let registry_pubkey: Pubkey = Registry::pubkey();
    let registry_data: Vec<u8> = client
        .get_account_data(&registry_pubkey)
        .map_err(|_err| CliError::AccountNotFound(registry_pubkey.to_string()))?;
    let registry = Registry::try_deserialize(&mut registry_data.as_slice())
        .map_err(|_err| CliError::AccountDataNotParsable(registry_pubkey.to_string()))?;

    let builder_id: u32 = registry.total_builders + 1;
    let builder_pubkey: Pubkey = Builder::pubkey(builder_id);
    let ix: Instruction = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::BuilderCreate {
            authority: client.payer_pubkey(),
            signatory: signatory.pubkey(),
            builder: builder_pubkey,
            registry: Registry::pubkey(),
            system_program: system_program::ID,
        }
        .to_account_metas(Some(false)),
        data: antegen_network_program::instruction::BuilderCreate {}.data(),
    };
    client
        .send_and_confirm(&[ix], &[client.payer(), &signatory])
        .unwrap();

    if !silent {
        get(client, builder_id)?;
    }
    Ok(())
}

pub fn update(
    client: &Client,
    id: u32,
    commission_bps: Option<u64>,
    signatory: Option<Keypair>,
) -> Result<(), CliError> {
    let builder_pubkey: Pubkey = Builder::pubkey(id);
    let builder: Builder = client
        .get::<Builder>(&builder_pubkey)
        .map_err(|_err| CliError::AccountDataNotParsable(builder_pubkey.to_string()))?;

    // Build and submit tx.
    let settings: BuilderSettings = BuilderSettings {
        commission_bps: commission_bps.unwrap_or(builder.commission_bps),
        signatory: signatory.map_or(builder.signatory, |v| v.pubkey()),
    };
    let ix: Instruction = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::BuilderUpdate {
            authority: client.payer_pubkey(),
            builder: builder_pubkey,
            registry: Registry::pubkey(),
        }
        .to_account_metas(Some(false)),
        data: antegen_network_program::instruction::BuilderUpdate { settings }.data(),
    };
    client.send_and_confirm(&[ix], &[client.payer()]).unwrap();
    get(client, builder.id)?;
    Ok(())
}

// Builder activation removed - builders are always active if they exist

// Builder deactivation removed - builders are always active if they exist
