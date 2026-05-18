use tokio_tungstenite::tungstenite::Message as Tungstenite;

pub use tokio_tungstenite::tungstenite::protocol::CloseFrame;

/// Application-level WebSocket message.
///
/// Ping/Pong frames are handled inside the transport task (auto-pong when
/// the `pong` feature is enabled) and never surface to the caller. The
/// `Frame` variant from `tungstenite` is intentionally not exposed.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Message {
    Text(String),
    Binary(Vec<u8>),
    Close(Option<CloseFrame>),
}

impl Message {
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text(s.into())
    }

    pub fn binary(b: impl Into<Vec<u8>>) -> Self {
        Self::Binary(b.into())
    }

    pub(crate) fn into_tungstenite(self) -> Tungstenite {
        match self {
            Self::Text(s) => Tungstenite::Text(s.into()),
            Self::Binary(b) => Tungstenite::Binary(b.into()),
            Self::Close(f) => Tungstenite::Close(f),
        }
    }
}
