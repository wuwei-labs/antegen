mod config;
mod crontab;
mod localnet;
mod thread;

use {
    crate::{
        cli::{CliCommand, ConfigSubcommand}, client::Client, config::CliConfig, errors::CliError,
        processor::thread::parse_pubkey_from_id_or_address,
    },
    anyhow::Result,
    clap::ArgMatches,
    solana_sdk::signature::read_keypair_file,
};

pub fn process(matches: &ArgMatches) -> Result<(), CliError> {
    // Parse command and config
    let command = CliCommand::try_from(matches)?;

    match command {
        // Set solana config if using localnet command
        CliCommand::LocalnetStart { .. } => {
            set_solana_config().map_err(|err| CliError::FailedLocalnet(err.to_string()))?
        }
        _ => {}
    }

    let config = CliConfig::load();

    // Build the RPC client (not needed for all localnet commands)
    let payer = read_keypair_file(&config.keypair_path)
        .map_err(|_| CliError::KeypairNotFound(config.keypair_path.clone()))?;

    let client = Client::new(payer, config.json_rpc_url.clone());

    // Process the command
    match command {
        CliCommand::Crontab { schedule } => crontab::get(&client, schedule),
        CliCommand::LocalnetStart {
            config: config_path,
            validator,
            clients,
            release,
            verbose,
        } => localnet::start(config_path, validator, clients, release, verbose),
        CliCommand::LocalnetStartRpc { release, verbose } => {
            // Start with RPC client
            localnet::start(None, None, vec!["rpc".to_string()], release, verbose)
        }
        CliCommand::LocalnetStartCarbon { release, verbose } => {
            // Start with Carbon client
            localnet::start(None, None, vec!["carbon".to_string()], release, verbose)
        }
        CliCommand::LocalnetStartGeyser { release, verbose } => {
            // Start with Geyser plugin enabled
            localnet::start_with_geyser(release, verbose)
        }
        CliCommand::LocalnetStop => localnet::stop(),
        CliCommand::LocalnetStatus => localnet::status(),
        CliCommand::LocalnetClientAdd {
            client_type,
            name,
            rpc_url,
            keypair,
        } => localnet::add_client(client_type, name, rpc_url, keypair),
        CliCommand::LocalnetClientRemove { name } => localnet::remove_client(name),
        CliCommand::LocalnetClientList => localnet::list_clients(),
        CliCommand::ThreadCreate { id, trigger } => thread::create(&client, id, trigger),
        CliCommand::ThreadDelete { id, address } => {
            let pubkey = parse_pubkey_from_id_or_address(client.payer_pubkey(), id, address)?;
            thread::delete(&client, pubkey)
        }
        CliCommand::ThreadToggle { id } => thread::toggle(&client, id),
        CliCommand::ThreadGet { id, address } => {
            let pubkey = parse_pubkey_from_id_or_address(client.payer_pubkey(), id, address)?;
            thread::get(&client, pubkey)
        }
        CliCommand::ThreadUpdate { id, schedule } => thread::update(&client, id, schedule),
        CliCommand::ThreadStressTest {
            count,
            interval,
            jitter,
            prefix,
            with_fibers,
            batch_size,
            durable_ratio,
            fiber_count,
        } => thread::stress_test(&client, count, interval, jitter, prefix, with_fibers, batch_size, durable_ratio, fiber_count),
        CliCommand::Config { subcommand } => match subcommand {
            ConfigSubcommand::Init { admin } => config::init(&client, admin),
            ConfigSubcommand::Show => config::show(&client),
            ConfigSubcommand::Update {
                commission_fee,
                executor_fee_bps,
                core_team_bps,
                grace_period,
                fee_decay,
                pause,
                unpause,
                multisig,
            } => config::update(
                &client,
                commission_fee,
                executor_fee_bps,
                core_team_bps,
                grace_period,
                fee_decay,
                pause,
                unpause,
                multisig,
            ),
        },
    }
}

fn set_solana_config() -> Result<()> {
    let mut process = std::process::Command::new("solana")
        .args(&["config", "set", "--url", "l"])
        .spawn()
        .expect("Failed to set solana config");
    process.wait()?;
    std::thread::sleep(std::time::Duration::from_secs(1));
    Ok(())
}
