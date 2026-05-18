use std::time::Duration;
use tokio_tungstenite::tungstenite::Error as TungsteniteError;

/// Lifecycle events emitted by the persistent transport task.
///
/// Events are delivered on a dedicated channel so callers can observe
/// reconnect activity without polluting the message stream. Reading them
/// is optional — if the events receiver is never drained the task drops
/// new events silently.
#[derive(Debug)]
pub enum Event {
    /// A new underlying socket was established. Fires once on the initial
    /// connect *and* on every successful reconnect.
    Connected,

    /// The underlying socket closed. `reason` describes whether it was a
    /// clean close, a transport error, or a server-initiated close frame.
    Disconnected { reason: DisconnectReason },

    /// Persistent mode is about to retry after `next_delay`. `attempt`
    /// starts at 1 for the first retry after a successful connection.
    Reconnecting { attempt: u64, next_delay: Duration },
}

#[derive(Debug)]
pub enum DisconnectReason {
    /// Server sent a Close frame.
    ServerClose,
    /// Transport-level error (broken pipe, TLS error, etc.).
    Transport(TungsteniteError),
    /// The transport task is shutting down because the user dropped or
    /// aborted the handle.
    Shutdown,
}
