use anchor_lang::{
    solana_program::{instruction::Instruction, system_program}, AccountDeserialize, InstructionData, ToAccountMetas
};
use antegen_thread_program::state::{SerializableInstruction, Thread, VersionedThread, ThreadSettings, Trigger};
use antegen_utils::CrateInfo;
use solana_sdk::{native_token::LAMPORTS_PER_SOL, pubkey::Pubkey};
use crate::{client::Client, errors::CliError};

pub fn crate_info(client: &Client) -> Result<(), CliError> {
    let ix = Instruction {
        program_id: antegen_thread_program::ID,
        accounts: antegen_thread_program::accounts::GetCrateInfo {
            system_program: system_program::ID,
        }.to_account_metas(Some(false)),
        data: antegen_thread_program::instruction::GetCrateInfo {}.data(),
    };
    let crate_info: CrateInfo = client.get_return_data(ix).unwrap();
    println!("{:#?}", crate_info);
    Ok(())
}

pub fn create(
    client: &Client,
    id: String,
    instructions: Vec<SerializableInstruction>,
    trigger: Trigger,
) -> Result<(), CliError> {
    let thread_pubkey = Thread::pubkey(client.payer_pubkey(), id.clone().into_bytes());
    let ix = Instruction {
        program_id: antegen_thread_program::ID,
        accounts: antegen_thread_program::accounts::ThreadCreate {
            authority: client.payer_pubkey(),
            payer: client.payer_pubkey(),
            system_program: system_program::ID,
            thread: thread_pubkey
        }.to_account_metas(Some(false)),
        data: antegen_thread_program::instruction::ThreadCreate {
            amount: 0,
            id: id.into(),
            instructions,
            trigger,
        }
        .data(),
    };
    client.send_and_confirm(&[ix], &[client.payer()]).unwrap();
    get(client, thread_pubkey)?;
    Ok(())
}

pub fn memo_test(
    client: &Client,
    id: Option<String>,
    schedule: Option<String>,
    skippable: Option<bool>,
) -> Result<(), CliError> {
    let cluster_url = client.client.url();
    if !cluster_url.contains("localhost") && !cluster_url.contains("127.0.0.1") {
        return Err(CliError::FailedLocalnet(
            "This command is for testing on localnet (localhost)".to_string()
        ));
    }

    let thread_id = id.unwrap_or_else(|| "memo-test".to_string());
    let thread_pubkey = Thread::pubkey(client.payer_pubkey(), thread_id.clone().into_bytes());

    // Airdrop 1 SOL to the thread for rent
    client
        .airdrop(&thread_pubkey, LAMPORTS_PER_SOL)
        .map_err(|e| CliError::Custom(format!("airdrop to thread failed: {}", e)))?;
    println!("Airdropped 1 SOL to thread: {}", thread_pubkey);

    let memo_ix = Instruction {
        program_id: spl_memo::id(),
        data: "Hello, Thread!".as_bytes().to_vec(),
        accounts: vec![]
    };

    let instructions = vec![memo_ix.into()];
    let trigger = Trigger::Cron {
        schedule: schedule.unwrap_or_else(|| "*/10 * * * * * *".to_string()),
        skippable: skippable.unwrap_or_default(),
    };

    create(
        client,
        thread_id,
        instructions,
        trigger,
    )
}

pub fn delete(client: &Client, id: String) -> Result<(), CliError> {
    let thread_pubkey = Thread::pubkey(client.payer_pubkey(), id.into_bytes());
    let ix = Instruction {
        program_id: antegen_thread_program::ID,
        accounts: antegen_thread_program::accounts::ThreadDelete {
            authority: client.payer_pubkey(),
            close_to: client.payer_pubkey(),
            thread: thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_thread_program::instruction::ThreadDelete {}.data(),
    };
    client.send_and_confirm(&[ix], &[client.payer()]).unwrap();
    Ok(())
}

pub fn get(client: &Client, address: Pubkey) -> Result<(), CliError> {
    let data = client.get_account_data(&address).unwrap();
    let thread = VersionedThread::try_deserialize(&mut data.as_slice()).unwrap();
    println!("Address: {}\n{:#?}", address, thread);
    Ok(())
}

pub fn pause(client: &Client, id: String) -> Result<(), CliError> {
    let thread_pubkey = Thread::pubkey(client.payer_pubkey(), id.into_bytes());
    let ix = Instruction {
        program_id: antegen_thread_program::ID,
        accounts: antegen_thread_program::accounts::ThreadPause {
            authority: client.payer_pubkey(),
            thread: thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_thread_program::instruction::ThreadPause {}.data(),
    };
    client.send_and_confirm(&[ix], &[client.payer()]).unwrap();
    get(client, thread_pubkey)?;
    Ok(())
}

pub fn resume(client: &Client, id: String) -> Result<(), CliError> {
    let thread_pubkey = Thread::pubkey(client.payer_pubkey(), id.into_bytes());
    let ix = Instruction {
        program_id: antegen_thread_program::ID,
        accounts: antegen_thread_program::accounts::ThreadResume {
            authority: client.payer_pubkey(),
            thread: thread_pubkey
        }.to_account_metas(Some(false)),
        data: antegen_thread_program::instruction::ThreadResume {}.data(),
    };
    client.send_and_confirm(&[ix], &[client.payer()]).unwrap();
    get(client, thread_pubkey)?;
    Ok(())
}

pub fn reset(client: &Client, id: String) -> Result<(), CliError> {
    let thread_pubkey = Thread::pubkey(client.payer_pubkey(), id.into_bytes());
    let ix = Instruction {
        program_id: antegen_thread_program::ID,
        accounts: antegen_thread_program::accounts::ThreadReset {
            authority: client.payer_pubkey(),
            thread: thread_pubkey
        }.to_account_metas(Some(false)),
        data: antegen_thread_program::instruction::ThreadReset {}.data(),
    };
    client.send_and_confirm(&[ix], &[client.payer()]).unwrap();
    get(client, thread_pubkey)?;
    Ok(())
}

pub fn update(
    client: &Client,
    id: String,
    rate_limit: Option<u64>,
    schedule: Option<String>,
) -> Result<(), CliError> {
    let thread_pubkey = Thread::pubkey(client.payer_pubkey(), id.into_bytes());
    let trigger = if let Some(schedule) = schedule {
        Some(Trigger::Cron {
            schedule,
            skippable: true,
        })
    } else {
        None
    };
    let settings = ThreadSettings {
        fee: None,
        instructions: None,
        name: None,
        rate_limit,
        trigger,
    };
    let ix = Instruction {
        program_id: antegen_thread_program::ID,
        accounts: antegen_thread_program::accounts::ThreadUpdate {
            authority: client.payer_pubkey(),
            system_program: system_program::ID,
            thread: thread_pubkey
        }.to_account_metas(Some(false)),
        data: antegen_thread_program::instruction::ThreadUpdate { settings }.data(),
    };
    client.send_and_confirm(&[ix], &[client.payer()]).unwrap();
    get(client, thread_pubkey)?;
    Ok(())
}

pub fn parse_pubkey_from_id_or_address(
    authority: Pubkey,
    id: Option<String>,
    address: Option<Pubkey>,
) -> Result<Pubkey, CliError> {
    let address_from_id = id.map(|str| Thread::pubkey(authority, str));
    address.or(address_from_id).ok_or(CliError::InvalidAddress)
}
