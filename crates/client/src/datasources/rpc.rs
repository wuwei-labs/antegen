//! RPC WebSocket datasource using WsClient
//!
//! This module provides WebSocket subscription functionality with:
//! - Automatic reconnection via pws (through WsClient)
//! - Thread account deserialization using Anchor
//! - Clock sysvar tracking
//! - Initial backfill via getProgramAccounts (using custom RpcPool)

use anchor_lang::Discriminator;
use antegen_thread_program::state::Thread;
use anyhow::Result;
use log::{debug, error, info, trace, warn};
use ractor::ActorRef;
use serde::Deserialize;
use solana_sdk::{clock::Clock, pubkey::Pubkey, sysvar};
use std::sync::Arc;

use crate::actors::messages::RpcSourceMessage;
use crate::rpc::response::decode_account_data;
use crate::rpc::websocket::{WsClient, WsMessage};
use crate::rpc::RpcPool;
use crate::types::AccountUpdate;

/// WebSocket subscription manager using pws for automatic reconnection
pub struct RpcSubscription {
    ws_url: String,
    program_id: Pubkey,
    rpc_client: Arc<RpcPool>,
}

impl RpcSubscription {
    /// Create a new RPC subscription manager
    pub fn new(ws_url: String, program_id: Pubkey, rpc_client: Arc<RpcPool>) -> Self {
        Self {
            ws_url,
            program_id,
            rpc_client,
        }
    }

    /// Perform backfill using getProgramAccounts via custom RpcPool
    ///
    /// This fetches all Thread accounts from the program and sends them
    /// through the actor's message channel. Used for:
    /// - Initial startup (to learn about all existing threads)
    /// - Reconnection (to catch any missed updates while disconnected)
    ///
    /// Returns the number of accounts backfilled.
    pub async fn perform_backfill(&self, actor_ref: ActorRef<RpcSourceMessage>) -> Result<usize> {
        debug!("[{}] Performing backfill via getProgramAccounts...", self.ws_url);

        // Use discriminator filter for Thread accounts
        let filters = vec![serde_json::json!({
            "memcmp": {
                "offset": 0,
                "bytes": bs58::encode(Thread::DISCRIMINATOR).into_string()
            }
        })];

        let accounts = self
            .rpc_client
            .get_program_accounts(&self.program_id, Some(filters))
            .await?;

        let count = accounts.len();
        debug!("[{}] Found {} Thread accounts to backfill", self.ws_url, count);

        for (pubkey, ui_account) in accounts {
            // Decode account data (supports base64 and base64+zstd)
            let data = match decode_account_data(&ui_account.data.0, &ui_account.data.1) {
                Ok(d) => d,
                Err(e) => {
                    warn!("[{}] Failed to decode account {}: {}", self.ws_url, pubkey, e);
                    continue;
                }
            };

            let update = AccountUpdate {
                pubkey,
                data,
                slot: 0, // Backfill uses slot 0; live updates will supersede with real slots
            };

            trace!("[{}] Backfilling Thread account: {}", self.ws_url, pubkey);

            if let Err(e) = actor_ref.send_message(RpcSourceMessage::UpdateReceived(update)) {
                error!("[{}] Failed to send backfilled account {}: {:?}", self.ws_url, pubkey, e);
                break;
            }
        }

        info!("[{}] Backfill complete: {} threads", self.ws_url, count);
        Ok(count)
    }

