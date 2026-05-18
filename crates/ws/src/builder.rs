use crate::backoff::Backoff;
use crate::error::{Error, SendError};
use crate::sender::Sender;
use crate::WsHandle;

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
use url::Url;

pub(crate) type OnConnectFn =
    Box<dyn FnMut(Sender) -> Pin<Box<dyn Future<Output = Result<(), SendError>> + Send>> + Send>;

/// Builder for a persistent WebSocket client.
///
/// Call [`crate::WsClient::builder`] to obtain one. Chain configuration
/// setters, then await [`build`](Self::build) to spawn the transport task
/// and obtain a [`WsHandle`].
pub struct WsClientBuilder {
    url: Url,
    keepalive: Option<Duration>,
    backoff: Backoff,
    capacity: usize,
    on_connect: Option<OnConnectFn>,
}

impl WsClientBuilder {
    pub(crate) fn new(url: Url) -> Self {
        Self {
            url,
            keepalive: None,
            backoff: Backoff::default(),
            capacity: crate::DEFAULT_CAPACITY,
            on_connect: None,
        }
    }

    /// Send a Ping on idle every `period`. Useful for keeping NAT and
    /// validator-side state alive on long-running subscriptions.
    pub fn keepalive(mut self, period: Duration) -> Self {
        self.keepalive = Some(period);
        self
    }

    /// Override the reconnect backoff schedule. Defaults to
    /// `Backoff::exponential(100ms, 30s, 2.0)`.
    pub fn backoff(mut self, b: Backoff) -> Self {
        self.backoff = b;
        self
    }

    /// Channel capacity for inbound/outbound/event queues. Defaults to 32.
    pub fn channel_capacity(mut self, c: usize) -> Self {
        self.capacity = c.max(1);
        self
    }

    /// Closure invoked on every successful connect (initial *and* every
    /// reconnect). Use it to (re-)send subscription requests so the
    /// caller doesn't have to manage `Event::Connected` themselves.
    ///
    /// Errors returned from the closure end the transport task; if you
    /// want best-effort sends, swallow `SendError` inside the closure.
    pub fn on_connect<F, Fut>(mut self, mut f: F) -> Self
    where
        F: FnMut(Sender) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), SendError>> + Send + 'static,
    {
        self.on_connect = Some(Box::new(move |s| Box::pin(f(s))));
        self
    }

    /// Spawn the transport task and await the first connect.
    pub async fn build(self) -> Result<WsHandle, Error> {
        #[cfg(feature = "persistent")]
        {
            crate::persistent::spawn(crate::persistent::Spawn {
                url: self.url,
                keepalive: self.keepalive,
                backoff: self.backoff,
                capacity: self.capacity,
                on_connect: self.on_connect,
            })
            .await
        }
        #[cfg(not(feature = "persistent"))]
        {
            compile_error!("antegen-ws build() requires the `persistent` feature for now");
        }
    }
}
