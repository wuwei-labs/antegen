use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use log::{debug, error, info, warn};
use solana_account_decoder::UiAccountEncoding;
use solana_client::nonblocking::pubsub_client::PubsubClient;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig};
use solana_client::rpc_filter::{Memcmp, RpcFilterType};
use solana_program::pubkey::Pubkey;
use solana_sdk::clock::Clock;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::service::ClockState;
use crate::sources::{ScannedThread, ThreadSource};
use anchor_lang::AccountDeserialize;
use antegen_thread_program::state::{Thread, Trigger, TriggerContext};

/// Sysvar clock pubkey
pub const SYSVAR_CLOCK: Pubkey = solana_program::sysvar::clock::ID;

/// RPC source that monitors for unclaimed threads ready to execute
pub struct RpcSource {
    rpc_client: Arc<RpcClient>,
    poll_interval: Duration,
    processed_threads: HashSet<Pubkey>,
    /// Current clock state
    clock_state: ClockState,
    /// Clock subscription handle
    clock_subscription: Option<JoinHandle<()>>,
    /// Clock state receiver for updates
    clock_rx: watch::Receiver<ClockState>,
}

impl RpcSource {
    pub async fn new(
        rpc_client: Arc<RpcClient>,
        poll_interval: Duration,
        ws_url: Option<String>,
    ) -> Result<Self> {
        // Create clock state channel
        let (clock_tx, clock_rx) = watch::channel(ClockState::default());

        // Start clock subscription if websocket URL provided
        let clock_subscription = if let Some(ws_url) = ws_url {
            Some(Self::start_clock_subscription(&ws_url, clock_tx).await?)
        } else {
            None
        };

        Ok(Self {
            rpc_client,
            poll_interval,
            processed_threads: HashSet::new(),
            clock_state: ClockState::default(),
            clock_subscription,
            clock_rx,
        })
    }

    /// Start pubsub subscription for clock updates
    async fn start_clock_subscription(
        ws_url: &str,
        clock_tx: watch::Sender<ClockState>,
    ) -> Result<JoinHandle<()>> {
        let ws_url = ws_url.to_string();

        let handle = tokio::spawn(async move {
            // Create pubsub client inside the spawned task
            let pubsub_client = match PubsubClient::new(&ws_url).await {
                Ok(client) => client,
                Err(e) => {
                    error!("RPC_SOURCE: Failed to create pubsub client: {}", e);
                    return;
                }
            };

            let config = RpcAccountInfoConfig {
                encoding: Some(UiAccountEncoding::Base64),
                commitment: Some(solana_sdk::commitment_config::CommitmentConfig::confirmed()),
                ..RpcAccountInfoConfig::default()
            };

            let (mut stream, unsub) = match pubsub_client
                .account_subscribe(&SYSVAR_CLOCK, Some(config))
                .await
            {
                Ok(sub) => sub,
                Err(e) => {
                    error!("RPC_SOURCE: Failed to subscribe to clock: {}", e);
                    return;
                }
            };

            info!("RPC_SOURCE: Subscribed to clock sysvar updates");

            loop {
                match stream.next().await {
                    Some(response) => {
                        // Decode the base64 data
                        if let Some(data_slice) = response.value.data.decode() {
                            match bincode::deserialize::<Clock>(&data_slice) {
                                Ok(clock) => {
                                    let state = ClockState {
                                        slot: clock.slot,
                                        epoch: clock.epoch,
                                        unix_timestamp: clock.unix_timestamp,
                                    };

                                    if clock_tx.send(state).is_err() {
                                        error!("RPC_SOURCE: Failed to send clock update");
                                        break;
                                    }
                                }
                                Err(e) => {
                                    error!("RPC_SOURCE: Failed to deserialize clock: {}", e);
                                }
                            }
                        }
                    }
                    None => {
                        info!("RPC_SOURCE: Clock stream ended");
                        break;
                    }
                }
            }

            unsub().await;
        });

        Ok(handle)
    }

    /// Get current clock state
    pub fn get_current_clock(&self) -> Option<ClockState> {
        if self.clock_subscription.is_some() {
            // Use subscription clock if available
            Some(self.clock_rx.borrow().clone())
        } else if self.clock_state.unix_timestamp > 0 {
            // Use last known clock from RPC polling
            Some(self.clock_state.clone())
        } else {
            None
        }
    }

