use {
    crate::{
        client::Client,
        errors::CliError,
        print::print_style,
        print_status
    }, anchor_lang::{
        prelude::Pubkey,
        solana_program::{
            instruction::Instruction,
            system_program,
        },
        InstructionData,
        ToAccountMetas,
    }, antegen_network_program::{
        state::{
            Config,
            Pool,
            Registry,
            Snapshot
        },
        ANTEGEN_SQUADS, EPOCH_THREAD_ID, HASHER_THREAD_ID
    },
    antegen_thread_program::state::{Thread, Trigger},
    antegen_utils::explorer::Explorer,
    anyhow::Context
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
            snapshot: Snapshot::pubkey(0),
            system_program: system_program::ID,
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::Initialize {}.data(),
    };
    let ix_b: Instruction = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::PoolCreate {
            payer,
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

pub fn create_threads(client: &Client, amount: u64) -> Result<(), CliError> {
    #[cfg(feature = "mainnet")]
    let cron_epoch: &str = "@hourly";

    #[cfg(not(feature = "mainnet"))]
    let cron_epoch: &str = "0 * * * * * *";

    #[cfg(feature = "mainnet")]
    let cron_hasher: &str = "0 */15 * * * * *";

    #[cfg(not(feature = "mainnet"))]
    let cron_hasher: &str = "*/15 * * * * * *";

    let explorer: Explorer = Explorer::from(client.client.url());
    let payer: Pubkey = client.payer_pubkey();
    let admin: Pubkey = if cfg!(feature = "mainnet") {
        ANTEGEN_SQUADS
    } else {
        payer
    };

    // Create epoch thread.
    let epoch_thread_pubkey: Pubkey = Thread::pubkey(admin, EPOCH_THREAD_ID);
    let ix_a1: Instruction = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::DistributeFeesJob {
            config: Config::pubkey(),
            registry: Registry::pubkey(),
            thread: epoch_thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::DistributeFeesJob {}.data(),
    };

    let ix_a2: Instruction = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::TakeSnapshotJob {
            config: Config::pubkey(),
            registry: Registry::pubkey(),
            thread: epoch_thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::TakeSnapshotJob {}.data(),
    };

    let ix_a3: Instruction = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::EpochCutover {
            config: Config::pubkey(),
            registry: Registry::pubkey(),
            thread: epoch_thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::IncrementEpoch {}.data(),
    };

    let ix_a4: Instruction = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::DeleteSnapshotJob {
            config: Config::pubkey(),
            registry: Registry::pubkey(),
            thread: epoch_thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::DeleteSnapshotJob {}.data(),
    };

    let ix_a: Instruction = Instruction {
        program_id: antegen_thread_program::ID,
        accounts: antegen_thread_program::accounts::ThreadCreate {
            authority: admin,
            payer,
            system_program: system_program::ID,
            thread: epoch_thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_thread_program::instruction::ThreadCreate {
            amount,
            id: EPOCH_THREAD_ID.into(),
            instructions: vec![
                ix_a1.into(),
                ix_a2.into(),
                ix_a3.into(),
                ix_a4.into(),
            ],
            trigger: Trigger::Cron {
                schedule: cron_epoch.into(),
                skippable: true,
            },
        }.data(),
    };

    // Create hasher thread.
    let hasher_thread_pubkey: Pubkey = Thread::pubkey(admin, HASHER_THREAD_ID);
    let ix_b1: Instruction = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::RegistryNonceHash {
            config: Config::pubkey(),
            registry: Registry::pubkey(),
            thread: hasher_thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::RegistryNonceHash {}.data(),
    };

    let ix_b: Instruction = Instruction {
        program_id: antegen_thread_program::ID,
        accounts: antegen_thread_program::accounts::ThreadCreate {
            authority: admin,
            payer,
            system_program: system_program::ID,
            thread: hasher_thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_thread_program::instruction::ThreadCreate {
            amount,
            id: HASHER_THREAD_ID.into(),
            instructions: vec![
                ix_b1.into(),
            ],
            trigger: Trigger::Cron {
                schedule: cron_hasher.into(),
                skippable: true,
            },
        }.data(),
    };

    client
        .send_and_confirm(&vec![ix_a], &[client.payer()])
        .context(format!(
            "Failed to create thread: {} or update config",
            EPOCH_THREAD_ID,
        ))?;
    client
        .send_and_confirm(&vec![ix_b], &[client.payer()])
        .context(format!("Failed to create thread: {}", HASHER_THREAD_ID))?;

    print_status!("Epoch    🧵", "{}", explorer.account(epoch_thread_pubkey.to_string()));
    print_status!("Hasher   🧵", "{}", explorer.account(hasher_thread_pubkey.to_string()));
    print_status!("Admin    👔", "{}", explorer.account(admin.to_string()));
    Ok(())
}
