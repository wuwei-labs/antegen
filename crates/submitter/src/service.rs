use anyhow::{anyhow, Context, Result};
use log::{debug, info, warn};
use solana_client::{
    nonblocking::rpc_client::RpcClient, nonblocking::tpu_client::TpuClient,
    tpu_client::TpuClientConfig,
    rpc_config::{RpcSimulateTransactionAccountsConfig, RpcSimulateTransactionConfig},
};
use solana_quic_client::{QuicConfig, QuicConnectionManager, QuicPool};
use solana_account_decoder::UiAccountEncoding;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    compute_budget::ComputeBudgetInstruction,
    hash::Hash,
    instruction::Instruction,
    message::{v0, VersionedMessage},
    pubkey::Pubkey,
    signature::Signature,
    signature::Keypair,
    transaction::{Transaction, VersionedTransaction},
};
use std::{cmp, sync::Arc, time::{Duration, Instant}};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::{
    ReplayConfig, ReplayConsumer, SubmissionMode, SubmitterMetrics, TpuConfig,
};
use antegen_sdk::{DurableTransactionMessage, ProcessorMessage};
use solana_sdk::clock::Clock;

/// Configuration for the submission service
#[derive(Debug, Clone)]
pub struct SubmissionConfig {
    /// TPU configuration (optional)
    pub tpu_config: Option<TpuConfig>,
    /// Replay configuration
    pub replay_config: ReplayConfig,
}

impl Default for SubmissionConfig {
    fn default() -> Self {
        Self {
            tpu_config: Some(TpuConfig::default()),
            replay_config: ReplayConfig::default(),
        }
    }
}

/// Unified service for all transaction submission and RPC operations
pub struct SubmissionService {
    /// RPC client for blockchain operations
    rpc_client: Arc<RpcClient>,
    /// TPU client for direct submission to leaders
    tpu_client: RwLock<Option<Arc<TpuClient<QuicPool, QuicConnectionManager, QuicConfig>>>>,
    /// Current submission mode
    submission_mode: RwLock<SubmissionMode>,
    /// NATS client for durable transactions
    nats_client: Option<async_nats::Client>,
    /// Replay consumer handle
    replay_handle: RwLock<Option<JoinHandle<Result<()>>>>,
    /// Metrics collector
    metrics: Arc<SubmitterMetrics>,
    /// Configuration
    config: SubmissionConfig,
    /// Broadcast channel for clock updates
    clock_broadcaster: tokio::sync::broadcast::Sender<Clock>,
}

impl SubmissionService {
    /// Create a new submission service
    pub async fn new(
        rpc_url: String,
        config: SubmissionConfig,
        metrics: Option<Arc<SubmitterMetrics>>,
    ) -> Result<Self> {
        // Create RPC client
        let rpc_client = Arc::new(RpcClient::new(rpc_url));
        let metrics = metrics.unwrap_or_else(|| Arc::new(SubmitterMetrics::default()));

        // Initialize submission mode (will be updated after connection)
        let submission_mode = RwLock::new(SubmissionMode::Rpc);

        // Initialize NATS client if configured
        let nats_client = if config.replay_config.enable_replay {
            if let Some(ref nats_url) = config.replay_config.nats_url {
                // Connect to NATS
                match async_nats::connect(nats_url).await {
                    Ok(client) => Some(client),
                    Err(e) => {
                        warn!("Failed to connect to NATS: {}, replay disabled", e);
                        None
                    }
                }
            } else {
                warn!("Replay enabled but no NATS URL configured");
                None
            }
        } else {
            None
        };

        // Create broadcast channel for clock updates
        let (clock_broadcaster, _) = tokio::sync::broadcast::channel(100);

        Ok(Self {
            rpc_client,
            tpu_client: RwLock::new(None),
            submission_mode,
            nats_client,
            replay_handle: RwLock::new(None),
            metrics,
            config,
            clock_broadcaster,
        })
    }

    /// Broadcast a clock update to all retry tasks
    pub fn broadcast_clock(&self, clock: Clock) {
        let _ = self.clock_broadcaster.send(clock);
    }
    
