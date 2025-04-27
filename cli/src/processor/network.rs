use {
    crate::{client::Client, errors::CliError, print::print_style, print_status},
    anchor_lang::{
        prelude::Pubkey,
        solana_program::{instruction::Instruction, system_program},
        InstructionData, ToAccountMetas,
    },
    antegen_network_program::state::{Config, Pool, Registry},
    antegen_thread_program::state::Thread,
    antegen_utils::{explorer::Explorer, thread::Trigger},
    solana_sdk::{
        signature::Keypair,
        signer::Signer,
        sysvar::{recent_blockhashes, rent},
    },
};

pub fn initialize(client: &Client) -> Result<(), CliError> {
    let payer: Pubkey = client.payer_pubkey();
    let registry: Pubkey = Registry::pubkey();
    let ix_a: Instruction = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::Initialize {
            payer,
            config: Config::pubkey(),
            registry,
            system_program: system_program::ID,
        }
        .to_account_metas(Some(false)),
        data: antegen_network_program::instruction::Initialize {}.data(),
    };
    let ix_b: Instruction = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::PoolCreate {
            payer,
            pool: Pool::pubkey(0),
            registry: Registry::pubkey(),
            system_program: system_program::ID,
        }
        .to_account_metas(Some(false)),
        data: antegen_network_program::instruction::PoolCreate {}.data(),
    };

    // Submit tx
    client
        .send_and_confirm(&[ix_a, ix_b], &[client.payer()])
        .unwrap();
    Ok(())
}

pub fn create_threads(client: &Client, amount: u64) -> Result<(), CliError> {
    let cron_schedule: &str = "*/15 * * * * * *";
    let explorer: Explorer = Explorer::from(client.client.url());
    let payer: Pubkey = client.payer_pubkey();
    let thread_pubkey: Pubkey = Thread::pubkey(payer, "memo-test".to_string());
    let nonce_keypair: Keypair = Keypair::new();

    let memo_ix = Instruction {
        program_id: spl_memo::id(),
        data: "Hello, Thread!".as_bytes().to_vec(),
        accounts: vec![],
    };

    let ix: Instruction = Instruction {
        program_id: antegen_thread_program::ID,
        accounts: antegen_thread_program::accounts::ThreadCreate {
            authority: payer,
            payer,
            thread: thread_pubkey,
            nonce_account: nonce_keypair.pubkey(),
            recent_blockhashes: recent_blockhashes::ID,
            rent: rent::ID,
            system_program: system_program::ID,
        }
        .to_account_metas(Some(false)),
        data: antegen_thread_program::instruction::ThreadCreate {
            amount,
            id: "memo-test".into(),
            instructions: vec![memo_ix.into()],
            trigger: Trigger::Cron {
                schedule: cron_schedule.into(),
                skippable: true,
            },
        }
        .data(),
    };

    client
        .send_and_confirm(&vec![ix], &[client.payer(), &nonce_keypair])
        .map_err(|err| {
            eprintln!("Transaction error details: {:?}", err);
            anyhow::anyhow!("Failed to create thread: memo-test: {}", err)
        })?;

    print_status!(
        "Memo Test  🧵",
        "{}",
        explorer.account(thread_pubkey.to_string())
    );
    Ok(())
}
