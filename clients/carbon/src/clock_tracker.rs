use anyhow::Result;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use crossbeam::channel::Sender;
use futures::StreamExt;
use log::{debug, error, info};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_config::RpcAccountInfoConfig;
use solana_pubsub_client::nonblocking::pubsub_client::PubsubClient;
use solana_sdk::{
    account::Account,
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    sysvar,
};
use solana_account_decoder::{UiAccount, UiAccountEncoding};
use std::str::FromStr;
use std::sync::Arc;

use antegen_adapter::events::ObservedEvent;

/// Track Clock updates via WebSocket subscription
pub async fn track_clock(
    ws_url: String,
    sender: Sender<ObservedEvent>,
    rpc_client: Arc<RpcClient>,
) -> Result<()> {
    info!("Connecting to WebSocket: {}", ws_url);
    
    // Get initial Clock state
    info!("Fetching initial Clock account state");
    let clock_account = rpc_client.get_account(&sysvar::clock::ID).await?;
    let slot = rpc_client.get_slot().await?;
    
    // Send initial Clock event
    sender.send(ObservedEvent::Account {
        pubkey: sysvar::clock::ID,
        account: clock_account,
        slot,
    })?;
    info!("Sent initial Clock account update at slot {}", slot);
    
    // Create PubSub client and subscribe to Clock account updates
    info!("Creating WebSocket connection for Clock subscription");
    let pubsub = PubsubClient::new(&ws_url).await?;
    
    let config = Some(RpcAccountInfoConfig {
        commitment: Some(CommitmentConfig::processed()),
        encoding: Some(UiAccountEncoding::Base64),
        data_slice: None,
        min_context_slot: None,
    });
    
    debug!("Subscribing to Clock account: {}", sysvar::clock::ID);
    let (mut stream, _unsub) = pubsub
        .account_subscribe(&sysvar::clock::ID, config)
        .await?;
    
    info!("Clock subscription active");
    
    // Process Clock updates
    while let Some(response) = stream.next().await {
        let slot = response.context.slot;
        
        // Convert UiAccount to Account
        let account = match convert_ui_account_to_account(&response.value) {
            Ok(acc) => acc,
            Err(e) => {
                error!("Failed to convert Clock account data: {}", e);
                continue;
            }
        };
        
        // Send Clock update event
        if let Err(e) = sender.send(ObservedEvent::Account {
            pubkey: sysvar::clock::ID,
            account,
            slot,
        }) {
            error!("Failed to send Clock update event: {}", e);
            break;
        }
        
        debug!("Clock update: slot {}", slot);
    }
    
    info!("Clock subscription stopped");
    Ok(())
}

/// Convert UiAccount to Account
fn convert_ui_account_to_account(ui_account: &UiAccount) -> Result<Account> {
    let data = match &ui_account.data {
        solana_account_decoder::UiAccountData::Binary(data, encoding) => {
            match encoding {
                UiAccountEncoding::Base64 => {
                    BASE64.decode(data)?
                }
                UiAccountEncoding::Base58 => {
                    bs58::decode(data).into_vec()?
                }
                _ => {
                    return Err(anyhow::anyhow!("Unsupported encoding: {:?}", encoding));
                }
            }
        }
        solana_account_decoder::UiAccountData::Json(_) => {
            return Err(anyhow::anyhow!("JSON encoding not supported for Clock account"));
        }
        solana_account_decoder::UiAccountData::LegacyBinary(data) => {
            bs58::decode(data).into_vec()?
        }
    };
    
    Ok(Account {
        lamports: ui_account.lamports,
        data,
        owner: Pubkey::from_str(&ui_account.owner)?,
        executable: ui_account.executable,
        rent_epoch: ui_account.rent_epoch,
    })
}

