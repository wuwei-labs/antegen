//! Persistent WebSocket transport task.
//!
//! Owns one tokio task that connects, pumps messages, and reconnects on
//! any disconnect. Lifecycle events surface on the event channel so the
//! caller can observe attempt counts, delays, and disconnect reasons.

use crate::backoff::Backoff;
use crate::builder::OnConnectFn;
use crate::error::Error;
use crate::event::{DisconnectReason, Event};
use crate::message::Message;
use crate::sender::Sender;
use crate::WsHandle;

use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message as Tungstenite;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use url::Url;

type Sock = WebSocketStream<MaybeTlsStream<TcpStream>>;
type Sink = SplitSink<Sock, Tungstenite>;

pub(crate) struct Spawn {
    pub url: Url,
    pub keepalive: Option<Duration>,
    pub backoff: Backoff,
    pub capacity: usize,
    pub on_connect: Option<OnConnectFn>,
}

pub(crate) async fn spawn(s: Spawn) -> Result<WsHandle, Error> {
    let Spawn {
        url,
        keepalive,
        backoff,
        capacity,
        on_connect,
    } = s;

    let (out_tx, out_rx) = mpsc::channel::<Message>(capacity);
    let (in_tx, in_rx) = mpsc::channel::<Message>(capacity);
    let (evt_tx, evt_rx) = mpsc::channel::<Event>(capacity);
    let (first_tx, first_rx) = oneshot::channel::<Result<(), Error>>();

    let sender = Sender::new(out_tx);
    let task_sender = sender.clone();

    let task = tokio::spawn(async move {
        run(Inner {
            url,
            keepalive,
            backoff,
            out_rx,
            in_tx,
            evt_tx,
            first_tx: Some(first_tx),
            on_connect,
            sender: task_sender,
        })
        .await;
    });

    match first_rx.await {
        Ok(Ok(())) => Ok(WsHandle {
            sender,
            messages: in_rx,
            events: evt_rx,
            task,
        }),
        Ok(Err(e)) => {
            task.abort();
            Err(e)
        }
        Err(_) => {
            task.abort();
            Err(Error::Config(
                "transport task exited before first connect".into(),
            ))
        }
    }
}

struct Inner {
    url: Url,
    keepalive: Option<Duration>,
    backoff: Backoff,
    out_rx: mpsc::Receiver<Message>,
    in_tx: mpsc::Sender<Message>,
    evt_tx: mpsc::Sender<Event>,
    first_tx: Option<oneshot::Sender<Result<(), Error>>>,
    on_connect: Option<OnConnectFn>,
    sender: Sender,
}

async fn run(mut s: Inner) {
    let mut attempt: u64 = 0;

    loop {
        let sock = match connect_async(s.url.as_str()).await {
            Ok((sock, _)) => sock,
            Err(e) => {
                if let Some(tx) = s.first_tx.take() {
                    let _ = tx.send(Err(Error::InitialConnect(e)));
                    return;
                }
                emit(
                    &s.evt_tx,
                    Event::Disconnected {
                        reason: DisconnectReason::Transport(e),
                    },
                )
                .await;
                attempt += 1;
                let delay = s.backoff.delay(attempt);
                emit(
                    &s.evt_tx,
                    Event::Reconnecting {
                        attempt,
                        next_delay: delay,
                    },
                )
                .await;
                tokio::time::sleep(delay).await;
                continue;
            }
        };

        if let Some(tx) = s.first_tx.take() {
            let _ = tx.send(Ok(()));
        }

        emit(&s.evt_tx, Event::Connected).await;
        attempt = 0;

        if let Some(cb) = s.on_connect.as_mut() {
            if cb(s.sender.clone()).await.is_err() {
                emit(
                    &s.evt_tx,
                    Event::Disconnected {
                        reason: DisconnectReason::Shutdown,
                    },
                )
                .await;
                return;
            }
        }

        let reason = pump(sock, &mut s.out_rx, &s.in_tx, s.keepalive).await;
        let is_shutdown = matches!(reason, DisconnectReason::Shutdown);
        emit(&s.evt_tx, Event::Disconnected { reason }).await;
        if is_shutdown {
            return;
        }

        attempt += 1;
        let delay = s.backoff.delay(attempt);
        emit(
            &s.evt_tx,
            Event::Reconnecting {
                attempt,
                next_delay: delay,
            },
        )
        .await;
        tokio::time::sleep(delay).await;
    }
}

async fn pump(
    sock: Sock,
    out_rx: &mut mpsc::Receiver<Message>,
    in_tx: &mpsc::Sender<Message>,
    keepalive: Option<Duration>,
) -> DisconnectReason {
    let (mut write, mut read) = sock.split();
    let mut interval = keepalive.map(tokio::time::interval);
    // First interval tick is immediate; skip it so we don't ping before the
    // server has had a chance to settle.
    if let Some(i) = interval.as_mut() {
        i.tick().await;
    }

    loop {
        tokio::select! {
            biased;

            maybe_out = out_rx.recv() => {
                let Some(msg) = maybe_out else {
                    let _ = write.send(Tungstenite::Close(None)).await;
                    return DisconnectReason::Shutdown;
                };
                if let Err(e) = write.send(msg.into_tungstenite()).await {
                    return DisconnectReason::Transport(e);
                }
            }

            maybe_in = read.next() => {
                let Some(item) = maybe_in else {
                    return DisconnectReason::Transport(
                        tokio_tungstenite::tungstenite::Error::ConnectionClosed,
                    );
                };
                match item {
                    Ok(msg) => {
                        if let Some(reason) = handle_inbound(msg, &mut write, in_tx).await {
                            return reason;
                        }
                    }
                    Err(e) => return DisconnectReason::Transport(e),
                }
            }

            _ = tick(interval.as_mut()) => {
                if let Err(e) = write.send(Tungstenite::Ping(Vec::new().into())).await {
                    return DisconnectReason::Transport(e);
                }
            }
        }
    }
}

async fn handle_inbound(
    msg: Tungstenite,
    write: &mut Sink,
    in_tx: &mpsc::Sender<Message>,
) -> Option<DisconnectReason> {
    match msg {
        Tungstenite::Text(s) => {
            if in_tx.send(Message::Text(s.to_string())).await.is_err() {
                return Some(DisconnectReason::Shutdown);
            }
            None
        }
        Tungstenite::Binary(b) => {
            if in_tx.send(Message::Binary(b.into())).await.is_err() {
                return Some(DisconnectReason::Shutdown);
            }
            None
        }
        Tungstenite::Ping(p) => {
            #[cfg(feature = "pong")]
            {
                if let Err(e) = write.send(Tungstenite::Pong(p)).await {
                    return Some(DisconnectReason::Transport(e));
                }
            }
            #[cfg(not(feature = "pong"))]
            {
                let _ = (write, p);
            }
            None
        }
        Tungstenite::Pong(_) => None,
        Tungstenite::Close(frame) => {
            let _ = in_tx.send(Message::Close(frame)).await;
            Some(DisconnectReason::ServerClose)
        }
        Tungstenite::Frame(_) => None,
    }
}

async fn tick(interval: Option<&mut tokio::time::Interval>) {
    match interval {
        Some(i) => {
            i.tick().await;
        }
        None => std::future::pending().await,
    }
}

async fn emit(tx: &mpsc::Sender<Event>, evt: Event) {
    let _ = tx.try_send(evt);
}
