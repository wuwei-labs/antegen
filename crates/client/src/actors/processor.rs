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
    CompletionReason, ExecutionResult, ProcessorMessage, ProcessorStatus, ReadyThread,
    StagingMessage,
};
use crate::actors::WorkerActor;
use crate::config::ClientConfig;
use crate::executor::ExecutorLogic;
use crate::load_balancer::LoadBalancer;
use crate::resources::SharedResources;
use log::warn;
use ractor::{Actor, ActorProcessingErr, ActorRef};
use solana_sdk::pubkey::Pubkey;
use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::sync::Arc;
use tokio::sync::{broadcast, Semaphore};

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
    clock_tx: broadcast::Sender<solana_sdk::clock::Clock>,

    // Shared resources (includes cache)
    resources: SharedResources,

    // Executor and load balancer
    executor: ExecutorLogic,
    load_balancer: Arc<LoadBalancer>,
}

#[ractor::async_trait]
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

        // Create broadcast channel for clock distribution
        let (clock_tx, _clock_rx) = broadcast::channel(10);

        Ok(ProcessorState {
            pending_queue: VecDeque::new(),
            active_workers: HashMap::new(),
            task_semaphore,
            available_permits: max_concurrent_threads,
            staging_ref,
            clock_tx,
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
                    "Worker completed for thread {}: success={}",
                    result.thread_pubkey,
                    result.success
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
        let Some(ready_thread) = state.pending_queue.pop_front() else {
            return Ok(());
        };

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
                            log::debug!(
                                "Thread {} exec_count mismatch (cache={}, expected={}), skipping",
                                ready_thread.thread_pubkey,
                                thread.exec_count,
                                ready_thread.exec_count
                            );
                            // Notify staging that this thread is done (was stale)
                            state
                                .staging_ref
                                .send_message(StagingMessage::ThreadCompleted {
                                    thread_pubkey: ready_thread.thread_pubkey,
                                    reason: CompletionReason::Executed,
                                })
                                .ok();
                            return Ok(());
                        }
                        thread
                    }
                    Err(e) => {
                        warn!(
                            "Failed to deserialize thread {} from cache: {:?}",
                            ready_thread.thread_pubkey, e
                        );
                        // Notify staging that this thread is done
                        state
                            .staging_ref
                            .send_message(StagingMessage::ThreadCompleted {
                                thread_pubkey: ready_thread.thread_pubkey,
                                reason: CompletionReason::Executed,
                            })
                            .ok();
                        return Ok(());
                    }
                }
            }
            None => {
                // Cache miss - try RPC fallback
                log::debug!(
                    "Thread {} not in cache, attempting RPC fetch",
                    ready_thread.thread_pubkey
                );

                match state
                    .resources
                    .cache
                    .get_thread_or_fetch(&ready_thread.thread_pubkey, &state.resources.rpc_client)
                    .await
                {
                    Ok(thread) => {
                        // Verify exec_count matches
                        if thread.exec_count != ready_thread.exec_count {
                            log::debug!(
                                "Thread {} exec_count mismatch after RPC fetch (fetched={}, expected={}), skipping",
                                ready_thread.thread_pubkey,
                                thread.exec_count,
                                ready_thread.exec_count
                            );
                            state
                                .staging_ref
                                .send_message(StagingMessage::ThreadCompleted {
                                    thread_pubkey: ready_thread.thread_pubkey,
                                    reason: CompletionReason::Executed,
                                })
                                .ok();
                            return Ok(());
                        }
                        thread
                    }
                    Err(e) => {
                        warn!(
                            "Failed to fetch thread {} from RPC: {}",
                            ready_thread.thread_pubkey, e
                        );
                        // Notify staging that this thread is done
                        state
                            .staging_ref
                            .send_message(StagingMessage::ThreadCompleted {
                                thread_pubkey: ready_thread.thread_pubkey,
                                reason: CompletionReason::Executed,
                            })
                            .ok();
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

        // Spawn WorkerActor with Thread data from cache
        let worker_args = crate::actors::worker::WorkerArgs {
            thread_pubkey: ready_thread.thread_pubkey,
            thread: thread.clone(),
            is_overdue: ready_thread.is_overdue,
            overdue_seconds: ready_thread.overdue_seconds,
            permit,
            processor_ref: myself.clone(),
            clock_rx: state.clock_tx.subscribe(),
            resources: state.resources.clone(),
            executor: state.executor.clone(),
            load_balancer: state.load_balancer.clone(),
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

    /// Handle execution result from worker
    async fn handle_execution_result(
        &self,
        state: &mut ProcessorState,
        result: ExecutionResult,
    ) -> Result<(), ActorProcessingErr> {
        // Check if this was a load balancer skip
        let is_lb_skip = result
            .error
            .as_ref()
            .map(|e| e.contains("load balancer") || e.contains("At capacity"))
            .unwrap_or(false);

        // Log the result
        if result.success {
            log::debug!("Thread {} execution succeeded", result.thread_pubkey);
        } else if is_lb_skip {
            log::debug!(
                "Thread {} skipped: {:?}",
                result.thread_pubkey,
                result.error
            );
        } else {
            log::warn!(
                "Thread {} execution failed after {} attempts: {:?}",
                result.thread_pubkey,
                result.attempt_count,
                result.error
            );
        }

        // Determine completion reason based on whether load balancer skipped
        let reason = if is_lb_skip {
            CompletionReason::Skipped
        } else {
            CompletionReason::Executed
        };

        // Notify StagingActor that thread completed
        state
            .staging_ref
            .send_message(StagingMessage::ThreadCompleted {
                thread_pubkey: result.thread_pubkey,
                reason,
            })
            .map_err(|e| format!("Failed to notify staging of completion: {:?}", e))?;

        Ok(())
    }
}
