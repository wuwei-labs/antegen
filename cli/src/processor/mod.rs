mod config;
mod crontab;
mod localnet;
mod network;
mod pool;
mod registry;
// mod snapshot;
mod thread;
mod worker;

use {
    crate::{
        cli::CliCommand, client::Client, config::CliConfig, errors::CliError,
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
        CliCommand::Localnet { .. } => {
            set_solana_config().map_err(|err| CliError::FailedLocalnet(err.to_string()))?
        }
        _ => {}
    }

    let mut config = CliConfig::load();

    // Build the RPC client
    let payer = read_keypair_file(&config.keypair_path)
        .map_err(|_| CliError::KeypairNotFound(config.keypair_path.clone()))?;

    let client = Client::new(payer, config.json_rpc_url.clone());

    // Process the command
    match command {
        CliCommand::Crontab { schedule } => crontab::get(&client, schedule),
        CliCommand::NetworkInitialize {} => network::initialize(&client),
        CliCommand::NetworkConfigGet => config::get(&client),
        CliCommand::NetworkConfigSet {
            admin,
            output_format,
        } => config::set(&client, admin, output_format),
        CliCommand::Localnet {
            clone_addresses,
            program_infos,
            force_init,
            solana_archive,
            antegen_archive,
            dev,
            trailing_args,
        } => localnet::start(
            &mut config,
            &client,
            clone_addresses,
            program_infos,
            force_init,
            solana_archive,
            antegen_archive,
            dev,
            trailing_args,
        ),
        CliCommand::PoolGet { id } => pool::get(&client, id),
        CliCommand::PoolList {} => pool::list(&client),
        CliCommand::ThreadCreate {
            id,
            kickoff_instruction,
            trigger,
        } => thread::create(&client, id, vec![kickoff_instruction], trigger),
        CliCommand::ThreadDelete { id, address } => {
            let pubkey = parse_pubkey_from_id_or_address(client.payer_pubkey(), id, address)?;
            thread::delete(&client, pubkey)
        }
        CliCommand::ThreadPause { id } => thread::pause(&client, id),
        CliCommand::ThreadResume { id } => thread::resume(&client, id),
        CliCommand::ThreadReset { id } => thread::reset(&client, id),
        CliCommand::ThreadGet { id, address } => {
            let pubkey = parse_pubkey_from_id_or_address(client.payer_pubkey(), id, address)?;
            thread::get(&client, pubkey)
        }
        CliCommand::ThreadUpdate {
            id,
            rate_limit,
            schedule,
        } => thread::update(&client, id, rate_limit, schedule),
        CliCommand::RegistryGet => registry::get(&client),
        CliCommand::RegistryReset => registry::reset(&client),
        CliCommand::RegistryUnlock => registry::unlock(&client),
        CliCommand::WorkerCreate { signatory } => worker::create(&client, signatory, false),
        CliCommand::WorkerGet { id } => worker::get(&client, id),
        CliCommand::WorkerUpdate {
            id,
            commission_rate,
            signatory,
        } => worker::update(&client, id, commission_rate, signatory),
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
