//! Processor Factory Actor
//!
//! The ProcessorFactory maintains a FIFO queue of threads ready for execution.
//! It receives ReadyThread messages from the StagingActor (pubkey + metadata only),
//! fetches full Thread data from cache, and spawns WorkerActor instances up to
//! the configured concurrency limit.
//!
//! Key design: ProcessorFactory fetches Thread data from cache on-demand, not upfront.
//! The cache is the single source of truth for account data.

use crate::actors::messages::{
    ExecutionResult, ProcessorMessage, ProcessorStatus, ReadyThread, StagingMessage,
};
use crate::actors::sched::Outcome as SchedOutcome;
use crate::actors::WorkerActor;
use crate::config::ClientConfig;
use crate::executor::ExecutorLogic;
use crate::load_balancer::LoadBalancer;
use crate::resources::SharedResources;
use crate::trace::Outcome;
use log::warn;
use ractor::{Actor, ActorProcessingErr, ActorRef};
use solana_sdk::pubkey::Pubkey;
use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Log target for the per-execution latency line. Kept separate so it can be
/// filtered up or down independently of the rest of the client's logging.
pub const LATENCY_TARGET: &str = "antegen::latency";

#[derive(Default)]
pub struct ProcessorFactory;

pub struct ProcessorState {
    // FIFO queue of ready threads (pubkey + metadata only)
    // Full Thread data is fetched from cache when spawning worker
    pending_queue: VecDeque<ReadyThread>,

    // Worker tracking
    active_workers: HashMap<Pubkey, ActorRef<crate::actors::messages::WorkerMessage>>,

    // Concurrency control
    task_semaphore: Arc<Semaphore>,
    available_permits: usize,

    // Communication
    staging_ref: ActorRef<StagingMessage>,

    // Shared resources (includes cache)
    resources: SharedResources,

    // Executor and load balancer
    executor: ExecutorLogic,
    load_balancer: Arc<LoadBalancer>,
}

impl Actor for ProcessorFactory {
    type Msg = ProcessorMessage;
    type State = ProcessorState;
    type Arguments = (
        ClientConfig,
        SharedResources,
        ActorRef<StagingMessage>,
        ExecutorLogic,
        Arc<LoadBalancer>,
    );

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        (config, resources, staging_ref, executor, load_balancer): Self::Arguments,
    ) -> Result<Self::State, Box<dyn Error + Send + Sync>> {
        let max_concurrent_threads = config.processor.max_concurrent_threads;
        log::debug!(
            "ProcessorFactory starting with max {} concurrent threads",
            max_concurrent_threads
        );

        // Create semaphore for concurrency control
        let task_semaphore = Arc::new(Semaphore::new(max_concurrent_threads));

        Ok(ProcessorState {
            pending_queue: VecDeque::new(),
            active_workers: HashMap::new(),
            task_semaphore,
            available_permits: max_concurrent_threads,
            staging_ref,
            resources,
            executor,
            load_balancer,
        })
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            ProcessorMessage::ProcessReady(ready_thread) => {
                log::debug!(
                    "Received thread {} for processing (queue_size={})",
                    ready_thread.thread_pubkey,
                    state.pending_queue.len()
                );

                // Add to FIFO queue (pubkey + metadata only)
                // Full Thread data will be fetched from cache when spawning worker
                state.pending_queue.push_back(ready_thread);

                // Try to spawn worker if capacity available
                self.try_spawn_next_worker(myself, state).await?;

                Ok(())
            }
            ProcessorMessage::CancelThread(thread_pubkey) => {
                // Remove from pending queue if present
                state
                    .pending_queue
                    .retain(|t| t.thread_pubkey != thread_pubkey);

                // Cancel active worker if exists
                if let Some(worker_ref) = state.active_workers.get(&thread_pubkey) {
                    log::debug!("Cancelling worker for thread {}", thread_pubkey);
                    let _ = worker_ref.send_message(crate::actors::messages::WorkerMessage::Cancel);
                }

                Ok(())
            }
            ProcessorMessage::WorkerCompleted(result) => {
                log::debug!(
                    "Worker completed for thread {}: {:?}",
                    result.thread_pubkey,
                    result.outcome
                );

                // Remove from active workers and stop the actor
                if let Some(worker_ref) = state.active_workers.remove(&result.thread_pubkey) {
                    log::debug!("Stopping worker actor for thread {}", result.thread_pubkey);
                    worker_ref.stop(None);
                }

                // Increment available permits
                state.available_permits += 1;

                // Handle result
                self.handle_execution_result(state, result).await?;

                // Try to spawn next worker from queue
                self.try_spawn_next_worker(myself, state).await?;

                Ok(())
            }
            ProcessorMessage::QueryStatus(tx) => {
                let status = ProcessorStatus {
                    pending_queue_size: state.pending_queue.len(),
                    active_workers: state.active_workers.len(),
                    available_permits: state.available_permits,
                };
                let _ = tx.send(status);
                Ok(())
            }
            ProcessorMessage::Shutdown => {
                log::info!("ProcessorFactory shutting down...");
                Err(From::from("Shutdown signal received"))
            }
        }
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        state: &mut Self::State,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        log::info!(
            "ProcessorFactory stopped. {} items in queue, {} active workers",
            state.pending_queue.len(),
            state.active_workers.len()
        );
        Ok(())
    }
}

