use anyhow::{anyhow, Context, Result};
use log::{debug, info, warn};
use solana_client::{
    nonblocking::rpc_client::RpcClient, nonblocking::tpu_client::TpuClient,
    tpu_client::TpuClientConfig,
};
use solana_quic_client::{QuicConfig, QuicConnectionManager, QuicPool};
use solana_sdk::{signature::Signature, transaction::Transaction};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::{SubmissionMode, TpuConfig};

/// Wait for RPC server to become available
async fn wait_for_rpc_availability(rpc_client: &RpcClient, max_wait: Duration) -> Result<()> {
    let start = Instant::now();
    let mut delay = Duration::from_secs(1);
    let max_delay = Duration::from_secs(30);
    let mut last_log = Instant::now();

    info!(
        "Waiting for RPC server to become available at {}...",
        rpc_client.url()
    );

    loop {
        // Try to connect to RPC
        match rpc_client.get_health().await {
            Ok(_) => {
                info!(
                    "RPC server is available (took {:.1}s)",
                    start.elapsed().as_secs_f32()
                );
                return Ok(());
            }
            Err(e) => {
                debug!("RPC not ready yet: {}", e);
            }
        }

        // Check timeout
        if start.elapsed() > max_wait {
            return Err(anyhow!(
                "RPC server at {} failed to become available after {} seconds",
                rpc_client.url(),
                max_wait.as_secs()
            ));
        }

        // Log progress every 30 seconds
        if last_log.elapsed() > Duration::from_secs(30) {
            info!(
                "Still waiting for RPC server (elapsed: {:.0}s of max {}s)...",
                start.elapsed().as_secs(),
                max_wait.as_secs()
            );
            last_log = Instant::now();
        }

        // Wait with exponential backoff
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(max_delay);
    }
}

/// Handles transaction submission via RPC and TPU
pub struct TransactionSubmitter {
    rpc_client: Arc<RpcClient>,
    tpu_client: RwLock<Option<Arc<TpuClient<QuicPool, QuicConnectionManager, QuicConfig>>>>,
    config: TpuConfig,
    submission_mode: RwLock<SubmissionMode>,
}

impl TransactionSubmitter {
    /// Create a new transaction submitter (non-blocking, requires initialize() to be called)
    pub async fn new(rpc_client: Arc<RpcClient>, tpu_config: Option<TpuConfig>) -> Result<Self> {
        let config = tpu_config.unwrap_or_default();

        // Don't wait for RPC or create TPU client here - defer to initialize()
        // Start in RPC-only mode until we verify connectivity
        let submission_mode = RwLock::new(SubmissionMode::Rpc);

        Ok(Self {
            rpc_client,
            tpu_client: RwLock::new(None),
            config,
            submission_mode,
        })
    }

    /// Complete initialization by waiting for RPC and creating TPU client
    pub async fn initialize(&self) -> Result<()> {
        // Wait for RPC to be available (max 5 minutes)
        wait_for_rpc_availability(&self.rpc_client, Duration::from_secs(300))
            .await
            .context("Failed to connect to RPC server")?;

        info!("RPC connection established, completing initialization");

        // Now try to create TPU client if configured
        if matches!(
            self.config.mode,
            SubmissionMode::Tpu | SubmissionMode::TpuWithFallback
        ) {
            match Self::create_tpu_client(self.rpc_client.clone(), &self.config).await {
                Ok(client) => {
                    info!("TPU client initialized successfully");
                    *self.tpu_client.write().await = Some(Arc::new(client));
                    *self.submission_mode.write().await = self.config.mode;
                }
                Err(e) => {
                    warn!("Failed to create TPU client: {}, using RPC only", e);
                    *self.submission_mode.write().await = SubmissionMode::Rpc;
                }
            }
        } else {
            info!("TPU disabled by configuration, using RPC only");
            *self.submission_mode.write().await = self.config.mode;
        }

        Ok(())
    }

