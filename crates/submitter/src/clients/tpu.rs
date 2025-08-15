use anyhow::{anyhow, Result};
use solana_client::{
    nonblocking::{rpc_client::RpcClient, tpu_client::TpuClient},
    tpu_client::TpuClientConfig,
};
use solana_quic_client::{QuicConfig, QuicConnectionManager, QuicPool};
use solana_sdk::commitment_config::CommitmentConfig;
use std::sync::Arc;

/// Configuration for TPU client
pub struct TpuClientManager {
    rpc_url: String,
    websocket_url: String,
    fanout_slots: u64,
}

impl TpuClientManager {
    pub fn new(rpc_url: String, websocket_url: String, fanout_slots: u64) -> Self {
        Self {
            rpc_url,
            websocket_url,
            fanout_slots,
        }
    }

    /// Create a new TPU client instance
    pub async fn get_tpu_client(
        &self,
    ) -> Result<TpuClient<QuicPool, QuicConnectionManager, QuicConfig>> {
        let rpc_client = Arc::new(RpcClient::new_with_commitment(
            self.rpc_url.clone(),
            CommitmentConfig::processed(),
        ));

        TpuClient::new(
            "tpu_client",
            rpc_client,
            &self.websocket_url,
            TpuClientConfig {
                fanout_slots: self.fanout_slots,
            },
        )
        .await
        .map_err(|e| anyhow!("Failed to create TPU client: {}", e))
    }
}

impl Default for TpuClientManager {
    fn default() -> Self {
        Self {
            rpc_url: "http://127.0.0.1:8899".to_string(),
            websocket_url: "ws://127.0.0.1:8900".to_string(),
            fanout_slots: 24,
        }
    }
}
