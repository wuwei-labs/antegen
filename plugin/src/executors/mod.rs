pub mod tx;

use std::{
    fmt::Debug,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use anchor_lang::{prelude::Pubkey, AccountDeserialize};
use async_trait::async_trait;
use log::info;
use solana_client::{
    client_error::{ClientError, ClientErrorKind, Result as ClientResult},
    nonblocking::{rpc_client::RpcClient, tpu_client::TpuClient},
    tpu_client::TpuClientConfig,
};
use solana_quic_client::{QuicConfig, QuicConnectionManager, QuicPool};
use agave_geyser_plugin_interface::geyser_plugin_interface::{GeyserPluginError, Result as PluginResult};
use solana_sdk::commitment_config::CommitmentConfig;
use tokio::{runtime::Runtime, sync::OnceCell};
use tx::TxExecutor;

use crate::{config::PluginConfig, observers::Observers};

static LOCAL_RPC_URL: &str = "http://127.0.0.1:8899";
static LOCAL_WEBSOCKET_URL: &str = "ws://127.0.0.1:8900";

pub struct Executors {
    pub tx: Arc<TxExecutor>,
    pub client: Arc<RpcClient>,
    pub tpu_client: OnceCell<Arc<TpuClient<QuicPool, QuicConnectionManager, QuicConfig>>>,
    pub lock: AtomicBool,
}

impl Executors {
    pub fn new(config: PluginConfig) -> Self {
        let client = Arc::new(RpcClient::new_with_commitment(
            LOCAL_RPC_URL.into(),
            CommitmentConfig::processed(),
        ));

        Executors {
            tx: Arc::new(TxExecutor::new(config, client.clone())),
            client,  
            tpu_client: OnceCell::new(),
            lock: AtomicBool::new(false),
        }
    }

    pub async fn initialize(&self) -> PluginResult<()> {
        // Only initialize TPU client if not already initialized
        if self.tpu_client.get().is_none() {
            let tpu_client = Arc::new(
                TpuClient::new(
                    "tpu_client",
                    self.client.clone(),
                    LOCAL_WEBSOCKET_URL.into(),
                    TpuClientConfig { fanout_slots: 24 },
                )
                .await
                .map_err(|e| GeyserPluginError::Custom(format!("Failed to create TPU client: {}", e).into()))?
            );

            // Set TPU client - this will only happen once
            let _ = self.tpu_client.set(tpu_client);
        }

        // Update TxExecutor with TPU client
        self.tx.set_tpu_client(self.tpu_client.get().unwrap().clone()).await;
        Ok(())
    }

    pub async fn process_slot(
        self: Arc<Self>,
        observers: Arc<Observers>,
        slot: u64,
        runtime: Arc<Runtime>,
    ) -> PluginResult<()> {
        info!("process_slot: {}", slot,);
        let now = std::time::Instant::now();

        // Return early if node is not healthy.
        if self.client.get_health().await.is_err() {
            info!(
                "processed_slot: {} duration: {:?} status: unhealthy",
                slot,
                now.elapsed()
            );
            return Ok(());
        }

        // Acquire lock.
        if self
            .clone()
            .lock
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            info!(
                "processed_slot: {} duration: {:?} status: locked",
                slot,
                now.elapsed()
            );
            return Ok(());
        }

        // Process the slot on the observers.
        let executable_threads = observers.thread.clone().process_slot(slot).await?;
        info!("executable_threads: {:#?}", executable_threads);

        // Process the slot in the transaction executor.
        self.tx
            .clone()
            .execute_txs(
                executable_threads,
                slot,
                runtime.clone(),
            )
            .await?;

        // Release the lock.
        self.clone()
            .lock
            .store(false, std::sync::atomic::Ordering::Relaxed);
        info!(
            "processed_slot: {} duration: {:?} status: processed",
            slot,
            now.elapsed()
        );
        Ok(())
    }
}

impl Debug for Executors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "executors")
    }
}

#[async_trait]
pub trait AccountGet {
    async fn get<T: AccountDeserialize>(&self, pubkey: &Pubkey) -> ClientResult<T>;
}

#[async_trait]
impl AccountGet for RpcClient {
    async fn get<T: AccountDeserialize>(&self, pubkey: &Pubkey) -> ClientResult<T> {
        let data = self.get_account_data(pubkey).await?;
        T::try_deserialize(&mut data.as_slice()).map_err(|_| {
            ClientError::from(ClientErrorKind::Custom(format!(
                "Failed to deserialize account data"
            )))
        })
    }
}
