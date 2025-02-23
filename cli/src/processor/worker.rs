use {
    crate::{client::Client, errors::CliError}, anchor_lang::{
        solana_program::{instruction::Instruction, system_program, sysvar},
        AccountDeserialize, InstructionData, ToAccountMetas,
    }, anchor_spl::{associated_token, token}, antegen_network_program::state::{
        Config, Registry, Snapshot, SnapshotFrame, Worker, WorkerCommission, WorkerSettings,
    }, antegen_utils::explorer::Explorer, solana_sdk::{
        pubkey::Pubkey,
        signature::{Keypair, Signer},
    }
};

#[derive(Debug)]
pub struct WorkerInfo {
    pub worker: Worker,
    pub worker_pubkey: Pubkey,
    pub worker_commission_balance: u64,
    pub worker_commissions_pubkey: Pubkey,
    pub snapshot_frame: Option<SnapshotFrame>,
    pub explorer: Explorer,
}

impl WorkerInfo {
    pub fn print_status(&self) {
        println!(
            "Address: {}\nCommissions: {} lamports\nCommissions Account: {}\n{:#?}",
            self.explorer.account(&self.worker_pubkey),
            self.worker_commission_balance,
            self.explorer.account(&self.worker_commissions_pubkey),
            self.worker
        );

        if let Some(frame) = &self.snapshot_frame {
            println!("{:#?}", frame);
        }
    }
}

pub fn _get(client: &Client, id: u64) -> Result<WorkerInfo, CliError> {
    let worker_pubkey = Worker::pubkey(id);
    let worker = client
        .get::<Worker>(&worker_pubkey)
        .map_err(|_err| CliError::AccountDataNotParsable(worker_pubkey.to_string()))?;
    let worker_commissions_pubkey = WorkerCommission::pubkey(worker_pubkey);
    
    // Get commission account data and calculate available balance
    let commission_data = client
        .get_account_data(&worker_commissions_pubkey)
        .map_err(|_err| CliError::AccountNotFound(worker_commissions_pubkey.to_string()))?;
    let commission_min_rent = client
        .get_minimum_balance_for_rent_exemption(commission_data.len())
        .unwrap();
    let commission_balance = client.get_balance(&worker_commissions_pubkey).unwrap();
    let worker_commission_balance = commission_balance.saturating_sub(commission_min_rent);

    // Get registry
    let registry_pubkey = Registry::pubkey();
    let registry_data = client
        .get_account_data(&registry_pubkey)
        .map_err(|_err| CliError::AccountNotFound(registry_pubkey.to_string()))?;
    let registry = Registry::try_deserialize(&mut registry_data.as_slice())
        .map_err(|_err| CliError::AccountDataNotParsable(registry_pubkey.to_string()))?;

    // Get snapshot frame
    let snapshot_pubkey = Snapshot::pubkey(registry.current_epoch);
    let snapshot_frame_pubkey = SnapshotFrame::pubkey(snapshot_pubkey, worker.id);
    let snapshot_frame = match client.get_account_data(&snapshot_frame_pubkey) {
        Ok(snapshot_frame_data) => {
            Some(SnapshotFrame::try_deserialize(
                &mut snapshot_frame_data.as_slice(),
            ).map_err(|_err| CliError::AccountDataNotParsable(registry_pubkey.to_string()))?)
        }
        Err(_) => None,
    };

    let worker_info = WorkerInfo {
        worker,
        worker_pubkey,
        worker_commission_balance,
        worker_commissions_pubkey,
        snapshot_frame,
        explorer: Explorer::from(client.client.url().clone()),  // Add this
    };

    Ok(worker_info)
}

pub fn get(client: &Client, id: u64) -> Result<(), CliError> {
    let worker_info = _get(client, id);
    worker_info?.print_status();
    Ok(())
}

pub fn create(client: &Client, signatory: Keypair, silent: bool) -> Result<(), CliError> {
    // Get registry
    let registry_pubkey = Registry::pubkey();
    let registry_data = client
        .get_account_data(&registry_pubkey)
        .map_err(|_err| CliError::AccountNotFound(registry_pubkey.to_string()))?;
    let registry = Registry::try_deserialize(&mut registry_data.as_slice())
        .map_err(|_err| CliError::AccountDataNotParsable(registry_pubkey.to_string()))?;

    let worker_id = registry.total_workers;
    let worker_pubkey = Worker::pubkey(worker_id);
    let ix = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::WorkerCreate {
            associated_token_program: associated_token::ID,
            authority: client.payer_pubkey(),
            config: Config::pubkey(),
            commission: WorkerCommission::pubkey(worker_pubkey),
            registry: Registry::pubkey(),
            rent: sysvar::rent::ID,
            signatory: signatory.pubkey(),
            system_program: system_program::ID,
            token_program: token::ID,
            worker: worker_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::WorkerCreate {}.data(),
    };
    client
        .send_and_confirm(&[ix], &[client.payer(), &signatory])
        .unwrap();

    if !silent {
        get(client, worker_id)?;
    }
    Ok(())
}

pub fn update(client: &Client, id: u64, commission_rate: Option<u64>, signatory: Option<Keypair>) -> Result<(), CliError> {
    // Derive worker keypair.
    let worker_pubkey = Worker::pubkey(id);
    let worker = client
        .get::<Worker>(&worker_pubkey)
        .map_err(|_err| CliError::AccountDataNotParsable(worker_pubkey.to_string()))?;

    // Build and submit tx.
    let settings = WorkerSettings {
        commission_rate: commission_rate.unwrap_or(worker.commission_rate),
        signatory: signatory.map_or(worker.signatory, |v| v.pubkey()),
    };
    let ix = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::WorkerUpdate {
            authority: client.payer_pubkey(),
            system_program: system_program::ID,
            worker: worker_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::WorkerUpdate { settings }.data(),
    };
    client.send_and_confirm(&[ix], &[client.payer()]).unwrap();
    get(client, worker.id)?;
    Ok(())
}
