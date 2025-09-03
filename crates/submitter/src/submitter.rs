use anyhow::{anyhow, Context, Result};
use log::{debug, info, warn};
use solana_account_decoder::UiAccountEncoding;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::{
    nonblocking::tpu_client::TpuClient,
    rpc_config::{RpcSimulateTransactionAccountsConfig, RpcSimulateTransactionConfig},
    tpu_client::TpuClientConfig,
};
use solana_quic_client::{QuicConfig, QuicConnectionManager, QuicPool};
use solana_sdk::{
    commitment_config::CommitmentConfig,
    compute_budget::ComputeBudgetInstruction,
    hash::Hash,
    instruction::Instruction,
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
    transaction::{Transaction, VersionedTransaction},
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::{SubmissionMode, SubmitterMetrics, TpuConfig};

/// Lightweight transaction submitter with honeybadger retry logic
pub struct TransactionSubmitter {
    /// RPC client for blockchain operations
    rpc_client: Arc<RpcClient>,
    /// TPU client for direct submission to leaders
    tpu_client: RwLock<Option<Arc<TpuClient<QuicPool, QuicConnectionManager, QuicConfig>>>>,
    /// Current submission mode
    submission_mode: SubmissionMode,
    /// TPU configuration
    tpu_config: Option<TpuConfig>,
    /// Metrics collector
    metrics: Arc<SubmitterMetrics>,
    /// Clock broadcast receiver for retry timing
    clock_rx: tokio::sync::broadcast::Receiver<solana_sdk::clock::Clock>,
}

impl TransactionSubmitter {
    /// Create a new transaction submitter
    pub fn new(
        rpc_url: String,
        tpu_config: Option<TpuConfig>,
        metrics: Arc<SubmitterMetrics>,
        clock_rx: tokio::sync::broadcast::Receiver<solana_sdk::clock::Clock>,
    ) -> Result<Self> {
        let rpc_client = Arc::new(RpcClient::new_with_commitment(
            rpc_url,
            CommitmentConfig::confirmed(),
        ));

        let submission_mode = tpu_config
            .as_ref()
            .map(|c| c.mode)
            .unwrap_or(SubmissionMode::Rpc);

        Ok(Self {
            rpc_client,
            tpu_client: RwLock::new(None),
            submission_mode,
            tpu_config,
            metrics,
            clock_rx,
        })
    }

    /// Create from existing RPC client
    pub fn from_client(
        rpc_client: Arc<RpcClient>,
        tpu_config: Option<TpuConfig>,
        metrics: Arc<SubmitterMetrics>,
        clock_rx: tokio::sync::broadcast::Receiver<solana_sdk::clock::Clock>,
    ) -> Self {
        let submission_mode = tpu_config
            .as_ref()
            .map(|c| c.mode)
            .unwrap_or(SubmissionMode::Rpc);

        Self {
            rpc_client,
            tpu_client: RwLock::new(None),
            submission_mode,
            tpu_config,
            metrics,
            clock_rx,
        }
    }

    /// Initialize TPU client if configured
    pub async fn initialize_tpu(&self) -> Result<()> {
        if let Some(ref config) = self.tpu_config {
            if matches!(
                self.submission_mode,
                SubmissionMode::Tpu | SubmissionMode::TpuWithFallback | SubmissionMode::Both
            ) {
                info!("Creating TPU client with config: {:?}", config);

                let tpu_client = TpuClient::new(
                    "antegen-submitter",
                    self.rpc_client.clone(),
                    "ws://127.0.0.1:8900", // Default websocket endpoint
                    TpuClientConfig {
                        fanout_slots: config.fanout_slots,
                    },
                )
                .await
                .context("Failed to create TPU client")?;

                *self.tpu_client.write().await = Some(Arc::new(tpu_client));
                info!("TPU client initialized successfully");
            }
        }
        Ok(())
    }

    /// Submit transaction with honeybadger retry logic (default behavior)
    /// Returns only on timeout - success is determined by thread updates
    pub async fn submit(
        &self,
        instructions: Vec<Instruction>,
        keypair: Arc<Keypair>,
        priority_fee: Option<u64>,
    ) -> Result<()> {
        // Honeybadger approach: Keep trying until timeout
        // Success is determined by thread updates, not transaction submission
        let task_start = Instant::now();
        const TASK_TIMEOUT: Duration = Duration::from_secs(600); // 10 minutes

        let mut clock_rx = self.clock_rx.resubscribe();
        let mut attempt_count = 0;
        let mut last_signature: Option<Signature> = None;

        info!(
            "Starting honeybadger submission for transaction with {} instructions",
            instructions.len()
        );

        loop {
            attempt_count += 1;

            // Check timeout first
            if task_start.elapsed() > TASK_TIMEOUT {
                warn!(
                    "Transaction timeout after 10 minutes ({} attempts, last sig: {:?})",
                    attempt_count, last_signature
                );
                self.metrics.transaction_failed();
                return Err(anyhow!(
                    "Transaction submission timeout after {} attempts",
                    attempt_count
                ));
            }

            // Get fresh blockhash for this attempt
            let blockhash = match self.rpc_client.get_latest_blockhash().await {
                Ok(bh) => bh,
                Err(e) => {
                    debug!("Failed to get blockhash: {}, retrying next tick", e);
                    // Wait for next clock tick and continue
                    tokio::select! {
                        Ok(_) = clock_rx.recv() => continue,
                        else => {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            continue;
                        }
                    }
                }
            };

            // Build transaction for simulation
            let initial_tx = self.build_transaction_with_compute_budget(
                instructions.clone(),
                &keypair.pubkey(),
                blockhash,
                None, // Let simulation determine compute units
                priority_fee,
            );

            // Sign for simulation
            let mut sim_tx = initial_tx.clone();
            sim_tx.sign(&[keypair.as_ref()], blockhash);
            let versioned_sim_tx = VersionedTransaction::from(sim_tx);

            // Try simulation
            match self
                .simulate_and_optimize_transaction(
                    &versioned_sim_tx,
                    1.2,       // 20% buffer on compute units
                    1_400_000, // max compute units
                    None,
                )
                .await
            {
                Ok((compute_units, _logs)) => {
                    debug!(
                        "Simulation successful (attempt {}), using {} CU",
                        attempt_count, compute_units
                    );

                    // Build optimized transaction
                    let optimized_tx = self.build_transaction_with_compute_budget(
                        instructions.clone(),
                        &keypair.pubkey(),
                        blockhash,
                        Some(compute_units),
                        priority_fee,
                    );

                    // Sign and submit
                    let mut signed_tx = optimized_tx;
                    signed_tx.sign(&[keypair.as_ref()], blockhash);
                    let versioned_tx = VersionedTransaction::from(signed_tx);

                    match self.submit_transaction(&versioned_tx).await {
                        Ok(sig) => {
                            info!(
                                "Transaction {} submitted (attempt {}), continuing honeybadger retry",
                                sig, attempt_count
                            );
                            last_signature = Some(sig);
                            self.metrics.transaction_submitted("direct");
                            // Don't return! Keep retrying until thread updates or timeout
                        }
                        Err(e) => {
                            debug!(
                                "Submit failed (attempt {}): {}, retrying next tick",
                                attempt_count, e
                            );
                        }
                    }
                }
                Err(e) => {
                    debug!(
                        "Simulation failed (attempt {}): {}, retrying next tick",
                        attempt_count, e
                    );
                    // Continue to wait for clock and retry
                }
            }

            // Wait for next clock tick before retrying
            tokio::select! {
                Ok(clock) = clock_rx.recv() => {
                    debug!("Clock tick at slot {}, retrying (attempt {})",
                           clock.slot, attempt_count + 1);
                }
                else => {
                    // Small sleep if no clock updates
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    /// Submit a pre-built transaction (internal, without retry)
    async fn submit_transaction(&self, tx: &VersionedTransaction) -> Result<Signature> {
        let signature = tx.signatures[0];

        match self.submission_mode {
            SubmissionMode::Rpc => {
                // RPC doesn't have a direct method for VersionedTransaction, use send_transaction
                self.rpc_client
                    .send_transaction(tx)
                    .await
                    .context("RPC submission failed")?;
            }
            SubmissionMode::Tpu => {
                if let Some(ref tpu_client) = *self.tpu_client.read().await {
                    let wire_transaction = bincode::serialize(tx)?;
                    if !tpu_client.send_wire_transaction(wire_transaction).await {
                        return Err(anyhow!("Failed to send transaction to TPU"));
                    }
                } else {
                    return Err(anyhow!("TPU client not initialized"));
                }
            }
            SubmissionMode::TpuWithFallback => {
                if let Some(ref tpu_client) = *self.tpu_client.read().await {
                    let wire_transaction = bincode::serialize(tx)?;
                    if !tpu_client
                        .send_wire_transaction(wire_transaction.clone())
                        .await
                    {
                        debug!("TPU submission failed, falling back to RPC");
                        self.rpc_client
                            .send_transaction(tx)
                            .await
                            .context("RPC fallback submission failed")?;
                    }
                } else {
                    self.rpc_client
                        .send_transaction(tx)
                        .await
                        .context("RPC submission failed (no TPU client)")?;
                }
            }
            SubmissionMode::Both => {
                // Send to both TPU and RPC
                if let Some(ref tpu_client) = *self.tpu_client.read().await {
                    let wire_transaction = bincode::serialize(tx)?;
                    let _ = tpu_client.send_wire_transaction(wire_transaction).await;
                }
                self.rpc_client
                    .send_transaction(tx)
                    .await
                    .context("RPC submission failed")?;
            }
        }

        Ok(signature)
    }

    /// Simulate transaction and optimize compute units
    pub async fn simulate_and_optimize_transaction(
        &self,
        tx: &VersionedTransaction,
        multiplier: f64,
        max_compute_units: u32,
        _priority_fee: Option<u64>,
    ) -> Result<(u32, Vec<String>)> {
        let config = RpcSimulateTransactionConfig {
            sig_verify: false,
            replace_recent_blockhash: true,
            commitment: Some(CommitmentConfig::processed()),
            accounts: Some(RpcSimulateTransactionAccountsConfig {
                encoding: Some(UiAccountEncoding::Base64),
                addresses: vec![],
            }),
            ..Default::default()
        };

        let result = self
            .rpc_client
            .simulate_transaction_with_config(tx, config)
            .await?;

        let logs = result.value.logs.unwrap_or_default();

        if let Some(err) = result.value.err {
            return Err(anyhow!("Simulation failed: {:?}, logs: {:?}", err, logs));
        }

        let units_consumed = result.value.units_consumed.unwrap_or(200_000);
        let optimized_units = ((units_consumed as f64 * multiplier) as u32).min(max_compute_units);

        debug!(
            "Simulation successful - consumed: {}, final: {}",
            units_consumed, optimized_units
        );

        Ok((optimized_units, logs))
    }

    /// Build optimized transaction with proper compute budget
    pub fn build_transaction_with_compute_budget(
        &self,
        mut instructions: Vec<Instruction>,
        payer: &Pubkey,
        blockhash: Hash,
        compute_units: Option<u32>,
        priority_fee: Option<u64>,
    ) -> Transaction {
        let mut final_instructions = Vec::new();

        // Add compute budget if specified
        if let Some(units) = compute_units {
            final_instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(units));
        }

        // Add priority fee if specified (in microlamports)
        if let Some(fee) = priority_fee {
            if fee > 0 {
                final_instructions.push(ComputeBudgetInstruction::set_compute_unit_price(fee));
            }
        }

        // Add the actual instructions
        final_instructions.append(&mut instructions);

        // Build transaction with legacy message for compatibility
        let mut tx = Transaction::new_with_payer(&final_instructions, Some(payer));
        tx.message.recent_blockhash = blockhash;
        tx
    }

    /// Get the RPC client
    pub fn rpc_client(&self) -> &Arc<RpcClient> {
        &self.rpc_client
    }
}
