use antegen_sdk::{
    state::{ThreadConfig, ConfigUpdateParams},
    ID,
};
use anchor_lang::{AccountDeserialize, AnchorSerialize};
use solana_program::pubkey::Pubkey;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    system_program,
};
use std::str::FromStr;

use crate::{client::Client, errors::CliError};

pub fn init(client: &Client, admin: Option<Pubkey>) -> Result<(), CliError> {
    // Use provided admin or default to payer
    let admin_pubkey = admin.unwrap_or_else(|| client.payer_pubkey());
    
    // Derive config PDA
    let config_pubkey = Pubkey::find_program_address(&[b"thread_config"], &ID).0;
    
    // Check if config already exists
    match client.get_account_data(&config_pubkey) {
        Ok(_) => {
            return Err(CliError::BadParameter(
                "Config already initialized. Use 'config update' to modify settings.".into()
            ));
        }
        Err(_) => {
            // Config doesn't exist, proceed with initialization
        }
    }
    
    // Build initialization instruction
    // Using the init_config instruction discriminator
    let ix = Instruction {
        program_id: ID,
        accounts: vec![
            AccountMeta::new(admin_pubkey, true),  // admin is signer and mutable
            AccountMeta::new(config_pubkey, false), // config is mutable
            AccountMeta::new_readonly(system_program::id(), false), // system_program is readonly
        ],
        data: anchor_lang::solana_program::hash::hash(b"global:init_config").to_bytes()[..8].to_vec(),
    };
    
    // Send transaction - include payer as signer
    client.send_and_confirm(&[ix], &[client.payer()]).map_err(|e| CliError::BadParameter(format!("Failed to send transaction: {}", e)))?;
    
    println!("✅ Config initialized successfully");
    println!("   Admin: {}", admin_pubkey);
    println!("   Config: {}", config_pubkey);
    
    Ok(())
}

pub fn show(client: &Client) -> Result<(), CliError> {
    // Derive config PDA
    let config_pubkey = Pubkey::find_program_address(&[b"thread_config"], &ID).0;
    
    // Fetch config account
    let account_data = client.get_account_data(&config_pubkey).map_err(|_| {
        CliError::BadParameter(
            "Config not initialized. Run 'antegen config init' first.".into()
        )
    })?;
    
    // Deserialize config
    // Deserialize config - skip discriminator (8 bytes)
    let config = ThreadConfig::try_deserialize(&mut &account_data[8..])
        .map_err(|e| CliError::BadParameter(format!("Failed to deserialize config: {}", e)))?;
    
    // Display config
    println!("🔧 Thread Program Configuration");
    println!("═══════════════════════════════");
    println!("📍 Config Address: {}", config_pubkey);
    println!("👤 Admin: {}", config.admin);
    println!("🔢 Version: {}", config.version);
    println!("⏸️  Paused: {}", if config.paused { "Yes ⚠️" } else { "No ✅" });
    println!();
    println!("💰 Fee Configuration:");
    println!("   Base Commission: {} lamports", config.commission_fee);
    println!("   Executor Fee: {}% ({} bps)", config.executor_fee_bps as f64 / 100.0, config.executor_fee_bps);
    println!("   Core Team Fee: {}% ({} bps)", config.core_team_bps as f64 / 100.0, config.core_team_bps);
    println!();
    println!("⏱️  Timing Configuration:");
    println!("   Grace Period: {} seconds", config.grace_period_seconds);
    println!("   Fee Decay Period: {} seconds", config.fee_decay_seconds);
    println!("   Total Window: {} seconds", config.grace_period_seconds + config.fee_decay_seconds);
    
    Ok(())
}