    /// Initialize the service (wait for RPC and create TPU client)
    pub async fn initialize(&self) -> Result<()> {
        // Wait for RPC to be available
        self.wait_for_rpc_availability(Duration::from_secs(300))
            .await
            .context("Failed to connect to RPC server")?;

        // RPC connection established

        // Try to create TPU client if configured
        if let Some(ref tpu_config) = self.config.tpu_config {
            if matches!(
                tpu_config.mode,
                SubmissionMode::Tpu | SubmissionMode::TpuWithFallback
            ) {
                match self.create_tpu_client(tpu_config).await {
                    Ok(client) => {
                        // TPU client initialized
                        *self.tpu_client.write().await = Some(Arc::new(client));
                        *self.submission_mode.write().await = tpu_config.mode;
                    }
                    Err(e) => {
                        warn!("Failed to create TPU client: {}, using RPC only", e);
                        *self.submission_mode.write().await = SubmissionMode::Rpc;
                    }
                }
            }
        } else {
            // TPU disabled by configuration
        }

        // Start replay consumer if configured
        if let Some(ref nats_client) = self.nats_client {
            self.start_replay_consumer(nats_client.clone()).await?;
        }

        Ok(())
    }

    /// Get the cached RPC client
    pub fn rpc_client(&self) -> &Arc<RpcClient> {
        &self.rpc_client
    }

    /// Submit a transaction using configured mode with automatic simulation
    pub async fn submit_with_options(
        &self, 
        instructions: Vec<Instruction>,
        payer: &Pubkey,
        signers: &[&Keypair],
        simulate: bool,
        thread_pubkey: Option<&Pubkey>,
    ) -> Result<Signature> {
        // Get blockhash
        let blockhash = if let Some(nonce_ix) = instructions.first() {
            // Check if this is a durable transaction with nonce
            if nonce_ix.program_id == solana_sdk::system_program::ID 
                && !nonce_ix.data.is_empty() 
                && nonce_ix.data[0] == 4 {
                // Extract nonce account from first instruction
                if let Some(nonce_pubkey) = nonce_ix.accounts.first() {
                    self.get_nonce_blockhash(&nonce_pubkey.pubkey).await?
                } else {
                    self.rpc_client.get_latest_blockhash().await?
                }
            } else {
                self.rpc_client.get_latest_blockhash().await?
            }
        } else {
            self.rpc_client.get_latest_blockhash().await?
        };

        // Build transaction with optional simulation
        let tx = if simulate && thread_pubkey.is_some() {
            // Simulate and optimize
            let initial_tx = VersionedTransaction::try_new(
                VersionedMessage::V0(v0::Message::try_compile(
                    payer,
                    &instructions,
                    &[], // No lookup tables for now
                    blockhash,
                )?),
                signers,
            )?;
            
            info!("SUBMITTER: Starting simulation at {}",
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());
            let (optimized_cu, _logs) = self.simulate_and_optimize_transaction(
                &initial_tx,
                thread_pubkey.unwrap(),
                1.2,  // Default multiplier
                1_400_000,  // Default max CU
                None,  // No min context slot
            ).await?;
            info!("SUBMITTER: Simulation complete, CU: {} at {}",
                optimized_cu, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());
            
            // Rebuild with optimized CU
            let mut final_instructions = vec![
                ComputeBudgetInstruction::set_compute_unit_limit(optimized_cu),
            ];
            final_instructions.extend(instructions);
            
            VersionedTransaction::try_new(
                VersionedMessage::V0(v0::Message::try_compile(
                    payer,
                    &final_instructions,
                    &[], // No lookup tables for now
                    blockhash,
                )?),
                signers,
            )?
        } else {
            // No simulation, just build transaction
            VersionedTransaction::try_new(
                VersionedMessage::V0(v0::Message::try_compile(
                    payer,
                    &instructions,
                    &[], // No lookup tables for now
                    blockhash,
                )?),
                signers,
            )?
        };
        
        self.submit_transaction(&tx).await
    }
    
    /// Submit a pre-built transaction
    pub async fn submit(&self, tx: &VersionedTransaction) -> Result<Signature> {
        self.submit_transaction(tx).await
    }
    
    /// Internal submit method
    async fn submit_transaction(&self, tx: &VersionedTransaction) -> Result<Signature> {
        let mode = *self.submission_mode.read().await;

        // Submit transaction

        match mode {
            SubmissionMode::Tpu => self.submit_via_tpu(tx).await,
            SubmissionMode::Rpc => self.submit_via_rpc(tx).await,
            SubmissionMode::TpuWithFallback => match self.submit_via_tpu(tx).await {
                Ok(sig) => Ok(sig),
                Err(tpu_err) => {
                    warn!("TPU submission failed: {}, falling back to RPC", tpu_err);
                    self.submit_via_rpc(tx).await
                }
            },
            SubmissionMode::Both => {
                // Try both TPU and RPC in parallel
                let tpu_future = self.submit_via_tpu(tx);
                let rpc_future = self.submit_via_rpc(tx);
                
                // Return whichever succeeds first
                tokio::select! {
                    tpu_result = tpu_future => tpu_result,
                    rpc_result = rpc_future => rpc_result,
                }
            }
        }
    }

