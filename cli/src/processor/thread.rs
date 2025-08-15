use crate::{client::Client, errors::CliError};
use anchor_lang::{
    solana_program::{instruction::Instruction, system_program},
    InstructionData, ToAccountMetas,
};
use antegen_thread_program::state::{Thread, Trigger};
use antegen_utils::thread::SerializableInstruction;
use solana_sdk::{
    native_token::LAMPORTS_PER_SOL,
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    sysvar::{recent_blockhashes, rent},
};

pub fn create(
    client: &Client,
    id: String,
    trigger: Trigger,
    initial_instruction: Option<SerializableInstruction>,
) -> Result<(), CliError> {
    let thread_pubkey = Thread::pubkey(client.payer_pubkey(), id.clone().into_bytes());
    let nonce_keypair = Keypair::new();
    
    // Calculate fiber PDA if instruction provided
    let fiber_pubkey = if initial_instruction.is_some() {
        Some(Pubkey::find_program_address(
            &[b"thread_fiber", thread_pubkey.as_ref(), &[0u8]],
            &antegen_thread_program::ID,
        ).0)
    } else {
        None
    };

    let ix = Instruction {
        program_id: antegen_thread_program::ID,
        accounts: antegen_thread_program::accounts::ThreadCreate {
            authority: client.payer_pubkey(),
            payer: client.payer_pubkey(),
            thread: thread_pubkey,
            fiber: fiber_pubkey,
            nonce_account: Some(nonce_keypair.pubkey()),
            recent_blockhashes: Some(recent_blockhashes::ID),
            rent: Some(rent::ID),
            system_program: system_program::ID,
        }
        .to_account_metas(Some(false)),
        data: antegen_thread_program::instruction::ThreadCreate {
            amount: LAMPORTS_PER_SOL,
            id: id.into(),
            trigger,
            initial_instruction,
        }
        .data(),
    };
    client
        .send_and_confirm(&[ix], &[client.payer(), &nonce_keypair])
        .unwrap();
    get(client, thread_pubkey)?;
    Ok(())
}

pub fn delete(client: &Client, address: Pubkey) -> Result<(), CliError> {
    let ix = Instruction {
        program_id: antegen_thread_program::ID,
        accounts: antegen_thread_program::accounts::ThreadDelete {
            authority: client.payer_pubkey(),
            close_to: client.payer_pubkey(),
            thread: address,
        }
        .to_account_metas(Some(false)),
        data: antegen_thread_program::instruction::ThreadDelete {}.data(),
    };
    client.send_and_confirm(&[ix], &[client.payer()]).unwrap();
    Ok(())
}

pub fn get(client: &Client, address: Pubkey) -> Result<(), CliError> {
    let data = client.get_account_data(&address).unwrap();
    let thread = Thread::try_from(data).unwrap();
    println!("Address: {}\n{:#?}", address, thread);
    Ok(())
}

pub fn toggle(client: &Client, id: String) -> Result<(), CliError> {
    let thread_pubkey = Thread::pubkey(client.payer_pubkey(), id.into_bytes());
    let ix = Instruction {
        program_id: antegen_thread_program::ID,
        accounts: antegen_thread_program::accounts::ThreadToggle {
            authority: client.payer_pubkey(),
            thread: thread_pubkey,
        }
        .to_account_metas(Some(false)),
        data: antegen_thread_program::instruction::ThreadToggle {}.data(),
    };
    client.send_and_confirm(&[ix], &[client.payer()]).unwrap();
    get(client, thread_pubkey)?;
    Ok(())
}

pub fn update(
    client: &Client,
    id: String,
    schedule: Option<String>,
) -> Result<(), CliError> {
    let thread_pubkey = Thread::pubkey(client.payer_pubkey(), id.into_bytes());
    let new_trigger = if let Some(schedule) = schedule {
        Some(Trigger::Cron {
            schedule,
            skippable: true,
        })
    } else {
        None
    };
    let ix = Instruction {
        program_id: antegen_thread_program::ID,
        accounts: antegen_thread_program::accounts::ThreadUpdate {
            authority: client.payer_pubkey(),
            thread: thread_pubkey,
        }
        .to_account_metas(Some(false)),
        data: antegen_thread_program::instruction::ThreadUpdate { new_trigger }.data(),
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
