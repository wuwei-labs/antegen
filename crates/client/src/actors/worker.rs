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
use crate::trace::{ExecTrace, SendPath};
use antegen_thread_program::state::Thread;
use ractor::{Actor, ActorProcessingErr, ActorRef};
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_sdk::{
    instruction::Instruction, message::Message, pubkey::Pubkey, signature::Signature,
    transaction::Transaction,
};
use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::OwnedSemaphorePermit;

/// Maximum number of submission attempts
const MAX_ATTEMPTS: u32 = 5;

/// Timeout for waiting for confirmation (seconds)
const CONFIRMATION_TIMEOUT_SECS: u64 = 30;

/// Base delay between retries (milliseconds)
const BASE_RETRY_DELAY_MS: u64 = 500;

/// Interval for re-sending via TPU during confirmation polling (milliseconds)
const TPU_RETRY_INTERVAL_MS: u64 = 2000;

/// Retry deadline for trigger-not-ready errors (seconds)
/// This bounds how long we'll retry before giving up
const TRIGGER_RETRY_DEADLINE_SECS: u64 = 10;

/// Check if an error indicates the trigger condition is not yet met (error 6004)
fn is_trigger_not_ready_error(error: &str) -> bool {
    error.contains("Custom(6004)") || error.contains("6004")
}

/// Check if an error indicates the thread is paused (error 6006)
fn is_thread_paused_error(error: &str) -> bool {
    error.contains("Custom(6006)") || error.contains("6006")
}

/// Errors that mean "try again", not "this execution is doomed".
///
/// These are cluster-level conditions, not program failures. Treating them as
/// fatal aborts the retry loop and records a spurious loss against the load
/// balancer.
fn is_retryable_chain_error(error: &str) -> bool {
    const RETRYABLE: &[&str] = &[
        "BlockhashNotFound",
        "AlreadyProcessed",
        "AccountInUse",
        "ClusterMaintenance",
        "WouldExceedMaxBlockCostLimit",
        "WouldExceedMaxAccountCostLimit",
        "WouldExceedAccountDataBlockLimit",
    ];
    RETRYABLE.iter().any(|kind| error.contains(kind))
}

/// Blockhash expiry specifically — the one retryable error that requires
/// re-signing rather than simply resending the same transaction.
fn is_blockhash_expired(error: &str) -> bool {
    error.contains("BlockhashNotFound")
}

/// How long a signed transaction is reused across retries before its blockhash
/// is assumed stale. A blockhash is valid for ~150 slots; staying well inside
/// that keeps the signature stable for the retries that actually matter.
const BLOCKHASH_MAX_AGE: Duration = Duration::from_secs(45);

/// Solana's per-transaction compute ceiling.
const MAX_COMPUTE_UNITS: u32 = 1_400_000;

/// Convert a simulated compute-unit measurement into the limit to request.
///
/// The simulation ran against the `processed` bank at a different clock and
/// possibly different account state, so the estimate is indicative rather than
/// exact. `ComputeBudgetExceeded` costs the whole trigger window plus a retry,
/// which is far more expensive than slightly over-reserving — a request above
/// what is consumed is not charged for the difference.
fn compute_unit_limit(estimate: u64) -> u32 {
    let scaled = (estimate as f64 * 1.25) as u64 + 10_000;
    scaled.min(MAX_COMPUTE_UNITS as u64) as u32
}

pub struct WorkerActor;

pub struct WorkerArgs {
    pub thread_pubkey: Pubkey,
    pub thread: Thread,
    pub is_overdue: bool,
    pub overdue_seconds: i64,
    pub permit: OwnedSemaphorePermit,
    pub processor_ref: ActorRef<ProcessorMessage>,
    pub resources: SharedResources,
    pub executor: ExecutorLogic,
    pub load_balancer: Arc<LoadBalancer>,
    pub trace: ExecTrace,
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

impl Actor for WorkerActor {
    type Msg = WorkerMessage;
    type State = WorkerState;
    type Arguments = WorkerArgs;

    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
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
        let myself_ref = myself.clone();
        let trace = args.trace;

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
                trace,
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

