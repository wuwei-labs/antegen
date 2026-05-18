//! WebSocket client for Solana RPC subscriptions.
//!
//! Built on `antegen-ws` for transport + persistent reconnect with a
//! rustls-only TLS stack. This module owns the Solana-specific layer:
//! subscribe-request JSON, notification parsing, and a `wait_until`
//! helper for one-shot account watchers.

use anyhow::{anyhow, Result};
use serde::Deserialize;
use solana_sdk::pubkey::Pubkey;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub use super::response::SafeUiAccount;

/// Subscription request ID counter (per process).
static SUBSCRIPTION_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Build a `programSubscribe` request and return `(id, json)`.
pub fn build_program_subscribe_request(
    program_id: &Pubkey,
    commitment: &str,
    filters: Option<Vec<serde_json::Value>>,
) -> (u64, String) {
    let subscription_id = SUBSCRIPTION_COUNTER.fetch_add(1, Ordering::SeqCst);

    let mut params = serde_json::json!({
        "encoding": "base64+zstd",
        "commitment": commitment,
    });

    if let Some(f) = filters {
        params["filters"] = serde_json::Value::Array(f);
    }

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": subscription_id,
        "method": "programSubscribe",
        "params": [program_id.to_string(), params],
    });

    (subscription_id, request.to_string())
}

/// Build an `accountSubscribe` request and return `(id, json)`.
pub fn build_account_subscribe_request(pubkey: &Pubkey, commitment: &str) -> (u64, String) {
    let subscription_id = SUBSCRIPTION_COUNTER.fetch_add(1, Ordering::SeqCst);

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": subscription_id,
        "method": "accountSubscribe",
        "params": [
            pubkey.to_string(),
            {
                "encoding": "base64+zstd",
                "commitment": commitment,
            }
        ],
    });

    (subscription_id, request.to_string())
}

/// High-level helpers around `antegen-ws`.
pub struct WsClient;

impl WsClient {
    /// Subscribe to an account and return as soon as `predicate` accepts
    /// the latest snapshot. Aborts the underlying transport on return.
    ///
    /// The subscription is sent inside the `on_connect` callback so that
    /// reconnects re-subscribe automatically — useful if the watch outlives
    /// a network blip.
    pub async fn wait_until<F>(ws_url: &str, pubkey: &Pubkey, predicate: F) -> Result<SafeUiAccount>
    where
        F: Fn(&SafeUiAccount) -> bool,
    {
        let subscribed_pubkey = *pubkey;
        let (_id, subscribe_msg) = build_account_subscribe_request(pubkey, "confirmed");

        let mut handle = antegen_ws::WsClient::builder(ws_url)
            .map_err(|e| anyhow!("invalid ws url: {e}"))?
            .keepalive(Duration::from_secs(10))
            .on_connect(move |tx| {
                let msg = subscribe_msg.clone();
                async move {
                    let _ = tx.send_text(msg).await;
                    Ok(())
                }
            })
            .build()
            .await
            .map_err(|e| anyhow!("ws connect failed: {e}"))?;

        while let Some(msg) = handle.recv().await {
            if let antegen_ws::Message::Text(text) = msg {
                if let Some(update) = parse_notification(&text, Some(subscribed_pubkey))? {
                    if predicate(&update.account) {
                        return Ok(update.account);
                    }
                }
            }
        }

        anyhow::bail!("Account subscription closed unexpectedly")
    }
}

/// Account update parsed from a `programNotification` or `accountNotification`.
#[derive(Debug, Clone)]
pub struct WsAccountUpdate {
    pub pubkey: Pubkey,
    pub account: SafeUiAccount,
    pub slot: u64,
}

#[derive(Debug, Deserialize)]
struct WsNotification {
    method: Option<String>,
    params: Option<NotificationParams>,
}

#[derive(Debug, Deserialize)]
struct NotificationParams {
    result: NotificationResult,
    #[serde(rename = "subscription")]
    _subscription: u64,
}

