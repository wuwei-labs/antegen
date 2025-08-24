use anyhow::{anyhow, Context, Result};
use log::{debug, info, warn};
use solana_client::{
    nonblocking::rpc_client::RpcClient, nonblocking::tpu_client::TpuClient,
    tpu_client::TpuClientConfig,
};
use solana_quic_client::{QuicConfig, QuicConnectionManager, QuicPool};
use solana_sdk::{
    commitment_config::CommitmentConfig, signature::Signature, transaction::Transaction,
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Submission mode for transactions
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SubmissionMode {
    /// Use TPU client for submission
    Tpu,
    /// Use RPC for submission  
    Rpc,
    /// Try TPU first, fallback to RPC
    TpuWithFallback,
}

impl Default for SubmissionMode {
    fn default() -> Self {
        SubmissionMode::TpuWithFallback
    }
}

/// Configuration for TPU client
#[derive(Debug, Clone)]
pub struct TpuConfig {
    /// Maximum retries for TPU submission
    pub max_retries: usize,
    /// Number of leaders to send to in parallel
    pub fanout_slots: u64,
    /// Connection pool size
    pub connection_pool_size: usize,
    /// Submission mode
    pub mode: SubmissionMode,
}

impl Default for TpuConfig {
    fn default() -> Self {
        Self {
            max_retries: 5,
            fanout_slots: 12, // Send to 12 leader slots
            connection_pool_size: 4,
            mode: SubmissionMode::TpuWithFallback,
        }
    }
}

/// Wait for RPC server to become available
async fn wait_for_rpc_availability(
    rpc_client: &RpcClient,
    max_wait: Duration,
) -> Result<()> {
    let start = Instant::now();
    let mut delay = Duration::from_secs(1);
    let max_delay = Duration::from_secs(30);
    let mut last_log = Instant::now();
    
    info!("Waiting for RPC server to become available at {}...", rpc_client.url());
    
    loop {
        // Try to connect to RPC
        match rpc_client.get_health().await {
            Ok(_) => {
                info!("RPC server is available (took {:.1}s)", 
                    start.elapsed().as_secs_f32());
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
            info!("Still waiting for RPC server (elapsed: {:.0}s of max {}s)...",
                start.elapsed().as_secs(),
                max_wait.as_secs());
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
        wait_for_rpc_availability(&self.rpc_client, Duration::from_secs(300)).await
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
                tpu_client_config.clone()
            ).await {
                Ok(client) => {
                    info!("TPU client created successfully on attempt {}", attempts);
                    return Ok(client);
                }
                Err(e) => {
                    if attempts >= max_attempts {
                        return Err(anyhow!("Failed to create TPU client after {} attempts: {}", 
                            max_attempts, e));
                    }
                    
                    let delay = Duration::from_secs(attempts as u64);
                    warn!("TPU client creation failed (attempt {}/{}): {}, retrying in {:?}...",
                        attempts, max_attempts, e, delay);
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
        }
    }

    /// Submit transaction via TPU
    async fn submit_via_tpu(&self, tx: &Transaction) -> Result<Signature> {
        let tpu_client_guard = self.tpu_client.read().await;
        let tpu_client = tpu_client_guard
            .as_ref()
            .ok_or_else(|| anyhow!("TPU client not available"))?;

        let signature = tx.signatures[0];
        debug!("Submitting to TPU: {}", signature);

        // Send transaction to TPU leaders
        let wire_transaction = bincode::serialize(tx)?;

        if !tpu_client
            .send_wire_transaction(wire_transaction.clone())
            .await
        {
            return Err(anyhow!("Failed to send transaction to TPU"));
        }

        // Send to multiple leaders for redundancy
        for i in 0..self.config.max_retries {
            debug!("TPU submission attempt {} for {}", i + 1, signature);

            // Send again for redundancy
            if !tpu_client
                .send_wire_transaction(wire_transaction.clone())
                .await
            {
                warn!("Failed to resend transaction to TPU (attempt {})", i + 1);
            }

            // Check if transaction landed
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Quick check for confirmation
            if let Ok(Some(status)) = self
                .rpc_client
                .get_signature_status_with_commitment(&signature, CommitmentConfig::processed())
                .await
            {
                if status.is_ok() {
                    info!("Transaction {} confirmed via TPU", signature);
                    return Ok(signature);
                }
            }

            // Brief delay before retry
            tokio::time::sleep(Duration::from_millis(500 * (i as u64 + 1))).await;
        }

        // After all retries, do a final confirmation check
        match self
            .rpc_client
            .get_signature_status_with_commitment(&signature, CommitmentConfig::confirmed())
            .await?
        {
            Some(status) if status.is_ok() => {
                info!("Transaction {} eventually confirmed via TPU", signature);
                Ok(signature)
            }
            _ => Err(anyhow!(
                "TPU submission failed after {} retries",
                self.config.max_retries
            )),
        }
    }

    /// Submit transaction via RPC
    async fn submit_via_rpc(&self, tx: &Transaction) -> Result<Signature> {
        debug!("Submitting via RPC");

        // Use send_and_confirm for reliability
        let signature = self.rpc_client.send_and_confirm_transaction(tx).await?;

        info!("Transaction {} confirmed via RPC", signature);
        Ok(signature)
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
