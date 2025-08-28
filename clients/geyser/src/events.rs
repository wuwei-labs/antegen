use agave_geyser_plugin_interface::geyser_plugin_interface::{
    GeyserPluginError, ReplicaAccountInfo,
};
use solana_program::pubkey::Pubkey;
use solana_sdk::account::Account;

/// Convert Geyser account info to a standard Solana account
/// No parsing - just pass through all accounts to observer
pub fn replica_account_to_account(
    account_info: &ReplicaAccountInfo,
) -> Result<(Pubkey, Account), GeyserPluginError> {
    // Parse pubkeys
    let account_pubkey = Pubkey::try_from(account_info.pubkey)
        .map_err(|e| GeyserPluginError::AccountsUpdateError {
            msg: format!("Failed to parse account pubkey: {}", e),
        })?;
    
    let owner_pubkey = Pubkey::try_from(account_info.owner)
        .map_err(|e| GeyserPluginError::AccountsUpdateError {
            msg: format!("Failed to parse owner pubkey: {}", e),
        })?;
    
    // Convert account
    
    // Create standard account
    let account = Account {
        lamports: account_info.lamports,
        data: account_info.data.to_vec(),
        owner: owner_pubkey,
        executable: account_info.executable,
        rent_epoch: account_info.rent_epoch,
    };
    
    Ok((account_pubkey, account))
}