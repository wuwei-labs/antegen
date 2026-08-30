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
use crate::confirm::Confirmation;
use crate::executor::ExecutorLogic;
use crate::load_balancer::{LoadBalancer, ProcessDecision};
use crate::resources::compute::{is_compute_exceeded, loaded_accounts_limit};
use crate::resources::SharedResources;
use crate::trace::{ExecTrace, SendPath};
use crate::tx::{self, TxConfig};
use antegen_thread_program::state::Thread;
use ractor::{Actor, ActorProcessingErr, ActorRef};
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;
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

/// Retry deadline for trigger-not-ready errors (seconds)
/// This bounds how long we'll retry before giving up
const TRIGGER_RETRY_DEADLINE_SECS: u64 = 10;

/// Floor for the 6004 retry backoff.
///
/// The clock projection is an estimate, so on a retry we may still be marginally
/// early. Small enough that being wrong costs little, large enough not to spin.
const TRIGGER_RETRY_MIN_BACKOFF: Duration = Duration::from_millis(75);

/// Check if an error indicates the trigger condition is not yet met (error 6004)
fn is_trigger_not_ready_error(error: &str) -> bool {
    error.contains("Custom(6004)") || error.contains("6004")
}

/// Check if an error indicates the thread is paused (error 6006)
fn is_thread_paused_error(error: &str) -> bool {
    error.contains("Custom(6006)") || error.contains("6006")
}

/// Whether the failure came from a program rejecting the instruction, rather
/// than from the cluster.
///
/// A `Custom(n)` or an `InstructionError` is the target program saying no to
/// this transaction against this state. Rebuilding it produces the same bytes
/// and simulating again reaches the same verdict, so retrying is pure cost. The
/// only thing that changes the answer is on-chain state moving — which is
/// exactly what parking waits for.
///
/// Checked after the cluster-level conditions, which can also surface as an
/// InstructionError-shaped payload but genuinely are worth retrying.
fn is_program_rejection(error: &str) -> bool {
    error.contains("InstructionError") || error.contains("Custom")
}

