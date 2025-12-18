//! WebSocket client for Solana RPC subscriptions
//!
//! Provides auto-reconnecting WebSocket connections using pws (persistent WebSocket).
//!
//! # Usage
//!
//! ## One-shot waiting (e.g., wait for balance)
//! ```ignore
//! let account = WsClient::wait_until(ws_url, &pubkey, |acc| acc.lamports >= 500_000).await?;
//! ```
//!
//! ## Continuous subscription stream
//! ```ignore
//! let (handle, mut rx) = WsClient::subscribe_account_stream(ws_url, &pubkey, "confirmed").await?;
//! while let Some(update) = rx.recv().await {
//!     // Process update
//! }
//! ```

use anyhow::{anyhow, Result};
pub use pws::Message as WsMessage;
use pws::{connect_persistent_websocket_async, Message, Url, WsMessageReceiver, WsMessageSender};
use serde::Deserialize;
use solana_sdk::pubkey::Pubkey;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;

pub use super::response::SafeUiAccount;

/// Subscription ID counter
static SUBSCRIPTION_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Handle to manage a WebSocket subscription lifecycle
pub struct WsHandle {
    handle: tokio::task::JoinHandle<()>,
}

impl WsHandle {
    /// Abort the subscription task
    pub fn abort(self) {
        self.handle.abort();
    }

    /// Check if the subscription task is finished
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
}

/// WebSocket client for Solana RPC subscriptions
///
/// Provides both one-shot helpers (like `wait_until`) and continuous subscription
/// streams for long-running subscriptions.
pub struct WsClient {
    /// WebSocket URL
    ws_url: String,
    /// Message sender
    sender: Option<WsMessageSender>,
    /// Message receiver
    receiver: Option<WsMessageReceiver>,
}

/// Backwards compatibility alias
#[deprecated(since = "3.0.1", note = "Use WsClient instead")]
pub type WsSubscriptionManager = WsClient;

impl WsClient {
    /// Create a new WebSocket client
    pub fn new(ws_url: impl Into<String>) -> Self {
        Self {
            ws_url: ws_url.into(),
            sender: None,
            receiver: None,
        }
    }

    // =========================================================================
    // High-level helpers (static methods)
    // =========================================================================

    /// Wait until account satisfies the given predicate
    ///
    /// This is a one-shot helper that connects, subscribes, waits for the
    /// predicate to return true, then returns.
    ///
    /// # Example
    /// ```ignore
    /// let account = WsClient::wait_until(
    ///     ws_url,
    ///     &pubkey,
    ///     |acc| acc.lamports >= 500_000
    /// ).await?;
    /// ```
    pub async fn wait_until<F>(
        ws_url: &str,
        pubkey: &Pubkey,
        predicate: F,
    ) -> Result<SafeUiAccount>
    where
        F: Fn(&SafeUiAccount) -> bool,
    {
        let (handle, mut rx) = Self::subscribe_account_stream(ws_url, pubkey, "confirmed").await?;

        while let Some(update) = rx.recv().await {
            if predicate(&update.account) {
                handle.abort();
                return Ok(update.account);
            }
        }

        anyhow::bail!("Account subscription closed unexpectedly")
    }

    /// Subscribe to account changes and return a stream
    ///
    /// Returns a handle to manage the subscription and a receiver for updates.
    ///
    /// # Example
    /// ```ignore
    /// let (handle, mut rx) = WsClient::subscribe_account_stream(
    ///     ws_url, &pubkey, "confirmed"
    /// ).await?;
    ///
    /// while let Some(update) = rx.recv().await {
    ///     println!("Account updated: {} lamports", update.account.lamports);
    /// }
    /// ```
    pub async fn subscribe_account_stream(
        ws_url: &str,
        pubkey: &Pubkey,
        commitment: &str,
    ) -> Result<(WsHandle, mpsc::Receiver<WsAccountUpdate>)> {
        let mut client = Self::new(ws_url);
        client.connect().await?;
        client.subscribe_account(pubkey, commitment).await?;

        let (tx, rx) = mpsc::channel(32);
        // Pass pubkey since accountNotification doesn't include it in the response
        let handle = client.start_receiver(tx, Some(*pubkey));

        Ok((WsHandle { handle }, rx))
    }

    /// Subscribe to program account changes and return a stream
    ///
    /// Returns a handle to manage the subscription and a receiver for updates.
    /// Optionally accepts filters (e.g., memcmp for discriminator filtering).
    ///
    /// # Example
    /// ```ignore
    /// let filters = vec![serde_json::json!({
    ///     "memcmp": { "offset": 0, "bytes": "..." }
    /// })];
    ///
    /// let (handle, mut rx) = WsClient::subscribe_program_stream(
    ///     ws_url, &program_id, "confirmed", Some(filters)
    /// ).await?;
    /// ```
    pub async fn subscribe_program_stream(
        ws_url: &str,
        program_id: &Pubkey,
        commitment: &str,
        filters: Option<Vec<serde_json::Value>>,
    ) -> Result<(WsHandle, mpsc::Receiver<WsAccountUpdate>)> {
        let mut client = Self::new(ws_url);
        client.connect().await?;
        client.subscribe_program_with_filters(program_id, commitment, filters).await?;

        let (tx, rx) = mpsc::channel(32);
        // programNotification includes pubkey in the response, so pass None
        let handle = client.start_receiver(tx, None);

        Ok((WsHandle { handle }, rx))
    }

