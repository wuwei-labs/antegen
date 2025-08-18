use anyhow::Result;
use async_trait::async_trait;
use log::{debug, info};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_program::pubkey::Pubkey;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::sources::{ClaimedThread, ClaimedThreadSource};
use anchor_lang::AccountDeserialize;
use antegen_thread_program::state::{FiberState, Thread, ThreadConfig, Trigger};

/// RPC source that monitors for unclaimed threads ready to execute
pub struct RpcSource {
    rpc_client: Arc<RpcClient>,
    poll_interval: Duration,
    processed_threads: HashSet<Pubkey>,
    observer_keypair: Option<Pubkey>, // Optional observer keypair for external executors
}

impl RpcSource {
    pub fn new(
        rpc_client: Arc<RpcClient>,
        poll_interval: Duration,
        observer_keypair: Option<Pubkey>,
    ) -> Self {
        Self {
            rpc_client,
            poll_interval,
            processed_threads: HashSet::new(),
            observer_keypair,
        }
    }

    /// Check if a thread is ready to execute
    async fn is_thread_ready(&self, thread: &Thread) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        match &thread.trigger {
            Trigger::Now => true, // Execute immediately
            Trigger::Timestamp { unix_ts } => {
                // Check if we've reached the timestamp
                now >= *unix_ts
            }
            Trigger::Interval { .. } => {
                // For interval triggers, check trigger context
                // Simplified for now - would need to check last execution time
                true
            }
            Trigger::Cron { .. } => {
                // For cron triggers, would need to parse schedule
                // Simplified for now
                true
            }
            Trigger::Account { .. } => {
                // For account triggers, assume ready (would need more complex logic)
                true
            }
            Trigger::Slot { slot } => {
                // Check if we've reached the slot
                // Would need to get current slot from RPC
                true
            }
            Trigger::Epoch { epoch } => {
                // Check if we've reached the epoch
                // Would need to get current epoch from RPC
                true
            }
        }
    }

    /// Check if a thread can be executed by external executors
    async fn can_external_execute(&self, thread_pubkey: &Pubkey, thread: &Thread) -> Result<bool> {
        // Get fiber PDA for current exec_index
        let fiber_pubkey = Pubkey::find_program_address(
            &[
                b"thread_fiber",
                thread_pubkey.as_ref(),
                &[thread.exec_index],
            ],
            &antegen_thread_program::ID,
        )
        .0;

        // Check if fiber exists and is unclaimed
        match self.rpc_client.get_account(&fiber_pubkey).await {
            Ok(account) => {
                let fiber = FiberState::try_deserialize(&mut account.data.as_slice())?;

                if fiber.observer.is_some() {
                    // Fiber is claimed
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64;

                    // Get config to check priority window
                    let config_pubkey = Pubkey::find_program_address(
                        &[b"thread_config"],
                        &antegen_thread_program::ID,
                    )
                    .0;
                    let config_account = self.rpc_client.get_account(&config_pubkey).await?;
                    let config =
                        ThreadConfig::try_deserialize(&mut config_account.data.as_slice())?;

                    // Check if priority window has expired
                    let priority_expired =
                        fiber.claimed_at + config.priority_window < now;

                    Ok(priority_expired)
                } else {
                    // Fiber exists but is unclaimed - can execute
                    Ok(true)
                }
            }
            Err(_) => {
                // Fiber doesn't exist yet - cannot execute
                Ok(false)
            }
        }
    }

    /// Scan for executable threads
    async fn scan_threads(&mut self) -> Result<Vec<ClaimedThread>> {
        let executable_threads = Vec::new();

        // Get all thread accounts (simplified - in production would use getProgramAccounts with filters)
        // For now, just return empty - real implementation would scan program accounts
        info!("RPC_SOURCE: Scanning for executable threads...");

        // This is a placeholder - in production you'd use:
        // let accounts = self.rpc_client.get_program_accounts_with_config(
        //     &antegen_thread_program::ID,
        //     RpcProgramAccountsConfig { ... }
        // ).await?;

        // Then filter for threads that are:
        // 1. Ready based on trigger
        // 2. Either unclaimed or past priority window
        // 3. Not already processed

        Ok(executable_threads)
    }
}

#[async_trait]
impl ClaimedThreadSource for RpcSource {
    async fn receive(&mut self) -> Result<Option<ClaimedThread>> {
        // Poll RPC periodically for executable threads
        let threads = self.scan_threads().await?;

        if let Some(thread) = threads.into_iter().next() {
            // Mark as processed to avoid duplicates
            self.processed_threads.insert(thread.thread_pubkey);
            debug!(
                "RPC_SOURCE: Found executable thread: {}",
                thread.thread_pubkey
            );
            Ok(Some(thread))
        } else {
            // Sleep before next poll
            tokio::time::sleep(self.poll_interval).await;
            Ok(None)
        }
    }

    async fn ack(&mut self, thread_pubkey: &Pubkey) -> Result<()> {
        debug!(
            "RPC_SOURCE: Thread execution acknowledged: {}",
            thread_pubkey
        );
        // Keep in processed set to avoid re-processing
        Ok(())
    }

    async fn nack(&mut self, thread_pubkey: &Pubkey) -> Result<()> {
        debug!(
            "RPC_SOURCE: Thread execution failed, will retry: {}",
            thread_pubkey
        );
        // Remove from processed set to allow retry
        self.processed_threads.remove(thread_pubkey);
        Ok(())
    }

    fn name(&self) -> &str {
        "RpcSource"
    }
}
