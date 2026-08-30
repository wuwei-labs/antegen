//! Shared transaction confirmation.
//!
//! Every in-flight transaction is polled by one background task on a single
//! tick, using a batched `getSignatureStatuses`. Previously each worker ran its
//! own 500ms polling loop, so confirmation cost one RPC request per in-flight
//! transaction per poll; at the default concurrency that is ~20 requests/second
//! against the same endpoint the execution path is trying to use. Batched, it is
//! one request per tick regardless of how many transactions are outstanding.
//!
//! The watcher also owns TPU rebroadcast. Resending is only safe because the
//! transaction is signed once and reused across retries — a re-signed
//! transaction has a different signature, and rebroadcasting that would risk
//! landing twice.

use crate::rpc::RpcPool;
use crate::tpu::TpuClient;
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

/// How often in-flight signatures are polled.
const POLL_INTERVAL: Duration = Duration::from_millis(400);

/// How often a pending transaction is rebroadcast via TPU, which may reach a
/// different leader.
const REBROADCAST_INTERVAL: Duration = Duration::from_secs(2);

/// `getSignatureStatuses` accepts at most this many signatures per call.
const MAX_BATCH: usize = 256;

/// Result of waiting for a transaction to land.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirmation {
    /// Landed successfully.
    Confirmed,
    /// Landed but failed on-chain. Carries the raw RPC error.
    Failed(String),
    /// Did not land before the deadline.
    TimedOut,
}

struct Pending {
    transaction: VersionedTransaction,
    deadline: Instant,
    last_rebroadcast: Instant,
    reply: oneshot::Sender<Confirmation>,
}

struct Registration {
    signature: Signature,
    pending: Pending,
}

/// Handle for registering transactions with the shared watcher.
#[derive(Clone)]
pub struct SignatureWatcher {
    tx: mpsc::UnboundedSender<Registration>,
}

impl SignatureWatcher {
    /// Start the watcher task. It runs until every handle is dropped.
    pub fn spawn(rpc: Arc<RpcPool>, tpu: Option<Arc<TpuClient>>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(run(rx, rpc, tpu));
        Self { tx }
    }

    /// Wait for `transaction` to land, up to `timeout`.
    ///
    /// Returns `Confirmation::TimedOut` if the watcher has stopped, so a caller
    /// can never block forever on a dead watcher.
    pub async fn wait(
        &self,
        signature: Signature,
        transaction: VersionedTransaction,
        timeout: Duration,
    ) -> Confirmation {
        let (reply, rx) = oneshot::channel();
        let now = Instant::now();
        let registration = Registration {
            signature,
            pending: Pending {
                transaction,
                deadline: now + timeout,
                last_rebroadcast: now,
                reply,
            },
        };

        if self.tx.send(registration).is_err() {
            return Confirmation::TimedOut;
        }
        rx.await.unwrap_or(Confirmation::TimedOut)
    }
}

async fn run(
    mut rx: mpsc::UnboundedReceiver<Registration>,
    rpc: Arc<RpcPool>,
    tpu: Option<Arc<TpuClient>>,
) {
    let mut pending: HashMap<Signature, Pending> = HashMap::new();
    let mut ticker = tokio::time::interval(POLL_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            incoming = rx.recv() => {
                match incoming {
                    Some(reg) => {
                        pending.insert(reg.signature, reg.pending);
                    }
                    // Every handle dropped and nothing left to settle.
                    None if pending.is_empty() => return,
                    None => {}
                }
            }
            _ = ticker.tick() => {
                if pending.is_empty() {
                    continue;
                }
                poll_once(&mut pending, &rpc, &tpu).await;
            }
        }
    }
}

async fn poll_once(
    pending: &mut HashMap<Signature, Pending>,
    rpc: &Arc<RpcPool>,
    tpu: &Option<Arc<TpuClient>>,
) {
    let now = Instant::now();

    // Expire first, so a hung batch cannot keep a caller waiting past its
    // deadline.
    let expired: Vec<Signature> = pending
        .iter()
        .filter(|(_, p)| p.deadline <= now)
        .map(|(sig, _)| *sig)
        .collect();
    for sig in expired {
        if let Some(p) = pending.remove(&sig) {
            let _ = p.reply.send(Confirmation::TimedOut);
        }
    }
    if pending.is_empty() {
        return;
    }

    let signatures: Vec<Signature> = pending.keys().copied().take(MAX_BATCH).collect();
    let statuses = match rpc.get_signature_statuses(&signatures).await {
        Ok(s) => s,
        Err(e) => {
            // Transient; the next tick tries again. Callers are still bounded by
            // their deadline.
            log::debug!("Batched signature status poll failed: {}", e);
            return;
        }
    };

    for (sig, status) in signatures.iter().zip(statuses) {
        match status {
            Some(Ok(())) => {
                if let Some(p) = pending.remove(sig) {
                    let _ = p.reply.send(Confirmation::Confirmed);
                }
            }
            Some(Err(err)) => {
                if let Some(p) = pending.remove(sig) {
                    let _ = p.reply.send(Confirmation::Failed(err));
                }
            }
            None => {
                // Still unconfirmed — rebroadcast periodically to reach a
                // different leader.
                let Some(tpu) = tpu else { continue };
                let Some(p) = pending.get_mut(sig) else {
                    continue;
                };
                if now.duration_since(p.last_rebroadcast) >= REBROADCAST_INTERVAL {
                    p.last_rebroadcast = now;
                    if let Err(e) = tpu.send_transaction(&p.transaction).await {
                        log::debug!("TPU rebroadcast failed: {}", e);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // getSignatureStatuses rejects more than 256 signatures per call, and
    // rebroadcasting on every poll would spam leaders with duplicates.
    const _: () = assert!(MAX_BATCH <= 256);
    const _: () = assert!(REBROADCAST_INTERVAL.as_millis() > POLL_INTERVAL.as_millis());

    #[tokio::test]
    async fn wait_returns_timed_out_when_the_watcher_is_gone() {
        let (tx, rx) = mpsc::unbounded_channel::<Registration>();
        drop(rx);
        let watcher = SignatureWatcher { tx };

        // A dead watcher must not leave the caller blocked forever.
        let got = watcher
            .wait(
                Signature::default(),
                VersionedTransaction::default(),
                Duration::from_secs(1),
            )
            .await;
        assert_eq!(got, Confirmation::TimedOut);
    }

    #[tokio::test]
    async fn pending_transactions_time_out() {
        let (reply, reply_rx) = oneshot::channel();

        let mut pending = HashMap::new();
        pending.insert(
            Signature::default(),
            Pending {
                transaction: VersionedTransaction::default(),
                // Already past its deadline.
                deadline: Instant::now() - Duration::from_secs(1),
                last_rebroadcast: Instant::now(),
                reply,
            },
        );

        let rpc = Arc::new(crate::rpc::RpcPool::with_url("http://127.0.0.1:1").unwrap());
        poll_once(&mut pending, &rpc, &None).await;

        assert!(pending.is_empty());
        assert_eq!(reply_rx.await.unwrap(), Confirmation::TimedOut);
    }
}
