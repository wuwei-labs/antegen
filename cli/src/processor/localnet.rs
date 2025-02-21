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
    antegen_utils::explorer::Explorer,
    solana_sdk::{
        commitment_config::CommitmentConfig,
        native_token::LAMPORTS_PER_SOL,
        pubkey::Pubkey,
        signature::{
            read_keypair_file,
            Signer,
        },
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
    super::network::initialize(client)?;
    super::network::create_threads(client, LAMPORTS_PER_SOL)?;
    register_worker(client, config)?;

    Ok(())
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

    let worker_info = super::worker::_get(client, 0);
    print_status!("Worker   👷", "{}", explorer.account(worker_info?.worker_pubkey));
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
        .bpf_program(config, antegen_test_program::ID, "test")
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
