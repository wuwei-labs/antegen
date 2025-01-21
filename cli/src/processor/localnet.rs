#[allow(deprecated)]
use {
    crate::{
        client::Client,
        config::CliConfig,
        deps,
        errors::CliError,
        parser::ProgramInfo,
        print::print_style,
        print_status
    },
    anyhow::{
        Context,
        Result,
    },
    anchor_lang::{
        solana_program::{instruction::Instruction, system_program},
        InstructionData, ToAccountMetas
    },
    antegen_network_program::state::{Config, ConfigSettings, Registry},
    antegen_thread_program::state::{Thread, Trigger},
    antegen_utils::explorer::Explorer,
    solana_sdk::{
        commitment_config::CommitmentConfig,
        native_token::LAMPORTS_PER_SOL,
        program_pack::Pack,
        pubkey::Pubkey,
        signature::{
            read_keypair_file,
            Keypair,
            Signer,
        },
        system_instruction,
    },
    spl_associated_token_account::{
        create_associated_token_account,
        get_associated_token_address,
    },
    spl_token::{
        instruction::{
            initialize_mint,
            mint_to,
        },
        state::Mint,
    },
    std::fs,
    std::process::{
        Command,
        Stdio
    },
};

pub fn start(
    config: &mut CliConfig,
    client: &Client,
    clone_addresses: Vec<Pubkey>,
    program_infos: Vec<ProgramInfo>,
    force_init: bool,
    solana_archive: Option<String>,
    antegen_archive: Option<String>,
    dev: bool,
    trailing_args: Vec<String>,
) -> Result<(), CliError> {
    config.dev = dev;

    if dev {
        std::env::set_var("RUST_LOG", "antegen_plugin=debug");
    }

    deps::download_deps(
        &CliConfig::default_runtime_dir(),
        force_init,
        solana_archive,
        antegen_archive,
        dev,
    )?;

    // Create Geyser Plugin Config file
    create_geyser_plugin_config(config)?;

    // Start the validator
    start_test_validator(
        config,
        program_infos, 
        clone_addresses,
        trailing_args,  // Pass trailing args to validator
    )?;

    wait_for_validator(client, 10)?;

    // Initialize Antegen
    let mint_pubkey = mint_antegen_token(client)?;
    super::initialize::initialize(client, mint_pubkey)?;
    register_worker(client, config)?;
    create_threads(client, mint_pubkey)?;

    Ok(())
}

fn mint_antegen_token(client: &Client) -> Result<Pubkey> {
    let explorer = Explorer::from(client.client.url());
    // Calculate rent and pubkeys
    let mint_keypair = Keypair::new();
    let mint_rent = client
        .get_minimum_balance_for_rent_exemption(Mint::LEN)
        .context("Failed to calculate mint rent")?;
    let token_account_pubkey =
        get_associated_token_address(&client.payer_pubkey(), &mint_keypair.pubkey());

    // Build ixs
    let ixs = vec![
        // Create mint account
        system_instruction::create_account(
            &client.payer_pubkey(),
            &mint_keypair.pubkey(),
            mint_rent,
            Mint::LEN as u64,
            &spl_token::ID,
        ),
        initialize_mint(
            &spl_token::ID,
            &mint_keypair.pubkey(),
            &client.payer_pubkey(),
            None,
            8,
        )
        .unwrap(),
        // Create associated token account
        #[allow(deprecated)]
        create_associated_token_account(
            &client.payer_pubkey(),
            &client.payer_pubkey(),
            &mint_keypair.pubkey(),
        ),
        // Mint 10 tokens to the local user
        mint_to(
            &spl_token::ID,
            &mint_keypair.pubkey(),
            &token_account_pubkey,
            &client.payer_pubkey(),
            &[&client.payer_pubkey()],
            1000000000,
        )
        .unwrap(),
    ];

    // Submit tx
    client
        .send_and_confirm(&ixs, &[client.payer(), &mint_keypair])
        .context("Failed to mint Antegen tokens")?;

    print_status!("Mint     💰", "{}", explorer.token(mint_keypair.pubkey()));
    Ok(mint_keypair.pubkey())
}