            // Always stop ourselves so the semaphore permit held in WorkerState is
            // released via drop, even if the completion message failed to deliver.
            myself_ref.stop(Some("execution complete".to_string()));
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
    mut trace: ExecTrace,
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
            trace,
        );
    }

    // Re-fetch thread from cache to get latest last_executor
    // This narrows the race window with other executors
    let current_last_executor = match resources.cache.get(&thread_pubkey).await {
        Some(cached) => {
            use anchor_lang::AccountDeserialize;
            match Thread::try_deserialize(&mut cached.data.as_slice()) {
                Ok(fresh_thread) => {
                    // Also check if exec_count changed - thread was already executed
                    if fresh_thread.exec_count != thread.exec_count {
                        log::debug!(
                            "Thread {} exec_count changed ({} -> {}), skipping",
                            thread_pubkey,
                            thread.exec_count,
                            fresh_thread.exec_count
                        );
                        return ExecutionResult::failed(
                            thread_pubkey,
                            "Thread already executed (exec_count changed)".to_string(),
                            0,
                            trace,
                        );
                    }
                    fresh_thread.last_executor
                }
                Err(_) => thread.last_executor, // Fall back to original if deserialize fails
            }
        }
        None => thread.last_executor, // Fall back to original if not in cache
    };

    // Check load balancer decision with fresh last_executor
    let decision = match load_balancer
        .should_process(
            &thread_pubkey,
            &current_last_executor,
            is_overdue,
            overdue_seconds,
        )
        .await
    {
        Ok(d) => d,
        Err(e) => {
            log::error!("Load balancer error for thread {}: {:?}", thread_pubkey, e);
            return ExecutionResult::failed(
                thread_pubkey,
                format!("Load balancer error: {}", e),
                0,
                trace,
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
                trace,
            );
        }
        ProcessDecision::AtCapacity => {
            log::debug!(
                "Load balancer at capacity for thread {}, skipping",
                thread_pubkey
            );
            return ExecutionResult::failed(thread_pubkey, "At capacity".to_string(), 0, trace);
        }
        ProcessDecision::Process => {
            log::debug!("Load balancer approved processing thread {}", thread_pubkey);
        }
    }

    // Add delay for new threads (no previous executor) if configured
    // This allows slower clients to avoid wasting fees on races
    if current_last_executor.eq(&Pubkey::default()) {
        let delay = load_balancer.thread_process_delay();
        if !delay.is_zero() {
            log::debug!(
                "Thread {} - waiting {:?} before claiming new thread",
                thread_pubkey,
                delay
            );
            tokio::time::sleep(delay).await;

            // Re-check cache after delay - another executor may have claimed it
            if let Some(cached) = resources.cache.get(&thread_pubkey).await {
                use anchor_lang::AccountDeserialize;
                if let Ok(t) = Thread::try_deserialize(&mut cached.data.as_slice()) {
                    if !t.last_executor.eq(&Pubkey::default()) {
                        log::debug!(
                            "Thread {} claimed by {} during delay, skipping",
                            thread_pubkey,
                            t.last_executor
                        );
                        return ExecutionResult::failed(
                            thread_pubkey,
                            "Claimed during delay".to_string(),
                            0,
                            trace,
                        );
                    }
                }
            }
        }
    }

    // Build and submit loop.
    // Each iteration builds one transaction batch, submits it, and confirms it.
    // If the executor signals continuation (instructions didn't fit in one tx),
    // we re-fetch the thread from on-chain and build the next batch.
    const MAX_CONTINUATION_BATCHES: u32 = 20;
    let mut thread = thread;
    let mut batch_num = 0u32;
    let mut max_priority_fee: u64 = 0;
    let mut pending_fiber_cursor: Option<u8> = None;

    loop {
        batch_num += 1;

        if batch_num > MAX_CONTINUATION_BATCHES {
            log::warn!(
                "{}: hit max continuation batches ({}), stopping",
                thread_pubkey,
                MAX_CONTINUATION_BATCHES
            );
            break;
        }

        // Build batch — first iteration uses trigger retry, subsequent don't need it
        let (ixs, priority_fee, needs_continuation, next_cursor, simulated_units) =
            if batch_num == 1 {
                let trigger_retry_deadline =
                    Instant::now() + Duration::from_secs(TRIGGER_RETRY_DEADLINE_SECS);
                loop {
                    if cancelled.load(Ordering::Relaxed) {
                        return ExecutionResult::failed(
                            thread_pubkey,
                            "Cancelled during build".to_string(),
                            0,
                            trace,
                        );
                    }
                    if Instant::now() > trigger_retry_deadline {
                        return ExecutionResult::failed(
                            thread_pubkey,
                            "Trigger window expired while waiting for trigger time".to_string(),
                            0,
                            trace,
                        );
                    }
                    match executor
                        .build_execute_transaction(&thread_pubkey, &thread, pending_fiber_cursor)
                        .await
                    {
                        Ok(result) => break result,
                        Err(e) => {
                            let error_str = e.to_string();
                            if is_trigger_not_ready_error(&error_str) {
                                log::debug!(
                                    "Thread {} trigger not ready (6004), retrying in 500ms",
                                    thread_pubkey
                                );
                                tokio::time::sleep(Duration::from_millis(500)).await;
                                continue;
                            } else if is_thread_paused_error(&error_str) {
                                log::debug!(
                                    "Thread {} is paused (6006), skipping execution",
                                    thread_pubkey
                                );
                                return ExecutionResult::failed(
                                    thread_pubkey,
                                    "Thread is paused".to_string(),
                                    0,
                                    trace,
                                );
                            } else {
                                log::error!(
                                    "Failed to build transaction for thread {}: {:?}",
                                    thread_pubkey,
                                    e
                                );
                                return ExecutionResult::failed(
                                    thread_pubkey,
                                    format!("Transaction build failed: {}", e),
                                    0,
                                    trace,
                                );
                            }
                        }
                    }
                }
            } else {
                // Continuation batch — build against fresh on-chain state
                match executor
                    .build_execute_transaction(&thread_pubkey, &thread, pending_fiber_cursor)
                    .await
                {
                    Ok(result) => result,
                    Err(e) => {
                        log::error!(
                            "{}: continuation batch {} build failed: {:?}",
                            thread_pubkey,
                            batch_num,
                            e
                        );
                        return ExecutionResult::failed(
                            thread_pubkey,
                            format!("Continuation batch {} build failed: {}", batch_num, e),
                            0,
                            trace,
                        );
                    }
                }
            };

        trace.mark_built();
        max_priority_fee = max_priority_fee.max(priority_fee);
        pending_fiber_cursor = next_cursor;

        // Empty fiber — nothing to submit
        if ixs.is_empty() {
            log::info!(
                "{}: batch {} has no instructions (empty fiber), skipping",
                thread_pubkey,
                batch_num
            );
            return ExecutionResult::skipped(thread_pubkey, trace);
        }

        log::info!(
            "{}: batch {} built ({} ix, continuation={})",
            thread_pubkey,
            batch_num,
            ixs.len(),
            needs_continuation
        );

        // Reuse the compute units the batching simulation already measured.
        // A separate estimate is only needed when that simulation did not cover
        // the final instruction set.
        let cu_estimate = match simulated_units {
            Some(units) => units,
            None => {
                trace.count_rpc();
                match executor.estimate_compute_units(&ixs, &thread_pubkey).await {
                    Ok(units) => {
                        trace.mark_simulated();
                        units
                    }
                    Err(e) => {
                        log::error!(
                            "{}: batch {} CU estimation failed: {:?}",
                            thread_pubkey,
                            batch_num,
                            e
                        );
                        return ExecutionResult::failed(
                            thread_pubkey,
                            format!("Batch {} CU estimation failed: {}", batch_num, e),
                            0,
                            trace,
                        );
                    }
                }
            }
        };

        // Prepend compute budget instructions
        let compute_units = compute_unit_limit(cu_estimate);
        let mut final_ixs = vec![ComputeBudgetInstruction::set_compute_unit_limit(
            compute_units,
        )];
        if max_priority_fee > 0 {
            final_ixs.push(ComputeBudgetInstruction::set_compute_unit_price(
                max_priority_fee,
            ));
        }
        final_ixs.extend_from_slice(&ixs);

        // Submit and confirm
        match submit_and_confirm_batch(
            &final_ixs,
            executor,
            resources,
            cancelled,
            &thread_pubkey,
            load_balancer,
            &mut trace,
        )
        .await
        {
            Ok(sig) => {
                log::info!("{}: batch {} confirmed ({})", thread_pubkey, batch_num, sig);
            }
            Err((error, attempts)) => {
                return ExecutionResult::failed(
                    thread_pubkey,
                    format!("Batch {} failed: {}", batch_num, error),
                    attempts,
                    trace,
                );
            }
        }

        if !needs_continuation {
            break;
        }

        // Re-fetch thread from on-chain for the next batch
        // (previous batch changed the thread's fiber_cursor, exec_index, etc.)
        log::info!(
            "{}: re-fetching thread for continuation batch",
            thread_pubkey
        );
        trace.count_rpc();
        thread = match executor.fetch_thread(&thread_pubkey).await {
            Ok(t) => t,
            Err(e) => {
                log::error!(
                    "{}: failed to re-fetch thread for continuation: {:?}",
                    thread_pubkey,
                    e
                );
                return ExecutionResult::failed(
                    thread_pubkey,
                    format!("Failed to re-fetch thread for continuation: {}", e),
                    0,
                    trace,
                );
            }
        };
    }

    ExecutionResult::success(thread_pubkey, trace)
}

