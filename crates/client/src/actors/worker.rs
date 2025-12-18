//! Worker Actor
//!
//! The WorkerActor handles atomic execution of a single thread:
//! 1. Check load balancer for process decision
//! 2. Build transaction instructions using ExecutorLogic
//! 3. Submit transaction with retries (up to 5 attempts)
//! 4. Wait for confirmation (with timeout)
//! 5. Report result back to ProcessorFactory
//!
//! Includes deadman's switch to prevent runaway workers.

use crate::actors::messages::{ExecutionResult, ProcessorMessage, WorkerMessage};
use crate::executor::ExecutorLogic;
use crate::load_balancer::{LoadBalancer, ProcessDecision};
use crate::resources::SharedResources;
use antegen_thread_program::state::Thread;
use ractor::{Actor, ActorProcessingErr, ActorRef};
use solana_sdk::{clock::Clock, message::Message, pubkey::Pubkey, transaction::Transaction};
use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, OwnedSemaphorePermit};

/// Maximum number of submission attempts
const MAX_ATTEMPTS: u32 = 5;

/// Timeout for waiting for confirmation (seconds)
const CONFIRMATION_TIMEOUT_SECS: u64 = 30;

/// Base delay between retries (milliseconds)
const BASE_RETRY_DELAY_MS: u64 = 500;

/// Interval for re-sending via TPU during confirmation polling (milliseconds)
const TPU_RETRY_INTERVAL_MS: u64 = 2000;

pub struct WorkerActor;

pub struct WorkerArgs {
    pub thread_pubkey: Pubkey,
    pub thread: Thread,
    pub is_overdue: bool,
    pub overdue_seconds: i64,
    pub permit: OwnedSemaphorePermit,
    pub processor_ref: ActorRef<ProcessorMessage>,
    pub clock_rx: broadcast::Receiver<Clock>,
    pub resources: SharedResources,
    pub executor: ExecutorLogic,
    pub load_balancer: Arc<LoadBalancer>,
}

pub struct WorkerState {
    thread_pubkey: Pubkey,
    #[allow(dead_code)] // Kept for potential debugging/logging in handle()
    thread: Thread,
    _permit: OwnedSemaphorePermit, // Auto-released on drop
    #[allow(dead_code)] // Kept for future cancellation completion signaling
    processor_ref: ActorRef<ProcessorMessage>,
    cancelled: Arc<AtomicBool>, // Flag for cancellation
}

#[ractor::async_trait]
impl Actor for WorkerActor {
    type Msg = WorkerMessage;
    type State = WorkerState;
    type Arguments = WorkerArgs;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, Box<dyn Error + Send + Sync>> {
        log::debug!("WorkerActor started for thread: {}", args.thread_pubkey);

        let cancelled = Arc::new(AtomicBool::new(false));

        let state = WorkerState {
            thread_pubkey: args.thread_pubkey,
            thread: args.thread.clone(),
            _permit: args.permit,
            processor_ref: args.processor_ref.clone(),
            cancelled: cancelled.clone(),
        };

        // Spawn background task for execution
        let thread_pubkey = args.thread_pubkey;
        let thread = args.thread;
        let is_overdue = args.is_overdue;
        let overdue_seconds = args.overdue_seconds;
        let processor_ref = args.processor_ref;
        let resources = args.resources;
        let executor = args.executor;
        let load_balancer = args.load_balancer;
        let cancelled_flag = cancelled;

        tokio::spawn(async move {
            let result = execute_thread(
                thread_pubkey,
                thread.clone(),
                is_overdue,
                overdue_seconds,
                &resources,
                &executor,
                &load_balancer,
                &cancelled_flag,
            )
            .await;

            // Send result back to processor
            if let Err(e) = processor_ref.send_message(ProcessorMessage::WorkerCompleted(result)) {
                log::error!(
                    "Failed to send completion result for thread {}: {:?}",
                    thread_pubkey,
                    e
                );
            }
        });

        Ok(state)
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            WorkerMessage::Cancel => {
                // Set cancellation flag so background task can check it
                state.cancelled.store(true, Ordering::Relaxed);
                // Note: We don't stop the actor immediately - let the background task
                // detect the flag and send completion message
                Ok(())
            }
        }
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        state: &mut Self::State,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        log::debug!("WorkerActor for {} stopped", state.thread_pubkey);
        Ok(())
    }
}

