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
use crate::actors::scheduler::Outcome as SchedOutcome;
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

    // Concurrency control. The semaphore is the single source of truth — a
    // shadow counter alongside it drifts, and when it does the actor blocks on
    // `acquire` with no log output.
    task_semaphore: Arc<Semaphore>,
    max_concurrent: usize,

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
            max_concurrent: max_concurrent_threads,
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

                self.drain_queue(myself, state).await?;

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

                // The permit is released when the worker actor's state drops,
                // not here — there is no counter to reconcile.
                self.handle_execution_result(state, result).await?;

                self.drain_queue(myself, state).await?;

                Ok(())
            }
            ProcessorMessage::QueryStatus(tx) => {
                let status = ProcessorStatus {
                    pending_queue_size: state.pending_queue.len(),
                    active_workers: state.active_workers.len(),
                    available_permits: state.task_semaphore.available_permits(),
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
    /// Spawn workers until capacity or the queue is exhausted.
    ///
    /// Spawning one per message meant a burst of N ready threads needed N
    /// messages to drain, adding a message hop of latency to each.
    async fn drain_queue(
        &self,
        myself: ActorRef<ProcessorMessage>,
        state: &mut ProcessorState,
    ) -> Result<(), ActorProcessingErr> {
        // Bounded by the queue length so a thread that keeps getting re-queued
        // (because it already has an active worker) cannot spin here.
        for _ in 0..state.pending_queue.len() {
            if !self.try_spawn_next_worker(myself.clone(), state).await? {
                break;
            }
        }
        Ok(())
    }

    /// Try to spawn one worker. Returns whether it should be called again.
    ///
    /// Fetches Thread data from cache before spawning worker.
    async fn try_spawn_next_worker(
        &self,
        myself: ActorRef<ProcessorMessage>,
        state: &mut ProcessorState,
    ) -> Result<bool, ActorProcessingErr> {
        use anchor_lang::AccountDeserialize;
        use antegen_thread_program::state::Thread;

        // Claim capacity up front and never await for it. Awaiting here blocks
        // this actor's entire mailbox, and the previous shadow counter could
        // report a free slot while the real permit had not yet been dropped.
        let Ok(permit) = state.task_semaphore.clone().try_acquire_owned() else {
            log::debug!("At concurrency limit ({}), waiting", state.max_concurrent);
            return Ok(false);
        };

        // Check if queue has work
        let Some(mut ready_thread) = state.pending_queue.pop_front() else {
            return Ok(false);
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
            return Ok(true);
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
                            return Ok(true);
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
                        return Ok(true);
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
                            return Ok(true);
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
                        return Ok(true);
                    }
                }
            }
        };

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

        Ok(true)
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
                exec_count: ready.exec_count,
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

        // Only landed executions. A skip or a failure carries a lag that
        // measures nothing anyone acted on, and mixing those in would describe
        // a distribution that does not exist.
        if trace_outcome == Outcome::Ok {
            if let Some(lag) = result.trace.lag_ms() {
                state.resources.latency_stats.record(lag);
            }
        }

        // Notify StagingActor that thread completed
        state
            .staging_ref
            .send_message(StagingMessage::ThreadCompleted {
                thread_pubkey: result.thread_pubkey,
                outcome: result.outcome,
                exec_count: result.exec_count,
            })
            .map_err(|e| format!("Failed to notify staging of completion: {:?}", e))?;

        Ok(())
    }
}
