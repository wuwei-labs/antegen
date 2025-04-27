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
pub struct WorkerInfo {
    pub worker: Builder,
    pub pubkey: Pubkey,
    pub explorer: Explorer,
}

impl WorkerInfo {
    pub fn print_status(&self) {
        println!(
            "Address: {}\n{:#?}",
            self.explorer.account(&self.pubkey),
            self.worker
        );
    }
}

pub fn _get(client: &Client, id: u32) -> Result<WorkerInfo, CliError> {
    let worker_pubkey = Builder::pubkey(id);
    let worker = client
        .get::<Builder>(&worker_pubkey)
        .map_err(|_err| CliError::AccountDataNotParsable(worker_pubkey.to_string()))?;

    let worker_info = WorkerInfo {
        worker,
        pubkey: worker_pubkey,
        explorer: Explorer::from(client.client.url().clone()),
    };

    Ok(worker_info)
}

pub fn get(client: &Client, id: u32) -> Result<(), CliError> {
    let worker_info: Result<WorkerInfo, CliError> = _get(client, id);
    worker_info?.print_status();
    Ok(())
}

pub fn create(client: &Client, signatory: Keypair, silent: bool) -> Result<(), CliError> {
    let registry_pubkey: Pubkey = Registry::pubkey();
    let registry_data: Vec<u8> = client
        .get_account_data(&registry_pubkey)
        .map_err(|_err| CliError::AccountNotFound(registry_pubkey.to_string()))?;
    let registry = Registry::try_deserialize(&mut registry_data.as_slice())
        .map_err(|_err| CliError::AccountDataNotParsable(registry_pubkey.to_string()))?;

    let worker_id: u32 = registry.total_builders + 1;
    let worker_pubkey: Pubkey = Builder::pubkey(worker_id);
    let ix: Instruction = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::BuilderCreate {
            authority: client.payer_pubkey(),
            signatory: signatory.pubkey(),
            builder: worker_pubkey,
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
        get(client, worker_id)?;
    }
    Ok(())
}

pub fn update(
    client: &Client,
    id: u32,
    commission_rate: Option<u64>,
    signatory: Option<Keypair>,
) -> Result<(), CliError> {
    let worker_pubkey: Pubkey = Builder::pubkey(id);
    let worker: Builder = client
        .get::<Builder>(&worker_pubkey)
        .map_err(|_err| CliError::AccountDataNotParsable(worker_pubkey.to_string()))?;

    // Build and submit tx.
    let settings: BuilderSettings = BuilderSettings {
        commission_rate: commission_rate.unwrap_or(worker.commission_rate),
        signatory: signatory.map_or(worker.signatory, |v| v.pubkey()),
    };
    let ix: Instruction = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::BuilderUpdate {
            authority: client.payer_pubkey(),
            builder: worker_pubkey,
        }
        .to_account_metas(Some(false)),
        data: antegen_network_program::instruction::BuilderUpdate { settings }.data(),
    };
    client.send_and_confirm(&[ix], &[client.payer()]).unwrap();
    get(client, worker.id)?;
    Ok(())
}
