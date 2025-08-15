mod crontab;
mod localnet;
mod network;
mod registry;
// mod snapshot;
mod builder;
mod thread;

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
        CliCommand::ThreadCreate { id, trigger } => thread::create(&client, id, trigger, None),
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
        CliCommand::RegistryGet => registry::get(&client),
        CliCommand::BuilderCreate { signatory } => builder::create(&client, signatory, false),
        CliCommand::BuilderGet { id } => builder::get(&client, id),
        CliCommand::BuilderUpdate {
            id,
            commission_bps,
            signatory,
        } => builder::update(&client, id, commission_bps, signatory),
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