fn register_worker(client: &Client, config: &CliConfig) -> Result<()> {
    let explorer = Explorer::from(client.client.url());
    // Create the worker
    let signatory = read_keypair_file(&config.signatory()).map_err(|err| {
        CliError::FailedLocalnet(format!(
            "Unable to read keypair {}: {}",
            &config.signatory(),
            err
        ))
    })?;

    client
        .airdrop(&signatory.pubkey(), LAMPORTS_PER_SOL)
        .context("airdrop to signatory failed")?;
    super::worker::create(client, signatory, true).context("worker::create failed")?;

    let worker_info = super::worker::get(client, 0);
    print_status!("Worker   👷", "{}", explorer.account(worker_info?.worker_pubkey));

    // Delegate stake to the worker
    super::delegation::create(client, 0).context("delegation::create failed")?;
    super::delegation::deposit(client, 100000000, 0, 0).context("delegation::deposit failed")?;
    let delegation_info = super::delegation::get(client,0, 0);
    print_status!("Delegate 🤝", "{}", explorer.account(delegation_info?.delegation_pubkey));
    Ok(())
}

fn create_threads(client: &Client, mint_pubkey: Pubkey) -> Result<()> {
    let explorer = Explorer::from(client.client.url());
    // Create epoch thread.
    let epoch_thread_id = "antegen.network.epoch";
    let epoch_thread_pubkey = Thread::pubkey(client.payer_pubkey(), epoch_thread_id.into());
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
        accounts: antegen_network_program::accounts::ProcessUnstakesJob {
            config: Config::pubkey(),
            registry: Registry::pubkey(),
            thread: epoch_thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::ProcessUnstakesJob {}.data(),
    };
    let ix_a3 = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::StakeDelegationsJob {
            config: Config::pubkey(),
            registry: Registry::pubkey(),
            thread: epoch_thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::StakeDelegationsJob {}.data(),
    };
    let ix_a4 = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::TakeSnapshotJob {
            config: Config::pubkey(),
            registry: Registry::pubkey(),
            thread: epoch_thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::TakeSnapshotJob {}.data(),
    };
    let ix_a5 = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::EpochCutover {
            config: Config::pubkey(),
            registry: Registry::pubkey(),
            thread: epoch_thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::IncrementEpoch {}.data(),
    };
    let ix_a6 = Instruction {
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
            authority: client.payer_pubkey(),
            payer: client.payer_pubkey(),
            system_program: system_program::ID,
            thread: epoch_thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_thread_program::instruction::ThreadCreate {
            amount: LAMPORTS_PER_SOL,
            id: epoch_thread_id.into(),
            instructions: vec![
                ix_a1.into(),
                ix_a2.into(),
                ix_a3.into(),
                ix_a4.into(),
                ix_a5.into(),
                ix_a6.into(),
            ],
            trigger: Trigger::Cron {
                schedule: "*/5 * * * *".into(),
                skippable: true,
            },
        }
        .data(),
    };

    // Create hasher thread.
    let hasher_thread_id = "antegen.network.hasher";
    let hasher_thread_pubkey = Thread::pubkey(client.payer_pubkey(), hasher_thread_id.into());
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
            authority: client.payer_pubkey(),
            payer: client.payer_pubkey(),
            system_program: system_program::ID,
            thread: hasher_thread_pubkey,
        }.to_account_metas(Some(false)),
        data: antegen_thread_program::instruction::ThreadCreate {
            amount: LAMPORTS_PER_SOL,
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
        admin: client.payer_pubkey(),
        epoch_thread: epoch_thread_pubkey,
        hasher_thread: hasher_thread_pubkey,
        mint: mint_pubkey,
    };
    let ix_c = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::ConfigUpdate {
            admin: client.payer_pubkey(),
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

    let config = super::config::get(client)?;
    print_status!("Epoch    🧵", "{}", explorer.account(config.clone().epoch_thread));
    print_status!("Hasher   🧵", "{}", explorer.account(config.clone().hasher_thread));
    print_status!("Admin    👔", "{}", explorer.account(config.clone().admin));
    Ok(())
}

fn create_geyser_plugin_config(config: &CliConfig) -> Result<()> {
    let geyser_config = antegen_plugin_utils::PluginConfig {
        keypath: Some(config.signatory().to_owned()),
        libpath: Some(config.geyser_lib().to_owned()),
        ..Default::default()
    };

    let content = serde_json::to_string_pretty(&geyser_config)
        .context("Unable to serialize PluginConfig to json")?;
    let path = &config.geyser_config();
    fs::write(&path, content).context(format!("Unable to serialize PluginConfig to {}", path))?;
    Ok(())
}