    /// Subscribe to program accounts using WsClient (auto-reconnecting)
    ///
    /// pws handles reconnection automatically, so no manual backoff needed.
    /// On reconnection, we re-send the subscription request.
    /// Sends periodic pings to keep the connection alive.
    pub async fn subscribe_to_program_accounts(&self, actor_ref: ActorRef<RpcSourceMessage>) {
        debug!("[{}] Connecting to WebSocket for program subscription...", self.ws_url);

        let (sender, mut receiver) = match WsClient::connect_raw(&self.ws_url).await {
            Ok((s, r)) => (s, r),
            Err(e) => {
                error!("[{}] Failed to connect WebSocket: {}", self.ws_url, e);
                return;
            }
        };

        // Build subscription message with Thread discriminator filter (reused on reconnection)
        let filters = vec![serde_json::json!({
            "memcmp": {
                "offset": 0,
                "bytes": bs58::encode(Thread::DISCRIMINATOR).into_string()
            }
        })];
        let (_, subscribe_msg) = WsClient::build_program_subscribe_request(
            &self.program_id,
            "confirmed",
            Some(filters),
        );

        // Send initial subscription
        if let Err(e) = sender.send(WsMessage::Text(subscribe_msg.clone())).await {
            error!("[{}] Failed to send program subscription: {}", self.ws_url, e);
            return;
        }

        debug!("[{}] Program subscription request sent", self.ws_url);

        // Spawn keep-alive ping task (every 10 seconds for aggressive keepalive)
        let ping_sender = sender.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                if ping_sender.send(WsMessage::Ping(vec![])).await.is_err() {
                    break;
                }
            }
        });

        // Process incoming messages
        loop {
            match receiver.recv().await {
                Ok(msg) => {
                    match &msg {
                        WsMessage::Text(text) => {
                            if let Some(update) = parse_program_notification(text) {
                                if let Err(e) = actor_ref.send_message(RpcSourceMessage::UpdateReceived(update)) {
                                    error!("[{}] Failed to send account update: {:?}", self.ws_url, e);
                                    break;
                                }
                            }
                        }
                        WsMessage::ConnectionOpened => {
                            debug!("[{}] WS program connected, subscribing...", self.ws_url);
                            if let Err(e) = sender.send(WsMessage::Text(subscribe_msg.clone())).await {
                                error!("[{}] Failed to send program subscription: {}", self.ws_url, e);
                            } else {
                                // Trigger backfill (handles both initial load and reconnection)
                                let _ = actor_ref.send_message(RpcSourceMessage::Reconnected);
                            }
                        }
                        WsMessage::Pong(_) | WsMessage::ConnectionClosed => {}
                        _ => {}
                    }
                }
                Err(_) => {
                    // pws handles reconnection automatically
                }
            }
        }
    }

    /// Subscribe to clock sysvar using WsClient (auto-reconnecting)
    ///
    /// On reconnection, we re-send the subscription request.
    pub async fn subscribe_to_clock(&self, actor_ref: ActorRef<RpcSourceMessage>) {
        debug!("[{}] Connecting to WebSocket for clock subscription...", self.ws_url);

        let (sender, mut receiver) = match WsClient::connect_raw(&self.ws_url).await {
            Ok((s, r)) => (s, r),
            Err(e) => {
                error!("[{}] Failed to connect WebSocket: {}", self.ws_url, e);
                return;
            }
        };

        // Build subscription message (reused on reconnection)
        let (_, subscribe_msg) = WsClient::build_account_subscribe_request(
            &sysvar::clock::ID,
            "confirmed",
        );

        // Send initial subscription
        if let Err(e) = sender.send(WsMessage::Text(subscribe_msg.clone())).await {
            error!("[{}] Failed to send clock subscription: {}", self.ws_url, e);
            return;
        }

        debug!("[{}] Clock subscription request sent", self.ws_url);

        // Process incoming messages
        loop {
            match receiver.recv().await {
                Ok(msg) => {
                    match &msg {
                        WsMessage::Text(text) => {
                            if let Some(clock) = parse_clock_notification(text) {
                                if let Err(e) = actor_ref.send_message(RpcSourceMessage::ClockReceived(clock)) {
                                    error!("[{}] Failed to send clock update: {:?}", self.ws_url, e);
                                    break;
                                }
                            }
                        }
                        WsMessage::ConnectionOpened => {
                            debug!("[{}] WS clock connected, subscribing...", self.ws_url);
                            if let Err(e) = sender.send(WsMessage::Text(subscribe_msg.clone())).await {
                                error!("[{}] Failed to send clock subscription: {}", self.ws_url, e);
                            }
                        }
                        WsMessage::ConnectionClosed => {}
                        _ => {}
                    }
                }
                Err(_) => {
                    // pws handles reconnection automatically
                }
            }
        }
    }
}

// ============================================================================
// Notification Parsing
// ============================================================================

#[derive(Debug, Deserialize)]
struct NotificationContext {
    slot: u64,
}

// ============================================================================
// Program Notification Types (programNotification)
// ============================================================================

#[derive(Debug, Deserialize)]
struct ProgramNotification {
    method: Option<String>,
    params: Option<ProgramNotificationParams>,
}

