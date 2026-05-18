//! Async WebSocket client with optional persistent reconnect, rustls-only.
//!
//! Designed as a drop-in for `pws` with the API rebuilt around the actual
//! usage patterns in `antegen-client`. The reconnect state machine is a
//! clean-room rewrite over `tokio-tungstenite` 0.28 with rustls; credit
//! `rellfy/pws` for the original design.
//!
//! # Example
//!
//! ```no_run
//! use antegen_ws::{Message, WsClient};
//! use std::time::Duration;
//!
//! # async fn run() -> Result<(), antegen_ws::Error> {
//! let mut handle = WsClient::builder("wss://example.com/socket")?
//!     .keepalive(Duration::from_secs(10))
//!     .on_connect(|tx| async move {
//!         tx.send_text(r#"{"jsonrpc":"2.0","method":"subscribe"}"#).await
//!     })
//!     .build()
//!     .await?;
//!
//! while let Some(msg) = handle.recv().await {
//!     if let Message::Text(text) = msg {
//!         println!("{text}");
//!     }
//! }
//! # Ok(())
//! # }
//! ```

mod backoff;
mod builder;
mod error;
mod event;
mod message;
mod sender;

#[cfg(feature = "persistent")]
mod persistent;

pub use backoff::Backoff;
pub use builder::WsClientBuilder;
pub use error::{Error, SendError};
pub use event::{DisconnectReason, Event};
pub use message::{CloseFrame, Message};
pub use sender::Sender;
pub use url::Url;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub(crate) const DEFAULT_CAPACITY: usize = 32;

/// Entry point for building a WebSocket client.
///
/// `WsClient` itself is not instantiable — use [`WsClient::builder`] to
/// obtain a [`WsClientBuilder`].
pub struct WsClient {
    _private: (),
}

impl WsClient {
    /// Construct a builder targeting `url`. Returns
    /// [`Error::Config`](crate::Error::Config) if `url` doesn't parse.
    pub fn builder(url: impl AsRef<str>) -> Result<WsClientBuilder, Error> {
        let url =
            Url::parse(url.as_ref()).map_err(|e| Error::Config(format!("invalid url: {e}")))?;
        Ok(WsClientBuilder::new(url))
    }
}

/// Handle to a running WebSocket transport task.
///
/// Owns the inbound message receiver, the events receiver, a clone-able
/// [`Sender`], and the task's [`JoinHandle`]. Dropping the handle detaches
/// the task (tokio's default); call [`WsHandle::abort`] for explicit
/// cancellation.
pub struct WsHandle {
    sender: Sender,
    messages: mpsc::Receiver<Message>,
    events: mpsc::Receiver<Event>,
    task: JoinHandle<()>,
}

impl WsHandle {
    /// Clone-able handle for sending messages.
    pub fn sender(&self) -> Sender {
        self.sender.clone()
    }

    /// Receive the next inbound message. Returns `None` when the
    /// transport task has exited.
    pub async fn recv(&mut self) -> Option<Message> {
        self.messages.recv().await
    }

    /// Receive the next lifecycle event (Connected, Disconnected,
    /// Reconnecting). Returns `None` when the transport task has exited.
    pub async fn next_event(&mut self) -> Option<Event> {
        self.events.recv().await
    }

    /// True once the transport task has exited (either via [`abort`] or
    /// because the user dropped the [`Sender`] and the pump cleaned up).
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// Abort the transport task immediately. Any pending sends are
    /// discarded; the inbound and events receivers will yield `None` on
    /// their next `recv`.
    pub fn abort(self) {
        self.task.abort();
    }

    /// Split the handle into its parts. Useful when you want to move the
    /// sender into one task and the receivers into another.
    pub fn into_split(
        self,
    ) -> (
        Sender,
        mpsc::Receiver<Message>,
        mpsc::Receiver<Event>,
        JoinHandle<()>,
    ) {
        (self.sender, self.messages, self.events, self.task)
    }
}
