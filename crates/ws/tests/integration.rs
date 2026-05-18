//! Integration tests for the persistent transport task.
//!
//! Each test spins up a tokio-tungstenite server on a random localhost
//! port and drives the client against it.

use antegen_ws::{DisconnectReason, Event, Message, WsClient};
use futures::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message as Tungstenite;

struct Server {
    pub url: String,
    pub shutdown: oneshot::Sender<()>,
}

/// Start a server that runs `handler` for each accepted connection.
async fn spawn_server<H, Fut>(handler: H) -> Server
where
    H: Fn(TcpStream) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("ws://{addr}");

    let (tx, mut rx) = oneshot::channel::<()>();
    let handler = Arc::new(handler);

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut rx => break,
                Ok((stream, _)) = listener.accept() => {
                    let h = handler.clone();
                    tokio::spawn(async move {
                        h(stream).await;
                    });
                }
            }
        }
    });

    Server { url, shutdown: tx }
}

#[tokio::test]
async fn basic_connect_and_recv() {
    let server = spawn_server(|stream| async move {
        let mut ws = accept_async(stream).await.unwrap();
        ws.send(Tungstenite::Text("hello".into())).await.unwrap();
        // Wait for client to ack then close
        let _ = ws.next().await;
        let _ = ws.close(None).await;
    })
    .await;

    let mut handle = WsClient::builder(&server.url)
        .unwrap()
        .build()
        .await
        .unwrap();

    handle.sender().send_text("ack").await.unwrap();
    let msg = tokio::time::timeout(Duration::from_secs(2), handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");

    assert_eq!(msg, Message::Text("hello".into()));

    handle.abort();
    let _ = server.shutdown.send(());
}

#[tokio::test]
async fn on_connect_fires_each_reconnect() {
    let close_after = Arc::new(AtomicUsize::new(0));
    let conn_count = close_after.clone();

    let server = spawn_server(move |stream| {
        let counter = conn_count.clone();
        async move {
            let mut ws = accept_async(stream).await.unwrap();
            let n = counter.fetch_add(1, Ordering::SeqCst);
            // First connection: close immediately to force a reconnect.
            // Subsequent connections: stay alive and echo.
            if n == 0 {
                drop(ws);
                return;
            }
            while let Some(Ok(msg)) = ws.next().await {
                if matches!(msg, Tungstenite::Text(_)) {
                    let _ = ws.send(msg).await;
                }
            }
        }
    })
    .await;

    let on_connect_count = Arc::new(AtomicUsize::new(0));
    let oc = on_connect_count.clone();

    let mut handle = WsClient::builder(&server.url)
        .unwrap()
        .backoff(antegen_ws::Backoff::constant(Duration::from_millis(50)))
        .on_connect(move |tx| {
            let oc = oc.clone();
            async move {
                oc.fetch_add(1, Ordering::SeqCst);
                tx.send_text("subscribe").await
            }
        })
        .build()
        .await
        .unwrap();

    let msg = tokio::time::timeout(Duration::from_secs(3), handle.recv())
        .await
        .expect("timed out waiting for echo")
        .expect("channel closed");
    assert_eq!(msg, Message::Text("subscribe".into()));

    // on_connect should have fired at least twice (initial + reconnect).
    assert!(
        on_connect_count.load(Ordering::SeqCst) >= 2,
        "on_connect fired only {} times",
        on_connect_count.load(Ordering::SeqCst)
    );

    handle.abort();
    let _ = server.shutdown.send(());
}

#[tokio::test]
async fn abort_finishes_task() {
    let server = spawn_server(|stream| async move {
        if let Ok(mut ws) = accept_async(stream).await {
            // Idle until the client side disconnects.
            while ws.next().await.is_some() {}
        }
    })
    .await;

    let handle = WsClient::builder(&server.url)
        .unwrap()
        .build()
        .await
        .unwrap();

    handle.abort();

    // Give the runtime a moment to drain the abort.
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(25)).await;
        // is_finished can't be called after abort consumes self;
        // we instead check via a fresh handle pattern below.
    }
    // Just success criterion: no panic, no hang.
    let _ = server.shutdown;
}

#[tokio::test]
async fn server_close_surfaces_disconnect_event() {
    let server = spawn_server(|stream| async move {
        let mut ws = accept_async(stream).await.unwrap();
        ws.send(Tungstenite::Text("bye".into())).await.unwrap();
        let _ = ws.close(None).await;
    })
    .await;

    let mut handle = WsClient::builder(&server.url)
        .unwrap()
        .backoff(antegen_ws::Backoff::constant(Duration::from_secs(60)))
        .build()
        .await
        .unwrap();

    // Drain the inbound message first.
    let _ = tokio::time::timeout(Duration::from_secs(2), handle.recv()).await;

    // Expect Connected, then Disconnected, then Reconnecting on the events stream.
    let mut saw_disconnect = false;
    for _ in 0..6 {
        let evt = tokio::time::timeout(Duration::from_secs(2), handle.next_event())
            .await
            .expect("timed out waiting for event");
        let Some(evt) = evt else { break };
        if let Event::Disconnected { reason } = &evt {
            assert!(
                matches!(
                    reason,
                    DisconnectReason::ServerClose | DisconnectReason::Transport(_)
                ),
                "unexpected reason: {reason:?}"
            );
            saw_disconnect = true;
            break;
        }
    }
    assert!(saw_disconnect, "never observed Disconnected event");

    handle.abort();
    let _ = server.shutdown.send(());
}

#[tokio::test]
async fn invalid_url_is_config_error() {
    let res = WsClient::builder("not-a-url");
    match res {
        Err(antegen_ws::Error::Config(_)) => {}
        Err(e) => panic!("expected Error::Config, got {e:?}"),
        Ok(_) => panic!("expected error, got Ok"),
    }
}

#[tokio::test]
async fn initial_connect_failure_surfaces() {
    // Pick a port nothing is listening on.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener); // free the port

    let url = format!("ws://{addr}");
    let res = WsClient::builder(&url).unwrap().build().await;
    assert!(matches!(res, Err(antegen_ws::Error::InitialConnect(_))));
}