impl ProcessorFactory {
    /// Try to spawn next worker from queue if capacity available
    ///
    /// Fetches Thread data from cache before spawning worker.
    /// If cache miss, skips the thread (will be re-queued on next update).
    async fn try_spawn_next_worker(
        &self,
        myself: ActorRef<ProcessorMessage>,
        state: &mut ProcessorState,
    ) -> Result<(), ActorProcessingErr> {
        use anchor_lang::AccountDeserialize;
        use antegen_thread_program::state::Thread;

        // Check if we have capacity
        if state.available_permits == 0 {
            log::debug!("No available permits, cannot spawn worker");
            return Ok(());
        }

        // Check if queue has work
        let Some(mut ready_thread) = state.pending_queue.pop_front() else {
            return Ok(());
        };

        // Guard against duplicate active workers
        if state
            .active_workers
            .contains_key(&ready_thread.thread_pubkey)
        {
            log::debug!(
                "Thread {} already has active worker, re-queuing",
                ready_thread.thread_pubkey
            );
            state.pending_queue.push_back(ready_thread);
            return Ok(());
        }

        log::debug!(
            "Spawning worker for thread {} (queue_size={}, active={})",
            ready_thread.thread_pubkey,
            state.pending_queue.len(),
            state.active_workers.len()
        );

        // Fetch Thread data from cache
        let thread = match state.resources.cache.get(&ready_thread.thread_pubkey).await {
            Some(cached) => {
                // Deserialize Thread from cached data
                match Thread::try_deserialize(&mut cached.data.as_slice()) {
                    Ok(thread) => {
                        // Verify exec_count matches (data might be stale)
                        if thread.exec_count != ready_thread.exec_count {
                            self.abandon(
                                state,
                                &ready_thread,
                                &format!(
                                    "exec_count mismatch (cache={}, expected={})",
                                    thread.exec_count, ready_thread.exec_count
                                ),
                                SchedOutcome::Superseded,
                            );
                            return Ok(());
                        }
                        thread
                    }
                    Err(e) => {
                        warn!(
                            "Failed to deserialize thread {} from cache: {:?}",
                            ready_thread.thread_pubkey, e
                        );
                        // Corrupt cache entry; a fresh update is the only fix.
                        self.abandon(
                            state,
                            &ready_thread,
                            "cache deserialize failed",
                            SchedOutcome::Fatal,
                        );
                        return Ok(());
                    }
                }
            }
            None => {
                // Cache miss - try RPC fallback
                log::warn!(
                    "Cache miss for thread {} during worker spawn, attempting RPC fetch",
                    ready_thread.thread_pubkey
                );
                ready_thread.trace.count_rpc();

                match state
                    .resources
                    .cache
                    .get_thread_or_fetch(&ready_thread.thread_pubkey, &state.resources.rpc_client)
                    .await
                {
                    Ok(thread) => {
                        // Verify exec_count matches
                        if thread.exec_count != ready_thread.exec_count {
                            self.abandon(
                                state,
                                &ready_thread,
                                &format!(
                                    "exec_count mismatch after RPC fetch (fetched={}, expected={})",
                                    thread.exec_count, ready_thread.exec_count
                                ),
                                SchedOutcome::Superseded,
                            );
                            return Ok(());
                        }
                        thread
                    }
                    Err(e) => {
                        warn!(
                            "Failed to fetch thread {} from RPC: {}",
                            ready_thread.thread_pubkey, e
                        );
                        // A transport failure says nothing about the thread.
                        self.abandon(
                            state,
                            &ready_thread,
                            "RPC fetch failed",
                            SchedOutcome::Retryable,
                        );
                        return Ok(());
                    }
                }
            }
        };

        // Acquire semaphore permit
        let permit = state
            .task_semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| format!("Semaphore error: {}", e))?;