#[derive(Debug, Deserialize)]
struct NotificationResult {
    context: NotificationContext,
    value: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct NotificationContext {
    slot: u64,
}

#[derive(Debug, Deserialize)]
struct ProgramNotificationValue {
    pubkey: String,
    account: SafeUiAccount,
}

/// Parse a `programNotification` or `accountNotification` message.
///
/// `accountNotification` doesn't include the pubkey in the response, so
/// callers must supply the subscribed pubkey via `subscribed_pubkey`.
pub fn parse_notification(
    text: &str,
    subscribed_pubkey: Option<Pubkey>,
) -> Result<Option<WsAccountUpdate>> {
    let notification: WsNotification = serde_json::from_str(text)?;

    let method = match notification.method {
        Some(m) => m,
        None => return Ok(None),
    };

    let params = notification
        .params
        .ok_or_else(|| anyhow!("Missing params"))?;

    let slot = params.result.context.slot;

    match method.as_str() {
        "programNotification" => {
            let value: ProgramNotificationValue = serde_json::from_value(params.result.value)
                .map_err(|e| anyhow!("Failed to parse programNotification value: {}", e))?;
            let pubkey: Pubkey = value
                .pubkey
                .parse()
                .map_err(|e| anyhow!("Invalid pubkey: {}", e))?;
            Ok(Some(WsAccountUpdate {
                pubkey,
                account: value.account,
                slot,
            }))
        }
        "accountNotification" => {
            let account: SafeUiAccount = serde_json::from_value(params.result.value)
                .map_err(|e| anyhow!("Failed to parse accountNotification value: {}", e))?;
            let pubkey = subscribed_pubkey.unwrap_or_default();
            Ok(Some(WsAccountUpdate {
                pubkey,
                account,
                slot,
            }))
        }
        _ => Ok(None),
    }
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
                            "lamports": 1000,
                            "data": ["", "base64"],
                            "owner": "11111111111111111111111111111111",
                            "executable": false,
                            "rentEpoch": 0
                        }
                    }
                },
                "subscription": 1
            }
        }"#;

        let result = parse_notification(json, None).unwrap();
        assert!(result.is_some());

        let update = result.unwrap();
        assert_eq!(update.slot, 12345);
        assert_eq!(update.account.lamports, 1000);
    }

    #[test]
    fn test_parse_account_notification() {
        let json = r#"{
            "jsonrpc": "2.0",
            "method": "accountNotification",
            "params": {
                "result": {
                    "context": {"slot": 67890},
                    "value": {
                        "lamports": 2000000,
                        "data": ["", "base64"],
                        "owner": "11111111111111111111111111111111",
                        "executable": false,
                        "rentEpoch": 0
                    }
                },
                "subscription": 5
            }
        }"#;

        let subscribed_pubkey: Pubkey = "H15fAqeJu1REwy7WrqMeRSgs7q3GDsi3WTeN8ZvwgVJb"
            .parse()
            .unwrap();

        let result = parse_notification(json, Some(subscribed_pubkey)).unwrap();
        assert!(result.is_some());

        let update = result.unwrap();
        assert_eq!(update.slot, 67890);
        assert_eq!(update.account.lamports, 2000000);
        assert_eq!(update.pubkey, subscribed_pubkey);
    }

    #[test]
    fn test_parse_subscription_response() {
        let json = r#"{"jsonrpc":"2.0","result":123,"id":1}"#;
        let result = parse_notification(json, None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_with_u64_max_rent_epoch() {
        let json = r#"{
            "jsonrpc": "2.0",
            "method": "programNotification",
            "params": {
                "result": {
                    "context": {"slot": 99999},
                    "value": {
                        "pubkey": "11111111111111111111111111111111",
                        "account": {
                            "lamports": 5000,
                            "data": ["dGVzdA==", "base64"],
                            "owner": "11111111111111111111111111111111",
                            "executable": false,
                            "rentEpoch": 1.8446744073709552e19
                        }
                    }
                },
                "subscription": 42
            }
        }"#;

        let result = parse_notification(json, None).unwrap();
        assert!(result.is_some());

        let update = result.unwrap();
        assert_eq!(update.account.rent_epoch, u64::MAX);
    }
}
