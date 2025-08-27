#[allow(deprecated)]
use {
    crate::utils::Explorer,
    crate::{
        client::Client, config::CliConfig, deps, errors::CliError, parser::ProgramInfo,
        print::print_style, print_status,
    },
    antegen_sdk::state::Trigger,
    anyhow::{Context, Result},
    solana_sdk::{commitment_config::CommitmentConfig, pubkey::Pubkey},
    std::process::{Command, Stdio},
    std::{fs, str::FromStr},
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
    enable_replay: bool,
    nats_url: Option<String>,
    replay_delay_ms: u64,
    forgo_commission: bool,
    trailing_args: Vec<String>,
) -> Result<(), CliError> {
    config.dev = dev;

    if dev {
        std::env::set_var(
            "RUST_LOG",
            "antegen_plugin=info,antegen_processor=info,antegen_adapter=warn,antegen_submitter=info,antegen_thread_program=info",
        );
    }

    deps::download_deps(
        &CliConfig::default_runtime_dir(),
        force_init,
        solana_archive,
        antegen_archive,
        dev,
    )?;

    // Create Geyser Plugin Config file
    create_geyser_plugin_config(
        config,
        enable_replay,
        nats_url,
        replay_delay_ms,
        forgo_commission,
    )?;

    // Start the validator
    start_test_validator(
        config,
        program_infos,
        clone_addresses,
        trailing_args, // Pass trailing args to validator
    )?;

    wait_for_validator(client, 120)?;

    // Initialize Antegen
    init_thread_config(client)?;
    create_test_thread(client)?;

    Ok(())
}

fn init_thread_config(client: &Client) -> Result<()> {
    // Initialize the thread config
    super::thread::init_config(client).context("Failed to initialize thread config")?;

    print_status!(
        "Thread Config ⚙️",
        "Initialized global thread configuration"
    );
    Ok(())
}

fn create_test_thread(client: &Client) -> Result<()> {
    let thread_id = "test-thread".to_string();
    // Use an interval trigger that fires every 30 seconds
    // This will let us see the submitter waiting and then submitting
    let trigger = Trigger::Interval {
        seconds: 30,
        skippable: true,
    };

    // Create thread
    super::thread::create(client, thread_id.clone(), trigger)
        .context("Failed to create test thread")?;

    // Create a simple SPL Memo instruction for testing
    // This is a safe no-op that just logs a message
    let memo_program_id = Pubkey::from_str("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr").unwrap();
    let test_instruction = solana_sdk::instruction::Instruction {
        program_id: memo_program_id,
        accounts: vec![], // Memo program doesn't require accounts
        data: b"Thread execution test!".to_vec(),
    };

    // Create initial fiber with test instruction
    super::thread::create_fiber(client, thread_id, 0, test_instruction)
        .context("Failed to create test fiber")?;

    print_status!(
        "Test Thread 🧵",
        "Created test thread with memo instruction (runs every 30 seconds)"
    );
    Ok(())
}

fn create_geyser_plugin_config(
    config: &CliConfig,
    enable_replay: bool,
    nats_url: Option<String>,
    replay_delay_ms: u64,
    forgo_commission: bool,
) -> Result<()> {
    // Create a simple plugin config without using external crate
    #[derive(serde::Serialize)]
    struct PrometheusConfig {
        port: u16,
        path: String,
    }
    
    #[derive(serde::Serialize)]
    struct MetricsConfig {
        enabled: bool,
        backend: String,
        prometheus: Option<PrometheusConfig>,
    }
    
    #[derive(serde::Serialize)]
    struct PluginConfig {
        name: String,
        keypath: Option<String>,
        libpath: Option<String>,
        rpc_url: Option<String>,
        ws_url: Option<String>,
        thread_count: usize,
        transaction_timeout_threshold: u64,
        forgo_executor_commission: Option<bool>,
        enable_replay: Option<bool>,
        nats_url: Option<String>,
        replay_delay_ms: Option<u64>,
        metrics: Option<MetricsConfig>,
    }

    let geyser_config = PluginConfig {
        name: "antegen".to_string(),
        keypath: Some(config.signatory().to_owned()),
        libpath: Some(config.geyser_lib().to_owned()),
        rpc_url: Some("http://localhost:8899".to_string()),
        ws_url: Some("ws://localhost:8900".to_string()),
        thread_count: 10,
        transaction_timeout_threshold: 150,
        forgo_executor_commission: Some(forgo_commission),
        enable_replay: Some(enable_replay),
        nats_url: nats_url,
        replay_delay_ms: Some(replay_delay_ms),
        metrics: Some(MetricsConfig {
            enabled: true,
            backend: "prometheus".to_string(),
            prometheus: Some(PrometheusConfig {
                port: 9090,
                path: "/metrics".to_string(),
            }),
        }),
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
    trailing_args: Vec<String>,
) -> Result<()> {
    let path = config.active_runtime("solana-test-validator").to_owned();
    let duration = chrono::Duration::hours(2);
    let end_time = chrono::Local::now() + duration;
    let network_url = config.json_rpc_url.to_owned();
    let explorer = Explorer::from(network_url);

    if trailing_args.contains(&"--help".to_string()) || trailing_args.contains(&"-h".to_string()) {
        let mut help_cmd = Command::new(&path);
        help_cmd.arg("--help");

        let status = help_cmd
            .status()
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
        .arg("--log") // Enable logging
        .bpf_program(config, antegen_sdk::ID, "thread")
        // .bpf_program(config, antegen_test_program::ID, "test")
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

    print_status!(
        "Running  🏃",
        "Solana Validator with Antegen {}",
        env!("CARGO_PKG_VERSION").to_owned()
    );
    print_status!("Explorer 🔍", "{}", explorer.base());
    print_status!("Metrics  📊", "http://localhost:9090/metrics");
    print_status!(
        "Timeout  ⏰",
        "Validator will automatically stop at {}",
        end_time.format("%Y-%m-%d %H:%M:%S")
    );

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