        state.available_permits -= 1;

        ready_thread.trace.mark_spawned();

        // Spawn WorkerActor with Thread data from cache
        let worker_args = crate::actors::worker::WorkerArgs {
            thread_pubkey: ready_thread.thread_pubkey,
            thread: thread.clone(),
            is_overdue: ready_thread.is_overdue,
            overdue_seconds: ready_thread.overdue_seconds,
            permit,
            processor_ref: myself.clone(),
            resources: state.resources.clone(),
            executor: state.executor.clone(),
            load_balancer: state.load_balancer.clone(),
            trace: ready_thread.trace.clone(),
        };

        let (worker_ref, _handle) = Actor::spawn(
            Some(format!("worker-{}", ready_thread.thread_pubkey)),
            WorkerActor,
            worker_args,
        )
        .await
        .map_err(|e| format!("Failed to spawn worker: {}", e))?;

        // Track worker
        state
            .active_workers
            .insert(ready_thread.thread_pubkey, worker_ref);

        Ok(())
    }

    /// Abandon a ready thread before a worker was ever spawned.
    ///
    /// These paths would otherwise terminate an execution attempt with no trace
    /// at all, which is exactly the case that is impossible to diagnose from the
    /// logs today.
    fn abandon(
        &self,
        state: &ProcessorState,
        ready: &ReadyThread,
        reason: &str,
        outcome: SchedOutcome,
    ) {
        log::debug!(
            "Thread {} abandoned before spawn: {}",
            ready.thread_pubkey,
            reason
        );
        log::debug!(target: LATENCY_TARGET, "{} reason={}", ready.trace.render(Outcome::Skip), reason);

        state
            .staging_ref
            .send_message(StagingMessage::ThreadCompleted {
                thread_pubkey: ready.thread_pubkey,
                outcome,
            })
            .ok();
    }

    /// Handle execution result from worker
    async fn handle_execution_result(
        &self,
        state: &mut ProcessorState,
        result: ExecutionResult,
    ) -> Result<(), ActorProcessingErr> {
        match result.outcome {
            SchedOutcome::Succeeded => {
                log::info!("Thread {} execution succeeded", result.thread_pubkey)
            }
            SchedOutcome::EmptyFiber => {
                log::debug!("Thread {} skipped: empty fiber", result.thread_pubkey)
            }
            SchedOutcome::LoadBalancerSkip | SchedOutcome::Superseded => log::debug!(
                "Thread {} not executed: {:?}",
                result.thread_pubkey,
                result.error
            ),
            SchedOutcome::Retryable | SchedOutcome::Fatal => log::warn!(
                "Thread {} execution failed after {} attempts: {:?}",
                result.thread_pubkey,
                result.attempt_count,
                result.error
            ),
        }

        // The single latency line for this execution attempt.
        let trace_outcome = match result.outcome {
            SchedOutcome::Succeeded => Outcome::Ok,
            SchedOutcome::EmptyFiber | SchedOutcome::Superseded => Outcome::Skip,
            SchedOutcome::LoadBalancerSkip => Outcome::LbSkip,
            SchedOutcome::Retryable | SchedOutcome::Fatal => Outcome::Fail,
        };
        log::debug!(target: LATENCY_TARGET, "{}", result.trace.render(trace_outcome));

        // Notify StagingActor that thread completed
        state
            .staging_ref
            .send_message(StagingMessage::ThreadCompleted {
                thread_pubkey: result.thread_pubkey,
                outcome: result.outcome,
            })
            .map_err(|e| format!("Failed to notify staging of completion: {:?}", e))?;

        Ok(())
    }
}
