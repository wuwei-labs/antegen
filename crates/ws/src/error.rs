use thiserror::Error;
use tokio_tungstenite::tungstenite::Error as TungsteniteError;

/// Errors returned from [`WsClientBuilder::build`](crate::WsClientBuilder::build).
///
/// Transport errors that occur *after* a successful initial connect do not
/// flow through `Error` — they surface as [`Event::Disconnected`](crate::Event)
/// so the persistent task can keep retrying.
#[derive(Debug, Error)]
pub enum Error {
    #[error("initial connect failed: {0}")]
    InitialConnect(#[from] TungsteniteError),

    #[error("config: {0}")]
    Config(String),
}

/// Returned by [`Sender::send`](crate::Sender::send) when the transport task
/// has shut down (either via [`WsHandle::abort`](crate::WsHandle::abort) or
/// because the user dropped the handle).
#[derive(Debug, Error)]
#[error("websocket transport closed")]
pub struct SendError;