    /// Create a TPU client with retry logic
    async fn create_tpu_client(
        rpc_client: Arc<RpcClient>,
        config: &TpuConfig,
    ) -> Result<TpuClient<QuicPool, QuicConnectionManager, QuicConfig>> {
        // Get websocket URL from RPC URL
        let rpc_url = rpc_client.url();
        let ws_url = rpc_url
            .replace("http://", "ws://")
            .replace("https://", "wss://")
            .replace("8899", "8900"); // Default port mapping

        info!("Creating TPU client with websocket: {}", ws_url);

        // TPU client configuration
        let tpu_client_config = TpuClientConfig {
            fanout_slots: config.fanout_slots,
            ..TpuClientConfig::default()
        };

        // Try up to 3 times with delays
        let mut attempts = 0;
        let max_attempts = 3;

        loop {
            attempts += 1;

            match TpuClient::new(
                "antegen-submitter",
                rpc_client.clone(),
                &ws_url,
                tpu_client_config.clone(),
            )
            .await
            {
                Ok(client) => {
                    info!("TPU client created successfully on attempt {}", attempts);
                    return Ok(client);
                }
                Err(e) => {
                    if attempts >= max_attempts {
                        return Err(anyhow!(
                            "Failed to create TPU client after {} attempts: {}",
                            max_attempts,
                            e
                        ));
                    }

                    let delay = Duration::from_secs(attempts as u64);
                    warn!(
                        "TPU client creation failed (attempt {}/{}): {}, retrying in {:?}...",
                        attempts, max_attempts, e, delay
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    /// Submit a transaction using configured mode
    pub async fn submit(&self, tx: &Transaction) -> Result<Signature> {
        let mode = *self.submission_mode.read().await;

        info!(
            "Submitting transaction with {} instruction(s) via {:?}",
            tx.message.instructions.len(),
            mode
        );

        match mode {
            SubmissionMode::Tpu => self.submit_via_tpu(tx).await,
            SubmissionMode::Rpc => self.submit_via_rpc(tx).await,
            SubmissionMode::TpuWithFallback => {
                // Try TPU first
                match self.submit_via_tpu(tx).await {
                    Ok(sig) => Ok(sig),
                    Err(tpu_err) => {
                        warn!("TPU submission failed: {}, falling back to RPC", tpu_err);
                        self.submit_via_rpc(tx).await
                    }
                }
            }
            SubmissionMode::Both => {
                // Submit to both in parallel, return first success
                let tpu_future = self.submit_via_tpu(tx);
                let rpc_future = self.submit_via_rpc(tx);
                
                tokio::select! {
                    tpu_result = tpu_future => tpu_result,
                    rpc_result = rpc_future => rpc_result,
                }
            }
        }
    }

    /// Submit multiple transactions in batch
    pub async fn submit_batch(&self, txs: &[Transaction]) -> Result<Vec<Result<Signature>>> {
        if txs.is_empty() {
            return Ok(Vec::new());
        }

        let mode = *self.submission_mode.read().await;
        info!("Batch submitting {} transactions via {:?}", txs.len(), mode);

        match mode {
            SubmissionMode::Tpu => self.submit_batch_via_tpu(txs).await,
            SubmissionMode::Rpc => self.submit_batch_via_rpc(txs).await,
            SubmissionMode::TpuWithFallback => {
                // Try TPU first for batch
                match self.submit_batch_via_tpu(txs).await {
                    Ok(results) => Ok(results),
                    Err(tpu_err) => {
                        warn!(
                            "Batch TPU submission failed: {}, falling back to RPC",
                            tpu_err
                        );
                        self.submit_batch_via_rpc(txs).await
                    }
                }
            }
            SubmissionMode::Both => {
                // Submit to both in parallel
                let tpu_future = self.submit_batch_via_tpu(txs);
                let rpc_future = self.submit_batch_via_rpc(txs);
                
                tokio::select! {
                    tpu_result = tpu_future => tpu_result,
                    rpc_result = rpc_future => rpc_result,
                }
            }
        }
    }

    /// Submit transaction via TPU (fire-and-forget)
    async fn submit_via_tpu(&self, tx: &Transaction) -> Result<Signature> {
        let tpu_client_guard = self.tpu_client.read().await;
        let tpu_client = tpu_client_guard
            .as_ref()
            .ok_or_else(|| anyhow!("TPU client not available"))?;

        let signature = tx.signatures[0];
        debug!("Submitting to TPU: {}", signature);

        // Send transaction to TPU leaders (single send, client handles fanout)
        let wire_transaction = bincode::serialize(tx)?;

        if !tpu_client.send_wire_transaction(wire_transaction).await {
            return Err(anyhow!("Failed to send transaction to TPU"));
        }

        info!("Transaction {} sent to TPU (fire-and-forget)", signature);
        Ok(signature)
    }

    /// Submit transaction via RPC with timeout protection
    pub async fn submit_via_rpc(&self, tx: &Transaction) -> Result<Signature> {
        debug!("Submitting via RPC");

        // Use send_and_confirm with 30 second timeout to prevent indefinite blocking
        match tokio::time::timeout(
            Duration::from_secs(30),
            self.rpc_client.send_and_confirm_transaction(tx),
        )
        .await
        {
            Ok(Ok(signature)) => {
                info!("Transaction {} confirmed via RPC", signature);
                Ok(signature)
            }
            Ok(Err(e)) => Err(e.into()),
            Err(_) => Err(anyhow!("RPC submission timed out after 30 seconds")),
        }
    }

    /// Submit batch of transactions via TPU (fire-and-forget)
    async fn submit_batch_via_tpu(&self, txs: &[Transaction]) -> Result<Vec<Result<Signature>>> {
        let tpu_client_guard = self.tpu_client.read().await;
        let tpu_client = tpu_client_guard
            .as_ref()
            .ok_or_else(|| anyhow!("TPU client not available"))?;

        let mut results = Vec::with_capacity(txs.len());
        let mut wire_transactions = Vec::with_capacity(txs.len());

        // Serialize all transactions
        for tx in txs {
            match bincode::serialize(tx) {
                Ok(wire_tx) => {
                    wire_transactions.push(wire_tx);
                    results.push(Ok(tx.signatures[0]));
                }
                Err(e) => {
                    results.push(Err(anyhow!("Failed to serialize transaction: {}", e)));
                }
            }
        }

        if !wire_transactions.is_empty() {
            debug!(
                "Batch submitting {} transactions to TPU",
                wire_transactions.len()
            );

            // Send all transactions in batch (fire-and-forget, single attempt)
            let batch_sent = tpu_client
                .try_send_wire_transaction_batch(wire_transactions)
                .await
                .is_ok();

            if !batch_sent {
                warn!("Failed to send transactions in batch to TPU");
                // Mark all as failed
                for result in &mut results {
                    if result.is_ok() {
                        *result = Err(anyhow!("Batch TPU submission failed"));
                    }
                }
            } else {
                info!(
                    "Batch of {} transactions sent to TPU (fire-and-forget)",
                    txs.len()
                );
            }
        }

        Ok(results)
    }

    /// Submit batch of transactions via RPC
    async fn submit_batch_via_rpc(&self, txs: &[Transaction]) -> Result<Vec<Result<Signature>>> {
        debug!("Batch submitting {} transactions via RPC", txs.len());

        // RPC doesn't have native batch support, so we submit in parallel with controlled concurrency
        use futures::stream::{self, StreamExt};

        const MAX_CONCURRENT_RPC: usize = 10;

        let results = stream::iter(txs.iter())
            .map(|tx| async move { self.submit_via_rpc(tx).await })
            .buffer_unordered(MAX_CONCURRENT_RPC)
            .collect::<Vec<_>>()
            .await;

        Ok(results)
    }

    /// Submit with retries (works with both TPU and RPC)
    pub async fn submit_with_retries(
        &self,
        tx: &Transaction,
        max_retries: u32,
    ) -> Result<Signature> {
        let mut attempts = 0;
        let mut last_error = None;

        while attempts < max_retries {
            match self.submit(tx).await {
                Ok(sig) => return Ok(sig),
                Err(e) => {
                    attempts += 1;
                    warn!("Submission attempt {} failed: {}", attempts, e);
                    last_error = Some(e);

                    if attempts < max_retries {
                        tokio::time::sleep(Duration::from_millis(1000 * attempts as u64)).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            anyhow!(
                "Failed to submit transaction after {} attempts",
                max_retries
            )
        }))
    }

    /// Check if transaction uses durable nonce
    pub fn is_durable_transaction(&self, tx: &Transaction) -> bool {
        // Check if transaction has advance_nonce_account instruction
        tx.message.instructions.iter().any(|ix| {
            // Check for system program and advance nonce instruction
            ix.program_id_index < tx.message.account_keys.len() as u8
                && tx.message.account_keys[ix.program_id_index as usize]
                    == solana_sdk::system_program::ID
                && !ix.data.is_empty()
                && ix.data[0] == 4 // advance_nonce_account instruction discriminator
        })
    }

    /// Get current submission mode
    pub async fn get_mode(&self) -> SubmissionMode {
        *self.submission_mode.read().await
    }

    /// Update submission mode (useful for runtime adjustments)
    pub async fn set_mode(&self, mode: SubmissionMode) -> Result<()> {
        // Validate we can use the requested mode
        if matches!(mode, SubmissionMode::Tpu | SubmissionMode::TpuWithFallback) {
            let tpu_client_guard = self.tpu_client.read().await;
            if tpu_client_guard.is_none() {
                return Err(anyhow!("Cannot set TPU mode: TPU client not available"));
            }
        }

        let mut submission_mode = self.submission_mode.write().await;
        *submission_mode = mode;
        info!("Submission mode updated to: {:?}", mode);
        Ok(())
    }

    /// Check if TPU client is available
    pub async fn has_tpu_client(&self) -> bool {
        self.tpu_client.read().await.is_some()
    }
}
