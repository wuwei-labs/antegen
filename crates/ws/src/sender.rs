use crate::error::SendError;
use crate::message::Message;
use tokio::sync::mpsc;

/// Clone-able handle for sending messages to the WebSocket task.
///
/// Backpressure model: the underlying channel is bounded; `send` awaits
/// available capacity rather than dropping messages. This is the intended
/// fix for the silent `broadcast::Lagged` drops that pws's design suffered
/// from — callers feel slowness, not lost messages.
#[derive(Debug, Clone)]
pub struct Sender {
    tx: mpsc::Sender<Message>,
}

impl Sender {
    pub(crate) fn new(tx: mpsc::Sender<Message>) -> Self {
        Self { tx }
    }

    pub async fn send(&self, msg: Message) -> Result<(), SendError> {
        self.tx.send(msg).await.map_err(|_| SendError)
    }

    pub async fn send_text(&self, text: impl Into<String>) -> Result<(), SendError> {
        self.send(Message::Text(text.into())).await
    }

    pub async fn send_binary(&self, bytes: impl Into<Vec<u8>>) -> Result<(), SendError> {
        self.send(Message::Binary(bytes.into())).await
    }

    /// Try to send without awaiting. Returns `Err(SendError)` if the
    /// channel is full or the transport task has stopped.
    pub fn try_send(&self, msg: Message) -> Result<(), SendError> {
        self.tx.try_send(msg).map_err(|_| SendError)
    }
}