    /// Check if a thread's trigger is ready based on trigger context
    fn is_trigger_ready(&self, thread: &Thread, clock: &ClockState) -> bool {
        // Use the trigger_context which has the accurate "next" execution time
        match &thread.trigger_context {
            TriggerContext::Timestamp { next, .. } => {
                // For time-based triggers
                clock.unix_timestamp >= *next
            }
            TriggerContext::Block { next, .. } => {
                // For block-based triggers
                match &thread.trigger {
                    Trigger::Slot { .. } => clock.slot >= *next,
                    Trigger::Epoch { .. } => clock.epoch >= *next,
                    _ => false,
                }
            }
            TriggerContext::Account { .. } => {
                // Account triggers need fresh observation
                false
            }
        }
    }

    /// Scan for executable threads
    async fn scan_threads(&mut self) -> Result<Vec<ScannedThread>> {
        let mut executable_threads = Vec::new();

        // Get current clock state
        let clock = if self.clock_subscription.is_some() {
            // Use subscription clock if available
            self.clock_rx.borrow().clone()
        } else {
            // Fallback to RPC polling
            let slot = self.rpc_client.get_slot().await?;
            let epoch_info = self.rpc_client.get_epoch_info().await?;

            let timestamp = match self.rpc_client.get_block_time(slot).await {
                Ok(t) => t,
                Err(_) => {
                    let recent_slot = slot.saturating_sub(10);
                    match self.rpc_client.get_block_time(recent_slot).await {
                        Ok(t) => t,
                        Err(_) => {
                            warn!("RPC_SOURCE: Unable to get cluster timestamp, skipping scan");
                            return Ok(vec![]);
                        }
                    }
                }
            };

            // Update our internal clock state
            self.clock_state = ClockState {
                slot,
                epoch: epoch_info.epoch,
                unix_timestamp: timestamp,
            };

            self.clock_state.clone()
        };

        // Configure RPC query for thread accounts
        // Thread accounts have a specific discriminator at the beginning
        let config = RpcProgramAccountsConfig {
            filters: Some(vec![
                // Filter for Thread account discriminator
                RpcFilterType::Memcmp(Memcmp::new_raw_bytes(
                    0,
                    vec![0x4e, 0x8f, 0xb9, 0x7d, 0xaf, 0x53, 0xf4, 0xb1], // Thread discriminator
                )),
            ]),
            account_config: RpcAccountInfoConfig {
                encoding: Some(UiAccountEncoding::Base64),
                ..RpcAccountInfoConfig::default()
            },
            ..RpcProgramAccountsConfig::default()
        };

        // Get all thread accounts
        let accounts = self
            .rpc_client
            .get_program_accounts_with_config(&antegen_thread_program::ID, config)
            .await?;

        debug!("RPC_SOURCE: Found {} thread accounts", accounts.len());

        for (pubkey, account) in accounts {
            // Skip if already processed
            if self.processed_threads.contains(&pubkey) {
                continue;
            }

            // Deserialize thread account
            let thread = match Thread::try_deserialize(&mut account.data.as_slice()) {
                Ok(t) => t,
                Err(e) => {
                    warn!("Failed to deserialize thread {}: {}", pubkey, e);
                    continue;
                }
            };

            // Skip paused threads
            if thread.paused {
                continue;
            }

            // Check if trigger is ready using trigger context
            if !self.is_trigger_ready(&thread, &clock) {
                continue;
            }

            // Check if fiber exists for current exec_index
            let fiber_pubkey = Pubkey::find_program_address(
                &[b"thread_fiber", pubkey.as_ref(), &[thread.exec_index]],
                &antegen_thread_program::ID,
            )
            .0;

            // Verify fiber exists
            match self.rpc_client.get_account(&fiber_pubkey).await {
                Ok(_) => {
                    // Fiber exists, thread can be executed
                    info!("RPC_SOURCE: Found executable thread {}", pubkey);

                    executable_threads.push(ScannedThread {
                        thread_pubkey: pubkey,
                        thread: thread.clone(),
                        discovered_at: clock.unix_timestamp,
                    });

                    // Mark as processed to avoid re-queuing in this scan
                    self.processed_threads.insert(pubkey);
                }
                Err(_) => {
                    // Fiber doesn't exist, skip
                    debug!(
                        "RPC_SOURCE: Thread {} has no fiber at index {}",
                        pubkey, thread.exec_index
                    );
                }
            }
        }

        if !executable_threads.is_empty() {
            info!(
                "RPC_SOURCE: Found {} executable threads",
                executable_threads.len()
            );
        }

        Ok(executable_threads)
    }
}

#[async_trait]
impl ThreadSource for RpcSource {
    async fn receive(&mut self) -> Result<Option<ScannedThread>> {
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

impl Drop for RpcSource {
    fn drop(&mut self) {
        if let Some(handle) = self.clock_subscription.take() {
            handle.abort();
        }
    }
}