    // =========================================================================
    // Low-level helpers (for advanced use cases like RpcSubscription)
    // =========================================================================

    /// Connect to WebSocket and return raw sender/receiver
    ///
    /// Use this for advanced cases that need direct message handling
    /// (e.g., reconnection events, custom message processing).
    pub async fn connect_raw(ws_url: &str) -> Result<(WsMessageSender, WsMessageReceiver)> {
        let url: Url = ws_url.parse().map_err(|e| anyhow!("Invalid URL: {}", e))?;

        let (sender, receiver) = connect_persistent_websocket_async(url)
            .await
            .map_err(|e| anyhow!("WebSocket connection failed: {}", e))?;

        log::debug!("WebSocket connected to {}", ws_url);
        Ok((sender, receiver))
    }

    /// Build a program subscription request JSON
    pub fn build_program_subscribe_request(
        program_id: &Pubkey,
        commitment: &str,
        filters: Option<Vec<serde_json::Value>>,
    ) -> (u64, String) {
        let subscription_id = SUBSCRIPTION_COUNTER.fetch_add(1, Ordering::SeqCst);

        let mut params = serde_json::json!({
            "encoding": "base64+zstd",
            "commitment": commitment
        });

        if let Some(f) = filters {
            params["filters"] = serde_json::Value::Array(f);
        }

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": subscription_id,
            "method": "programSubscribe",
            "params": [program_id.to_string(), params]
        });

        (subscription_id, request.to_string())
    }

    /// Build an account subscription request JSON
    pub fn build_account_subscribe_request(
        pubkey: &Pubkey,
        commitment: &str,
    ) -> (u64, String) {
        let subscription_id = SUBSCRIPTION_COUNTER.fetch_add(1, Ordering::SeqCst);

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": subscription_id,
            "method": "accountSubscribe",
            "params": [
                pubkey.to_string(),
                {
                    "encoding": "base64+zstd",
                    "commitment": commitment
                }
            ]
        });

        (subscription_id, request.to_string())
    }

    // =========================================================================
    // Instance methods (used internally by high-level helpers)
    // =========================================================================

    /// Connect to the WebSocket endpoint
    async fn connect(&mut self) -> Result<()> {
        let (sender, receiver) = Self::connect_raw(&self.ws_url).await?;
        self.sender = Some(sender);
        self.receiver = Some(receiver);
        Ok(())
    }

    /// Subscribe to program account changes
    pub async fn subscribe_program(
        &self,
        program_id: &Pubkey,
        commitment: &str,
    ) -> Result<u64> {
        self.subscribe_program_with_filters(program_id, commitment, None).await
    }

    /// Subscribe to program account changes with optional filters
    pub async fn subscribe_program_with_filters(
        &self,
        program_id: &Pubkey,
        commitment: &str,
        filters: Option<Vec<serde_json::Value>>,
    ) -> Result<u64> {
        let sender = self.sender.as_ref().ok_or_else(|| anyhow!("Not connected"))?;

        let subscription_id = SUBSCRIPTION_COUNTER.fetch_add(1, Ordering::SeqCst);

        let mut params = serde_json::json!({
            "encoding": "base64+zstd",
            "commitment": commitment
        });

        if let Some(f) = filters {
            params["filters"] = serde_json::Value::Array(f);
        }

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": subscription_id,
            "method": "programSubscribe",
            "params": [
                program_id.to_string(),
                params
            ]
        });

        sender
            .send(Message::Text(request.to_string()))
            .await
            .map_err(|e| anyhow!("Failed to send subscription request: {}", e))?;

        log::debug!(
            "Subscribed to program {} with id {}",
            program_id,
            subscription_id
        );

        Ok(subscription_id)
    }

    /// Subscribe to account changes
    pub async fn subscribe_account(
        &self,
        pubkey: &Pubkey,
        commitment: &str,
    ) -> Result<u64> {
        let sender = self.sender.as_ref().ok_or_else(|| anyhow!("Not connected"))?;

        let subscription_id = SUBSCRIPTION_COUNTER.fetch_add(1, Ordering::SeqCst);

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": subscription_id,
            "method": "accountSubscribe",
            "params": [
                pubkey.to_string(),
                {
                    "encoding": "base64+zstd",
                    "commitment": commitment
                }
            ]
        });

        sender
            .send(Message::Text(request.to_string()))
            .await
            .map_err(|e| anyhow!("Failed to send subscription request: {}", e))?;

        log::debug!(
            "Subscribed to account {} with id {}",
            pubkey,
            subscription_id
        );

        Ok(subscription_id)
    }

    /// Unsubscribe from a program subscription
    pub async fn unsubscribe_program(&self, subscription_id: u64) -> Result<()> {
        let sender = self.sender.as_ref().ok_or_else(|| anyhow!("Not connected"))?;

        let request_id = SUBSCRIPTION_COUNTER.fetch_add(1, Ordering::SeqCst);

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "programUnsubscribe",
            "params": [subscription_id]
        });

        sender
            .send(Message::Text(request.to_string()))
            .await
            .map_err(|e| anyhow!("Failed to send unsubscribe request: {}", e))?;

        Ok(())
    }

    /// Unsubscribe from an account subscription
    pub async fn unsubscribe_account(&self, subscription_id: u64) -> Result<()> {
        let sender = self.sender.as_ref().ok_or_else(|| anyhow!("Not connected"))?;

        let request_id = SUBSCRIPTION_COUNTER.fetch_add(1, Ordering::SeqCst);

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "accountUnsubscribe",
            "params": [subscription_id]
        });

        sender
            .send(Message::Text(request.to_string()))
            .await
            .map_err(|e| anyhow!("Failed to send unsubscribe request: {}", e))?;

        Ok(())
    }

    /// Start receiving messages and forward to a channel
    ///
    /// For account subscriptions, pass the subscribed pubkey so it can be
    /// included in the update (accountNotification doesn't include pubkey).
    pub fn start_receiver(
        mut self,
        update_tx: mpsc::Sender<WsAccountUpdate>,
        subscribed_pubkey: Option<Pubkey>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut receiver = match self.receiver.take() {
                Some(r) => r,
                None => {
                    log::error!("No receiver available");
                    return;
                }
            };

            loop {
                match receiver.recv().await {
                    Ok(msg) => {
                        if let Message::Text(text) = msg {
                            match parse_notification(&text, subscribed_pubkey) {
                                Ok(Some(update)) => {
                                    if update_tx.send(update).await.is_err() {
                                        log::info!("Update channel closed, stopping receiver");
                                        break;
                                    }
                                }
                                Ok(None) => {
                                    // Not a notification (could be subscription confirmation)
                                    log::trace!("Non-notification message: {}", &text[..text.len().min(200)]);
                                }
                                Err(e) => {
                                    log::warn!("Failed to parse message: {} - {}", e, &text[..text.len().min(200)]);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("WebSocket receive error: {}", e);
                        // pws handles reconnection automatically
                    }
                }
            }
        })
    }
}