    /// Submit multiple transactions in batch
    pub async fn submit_batch(&self, txs: &[VersionedTransaction]) -> Result<Vec<Result<Signature>>> {
        if txs.is_empty() {
            return Ok(Vec::new());
        }

        let mode = *self.submission_mode.read().await;
        // Batch submitting transactions

        match mode {
            SubmissionMode::Tpu => self.submit_batch_via_tpu(txs).await,
            SubmissionMode::Rpc => self.submit_batch_via_rpc(txs).await,
            SubmissionMode::TpuWithFallback => match self.submit_batch_via_tpu(txs).await {
                Ok(results) => Ok(results),
                Err(tpu_err) => {
                    warn!(
                        "Batch TPU submission failed: {}, falling back to RPC",
                        tpu_err
                    );
                    self.submit_batch_via_rpc(txs).await
                }
            },
            SubmissionMode::Both => {
                // Try both in parallel and return the first to succeed
                let tpu_future = self.submit_batch_via_tpu(txs);
                let rpc_future = self.submit_batch_via_rpc(txs);
                
                tokio::select! {
                    tpu_result = tpu_future => tpu_result,
                    rpc_result = rpc_future => rpc_result,
                }
            }
        }
    }

    /// Publish a durable transaction to NATS for replay
    pub async fn publish_durable_transaction(&self, msg: DurableTransactionMessage) -> Result<()> {
        if let Some(ref nats_client) = self.nats_client {
            let payload = serde_json::to_vec(&msg)?;
            nats_client
                .publish("antegen.durable_txs", payload.into())
                .await?;
            debug!(
                "Published durable transaction for thread {}",
                msg.thread_pubkey
            );

            if let Some(ref metrics) = Some(&self.metrics) {
                metrics.durable_tx_published();
            }
        }
        Ok(())
    }

    /// Get blockhash from nonce account
    pub async fn get_nonce_blockhash(&self, nonce_pubkey: &Pubkey) -> Result<Hash> {
        // Fetch nonce account blockhash
        
        // Always bypass cache for nonce accounts since they can be advanced
        let account = self.rpc_client.get_account(nonce_pubkey).await?;
        debug!("Nonce account data length: {}", account.data.len());
        
        // Use proper nonce utilities to extract data
        let nonce_data = solana_rpc_client_nonce_utils::data_from_account(&account)
            .map_err(|e| anyhow!("Failed to extract nonce data: {}", e))?;
        
        let blockhash = nonce_data.blockhash();
        // Got nonce blockhash
        Ok(blockhash)
    }

    /// Simulate transaction and optimize compute units
    pub async fn simulate_and_optimize_transaction(
        &self,
        tx: &VersionedTransaction,
        thread_pubkey: &Pubkey,
        cu_multiplier: f64,
        max_compute_units: u32,
        min_context_slot: Option<u64>,
    ) -> Result<(u32, Vec<String>)> {
        // Start timing simulation
        let sim_start = Instant::now();
        
        self.metrics.rpc_request("_simulate_transaction");
        let sim_result = match self.rpc_client.simulate_transaction_with_config(
            tx,
            RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: true,
                commitment: Some(CommitmentConfig::processed()),
                accounts: Some(RpcSimulateTransactionAccountsConfig {
                    encoding: Some(UiAccountEncoding::Base64Zstd),
                    addresses: vec![thread_pubkey.to_string()],
                }),
                min_context_slot,
                ..Default::default()
            },
        ).await {
            Ok(result) => result,
            Err(err) => {
                // Check for min context slot error
                let error_str = err.to_string();
                if error_str.contains("Minimum context slot has not been reached") 
                    || error_str.contains("MinContextSlotNotReached")
                    || error_str.contains("-32016") {
                    debug!("RPC not caught up to slot {:?}, will retry", min_context_slot);
                    return Err(anyhow!("RPC not caught up to slot {:?}", min_context_slot));
                }
                return Err(anyhow!("Simulation failed: {}", err));
            }
        };
        
