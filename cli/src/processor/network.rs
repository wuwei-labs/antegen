use {
    crate::{
        client::Client,
        errors::CliError,
        print::print_style,
        print_status
    },
    anchor_lang::{
        solana_program::{
            instruction::Instruction,
            system_program,
        },
        InstructionData,
        ToAccountMetas,
    },
    antegen_thread_program::state::{Thread, Trigger},
    antegen_network_program::{
        state::{
            Config,
            ConfigSettings,
            Pool,
            Registry,
            RegistryFee,
            Snapshot
        },
        ANTEGEN_SQUADS
    },
    antegen_utils::explorer::Explorer,
    anyhow::Context,
};

pub fn initialize(client: &Client) -> Result<(), CliError> {
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

pub fn create_threads(client: &Client, amount: u64) -> Result<(), CliError> {
    let explorer = Explorer::from(client.client.url());
    let payer = client.payer_pubkey();
    let admin = if cfg!(feature = "mainnet") {
        ANTEGEN_SQUADS
    } else {
        payer
    };

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
            authority: admin,
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
                schedule: "0 * * * * * *".into(),
                skippable: true,
            },
        }
        .data(),
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
            authority: admin,
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
                schedule: "*/15 * * * * * *".into(),
                skippable: true,
            },
        }
        .data(),
    };

    // Update config with thread pubkeys
    let settings = ConfigSettings {
        admin,
        epoch_thread: epoch_thread_pubkey,
        hasher_thread: hasher_thread_pubkey
    };
    let ix_c = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::ConfigUpdate {
            admin,
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

    let config = super::config::fetch(client)?;
    print_status!("Epoch    🧵", "{}", explorer.account(config.clone().epoch_thread));
    print_status!("Hasher   🧵", "{}", explorer.account(config.clone().hasher_thread));
    print_status!("Admin    👔", "{}", explorer.account(config.clone().admin));
    Ok(())
}