/// Whether the build failed because an account it needs does not exist on chain.
///
/// A fiber the thread still lists but that has been closed is not a transient
/// condition — every rebuild derives the same PDA and gets the same null back.
/// Retrying costs one `getAccount` per attempt forever, which is what three
/// mainnet threads did for forty minutes after their fibers were closed out
/// from under them. Park instead: the watchdog still re-examines the thread,
/// and an account update re-arms it the moment the fiber is recreated.
fn is_missing_account_error(error: &str) -> bool {
    error.contains("not found")
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

/// Pages of headroom added to the measured loaded-accounts size.
///
/// Simulation runs against a different bank, so an account may have grown by
/// the time the transaction executes. A page costs 8 cost units against the
/// 16,384 that requesting nothing costs, so headroom here is close to free
/// while being short is a failed transaction.
const LOADED_ACCOUNTS_SLACK_PAGES: u32 = 2;

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
        // The permit travels with the work rather than with the actor, so it can
        // be released the moment the transaction is on the wire instead of being
        // held through confirmation.
        let permit = args.permit;

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
                permit,
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

            // Stop ourselves even if the completion message failed to deliver.
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
    permit: OwnedSemaphorePermit,
) -> ExecutionResult {
    // Held until the transaction is on the wire, then dropped. Confirmation can
    // take up to 30s per attempt, and holding the permit across it capped
    // throughput at max_concurrent_threads per confirmation window rather than
    // per build.
    let mut permit = Some(permit);
    // Check cancellation before starting
    if cancelled.load(Ordering::Relaxed) {
        log::debug!(
            "Worker cancelled before execution for thread: {}",
            thread_pubkey
        );
        // Cancellation follows a schedule change; the resulting account update
        // will re-arm this thread, but keep it retryable in case it does not.
        return ExecutionResult::retryable(
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
                        return ExecutionResult::superseded(
                            thread_pubkey,
                            "Thread already executed (exec_count changed)".to_string(),
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
            return ExecutionResult::retryable(
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
            return ExecutionResult::lb_skip(
                thread_pubkey,
                "Skipped by load balancer".to_string(),
                trace,
            );
        }
        ProcessDecision::AtCapacity => {
            log::debug!(
                "Load balancer at capacity for thread {}, skipping",
                thread_pubkey
            );
            return ExecutionResult::lb_skip(thread_pubkey, "At capacity".to_string(), trace);
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
                        return ExecutionResult::superseded(
                            thread_pubkey,
                            "Claimed during delay".to_string(),
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
        let batch = if batch_num == 1 {
            // If we were released before the on-chain deadline, wait for it
            // rather than building a transaction the chain is certain to
            // reject with 6004. A rejected build costs a fiber fetch and a
            // simulation, so polling into the deadline is more expensive
            // than sleeping to it.
            if let Some(due_at) = trace.due_at {
                if due_at > Instant::now() {
                    log::debug!(
                        "{}: released {}ms before deadline, waiting",
                        thread_pubkey,
                        due_at.duration_since(Instant::now()).as_millis()
                    );
                    tokio::time::sleep_until(due_at.into()).await;
                }
            }

            let trigger_retry_deadline =
                Instant::now() + Duration::from_secs(TRIGGER_RETRY_DEADLINE_SECS);
            loop {
                if cancelled.load(Ordering::Relaxed) {
                    return ExecutionResult::retryable(
                        thread_pubkey,
                        "Cancelled during build".to_string(),
                        0,
                        trace,
                    );
                }
                if Instant::now() > trigger_retry_deadline {
                    return ExecutionResult::retryable(
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
                            // Wake when the chain's clock is projected to
                            // cross the deadline, rather than polling on a
                            // fixed quantum that adds up to half a second of
                            // avoidable delay per attempt.
                            let backoff = trace
                                .due_at
                                .and_then(|due| due.checked_duration_since(Instant::now()))
                                .map(|remaining| remaining.max(TRIGGER_RETRY_MIN_BACKOFF))
                                .unwrap_or(TRIGGER_RETRY_MIN_BACKOFF);
                            log::debug!(
                                "Thread {} trigger not ready (6004), retrying in {}ms",
                                thread_pubkey,
                                backoff.as_millis()
                            );
                            tokio::time::sleep(backoff).await;
                            continue;
                        } else if is_thread_paused_error(&error_str) {
                            log::debug!(
                                "Thread {} is paused (6006), skipping execution",
                                thread_pubkey
                            );
                            // Unpausing writes to the thread account, so an
                            // update will re-arm this; parking is correct.
                            return ExecutionResult::fatal(
                                thread_pubkey,
                                "Thread is paused".to_string(),
                                0,
                                trace,
                            );
                        } else if is_missing_account_error(&error_str) {
                            log::warn!(
                                    "Thread {} references an account that does not exist, parking until state changes: {}",
                                    thread_pubkey,
                                    error_str
                                );
                            return ExecutionResult::fatal(
                                thread_pubkey,
                                format!("Transaction build failed: {}", e),
                                0,
                                trace,
                            );
                        } else if !is_retryable_chain_error(&error_str)
                            && is_program_rejection(&error_str)
                        {
                            // A program rejected the instruction. Retrying
                            // rebuilds the same transaction and simulates it
                            // against the same state, so it fails the same
                            // way — a thread whose fiber had gone stale sat
                            // eight hours overdue doing exactly that, one
                            // simulation every few seconds. Park instead:
                            // the watchdog still re-examines it, and an
                            // account update re-arms it immediately if the
                            // state it needs comes back.
                            log::warn!(
                                "Thread {} rejected by program, parking until state changes: {}",
                                thread_pubkey,
                                error_str
                            );
                            return ExecutionResult::fatal(
                                thread_pubkey,
                                format!("Program rejected the instruction: {}", e),
                                0,
                                trace,
                            );
                        } else {
                            log::error!(
                                "Failed to build transaction for thread {}: {:?}",
                                thread_pubkey,
                                e
                            );
                            return ExecutionResult::retryable(
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
                    return ExecutionResult::retryable(
                        thread_pubkey,
                        format!("Continuation batch {} build failed: {}", batch_num, e),
                        0,
                        trace,
                    );
                }
            }
        };

        trace.mark_built();
        let ixs = batch.instructions.clone();
        let needs_continuation = batch.needs_continuation;
        max_priority_fee = max_priority_fee.max(batch.priority_fee);
        pending_fiber_cursor = batch.next_fiber_cursor;

        // Empty fiber — nothing to submit
        if ixs.is_empty() {
            log::info!(
                "{}: batch {} has no instructions (empty fiber), skipping",
                thread_pubkey,
                batch_num
            );
            return ExecutionResult::empty_fiber(thread_pubkey, trace);
        }

        log::info!(
            "{}: batch {} built ({} ix, continuation={})",
            thread_pubkey,
            batch_num,
            ixs.len(),
            needs_continuation
        );

        // Reuse what the batching simulation already measured. A separate
        // estimate is only needed when that simulation did not cover the final
        // instruction set.
        let (cu_estimate, loaded_accounts_bytes) = match batch.simulated_units {
            Some(units) => (units, batch.loaded_accounts_bytes),
            None => {
                trace.count_rpc();
                match executor.estimate_resources(&ixs, &thread_pubkey).await {
                    Ok(resources) => {
                        trace.mark_simulated();
                        (resources.units, resources.loaded_accounts_bytes)
                    }
                    Err(e) => {
                        log::error!(
                            "{}: batch {} CU estimation failed: {:?}",
                            thread_pubkey,
                            batch_num,
                            e
                        );
                        return ExecutionResult::retryable(
                            thread_pubkey,
                            format!("Batch {} CU estimation failed: {}", batch_num, e),
                            0,
                            trace,
                        );
                    }
                }
            }
        };

        // Resource limits. Encoded as instructions or header fields depending
        // on the format — the seam in `crate::tx` owns that decision.
        //
        // The compute limit comes from what this thread has historically needed
        // rather than a fixed multiple of the simulation, because the limit is
        // what gets billed once the SIMD-0553 resource fee ramps in.
        let compute_units = resources.cu_oracle.limit(&thread_pubkey, cu_estimate);
        log::debug!(
            "{}: requesting {} CU (simulated {}, margin {} bps)",
            thread_pubkey,
            compute_units,
            cu_estimate,
            resources.cu_oracle.margin_bps(&thread_pubkey)
        );
        let mut tx_config = TxConfig::new()
            .with_compute_unit_limit(compute_units)
            .with_compute_unit_price(max_priority_fee);

        // A transaction that requests no loaded-accounts limit is charged the
        // runtime's 64 MiB default — 16,384 cost units, more than today's entire
        // fee at the terminal resource-fee rate, for account data it never
        // touches. Requested only when the simulation measured it: guessing low
        // costs a failed transaction and a missed trigger window, and there is
        // nothing to gain from guessing at all.
        if let Some(measured) = loaded_accounts_bytes {
            let limit = loaded_accounts_limit(measured, LOADED_ACCOUNTS_SLACK_PAGES);
            log::debug!(
                "{}: requesting {} B loaded-accounts limit (simulation loaded {} B)",
                thread_pubkey,
                limit,
                measured
            );
            tx_config = tx_config.with_loaded_accounts_data_size_limit(limit);
        }

        // Submit and confirm
        match submit_and_confirm_batch(
            &ixs,
            &tx_config,
            cu_estimate,
            executor,
            resources,
            cancelled,
            &thread_pubkey,
            load_balancer,
            &mut trace,
            &mut permit,
        )
        .await
        {
            Ok(sig) => {
                log::info!("{}: batch {} confirmed ({})", thread_pubkey, batch_num, sig);
                // Landed inside the requested budget, so that budget was at
                // least sufficient. Only landed executions are evidence: a
                // transaction that never executed says nothing about how much
                // compute it would have needed.
                resources.cu_oracle.record_landed(&thread_pubkey);
            }
            Err((error, attempts)) => {
                if is_compute_exceeded(&error) {
                    resources.cu_oracle.record_exceeded(&thread_pubkey);
                    log::warn!(
                        "{}: exceeded its compute budget, margin raised to {} bps",
                        thread_pubkey,
                        resources.cu_oracle.margin_bps(&thread_pubkey)
                    );
                }
                return ExecutionResult::retryable(
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
                return ExecutionResult::retryable(
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
    tx_config: &TxConfig,
    simulated_units: u64,
    executor: &ExecutorLogic,
    resources: &SharedResources,
    cancelled: &AtomicBool,
    thread_pubkey: &Pubkey,
    load_balancer: &LoadBalancer,
    trace: &mut ExecTrace,
    permit: &mut Option<OwnedSemaphorePermit>,
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
    let mut signed: Option<(VersionedTransaction, Signature, Instant)> = None;

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

            let tx = match tx::build_transaction(
                executor.tx_version(),
                &[executor.keypair().as_ref()],
                &executor.pubkey(),
                instructions,
                tx_config,
                blockhash,
            ) {
                Ok(tx) => tx,
                Err(e) => {
                    // A format the build path cannot emit is a configuration
                    // problem, not a transient one. Retrying re-runs the same
                    // encoding against the same config and fails identically,
                    // so stop and say which format was asked for.
                    return Err((format!("Failed to build transaction: {}", e), attempt));
                }
            };
            // Signature is captured up front: TPU submission is fire-and-forget,
            // so confirmation polling needs it before the send.
            let signature = tx.signatures[0];
            // Registered before the send so the logs cannot arrive first. The
            // estimate is recorded alongside, since the margin being learned is
            // expressed relative to it and this is the last point that knows it.
            resources
                .cu_oracle
                .register(signature, *thread_pubkey, simulated_units);
            trace.mark_signed();
            signed = Some((tx, signature, Instant::now()));
        }

        let (tx, signature, _) = signed.as_ref().expect("signed above");
        let tx = tx.clone();
        let signature = *signature;

        // Broadcast on every available path. The transaction is signed once,
        // so both paths carry the same signature and at most one can land —
        // sending to both is idempotent and strictly faster than the previous
        // sequential fallback, which waited out a 30s TPU timeout before trying
        // RPC at all.
        let mut sent = false;

        if let Some(tpu_client) = &resources.tpu_client {
            match tpu_client.send_transaction(&tx).await {
                Ok(()) => {
                    trace.mark_sent(SendPath::Tpu);
                    sent = true;
                }
                Err(e) => log::debug!("TPU send failed: {}", e),
            }
        }

        trace.count_rpc();
        match resources.rpc_client.send_transaction(&tx).await {
            Ok(sig) => {
                // Recorded unconditionally: the trace widens to `both` rather
                // than crediting whichever path happened to be tried first.
                trace.mark_sent(SendPath::Rpc);
                sent = true;
                log::debug!("{}: sent via RPC ({})", thread_pubkey, sig);
            }
            Err(e) => log::debug!("RPC send failed: {}", e),
        }

        if !sent {
            last_error = "All submission paths failed".to_string();
            log::warn!(
                "Failed to send transaction for thread {} (attempt {})",
                thread_pubkey,
                attempt
            );
            let _ = load_balancer
                .record_execution_result(thread_pubkey, false, chrono::Utc::now().timestamp())
                .await;
            tokio::time::sleep(Duration::from_millis(
                BASE_RETRY_DELAY_MS * (1 << attempt.min(4)),
            ))
            .await;
            continue;
        }

        // On the wire — free the slot for the next thread rather than holding it
        // through confirmation.
        drop(permit.take());

        // Confirmation is handled by the shared watcher: one batched poll for
        // every in-flight transaction, instead of one poll per worker.
        let outcome = resources
            .confirmations
            .wait(
                signature,
                tx.clone(),
                Duration::from_secs(CONFIRMATION_TIMEOUT_SECS),
            )
            .await;

        match outcome {
            Confirmation::Confirmed => {
                trace.mark_settled();
                log::debug!("{}: confirmed ({})", thread_pubkey, signature);
                let _ = load_balancer
                    .record_execution_result(thread_pubkey, true, chrono::Utc::now().timestamp())
                    .await;
                return Ok(signature);
            }

            Confirmation::Failed(error_str) => {
                if is_trigger_not_ready_error(&error_str) {
                    log::debug!(
                        "{}: 6004 on-chain (trigger not ready), will retry",
                        thread_pubkey
                    );
                } else if is_thread_paused_error(&error_str) {
                    log::debug!("{}: 6006 on-chain (thread paused), stopping", thread_pubkey);
                    return Err(("Thread is paused".to_string(), attempt));
                } else if is_retryable_chain_error(&error_str) {
                    log::debug!(
                        "{}: retryable chain error, will retry: {}",
                        thread_pubkey,
                        error_str
                    );
                    if is_blockhash_expired(&error_str) {
                        // Only expiry requires a new signature; every other
                        // retry reuses the same one.
                        signed = None;
                    }
                } else {
                    // A genuine program failure will fail again identically.
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
                last_error = error_str;
            }

            Confirmation::TimedOut => {
                last_error = format!("Confirmation timeout after {}s", CONFIRMATION_TIMEOUT_SECS);
                log::warn!(
                    "Transaction confirmation timed out for thread {} (attempt {})",
                    thread_pubkey,
                    attempt
                );
                let _ = load_balancer
                    .record_execution_result(thread_pubkey, false, chrono::Utc::now().timestamp())
                    .await;
            }
        }

        // Exponential backoff
        if attempt < MAX_ATTEMPTS {
            tokio::time::sleep(Duration::from_millis(
                BASE_RETRY_DELAY_MS * (1 << attempt.min(4)),
            ))
            .await;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The RPC reports program errors as raw JSON. These are the shapes the
    /// confirmation path actually receives.
    /// The exact payload from a mainnet node: a thread whose fiber called into
    /// SRSLY, which rejected it with RentalNotQueued. The thread retried that
    /// simulation for eight hours because every non-6004/6006 build failure was
    /// classified retryable.
    const PROGRAM_REJECTION: &str = r#"Simulation error: Object {"InstructionError": Array [Number(1), Object {"Custom": Number(6078)}]}"#;

    #[test]
    fn a_program_rejection_is_not_retried() {
        assert!(is_program_rejection(PROGRAM_REJECTION));
        assert!(!is_retryable_chain_error(PROGRAM_REJECTION));
    }

    /// Cluster conditions can carry an InstructionError-shaped payload but are
    /// genuinely worth retrying, so they are checked first.
    #[test]
    fn cluster_conditions_still_retry() {
        for e in ["BlockhashNotFound", "AlreadyProcessed", "AccountInUse"] {
            assert!(is_retryable_chain_error(e), "{e} should retry");
        }
    }

    /// Trigger-not-ready is a program rejection by shape, so ordering matters:
    /// it is handled by its own branch before this one is reached.
    #[test]
    fn trigger_not_ready_is_recognised_before_the_generic_check() {
        assert!(is_trigger_not_ready_error(CUSTOM_6004));
        assert!(is_program_rejection(CUSTOM_6004));
    }

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
    fn a_closed_fiber_is_not_a_transient_condition() {
        // The exact string the executor produces when the PDA a thread still
        // lists resolves to nothing on chain. Retrying re-derives the same
        // address and gets the same null, so this must park.
        assert!(is_missing_account_error(
            "Fiber J6FXwz1QPHNKMxAzBwUXy2t2XEELLCjYBMH4J4dkZqr9 not found"
        ));
        assert!(is_missing_account_error("Thread config 5Nn3 not found"));

        // A blockhash the cluster has not caught up to is a different thing
        // entirely, and parking on it would strand a perfectly good execution.
        assert!(!is_missing_account_error(r#""BlockhashNotFound""#));
        assert!(!is_missing_account_error(CUSTOM_OTHER));
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

    // Compute-limit headroom and its ceiling clamp now live with the oracle
    // that learns them — see `resources::compute`.

    #[test]
    fn blockhash_reuse_window_stays_inside_validity() {
        // A blockhash is valid for ~150 slots (~60s). The reuse window must sit
        // comfortably under that, or a retry resends a transaction that can no
        // longer land.
        assert!(BLOCKHASH_MAX_AGE < Duration::from_secs(60));
    }
}
