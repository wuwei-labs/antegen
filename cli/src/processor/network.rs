use {
    crate::{
        client::Client,
        errors::CliError,
        print::print_style,
        print_status
    }, anchor_lang::{
        solana_program::{
            instruction::Instruction,
            system_program,
        },
        InstructionData,
        ToAccountMetas,
    }, antegen_network_program::{
        state::{
            Config,
            ConfigSettings,
            Pool,
            Registry,
            Snapshot
        },
        ANTEGEN_SQUADS
    },
    antegen_thread_program::state::{Thread, Trigger},
    antegen_utils::explorer::Explorer, anyhow::Context, 
    solana_sdk::{native_token::LAMPORTS_PER_SOL, system_instruction}
};

pub fn initialize(client: &Client) -> Result<(), CliError> {
    let payer = client.payer_pubkey();

    let mut ixs: Vec<Instruction> = vec![];
    let ix_a = system_instruction::transfer(
        &payer,
        &ANTEGEN_SQUADS,
        LAMPORTS_PER_SOL.saturating_div(10),
    );
    ixs.push(ix_a);

    let registry = Registry::pubkey();
    let ix_b = Instruction {
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
    let ix_c = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::PoolCreate {
            payer,
            pool: Pool::pubkey(0),
            registry: Registry::pubkey(),
            system_program: system_program::ID,
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::PoolCreate {}.data(),
    };
    ixs.extend([ix_b, ix_c]);

    // Submit tx
    client
        .send_and_confirm(&ixs, &[client.payer()])
        .unwrap();
    Ok(())
}

pub fn create_threads(client: &Client, amount: u64) -> Result<(), CliError> {
    #[cfg(feature = "mainnet")]
    let cron_epoch = "@hourly";

    #[cfg(not(feature = "mainnet"))]
    let cron_epoch = "0 * * * * * *";

    #[cfg(feature = "mainnet")]
    let cron_hasher = "0 */15 * * * * *";

    #[cfg(not(feature = "mainnet"))]
    let cron_hasher = "*/15 * * * * * *";

    let explorer = Explorer::from(client.client.url());
    let payer = client.payer_pubkey();

    // Create epoch thread.
    let epoch_thread_id = "antegen.network.epoch";
    let epoch_thread_pubkey = Thread::pubkey(client.payer_pubkey(), epoch_thread_id);
    let ix_a1 = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::DistributeFeesJob {
            config: Config::pubkey(),
            registry: Registry::pubkey(),
            thread: epoch_thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::DistributeFeesJob {}.data(),
    };
    let ix_a2 = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::TakeSnapshotJob {
            config: Config::pubkey(),
            registry: Registry::pubkey(),
            thread: epoch_thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::TakeSnapshotJob {}.data(),
    };
    let ix_a3 = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::EpochCutover {
            config: Config::pubkey(),
            registry: Registry::pubkey(),
            thread: epoch_thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::IncrementEpoch {}.data(),
    };
    let ix_a4 = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::DeleteSnapshotJob {
            config: Config::pubkey(),
            registry: Registry::pubkey(),
            thread: epoch_thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::DeleteSnapshotJob {}.data(),
    };
    let ix_a = Instruction {
        program_id: antegen_thread_program::ID,
        accounts: antegen_thread_program::accounts::ThreadCreate {
            authority: payer,
            payer,
            system_program: system_program::ID,
            thread: epoch_thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_thread_program::instruction::ThreadCreate {
            amount,
            id: epoch_thread_id.into(),
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
    let hasher_thread_id = "antegen.network.hasher";
    let hasher_thread_pubkey = Thread::pubkey(client.payer_pubkey(), hasher_thread_id);
    let registry_hash_ix = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::RegistryNonceHash {
            config: Config::pubkey(),
            registry: Registry::pubkey(),
            thread: hasher_thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::RegistryNonceHash {}.data(),
    };
    let ix_b = Instruction {
        program_id: antegen_thread_program::ID,
        accounts: antegen_thread_program::accounts::ThreadCreate {
            authority: payer,
            payer,
            system_program: system_program::ID,
            thread: hasher_thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_thread_program::instruction::ThreadCreate {
            amount,
            id: hasher_thread_id.into(),
            instructions: vec![
                registry_hash_ix.into(),
            ],
            trigger: Trigger::Cron {
                schedule: cron_hasher.into(),
                skippable: true,
            },
        }.data(),
    };

    // Update config with thread pubkeys
    let settings = ConfigSettings {
        admin: payer,
        epoch_thread: epoch_thread_pubkey,
        hasher_thread: hasher_thread_pubkey
    };
    let ix_c = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::ConfigUpdate {
            admin: payer,
            config: Config::pubkey()
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::ConfigUpdate { settings }.data(),
    };

    client
        .send_and_confirm(&vec![ix_a], &[client.payer()])
        .context(format!(
            "Failed to create thread: {} or update config",
            epoch_thread_id,
        ))?;
    client
        .send_and_confirm(&vec![ix_b, ix_c], &[client.payer()])
        .context(format!("Failed to create thread: {}", hasher_thread_id))?;

    let config = super::config::_get(client)?;
    print_status!("Epoch", "🧵", "{}", explorer.account(config.clone().epoch_thread));
    print_status!("Hasher", "🧵", "{}", explorer.account(config.clone().hasher_thread));
    print_status!("Admin", "👔", "{}", explorer.account(config.clone().admin));
    Ok(())
}