/// Execute a thread with retries and proper error handling
async fn execute_thread(
    thread_pubkey: Pubkey,
    thread: Thread,
    is_overdue: bool,
    overdue_seconds: i64,
    resources: &SharedResources,
    executor: &ExecutorLogic,
    load_balancer: &LoadBalancer,
    cancelled: &AtomicBool,
) -> ExecutionResult {
    // Check cancellation before starting
    if cancelled.load(Ordering::Relaxed) {
        log::debug!(
            "Worker cancelled before execution for thread: {}",
            thread_pubkey
        );
        return ExecutionResult::failed(
            thread_pubkey,
            "Cancelled before execution".to_string(),
            0,
        );
    }

    // Check load balancer decision
    let decision = match load_balancer
        .should_process(&thread_pubkey, &thread.last_executor, is_overdue, overdue_seconds)
        .await
    {
        Ok(d) => d,
        Err(e) => {
            log::error!(
                "Load balancer error for thread {}: {:?}",
                thread_pubkey,
                e
            );
            return ExecutionResult::failed(
                thread_pubkey,
                format!("Load balancer error: {}", e),
                0,
            );
        }
    };

    match decision {
        ProcessDecision::Skip => {
            log::debug!(
                "Load balancer decided to skip thread {} (owned by another executor)",
                thread_pubkey
            );
            return ExecutionResult::failed(
                thread_pubkey,
                "Skipped by load balancer".to_string(),
                0,
            );
        }
        ProcessDecision::AtCapacity => {
            log::debug!(
                "Load balancer at capacity for thread {}, skipping",
                thread_pubkey
            );
            return ExecutionResult::failed(
                thread_pubkey,
                "At capacity".to_string(),
                0,
            );
        }
        ProcessDecision::Process => {
            log::debug!("Load balancer approved processing thread {}", thread_pubkey);
        }
    }

    // Build transaction instructions using the new executor interface
    let (instructions, priority_fee) = match executor
        .build_execute_transaction(&thread_pubkey, &thread)
        .await
    {
        Ok(result) => result,
        Err(e) => {
            log::error!(
                "Failed to build transaction for thread {}: {:?}",
                thread_pubkey,
                e
            );
            return ExecutionResult::failed(
                thread_pubkey,
                format!("Transaction build failed: {}", e),
                0,
            );
        }
    };

    log::debug!(
        "Built transaction for thread {} with {} instructions, priority_fee={}",
        thread_pubkey,
        instructions.len(),
        priority_fee
    );

    // Retry loop for submission
    let mut attempt = 0;
    let mut last_error = String::new();

    while attempt < MAX_ATTEMPTS {
        attempt += 1;

        // Check cancellation
        if cancelled.load(Ordering::Relaxed) {
            log::debug!(
                "Worker cancelled during execution for thread: {}",
                thread_pubkey
            );
            return ExecutionResult::failed(
                thread_pubkey,
                "Cancelled during execution".to_string(),
                attempt,
            );
        }

        log::debug!(
            "Submitting transaction for thread {} (attempt {}/{})",
            thread_pubkey,
            attempt,
            MAX_ATTEMPTS
        );

        // Get recent blockhash using custom RPC client
        let (blockhash, _) = match resources.rpc_client.get_latest_blockhash().await {
            Ok(bh) => bh,
            Err(e) => {
                last_error = format!("Failed to get blockhash: {}", e);
                log::warn!(
                    "Failed to get blockhash for thread {} (attempt {}): {:?}",
                    thread_pubkey,
                    attempt,
                    e
                );
                tokio::time::sleep(Duration::from_millis(
                    BASE_RETRY_DELAY_MS * (1 << attempt.min(4)),
                ))
                .await;
                continue;
            }
        };

        // Build and sign transaction
        let message = Message::new(&instructions, Some(&executor.pubkey()));
        let tx = Transaction::new(&[executor.keypair().as_ref()], message, blockhash);

        // Compute signature before sending (needed for confirmation polling)
        // TPU submission is fire-and-forget so we need the signature upfront
        let signature = tx.signatures[0];

        log::info!("{}: sent", thread_pubkey);
        log::debug!("  txn: {}", signature);

        // TPU retry loop: send via TPU and poll for confirmation, re-sending every 2s
        // This handles the case where TPU send appears to succeed but transaction doesn't land
        let mut tpu_confirmed = false;
        if let Some(tpu_client) = &resources.tpu_client {
            let start = Instant::now();
            let timeout = Duration::from_secs(CONFIRMATION_TIMEOUT_SECS);
            let mut last_tpu_send = Instant::now();

            // Initial TPU send
            if let Err(e) = tpu_client.send_transaction(&tx).await {
                log::debug!("Initial TPU send failed: {}", e);
            }

            // Combined send + confirmation polling loop
            loop {
                // Check timeout
                if start.elapsed() > timeout {
                    log::debug!("TPU confirmation timeout, falling back to RPC");
                    break;
                }

                // Re-send via TPU every 2 seconds (may hit different leader)
                if last_tpu_send.elapsed() > Duration::from_millis(TPU_RETRY_INTERVAL_MS) {
                    if let Err(e) = tpu_client.send_transaction(&tx).await {
                        log::debug!("TPU re-send failed: {}", e);
                    }
                    last_tpu_send = Instant::now();
                }

                // Check confirmation
                match resources.rpc_client.get_signature_status(&signature).await {
                    Ok(Some(Ok(()))) => {
                        // Confirmed!
                        tpu_confirmed = true;
                        break;
                    }
                    Ok(Some(Err(e))) => {
                        // Transaction failed on-chain - don't retry, return failure
                        log::warn!("{}: transaction failed on-chain: {:?}", thread_pubkey, e);

                        let _ = load_balancer
                            .record_execution_result(&thread_pubkey, false, chrono::Utc::now().timestamp())
                            .await;

                        return ExecutionResult::failed(
                            thread_pubkey,
                            format!("Transaction failed on-chain: {:?}", e),
                            attempt,
                        );
                    }
                    Ok(None) => {
                        // Not yet confirmed, continue polling
                    }
                    Err(e) => {
                        // RPC error, continue polling
                        log::debug!("Error checking signature status: {:?}", e);
                    }
                }

                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }

        if tpu_confirmed {
            log::info!("{}: confirmed", thread_pubkey);
            log::debug!("  txn: {}", signature);

            // Record success in load balancer
            let _ = load_balancer
                .record_execution_result(&thread_pubkey, true, chrono::Utc::now().timestamp())
                .await;

            return ExecutionResult::success(thread_pubkey);
        }

        // Fall back to RPC if TPU not available or TPU loop timed out
        match resources.rpc_client.send_transaction(&tx).await {
            Ok(sig) => {
                log::debug!("Transaction sent via RPC: {}", sig);
            }
            Err(e) => {
                last_error = format!("Transaction send failed: {}", e);
                log::warn!(
                    "Failed to send transaction for thread {} (attempt {}): {:?}",
                    thread_pubkey,
                    attempt,
                    e
                );

                // Record loss in load balancer
                let _ = load_balancer
                    .record_execution_result(&thread_pubkey, false, chrono::Utc::now().timestamp())
                    .await;

                tokio::time::sleep(Duration::from_millis(
                    BASE_RETRY_DELAY_MS * (1 << attempt.min(4)),
                ))
                .await;
                continue;
            }
        }

        // Wait for RPC confirmation
        match wait_for_confirmation(&resources.rpc_client, &signature, CONFIRMATION_TIMEOUT_SECS).await {
            Ok(()) => {
                log::info!("{}: confirmed", thread_pubkey);
                log::debug!("  txn: {}", signature);

                // Record success in load balancer
                let _ = load_balancer
                    .record_execution_result(&thread_pubkey, true, chrono::Utc::now().timestamp())
                    .await;

                return ExecutionResult::success(thread_pubkey);
            }
            Err(e) => {
                last_error = format!("Confirmation failed: {}", e);
                log::warn!(
                    "Transaction confirmation failed for thread {} (attempt {}): {:?}",
                    thread_pubkey,
                    attempt,
                    e
                );

                // Record loss in load balancer
                let _ = load_balancer
                    .record_execution_result(&thread_pubkey, false, chrono::Utc::now().timestamp())
                    .await;

                // Exponential backoff
                if attempt < MAX_ATTEMPTS {
                    tokio::time::sleep(Duration::from_millis(
                        BASE_RETRY_DELAY_MS * (1 << attempt.min(4)),
                    ))
                    .await;
                }
            }
        }
    }

    // All attempts exhausted
    log::error!(
        "All {} attempts failed for thread {}: {}",
        MAX_ATTEMPTS,
        thread_pubkey,
        last_error
    );

    ExecutionResult::failed(thread_pubkey, last_error, attempt)
}

/// Wait for transaction confirmation with timeout
async fn wait_for_confirmation(
    rpc_client: &crate::rpc::RpcPool,
    signature: &solana_sdk::signature::Signature,
    timeout_secs: u64,
) -> Result<(), String> {
    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(timeout_secs);

    loop {
        if start.elapsed() > timeout {
            return Err(format!(
                "Confirmation timeout after {}s",
                timeout_secs
            ));
        }

        match rpc_client.get_signature_status(signature).await {
            Ok(Some(result)) => match result {
                Ok(()) => return Ok(()),
                Err(e) => return Err(format!("Transaction failed: {:?}", e)),
            },
            Ok(None) => {
                // Not yet confirmed, wait and retry
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => {
                // RPC error, could be transient
                log::debug!("Error checking signature status: {:?}", e);
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }
}