#[derive(Debug, Deserialize)]
struct ProgramNotificationParams {
    result: ProgramNotificationResult,
}

#[derive(Debug, Deserialize)]
struct ProgramNotificationResult {
    context: NotificationContext,
    value: ProgramNotificationValue,
}

/// programNotification value has pubkey + account
#[derive(Debug, Deserialize)]
struct ProgramNotificationValue {
    pubkey: String,
    account: AccountData,
}

#[derive(Debug, Deserialize)]
struct AccountData {
    data: (String, String), // (base64_data, encoding)
}

// ============================================================================
// Account Notification Types (accountNotification - e.g., Clock)
// ============================================================================

#[derive(Debug, Deserialize)]
struct AccountNotification {
    method: Option<String>,
    params: Option<AccountNotificationParams>,
}

#[derive(Debug, Deserialize)]
struct AccountNotificationParams {
    result: AccountNotificationResult,
}

#[derive(Debug, Deserialize)]
struct AccountNotificationResult {
    #[allow(dead_code)] // Context available for future use (e.g., slot-based filtering)
    context: NotificationContext,
    value: AccountNotificationValue,
}

/// accountNotification value IS the account data directly (no nested account field)
#[derive(Debug, Deserialize)]
struct AccountNotificationValue {
    data: (String, String), // (base64_data, encoding)
}

/// Parse a program notification message
fn parse_program_notification(text: &str) -> Option<AccountUpdate> {
    let notification: ProgramNotification = serde_json::from_str(text).ok()?;

    if notification.method.as_deref() != Some("programNotification") {
        return None;
    }

    let params = notification.params?;
    let pubkey: Pubkey = params.result.value.pubkey.parse().ok()?;
    let account_data = &params.result.value.account.data;
    let data = decode_account_data(&account_data.0, &account_data.1).ok()?;

    Some(AccountUpdate {
        pubkey,
        data,
        slot: params.result.context.slot,
    })
}

/// Parse a clock account notification message
fn parse_clock_notification(text: &str) -> Option<Clock> {
    let notification: AccountNotification = serde_json::from_str(text).ok()?;

    if notification.method.as_deref() != Some("accountNotification") {
        return None;
    }

    let params = notification.params?;
    // accountNotification value IS the account data directly (no nested .account field)
    let account_data = &params.result.value.data;
    let data = decode_account_data(&account_data.0, &account_data.1).ok()?;

    // Deserialize clock from binary data
    bincode::deserialize(&data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_program_notification() {
        let json = r#"{
            "jsonrpc": "2.0",
            "method": "programNotification",
            "params": {
                "result": {
                    "context": {"slot": 12345},
                    "value": {
                        "pubkey": "11111111111111111111111111111111",
                        "account": {
                            "data": ["", "base64"],
                            "lamports": 1000,
                            "owner": "11111111111111111111111111111111",
                            "executable": false,
                            "rentEpoch": 0
                        }
                    }
                },
                "subscription": 1
            }
        }"#;

        let result = parse_program_notification(json);
        assert!(result.is_some());

        let update = result.unwrap();
        assert_eq!(update.slot, 12345);
    }

    #[test]
    fn test_parse_non_notification() {
        let json = r#"{"jsonrpc":"2.0","result":123,"id":1}"#;
        let result = parse_program_notification(json);
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_clock_notification() {
        // Real clock notification from devnet (base64 encoded Clock sysvar)
        let json = r#"{
            "jsonrpc": "2.0",
            "method": "accountNotification",
            "params": {
                "result": {
                    "context": {"slot": 136071883},
                    "value": {
                        "lamports": 1169280,
                        "data": ["y0ocCAAAAABJVDlpAAAAAEcBAAAAAAAASAEAAAAAAADGGztpAAAAAA==", "base64"],
                        "owner": "Sysvar1111111111111111111111111111111111111",
                        "executable": false,
                        "rentEpoch": 18446744073709551615,
                        "space": 40
                    }
                },
                "subscription": 134541
            }
        }"#;

        let result = parse_clock_notification(json);
        assert!(result.is_some(), "Failed to parse clock notification");

        let clock = result.unwrap();
        assert!(clock.slot > 0, "Clock slot should be positive");
    }
}