/// Submit a batch of instructions as a transaction, with retries and confirmation.
///
/// Handles: get blockhash, build+sign transaction, TPU send + confirmation polling,
/// RPC fallback, retry up to MAX_ATTEMPTS.
///
/// Returns Ok(signature) on success, Err((error_msg, attempts)) on failure.
async fn submit_and_confirm_batch(
    instructions: &[Instruction],
    executor: &ExecutorLogic,
    resources: &SharedResources,
    cancelled: &AtomicBool,
    thread_pubkey: &Pubkey,
    load_balancer: &LoadBalancer,
    trace: &mut ExecTrace,
) -> Result<Signature, (String, u32)> {
    let mut attempt = 0u32;
    let mut last_error = String::new();

    // Sign once, up front, and reuse the same transaction across retries.
    //
    // Re-signing per attempt produces a *different signature for the same
    // logical execution*: if an earlier attempt was actually in flight, both
    // land, the executor pays twice, and for a chained batch the ordering is no
    // longer guaranteed. Resending an identical transaction is idempotent at the
    // validator; resending a re-signed one is not.
    let mut signed: Option<(Transaction, Signature, Instant)> = None;

    while attempt < MAX_ATTEMPTS {
        attempt += 1;
        trace.attempts = attempt;

        // Check cancellation
        if cancelled.load(Ordering::Relaxed) {
            log::debug!(
                "Worker cancelled during execution for thread: {}",
                thread_pubkey
            );
            return Err(("Cancelled during execution".to_string(), attempt));
        }

        log::debug!(
            "Submitting transaction for thread {} (attempt {}/{})",
            thread_pubkey,
            attempt,
            MAX_ATTEMPTS
        );

        // (Re-)sign only when we have nothing, or when the blockhash is old
        // enough that it can no longer land.
        let needs_signing = match &signed {
            None => true,
            Some((_, _, signed_at)) => signed_at.elapsed() > BLOCKHASH_MAX_AGE,
        };

        if needs_signing {
            trace.count_rpc();
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

            let message = Message::new(instructions, Some(&executor.pubkey()));
            let tx = Transaction::new(&[executor.keypair().as_ref()], message, blockhash);
            // Signature is captured up front: TPU submission is fire-and-forget,
            // so confirmation polling needs it before the send.
            let signature = tx.signatures[0];
            trace.mark_signed();
            signed = Some((tx, signature, Instant::now()));
        }

        let (tx, signature, _) = signed.as_ref().expect("signed above");
        let tx = tx.clone();
        let signature = *signature;

        log::debug!("{}: sent", thread_pubkey);
        log::debug!("  txn: {}", signature);

        // TPU retry loop: send via TPU and poll for confirmation, re-sending every 2s
        // This handles the case where TPU send appears to succeed but transaction doesn't land
        let mut tpu_confirmed = false;
        // Set when the chain reported a condition that warrants another attempt
        // rather than falling through to the RPC fallback with a transaction we
        // already know will not land.
        let mut retry_attempt = false;
        if let Some(tpu_client) = &resources.tpu_client {
            let start = Instant::now();
            let timeout = Duration::from_secs(CONFIRMATION_TIMEOUT_SECS);
            let mut last_tpu_send = Instant::now();

            // Initial TPU send
            if let Err(e) = tpu_client.send_transaction(&tx).await {
                log::debug!("Initial TPU send failed: {}", e);
            }
            trace.mark_sent(SendPath::Tpu);

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
                    Ok(Some(Err(error_str))) => {
                        if is_trigger_not_ready_error(&error_str) {
                            log::debug!(
                                "{}: 6004 on-chain (trigger not ready), will retry",
                                thread_pubkey
                            );
                            break;
                        }

                        if is_thread_paused_error(&error_str) {
                            log::debug!(
                                "{}: 6006 on-chain (thread paused), skipping",
                                thread_pubkey
                            );
                            return Err(("Thread is paused".to_string(), attempt));
                        }

                        // Cluster-level conditions are not program failures —
                        // retry rather than abandoning the execution.
                        if is_retryable_chain_error(&error_str) {
                            log::debug!(
                                "{}: retryable chain error, will retry: {}",
                                thread_pubkey,
                                error_str
                            );
                            if is_blockhash_expired(&error_str) {
                                signed = None;
                            }
                            last_error = error_str;
                            retry_attempt = true;
                            break;
                        }

                        // Genuine program failure - don't retry, return failure
                        log::warn!(
                            "{}: transaction failed on-chain: {}",
                            thread_pubkey,
                            error_str
                        );

                        let _ = load_balancer
                            .record_execution_result(
                                thread_pubkey,
                                false,
                                chrono::Utc::now().timestamp(),
                            )
                            .await;

                        return Err((
                            format!("Transaction failed on-chain: {}", error_str),
                            attempt,
                        ));
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
            trace.mark_settled();
            log::debug!("{}: confirmed", thread_pubkey);
            log::debug!("  txn: {}", signature);

            // Record success in load balancer
            let _ = load_balancer
                .record_execution_result(thread_pubkey, true, chrono::Utc::now().timestamp())
                .await;

            return Ok(signature);
        }

        // A retryable chain condition means this transaction cannot land as-is;
        // back off and take another attempt rather than resending it over RPC.
        if retry_attempt {
            if attempt < MAX_ATTEMPTS {
                tokio::time::sleep(Duration::from_millis(
                    BASE_RETRY_DELAY_MS * (1 << attempt.min(4)),
                ))
                .await;
            }
            continue;
        }

        // Fall back to RPC if TPU not available or TPU loop timed out
        trace.count_rpc();
        match resources.rpc_client.send_transaction(&tx).await {
            Ok(sig) => {
                trace.mark_sent(SendPath::Rpc);
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
                    .record_execution_result(thread_pubkey, false, chrono::Utc::now().timestamp())
                    .await;

                tokio::time::sleep(Duration::from_millis(
                    BASE_RETRY_DELAY_MS * (1 << attempt.min(4)),
                ))
                .await;
                continue;
            }
        }

        // Wait for RPC confirmation
        match wait_for_confirmation(&resources.rpc_client, &signature, CONFIRMATION_TIMEOUT_SECS)
            .await
        {
            Ok(()) => {
                trace.mark_settled();
                log::debug!("{}: confirmed", thread_pubkey);
                log::debug!("  txn: {}", signature);

                // Record success in load balancer
                let _ = load_balancer
                    .record_execution_result(thread_pubkey, true, chrono::Utc::now().timestamp())
                    .await;

                return Ok(signature);
            }
            Err(e) => {
                last_error = format!("Confirmation failed: {}", e);

                // 6004/6006 errors are transient or expected — log as DEBUG, not WARN
                if is_trigger_not_ready_error(&e) {
                    log::debug!(
                        "{}: 6004 on RPC confirmation (trigger not ready), will retry",
                        thread_pubkey
                    );
                } else if is_thread_paused_error(&e) {
                    log::debug!(
                        "{}: 6006 on RPC confirmation (thread paused), stopping",
                        thread_pubkey
                    );
                    return Err(("Thread is paused".to_string(), attempt));
                } else if is_retryable_chain_error(&e) {
                    log::debug!(
                        "{}: retryable chain error on RPC confirmation, will retry: {}",
                        thread_pubkey,
                        e
                    );
                    if is_blockhash_expired(&e) {
                        signed = None;
                    }
                } else {
                    log::warn!(
                        "Transaction confirmation failed for thread {} (attempt {}): {:?}",
                        thread_pubkey,
                        attempt,
                        e
                    );

                    // Only record loss for non-6004 errors
                    let _ = load_balancer
                        .record_execution_result(
                            thread_pubkey,
                            false,
                            chrono::Utc::now().timestamp(),
                        )
                        .await;
                }

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

    Err((last_error, attempt))
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
            return Err(format!("Confirmation timeout after {}s", timeout_secs));
        }

        match rpc_client.get_signature_status(signature).await {
            Ok(Some(result)) => match result {
                Ok(()) => return Ok(()),
                Err(e) => return Err(format!("Transaction failed: {}", e)),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The RPC reports program errors as raw JSON. These are the shapes the
    /// confirmation path actually receives.
    const CUSTOM_6004: &str = r#"{"InstructionError":[0,{"Custom":6004}]}"#;
    const CUSTOM_6006: &str = r#"{"InstructionError":[0,{"Custom":6006}]}"#;
    const CUSTOM_OTHER: &str = r#"{"InstructionError":[2,{"Custom":1770}]}"#;

    #[test]
    fn program_errors_are_classified_by_code() {
        assert!(is_trigger_not_ready_error(CUSTOM_6004));
        assert!(!is_thread_paused_error(CUSTOM_6004));

        assert!(is_thread_paused_error(CUSTOM_6006));
        assert!(!is_trigger_not_ready_error(CUSTOM_6006));

        assert!(!is_trigger_not_ready_error(CUSTOM_OTHER));
        assert!(!is_thread_paused_error(CUSTOM_OTHER));
    }

    #[test]
    fn cluster_conditions_are_retryable_not_fatal() {
        // These used to collapse to Custom(0) and abort the retry loop.
        for err in [
            r#""BlockhashNotFound""#,
            r#""AlreadyProcessed""#,
            r#""AccountInUse""#,
            r#"{"WouldExceedMaxBlockCostLimit":null}"#,
        ] {
            assert!(is_retryable_chain_error(err), "should retry: {err}");
            assert!(
                !is_trigger_not_ready_error(err) && !is_thread_paused_error(err),
                "should not look like a program error: {err}"
            );
        }
    }

    #[test]
    fn genuine_program_failure_is_not_retryable() {
        assert!(!is_retryable_chain_error(CUSTOM_OTHER));
    }

    #[test]
    fn only_blockhash_expiry_forces_resigning() {
        assert!(is_blockhash_expired(r#""BlockhashNotFound""#));
        assert!(!is_blockhash_expired(r#""AlreadyProcessed""#));
        assert!(!is_blockhash_expired(CUSTOM_6004));
    }

    #[test]
    fn compute_unit_limit_adds_headroom() {
        // Headroom must exceed the estimate, since the simulate ran against a
        // different bank state.
        assert!(compute_unit_limit(200_000) > 200_000);
        assert_eq!(compute_unit_limit(200_000), 260_000);

        // A trivial estimate still gets a usable floor.
        assert_eq!(compute_unit_limit(0), 10_000);
    }

    #[test]
    fn compute_unit_limit_is_clamped_to_the_chain_maximum() {
        // Scaling a near-ceiling estimate must not request more than Solana
        // permits, which would be rejected outright.
        assert_eq!(compute_unit_limit(1_400_000), MAX_COMPUTE_UNITS);
        assert_eq!(compute_unit_limit(u64::MAX / 2), MAX_COMPUTE_UNITS);
    }

    #[test]
    fn blockhash_reuse_window_stays_inside_validity() {
        // A blockhash is valid for ~150 slots (~60s). The reuse window must sit
        // comfortably under that, or a retry resends a transaction that can no
        // longer land.
        assert!(BLOCKHASH_MAX_AGE < Duration::from_secs(60));
    }
}
