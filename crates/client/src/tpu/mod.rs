//! TPU Client wrapper for transaction submission
//!
//! Uses solana-tpu-client-next to send transactions directly to validators
//! via QUIC protocol for faster landing times.
//!
//! This client uses the standard solana-rpc-client internally for leader
//! schedule queries, sharing the same endpoint URLs as our custom RpcPool.
//!
//! # Architecture
//!
//! All `WorkerActor` instances share a single `TpuClient` instance via `Arc`.
//! The internal channel-based architecture allows concurrent access:
//!
//! ```text
//! ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
//! │  Worker #1   │  │  Worker #2   │  │  Worker #N   │
//! └──────┬───────┘  └──────┬───────┘  └──────┬───────┘
//!        │                 │                 │
//!        │  send_transaction()               │
//!        ▼                 ▼                 ▼
//! ┌─────────────────────────────────────────────────┐
//! │        Arc<TpuClient>  (shared instance)        │
//! │  ┌───────────────────────────────────────────┐  │
//! │  │   mpsc::Sender<TransactionBatch> (queue)  │  │
//! │  └─────────────────────┬─────────────────────┘  │
//! │                        ▼                        │
//! │  ┌───────────────────────────────────────────┐  │
//! │  │  ConnectionWorkersScheduler (single)      │  │
//! │  │  - Maintains QUIC connections to leaders  │  │
//! │  │  - Broadcasts batches to upcoming leaders │  │
//! │  └───────────────────────────────────────────┘  │
//! └─────────────────────────────────────────────────┘
//! ```

use anyhow::{anyhow, Result};
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::transaction::Transaction;
use solana_tpu_client_next::{
    connection_workers_scheduler::{
        BindTarget, ConnectionWorkersScheduler, ConnectionWorkersSchedulerConfig, Fanout,
    },
    leader_updater::create_leader_updater,
    send_transaction_stats::SendTransactionStats,
    transaction_batch::TransactionBatch,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

/// TPU client for direct validator transaction submission
///
/// This client wraps `solana-tpu-client-next`'s `ConnectionWorkersScheduler`
/// to provide a simple interface for sending transactions via TPU.
///
/// # Thread Safety
///
/// `TpuClient` is designed to be shared across multiple tasks/threads via `Arc`.
/// The internal `mpsc::Sender` allows multiple concurrent callers to queue
/// transactions for submission.
pub struct TpuClient {
    tx_sender: mpsc::Sender<TransactionBatch>,
    stats: Arc<SendTransactionStats>,
    cancel: CancellationToken,
}

/// Configuration for the TPU client
///
/// The `rpc_url` and `websocket_url` should come from the same endpoint
/// configuration used by the RpcPool for consistency.
#[derive(Debug, Clone)]
pub struct TpuClientConfig {
    /// RPC URL for leader schedule queries (shared with RpcPool config)
    pub rpc_url: String,
    /// WebSocket URL for slot subscriptions (derived from RpcPool config)
    pub websocket_url: String,
    /// Number of QUIC connections per leader
    pub num_connections: usize,
    /// Number of leaders to fan out transactions to
    pub leaders_fanout: usize,
    /// Channel buffer size for transaction batches
    pub worker_channel_size: usize,
}

impl TpuClient {
    /// Create a new TPU client
    ///
    /// Uses the standard solana-rpc-client internally for leader tracking,
    /// configured with the same endpoint URL as the main RpcPool.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The leader updater fails to initialize (RPC/WebSocket connection issues)
    /// - The scheduler fails to start
    pub async fn new(config: TpuClientConfig) -> Result<Self> {
        log::info!("Initializing TPU client with RPC: {}", config.rpc_url);

        // Standard RPC client for leader schedule queries only
        // Uses same endpoint URL as our custom RpcPool
        let rpc_client = Arc::new(RpcClient::new(config.rpc_url.clone()));

        let leader_updater = create_leader_updater(rpc_client, config.websocket_url, None)
            .await
            .map_err(|e| anyhow!("Failed to create leader updater: {:?}", e))?;

        let (tx_sender, tx_receiver) =
            mpsc::channel::<TransactionBatch>(config.worker_channel_size);

        // Watch channel for stake identity updates (None = unstaked connection)
        let (_identity_sender, identity_receiver) = watch::channel(None);

        let cancel = CancellationToken::new();

        let scheduler = ConnectionWorkersScheduler::new(
            leader_updater,
            tx_receiver,
            identity_receiver,
            cancel.clone(),
        );

        let stats = scheduler.get_stats();

        let scheduler_config = ConnectionWorkersSchedulerConfig {
            bind: BindTarget::Address(SocketAddr::from(([0, 0, 0, 0], 0))),
            stake_identity: None,
            num_connections: config.num_connections,
            skip_check_transaction_age: false,
            worker_channel_size: config.worker_channel_size,
            max_reconnect_attempts: 4,
            leaders_fanout: Fanout {
                send: config.leaders_fanout,
                connect: config.leaders_fanout + 2,
            },
        };

        // Spawn scheduler in background
        let cancel_clone = cancel.clone();
        tokio::spawn(async move {
            match scheduler.run(scheduler_config).await {
                Ok(final_stats) => {
                    log::info!("TPU scheduler stopped. Final stats: {:?}", final_stats);
                }
                Err(e) => {
                    if !cancel_clone.is_cancelled() {
                        log::error!("TPU scheduler error: {:?}", e);
                    }
                }
            }
        });

        log::info!("TPU client initialized successfully");
        Ok(Self {
            tx_sender,
            stats,
            cancel,
        })
    }

    /// Send a single transaction via TPU (fire-and-forget)
    ///
    /// This method queues the transaction for submission to upcoming slot leaders.
    /// It does NOT wait for confirmation - the caller should poll for confirmation
    /// via RPC separately.
    ///
    /// # Note
    ///
    /// The signature should be computed from the transaction before calling this
    /// method, as TPU submission is fire-and-forget and doesn't return a signature.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Transaction serialization fails
    /// - The internal channel is closed (scheduler has stopped)
    pub async fn send_transaction(&self, transaction: &Transaction) -> Result<()> {
        let wire_tx = bincode::serialize(transaction)?;
        let batch = TransactionBatch::new(vec![wire_tx]);

        self.tx_sender
            .send(batch)
            .await
            .map_err(|_| anyhow!("TPU channel closed"))?;

        Ok(())
    }

    /// Get current send statistics
    ///
    /// Returns statistics about transaction sending including success/failure counts
    /// and timing information.
    pub fn stats(&self) -> &Arc<SendTransactionStats> {
        &self.stats
    }

    /// Check if the TPU client is still running
    pub fn is_running(&self) -> bool {
        !self.cancel.is_cancelled()
    }

    /// Gracefully shutdown the TPU client
    ///
    /// This cancels the background scheduler and closes all QUIC connections.
    pub fn shutdown(&self) {
        log::info!("Shutting down TPU client");
        self.cancel.cancel();
    }
}

impl Drop for TpuClient {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