/// Account update from WebSocket notification
#[derive(Debug, Clone)]
pub struct WsAccountUpdate {
    /// The account pubkey
    pub pubkey: Pubkey,
    /// The account data
    pub account: SafeUiAccount,
    /// Slot when the update occurred
    pub slot: u64,
}

/// WebSocket notification wrapper
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
    value: serde_json::Value, // Parse based on method type
}

#[derive(Debug, Deserialize)]
struct NotificationContext {
    slot: u64,
}

/// For programNotification - has pubkey + account wrapper
#[derive(Debug, Deserialize)]
struct ProgramNotificationValue {
    pubkey: String,
    account: SafeUiAccount,
}

// For accountNotification - account data IS the value directly (use SafeUiAccount)

/// Parse a WebSocket notification message
///
/// For `accountNotification`, the pubkey is not included in the response,
/// so `subscribed_pubkey` must be provided to fill it in.
fn parse_notification(
    text: &str,
    subscribed_pubkey: Option<Pubkey>,
) -> Result<Option<WsAccountUpdate>> {
    let notification: WsNotification = serde_json::from_str(text)?;

    // Check if this is a notification
    let method = match notification.method {
        Some(m) => m,
        None => return Ok(None), // Not a notification
    };

    let params = notification
        .params
        .ok_or_else(|| anyhow!("Missing params"))?;

    let slot = params.result.context.slot;

    match method.as_str() {
        "programNotification" => {
            // Has pubkey + account wrapper
            let value: ProgramNotificationValue =
                serde_json::from_value(params.result.value)
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
            // Account data IS the value directly (no pubkey wrapper)
            let account: SafeUiAccount =
                serde_json::from_value(params.result.value)
                    .map_err(|e| anyhow!("Failed to parse accountNotification value: {}", e))?;
            // Use the subscribed pubkey since accountNotification doesn't include it
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

        // programNotification includes pubkey, so subscribed_pubkey is not needed
        let result = parse_notification(json, None).unwrap();
        assert!(result.is_some());

        let update = result.unwrap();
        assert_eq!(update.slot, 12345);
        assert_eq!(update.account.lamports, 1000);
    }

    #[test]
    fn test_parse_account_notification() {
        // accountNotification has account data directly in value (no pubkey wrapper)
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
        assert!(result.is_none()); // Not a notification
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