pub fn update(
    client: &Client,
    commission_fee: Option<u64>,
    executor_fee_bps: Option<u16>,
    core_team_bps: Option<u16>,
    grace_period: Option<i64>,
    fee_decay: Option<i64>,
    pause: bool,
    unpause: bool,
    multisig: bool,
) -> Result<(), CliError> {
    // Derive config PDA
    let config_pubkey = Pubkey::find_program_address(&[b"thread_config"], &ID).0;
    
    // Fetch current config to get admin
    let account_data = client.get_account_data(&config_pubkey).map_err(|_| {
        CliError::BadParameter(
            "Config not initialized. Run 'antegen config init' first.".into()
        )
    })?;
    
    // Deserialize config - skip discriminator (8 bytes)
    let config = ThreadConfig::try_deserialize(&mut &account_data[8..])
        .map_err(|e| CliError::BadParameter(format!("Failed to deserialize config: {}", e)))?;
    
    // Check if pause and unpause are both set
    if pause && unpause {
        return Err(CliError::BadParameter(
            "Cannot both pause and unpause. Choose one.".into()
        ));
    }
    
    // Build update params
    let params = ConfigUpdateParams {
        admin: None,  // Not changing admin for now
        paused: if pause { Some(true) } else if unpause { Some(false) } else { None },
        commission_fee,
        executor_fee_bps: executor_fee_bps.map(|v| v as u64),
        core_team_bps: core_team_bps.map(|v| v as u64),
        grace_period_seconds: grace_period,
        fee_decay_seconds: fee_decay,
    };
    
    // Check if admin is payer or if we need multisig
    let admin_is_payer = config.admin == client.payer_pubkey();
    
    if multisig || !admin_is_payer {
        // Check if admin is a SQUADs multisig
        if let Ok(admin_account) = client.get_account(&config.admin) {
            // Check if this is a SQUADs multisig account
            // SQUADs multisig accounts are owned by the SQUADs program
            let squads_program_id = Pubkey::from_str("SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf")
                .unwrap_or_default();
            
            if admin_account.owner == squads_program_id {
                println!("⚠️  Admin is a SQUADs multisig. Creating proposal...");
                return create_squads_proposal(client, config_pubkey, config.admin, params);
            }
        }
        
        if !admin_is_payer {
            return Err(CliError::BadParameter(
                format!("Current keypair {} is not the admin {}. Use the admin keypair or --multisig flag.", 
                    client.payer_pubkey(), config.admin)
            ));
        }
    }
    
    // Build update instruction
    // Using the update_config instruction discriminator with params
    let mut data = anchor_lang::solana_program::hash::hash(b"global:update_config").to_bytes()[..8].to_vec();
    // Serialize the params using AnchorSerialize
    params.serialize(&mut data).map_err(|e| CliError::BadParameter(format!("Failed to serialize params: {}", e)))?;
    
    let ix = Instruction {
        program_id: ID,
        accounts: vec![
            AccountMeta::new(config.admin, true),  // admin is signer and mutable
            AccountMeta::new(config_pubkey, false), // config is mutable
        ],
        data,
    };
    
    // Send transaction - include payer as signer
    client.send_and_confirm(&[ix], &[client.payer()]).map_err(|e| CliError::BadParameter(format!("Failed to send transaction: {}", e)))?;
    
    println!("✅ Config updated successfully");
    
    // Show what was updated
    if commission_fee.is_some() {
        println!("   Commission fee: {} lamports", commission_fee.unwrap());
    }
    if executor_fee_bps.is_some() {
        println!("   Executor fee: {} bps", executor_fee_bps.unwrap());
    }
    if core_team_bps.is_some() {
        println!("   Core team fee: {} bps", core_team_bps.unwrap());
    }
    if grace_period.is_some() {
        println!("   Grace period: {} seconds", grace_period.unwrap());
    }
    if fee_decay.is_some() {
        println!("   Fee decay: {} seconds", fee_decay.unwrap());
    }
    if pause {
        println!("   Program PAUSED ⏸️");
    }
    if unpause {
        println!("   Program UNPAUSED ▶️");
    }
    
    Ok(())
}

fn create_squads_proposal(
    _client: &Client,
    _config_pubkey: Pubkey,
    _multisig: Pubkey,
    _params: ConfigUpdateParams,
) -> Result<(), CliError> {
    // TODO: Implement SQUADs proposal creation
    // This would use the SQUADs SDK to create a proposal
    // For now, return an informative error
    Err(CliError::BadParameter(
        "SQUADs multisig integration not yet implemented. Please create the proposal manually.".into()
    ))
}