        // Record simulation duration
        let duration = sim_start.elapsed();
        self.metrics.submission_latency.record(duration.as_millis() as f64, &[]);
        
        // Check for simulation errors
        if let Some(err) = sim_result.value.err {
            let logs = sim_result.value.logs.clone().unwrap_or_default();
            warn!("Simulation failed for thread {}: {:?}", thread_pubkey, err);
            return Err(anyhow!("Simulation failed: {:?}, logs: {:?}", err, logs));
        }
        
        // Verify thread account was returned
        let _thread_account = sim_result.value.accounts
            .and_then(|accounts| accounts.get(0).cloned().flatten())
            .ok_or_else(|| anyhow!("No thread account in simulation response"))?;
        
        // Calculate optimized compute units with multiplier
        let optimized_cu = if let Some(units_consumed) = sim_result.value.units_consumed {
            let with_multiplier = (units_consumed as f64 * cu_multiplier) as u32;
            let final_cu = cmp::min(with_multiplier, max_compute_units);
            debug!(
                "Simulation successful for thread {} - consumed: {}, final: {}",
                thread_pubkey, units_consumed, final_cu
            );
            final_cu
        } else {
            warn!("No compute units consumed in simulation, using max: {}", max_compute_units);
            max_compute_units
        };
        
        let logs = sim_result.value.logs.unwrap_or_default();
        Ok((optimized_cu, logs))
    }

    /// Build optimized transaction with proper compute budget
    pub fn build_transaction_with_compute_budget(
        &self,
        instructions: Vec<Instruction>,
        payer: &Pubkey,
        _blockhash: Hash,
        compute_units: Option<u32>,
    ) -> Transaction {
        let mut final_instructions = vec![];
        
        // Add compute budget instruction if specified
        if let Some(cu) = compute_units {
            final_instructions.push(
                ComputeBudgetInstruction::set_compute_unit_limit(cu)
            );
        }
        
        // Add the actual instructions
        final_instructions.extend(instructions);
        
        Transaction::new_with_payer(
            &final_instructions,
            Some(payer),
        )
    }

    /// Check if transaction uses durable nonce
    pub fn is_durable_transaction(&self, tx: &Transaction) -> bool {
        tx.message.instructions.iter().any(|ix| {
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

    /// Update submission mode
    pub async fn set_mode(&self, mode: SubmissionMode) -> Result<()> {
        if matches!(mode, SubmissionMode::Tpu | SubmissionMode::TpuWithFallback | SubmissionMode::Both) {
            let tpu_client_guard = self.tpu_client.read().await;
            if tpu_client_guard.is_none() {
                return Err(anyhow!("Cannot set TPU mode: TPU client not available"));
            }
        }

        *self.submission_mode.write().await = mode;
        // Submission mode updated
        Ok(())
    }

    /// Check if TPU client is available
    pub async fn has_tpu_client(&self) -> bool {
        self.tpu_client.read().await.is_some()
    }

    // ===== Private helper methods =====

    async fn wait_for_rpc_availability(&self, max_wait: Duration) -> Result<()> {
        let start = Instant::now();
        let mut delay = Duration::from_secs(1);
        let max_delay = Duration::from_secs(30);
        let mut last_log = Instant::now();

        // Wait for RPC to become available
        loop {
            match self.rpc_client.get_health().await {
                Ok(_) => {
                    // RPC server available
                    return Ok(());
                }
                Err(e) => {
                    debug!("RPC not ready yet: {}", e);
                }
            }

            if start.elapsed() > max_wait {
                return Err(anyhow!(
                    "RPC server at {} failed to become available after {} seconds",
                    self.rpc_client.url(),
                    max_wait.as_secs()
                ));
            }

            if last_log.elapsed() > Duration::from_secs(30) {
                // Still waiting for RPC
                last_log = Instant::now();
            }

            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(max_delay);
        }
    }

    async fn create_tpu_client(
        &self,
        config: &TpuConfig,
    ) -> Result<TpuClient<QuicPool, QuicConnectionManager, QuicConfig>> {
        let rpc_url = self.rpc_client.url();
        let ws_url = rpc_url
            .replace("http://", "ws://")
            .replace("https://", "wss://")
            .replace("8899", "8900");

        // Creating TPU client

        let tpu_client_config = TpuClientConfig {
            fanout_slots: config.fanout_slots,
            ..TpuClientConfig::default()
        };

        let mut attempts = 0;
        let max_attempts = 3;

        loop {
            attempts += 1;

            match TpuClient::new(
                "antegen-submitter",
                self.rpc_client.clone(),
                &ws_url,
                tpu_client_config.clone(),
            )
            .await
            {
                Ok(client) => {
                    // TPU client created
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

    async fn submit_via_tpu(&self, tx: &VersionedTransaction) -> Result<Signature> {
        let tpu_client_guard = self.tpu_client.read().await;
        let tpu_client = tpu_client_guard
            .as_ref()
            .ok_or_else(|| anyhow!("TPU client not available"))?;

        let signature = tx.signatures[0];
        // Submit to TPU

        let wire_transaction = bincode::serialize(tx)?;

        if !tpu_client.send_wire_transaction(wire_transaction).await {
            return Err(anyhow!("Failed to send transaction to TPU"));
        }

        // Sent to TPU, now confirm it
        debug!("Transaction sent to TPU, waiting for confirmation: {}", signature);
        
        // Wait for confirmation using RPC
        let confirmation = self.rpc_client
            .confirm_transaction(&signature)
            .await;
        
        match confirmation {
            Ok(_) => {
                info!("Transaction confirmed via TPU: {}", signature);
                if let Some(ref metrics) = Some(&self.metrics) {
                    metrics.transaction_submitted("tpu");
                }
                Ok(signature)
            }
            Err(e) => {
                warn!("Transaction sent to TPU but not confirmed: {}: {}", signature, e);
                Err(anyhow!("Transaction not confirmed: {}", e))
            }
        }
    }

    async fn submit_via_rpc(&self, tx: &VersionedTransaction) -> Result<Signature> {
        // Submit via RPC

        match tokio::time::timeout(
            Duration::from_secs(30),
            self.rpc_client.send_and_confirm_transaction(tx),
        )
        .await
        {
            Ok(Ok(signature)) => {
                // Transaction confirmed

                if let Some(ref metrics) = Some(&self.metrics) {
                    metrics.transaction_submitted("rpc");
                }

                Ok(signature)
            }
            Ok(Err(e)) => Err(e.into()),
            Err(_) => Err(anyhow!("RPC submission timed out after 30 seconds")),
        }
    }

    async fn submit_batch_via_tpu(&self, txs: &[VersionedTransaction]) -> Result<Vec<Result<Signature>>> {
        let tpu_client_guard = self.tpu_client.read().await;
        let tpu_client = tpu_client_guard
            .as_ref()
            .ok_or_else(|| anyhow!("TPU client not available"))?;

        let mut results = Vec::with_capacity(txs.len());
        let mut wire_transactions = Vec::with_capacity(txs.len());

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

            let batch_sent = tpu_client
                .try_send_wire_transaction_batch(wire_transactions)
                .await
                .is_ok();

            if !batch_sent {
                warn!("Failed to send transactions in batch to TPU");
                for result in &mut results {
                    if result.is_ok() {
                        *result = Err(anyhow!("Batch TPU submission failed"));
                    }
                }
            } else {
                // Batch sent to TPU, now confirm them
                debug!("Batch sent {} transactions to TPU, confirming...", txs.len());
                
                // Confirm each transaction
                for (_i, result) in results.iter_mut().enumerate() {
                    if let Ok(sig) = result {
                        match self.rpc_client.confirm_transaction(sig).await {
                            Ok(_) => {
                                debug!("Transaction confirmed: {}", sig);
                            }
                            Err(e) => {
                                warn!("Transaction not confirmed: {}: {}", sig, e);
                                *result = Err(anyhow!("Transaction not confirmed: {}", e));
                            }
                        }
                    }
                }

                if let Some(ref metrics) = Some(&self.metrics) {
                    metrics.batch_submitted("tpu", txs.len());
                }
            }
        }

        Ok(results)
    }

    async fn submit_batch_via_rpc(&self, txs: &[VersionedTransaction]) -> Result<Vec<Result<Signature>>> {
        // Batch submit via RPC

        use futures::future::join_all;

        let mut futures = Vec::new();
        for tx in txs {
            futures.push(self.submit_via_rpc(tx));
        }

        let results = join_all(futures).await;

        if let Some(ref metrics) = Some(&self.metrics) {
            metrics.batch_submitted("rpc", txs.len());
        }

        Ok(results)
    }

    async fn start_replay_consumer(&self, nats_client: async_nats::Client) -> Result<()> {
        // Start replay consumer

        let mut consumer = ReplayConsumer::new(
            nats_client,
            Arc::new(self.clone()), // We'll need to implement Clone
            self.rpc_client.clone(),
            self.config.replay_config.clone(),
        )
        .await?;

        let handle = tokio::spawn(async move { consumer.run().await });

        *self.replay_handle.write().await = Some(handle);
        Ok(())
    }
    
    /// Process incoming transaction messages from processor
    /// Handle a single transaction with honeybadger retry logic
    async fn handle_transaction_task(
        self: Arc<Self>,
        msg: antegen_sdk::TransactionMessage,
        executor_keypair: Arc<solana_sdk::signature::Keypair>,
    ) {
        info!("Starting task for thread {}", msg.thread_pubkey);
        
        // Create a TransactionSubmitter for this task
        let submitter = crate::submitter::TransactionSubmitter::from_client(
            self.rpc_client.clone(),
            self.config.tpu_config.clone(),
            self.metrics.clone(),
            self.clock_broadcaster.subscribe(),
        );
        
        // Initialize TPU if configured
        if let Err(e) = submitter.initialize_tpu().await {
            warn!("Failed to initialize TPU for thread {}: {}", msg.thread_pubkey, e);
            // Continue anyway - will fall back to RPC
        }
        
        // Submit using the honeybadger approach
        // This will run until timeout - success is determined by thread updates
        match submitter.submit(msg.instructions, executor_keypair).await {
            Ok(()) => {
                // Should never happen - submit only returns on timeout (error)
                warn!("Unexpected return from honeybadger submit for thread {}", msg.thread_pubkey);
            }
            Err(e) => {
                // Timeout reached without thread update
                log::warn!("Honeybadger timeout for thread {}: {}", msg.thread_pubkey, e);
                // Metrics already tracked in submitter
            }
        }
    }

    pub async fn process_transaction_messages(
        self: Arc<Self>,
        receiver: crossbeam::channel::Receiver<ProcessorMessage>,
        executor_keypair: Arc<solana_sdk::signature::Keypair>,
    ) -> Result<()> {
        info!("Starting transaction message processor");
        
        // Initialize service (wait for RPC) before processing
        self.initialize().await?;
        
        // Simple message routing loop - wrap blocking recv in spawn_blocking
        loop {
            let receiver_clone = receiver.clone();
            let recv_result = tokio::task::spawn_blocking(move || receiver_clone.recv())
                .await
                .map_err(|e| anyhow!("Spawn blocking task failed: {}", e))?;
            
            match recv_result {
                Ok(ProcessorMessage::Clock(clock)) => {
                    // Broadcast clock update immediately to all retry tasks
                    debug!("Broadcasted clock update: slot {}, timestamp {}", clock.slot, clock.unix_timestamp);
                    let _ = self.clock_broadcaster.send(clock);
                }
                Ok(ProcessorMessage::Transaction(msg)) => {
                    info!("Received transaction message for thread {}", msg.thread_pubkey);
                    // Spawn independent task for this transaction
                    let service_clone = self.clone();
                    let executor_keypair_clone = executor_keypair.clone();
                    
                    let thread_pubkey = msg.thread_pubkey;
                    let handle = tokio::spawn(async move {
                        log::debug!("Task spawned, calling handle_transaction_task for thread {}", thread_pubkey);
                        service_clone.handle_transaction_task(msg, executor_keypair_clone).await;
                        log::debug!("Task completed for thread {}", thread_pubkey);
                    });
                    debug!("Spawned task for transaction submission: {:?}", handle);
                }
                Err(_) => {
                    info!("Transaction receiver channel closed, shutting down processor");
                    break;
                }
            }
        }
        
        Ok(())
    }
}

// Implement Clone for SubmissionService (needed for replay consumer)
impl Clone for SubmissionService {
    fn clone(&self) -> Self {
        Self {
            rpc_client: self.rpc_client.clone(),
            tpu_client: RwLock::new(None), // Don't clone TPU client
            submission_mode: RwLock::new(SubmissionMode::Rpc), // Reset to RPC
            nats_client: self.nats_client.clone(),
            replay_handle: RwLock::new(None), // Don't clone handle
            metrics: self.metrics.clone(),
            config: self.config.clone(),
            clock_broadcaster: self.clock_broadcaster.clone(),
        }
    }
}