fn start_test_validator(
    config: &CliConfig,
    program_infos: Vec<ProgramInfo>,
    clone_addresses: Vec<Pubkey>,
    trailing_args: Vec<String>
) -> Result<()> {
    let path = config.active_runtime("solana-test-validator").to_owned();
    let duration = chrono::Duration::hours(2);
    let end_time = chrono::Local::now() + duration;
    let network_url = config.json_rpc_url.to_owned();
    let explorer = Explorer::from(network_url);

    if trailing_args.contains(&"--help".to_string()) || trailing_args.contains(&"-h".to_string()) {
        let mut help_cmd = Command::new(&path);
        help_cmd.arg("--help");
        
        let status = help_cmd.status()
            .context("Failed to execute solana-test-validator --help")?;
        std::process::exit(status.code().unwrap_or(1));
    }

    let log_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("validator.log")
        .context("Failed to create log file")?;

    let cmd = &mut Command::new("timeout");
    cmd.arg(format!("{}h", duration.num_hours()))
        .arg(&path)
        .arg("--reset")
        .arg("--log")  // Enable logging
        .bpf_program(config, antegen_network_program::ID, "network")
        .bpf_program(config, antegen_thread_program::ID, "thread")
        .clone_addresses(clone_addresses)
        .add_programs_with_path(program_infos)
        .geyser_plugin_config(config)
        .args(trailing_args);

    let process = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file.try_clone()?))
        .stderr(Stdio::from(log_file))
        .spawn()
        .context(format!("solana-test-validator command: {:#?}", cmd))?;

    // Detach the process
    std::mem::forget(process);

    print_status!("Running  🏃", "Solana Validator with Antegen {}", env!("CARGO_PKG_VERSION").to_owned());
    print_status!("Explorer 🔍", "{}", explorer.base());
    print_status!("Timeout  ⏰", "Validator will automatically stop at {}", end_time.format("%Y-%m-%d %H:%M:%S"));

    Ok(())
}

pub fn wait_for_validator(client: &Client, timeout_secs: u64) -> Result<(), CliError> {
    let timeout = std::time::Duration::from_secs(timeout_secs);
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        match client.get_slot_with_commitment(CommitmentConfig::processed()) {
            Ok(slot) if slot > 0 => return Ok(()),
            _ => std::thread::sleep(std::time::Duration::from_millis(500)),
        }
    }

    Err(CliError::FailedLocalnet(format!(
        "Validator failed to start within {} seconds",
        timeout_secs
    )))
}

trait TestValidatorHelpers {
    fn add_programs_with_path(&mut self, program_infos: Vec<ProgramInfo>) -> &mut Command;
    fn bpf_program(
        &mut self,
        config: &CliConfig,
        program_id: Pubkey,
        program_name: &str,
    ) -> &mut Command;
    fn geyser_plugin_config(&mut self, config: &CliConfig) -> &mut Command;
    fn clone_addresses(&mut self, clone_addresses: Vec<Pubkey>) -> &mut Command;
}

impl TestValidatorHelpers for Command {
    fn add_programs_with_path(&mut self, program_infos: Vec<ProgramInfo>) -> &mut Command {
        for program_info in program_infos {
            self.arg("--bpf-program")
                .arg(program_info.program_id.to_string())
                .arg(program_info.program_path);
        }

        self
    }
    fn bpf_program(
        &mut self,
        config: &CliConfig,
        program_id: Pubkey,
        program_name: &str,
    ) -> &mut Command {
        let filename = format!("antegen_{}_program.so", program_name);
        self.arg("--bpf-program")
            .arg(program_id.to_string())
            .arg(config.active_runtime(filename.as_str()).to_owned())
    }

    fn geyser_plugin_config(&mut self, config: &CliConfig) -> &mut Command {
        self.arg("--geyser-plugin-config")
            .arg(config.geyser_config().to_owned())
    }

    fn clone_addresses(&mut self, clone_addresses: Vec<Pubkey>) -> &mut Command {
        for clone_address in clone_addresses {
            self.arg("--clone").arg(clone_address.to_string());
        }
        self
    }
}
