use antegen_network_program::state::{Registry, Snapshot};
use crate::{client::Client, errors::CliError};

pub fn get(client: &Client, id: Option<u64>) -> Result<(), CliError> {
    let registry_pubkey = Registry::pubkey();
    let registry = client
        .get::<Registry>(&registry_pubkey)
        .map_err(|_err| CliError::AccountDataNotParsable(registry_pubkey.to_string()))?;

    let snapshot_id = match id {
        Some(i) => i,
        None => registry.current_epoch
    };

    let snapshot_pubkey = Snapshot::pubkey(snapshot_id);
    let snapshot = client
        .get::<Snapshot>(&snapshot_pubkey)
        .map_err(|_err| CliError::AccountDataNotParsable(snapshot_pubkey.to_string()))?;

    println!("{:#?}", snapshot);
    Ok(())
}

// delete
