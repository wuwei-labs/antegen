pub mod tx;

use std::{
    fmt::Debug,
    sync::Arc,
    time::Duration,
};

use agave_geyser_plugin_interface::geyser_plugin_interface::Result as PluginResult;
use anchor_lang::{prelude::Pubkey, AccountDeserialize};
use async_trait::async_trait;
use log::info;
use solana_client::{
    client_error::{ClientError, ClientErrorKind, Result as ClientResult},
    nonblocking::rpc_client::RpcClient,
};
use solana_sdk::commitment_config::CommitmentConfig;
use tokio::{runtime::Runtime, sync::Mutex, time::timeout};
use tx::TxExecutor;

use crate::{config::PluginConfig, observers::Observers};

static LOCAL_RPC_URL: &str = "http://127.0.0.1:8899";

pub struct Executors {
    pub tx: Arc<TxExecutor>,
    pub client: Arc<RpcClient>,
    pub lock: Mutex<()>
}

impl Executors {
    pub fn new(config: PluginConfig) -> Self {
        Executors {
            tx: Arc::new(TxExecutor::new(config.clone())),
            client: Arc::new(RpcClient::new_with_commitment(
                LOCAL_RPC_URL.into(),
                CommitmentConfig::processed(),
            )),
            lock: Mutex::new(())
        }
    }

    pub async fn process_slot(
        self: Arc<Self>,
        observers: Arc<Observers>,
        slot: u64,
        runtime: Arc<Runtime>,
    ) -> PluginResult<()> {
        info!("process_slot: {}", slot);
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

        // Try to acquire lock with timeout
        let lock_result = timeout(
            Duration::from_millis(400),
            self.lock.lock()
        ).await;

        match lock_result {
            Ok(_guard) => {
                // Process the slot on the observers.
                let executable_threads = observers.thread.clone().process_slot(slot).await?;
                info!("executable_threads: {:#?}", executable_threads);

                // Process the slot in the transaction executor.
                self.tx
                    .clone()
                    .execute_txs(
                        self.client.clone(),
                        executable_threads,
                        slot,
                        runtime.clone(),
                    )
                    .await?;

                info!(
                    "processed_slot: {} duration: {:?} status: processed",
                    slot,
                    now.elapsed()
                );
                Ok(())
            },
            Err(_) => {
                info!(
                    "processed_slot: {} duration: {:?} status: locked",
                    slot,
                    now.elapsed()
                );
                Ok(())
            }
        }
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
