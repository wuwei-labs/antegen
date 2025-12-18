//! Staging Actor
//!
//! The StagingActor tracks thread triggers and maintains priority queues (time/slot/epoch)
//! for thread scheduling. On ClockTick, it evaluates which threads are ready and pushes
//! pubkeys to the ProcessorFactory (which fetches full Thread data from cache).
//!
//! Key design: StagingActor only tracks trigger metadata, NOT full Thread data.
//! The cache is the single source of truth for account data.

use crate::actors::messages::{
    ProcessorMessage, ReadyThread, ScheduledThread, StagingMessage, StagingStatus,
};
use crate::config::ClientConfig;
use crate::resources::SharedResources;
use anyhow::Result;
use antegen_thread_program::state::{Schedule, Thread, Trigger};
use dashmap::DashSet;
use log::{debug, info, trace, warn};
use ractor::{Actor, ActorProcessingErr, ActorRef};
use solana_sdk::{clock::Clock, pubkey::Pubkey};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::error::Error;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

#[derive(Default)]
pub struct StagingActor;

/// Lightweight trigger info for a thread (only what StagingActor needs)
#[derive(Debug, Clone)]
struct TrackedThread {
    exec_count: u64,
    schedule: Schedule,
}

pub struct StagingState {
    // Lightweight trigger tracking (exec_count only, NOT full Thread data)
    // Cache is the source of truth for account data
    tracked_threads: HashMap<Pubkey, TrackedThread>,

    // Priority queues (min-heap via Reverse)
    time_queue: Arc<Mutex<BinaryHeap<Reverse<ScheduledThread>>>>,
    slot_queue: Arc<Mutex<BinaryHeap<Reverse<ScheduledThread>>>>,
    epoch_queue: Arc<Mutex<BinaryHeap<Reverse<ScheduledThread>>>>,

    // Deduplication tracking
    queued_threads: DashSet<Pubkey>, // Threads already pushed to ProcessorFactory

    // Clock deduplication (handle multiple datasources sending same clock)
    // Only track slot since slots are monotonically increasing
    last_processed_slot: u64,

    // Communication
    processor_ref: Option<ActorRef<ProcessorMessage>>,

    // Shared resources for RPC access
    resources: SharedResources,

    // Cache eviction receiver - threads to refetch after TTL expiry
    eviction_rx: mpsc::UnboundedReceiver<Pubkey>,
}

#[ractor::async_trait]
impl Actor for StagingActor {
    type Msg = StagingMessage;
    type State = StagingState;
    type Arguments = (ClientConfig, SharedResources, mpsc::UnboundedReceiver<Pubkey>);

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        (_config, resources, eviction_rx): Self::Arguments,
    ) -> Result<Self::State, Box<dyn Error + Send + Sync>> {
        log::debug!("StagingActor starting...");
        log::debug!("Thread program ID: {}", antegen_thread_program::ID);

        Ok(StagingState {
            tracked_threads: HashMap::new(),
            time_queue: Arc::new(Mutex::new(BinaryHeap::new())),
            slot_queue: Arc::new(Mutex::new(BinaryHeap::new())),
            epoch_queue: Arc::new(Mutex::new(BinaryHeap::new())),
            queued_threads: DashSet::new(),
            last_processed_slot: 0,
            processor_ref: None, // Will be set by RootSupervisor after processor spawns
            resources,
            eviction_rx,
        })
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            StagingMessage::AccountUpdate(update) => {
                self.handle_account_update(state, update).await?;
                Ok(())
            }
            StagingMessage::ClockTick(clock) => {
                self.handle_clock_tick(state, clock).await?;
                Ok(())
            }
            StagingMessage::ThreadCompleted(thread_pubkey) => {
                // Remove from queued_threads to allow re-execution
                state.queued_threads.remove(&thread_pubkey);
                debug!("Thread {} completed, removed from queued set", thread_pubkey);
                Ok(())
            }
            StagingMessage::SetProcessorRef(processor_ref) => {
                log::debug!("StagingActor received processor reference");
                state.processor_ref = Some(processor_ref);
                Ok(())
            }
            StagingMessage::QueryStatus(tx) => {
                let status = StagingStatus {
                    total_threads: state.tracked_threads.len(),
                    queued_threads: state.queued_threads.len(),
                    time_queue_size: state.time_queue.lock().await.len(),
                    slot_queue_size: state.slot_queue.lock().await.len(),
                    epoch_queue_size: state.epoch_queue.lock().await.len(),
                };
                let _ = tx.send(status);
                Ok(())
            }
            StagingMessage::Shutdown => {
                log::info!("StagingActor shutting down...");
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
            "StagingActor stopped. {} threads tracked, {} queued",
            state.tracked_threads.len(),
            state.queued_threads.len()
        );
        Ok(())
    }
}

impl StagingActor {
    /// Handle incoming account update
    ///
    /// NOTE: Data has already been stored in cache by datasource.
    /// We only extract trigger info for scheduling here.
    async fn handle_account_update(
        &self,
        state: &mut StagingState,
        update: crate::types::AccountUpdate,
    ) -> Result<(), ActorProcessingErr> {
        // Classify the account type and extract trigger info
        match self.classify_account(&update.data, &update.pubkey) {
            AccountType::Thread(thread) => {
                // Check if we already have a newer or same version
                if let Some(existing) = state.tracked_threads.get(&update.pubkey) {
                    // Skip if nothing changed (same exec_count AND same schedule)
                    if thread.exec_count <= existing.exec_count
                        && thread.schedule == existing.schedule
                    {
                        return Ok(());
                    }

                    // Log what changed
                    if thread.schedule != existing.schedule {
                        debug!(
                            "Thread {} schedule changed: {:?} -> {:?}",
                            update.pubkey, existing.schedule, thread.schedule
                        );
                    }
                    if thread.exec_count > existing.exec_count {
                        debug!(
                            "Thread {} exec_count updated: {} -> {}",
                            update.pubkey, existing.exec_count, thread.exec_count
                        );
                    }

                    if state.queued_threads.contains(&update.pubkey) {
                        // Send cancel message to ProcessorFactory
                        if let Some(ref processor_ref) = state.processor_ref {
                            if let Err(e) = processor_ref
                                .send_message(ProcessorMessage::CancelThread(update.pubkey))
                            {
                                warn!("Failed to send cancel for thread {}: {:?}", update.pubkey, e);
                            }
                        }
                    }
                } else {
                    debug!("Thread {} discovered (exec_count={})", update.pubkey, thread.exec_count);
                }

                // Track exec_count and schedule (cache has full data)
                state.tracked_threads.insert(
                    update.pubkey,
                    TrackedThread {
                        exec_count: thread.exec_count,
                        schedule: thread.schedule.clone(),
                    },
                );

                // Schedule in appropriate priority queue
                self.schedule_thread(state, update.pubkey, &thread).await?;
            }
            AccountType::Clock => {
                // Clock updates should come via ClockTick message
            }
            AccountType::Deleted => {
                debug!("Thread {} deleted", update.pubkey);
                state.tracked_threads.remove(&update.pubkey);
                state.queued_threads.remove(&update.pubkey);
            }
            AccountType::Other => {
                // Not a thread account (could be Fiber, ThreadConfig, etc.)
            }
        }

        Ok(())
    }

    /// Handle clock tick - evaluate ready threads and push to processor
    async fn handle_clock_tick(
        &self,
        state: &mut StagingState,
        clock: Clock,
    ) -> Result<(), ActorProcessingErr> {
        // Dedup: Drop stale clocks (slots move forward only)
        if clock.slot <= state.last_processed_slot {
            debug!(
                "Dropping stale clock tick (slot={} <= last_processed={})",
                clock.slot, state.last_processed_slot
            );
            return Ok(());
        }

        // Update last processed slot
        state.last_processed_slot = clock.slot;

        trace!("ClockTick: slot={}, epoch={}, timestamp={}",
            clock.slot, clock.epoch, clock.unix_timestamp);

        // Process cache eviction refetches
        // These are threads whose cache entries expired (trigger_time + grace_period)
        // We refetch them via RPC to ensure they're not lost
        while let Ok(pubkey) = state.eviction_rx.try_recv() {
            debug!("Processing cache eviction refetch for thread {}", pubkey);
            match state
                .resources
                .cache
                .get_thread_or_fetch(&pubkey, &state.resources.rpc_client)
                .await
            {
                Ok(thread) => {
                    // Update tracked thread with fresh data
                    state.tracked_threads.insert(
                        pubkey,
                        TrackedThread {
                            exec_count: thread.exec_count,
                            schedule: thread.schedule.clone(),
                        },
                    );
                    // Re-schedule based on new trigger info
                    if let Err(e) = self.schedule_thread(state, pubkey, &thread).await {
                        warn!("Failed to reschedule thread {} after refetch: {:?}", pubkey, e);
                    } else {
                        info!("Refetched and rescheduled thread {} after cache expiry", pubkey);
                    }
                }
                Err(e) => {
                    // Thread no longer exists or RPC failed - clean up tracking
                    debug!("Thread {} no longer exists or fetch failed: {}", pubkey, e);
                    state.tracked_threads.remove(&pubkey);
                    state.queued_threads.remove(&pubkey);
                }
            }
        }

        // Get ready threads from all priority queues
        let ready_threads = self
            .get_ready_threads(state, clock.unix_timestamp, clock.slot, clock.epoch)
            .await;

        if !ready_threads.is_empty() {
            debug!("Found {} ready threads", ready_threads.len());
        }

        // Push each ready thread to ProcessorFactory
        for ready_thread in ready_threads {
            // Check if already queued (additional dedup safety)
            if state.queued_threads.contains(&ready_thread.thread_pubkey) {
                debug!(
                    "Thread {} already queued, skipping",
                    ready_thread.thread_pubkey
                );
                continue;
            }

            // Mark as queued
            state.queued_threads.insert(ready_thread.thread_pubkey);

            // Push to ProcessorFactory (if processor ref is set)
            // ProcessorFactory will fetch full Thread data from cache
            if let Some(ref processor_ref) = state.processor_ref {
                if let Err(e) =
                    processor_ref.send_message(ProcessorMessage::ProcessReady(ready_thread.clone()))
                {
                    warn!(
                        "Failed to send thread {} to processor: {:?}",
                        ready_thread.thread_pubkey, e
                    );
                    // Remove from queued since it wasn't successfully sent
                    state.queued_threads.remove(&ready_thread.thread_pubkey);
                } else {
                    debug!(
                        "Pushed thread {} to processor queue",
                        ready_thread.thread_pubkey
                    );
                }
            } else {
                warn!(
                    "ProcessorFactory not initialized yet, skipping thread {}",
                    ready_thread.thread_pubkey
                );
                state.queued_threads.remove(&ready_thread.thread_pubkey);
            }
        }

        Ok(())
    }

    /// Schedule a thread in the appropriate priority queue
    async fn schedule_thread(
        &self,
        state: &mut StagingState,
        thread_pubkey: Pubkey,
        thread: &Thread,
    ) -> Result<(), ActorProcessingErr> {
        // Determine which queue to add to based on trigger type
        let (queue_type, trigger_value) = match &thread.trigger {
            Trigger::Immediate { .. }
            | Trigger::Timestamp { .. }
            | Trigger::Interval { .. }
            | Trigger::Cron { .. } => {
                if let Schedule::Timed { next, .. } = thread.schedule {
                    ("timestamp", next.max(0) as u64)
                } else {
                    warn!("Time-based trigger with non-Timed schedule for thread {}", thread_pubkey);
                    return Ok(());
                }
            }
            Trigger::Slot { .. } => {
                if let Schedule::Block { next, .. } = thread.schedule {
                    ("slot", next)
                } else {
                    warn!("Slot trigger with non-Block schedule for thread {}", thread_pubkey);
                    return Ok(());
                }
            }
            Trigger::Epoch { .. } => {
                if let Schedule::Block { next, .. } = thread.schedule {
                    ("epoch", next)
                } else {
                    warn!("Epoch trigger with non-Block schedule for thread {}", thread_pubkey);
                    return Ok(());
                }
            }
            Trigger::Account { .. } => {
                warn!("Account triggers not yet supported for thread {}", thread_pubkey);
                return Ok(());
            }
        };

        let scheduled = ScheduledThread {
            trigger_value,
            thread_pubkey,
            exec_count: thread.exec_count,
        };

        // Add to appropriate queue
        match queue_type {
            "timestamp" => {
                state.time_queue.lock().await.push(Reverse(scheduled));
            }
            "slot" => {
                state.slot_queue.lock().await.push(Reverse(scheduled));
            }
            "epoch" => {
                state.epoch_queue.lock().await.push(Reverse(scheduled));
            }
            _ => unreachable!(),
        }

        Ok(())
    }

    /// Get all threads ready for execution based on current time/slot/epoch
    ///
    /// Returns ReadyThread structs (pubkey + metadata only). ProcessorFactory
    /// will fetch full Thread data from cache.
    async fn get_ready_threads(
        &self,
        state: &StagingState,
        timestamp: i64,
        slot: u64,
        epoch: u64,
    ) -> Vec<ReadyThread> {
        let mut ready = Vec::new();
        let timestamp_u64 = timestamp.max(0) as u64;

        // Track threads already processed in this call to prevent duplicates
        let mut processed_in_this_call: HashSet<Pubkey> = HashSet::new();

        // Check timestamp-triggered threads
        self.check_queue(
            &state.time_queue,
            timestamp_u64,
            timestamp,
            state,
            &mut ready,
            &mut processed_in_this_call,
            "timestamp",
        )
        .await;

        // Check slot-triggered threads
        self.check_queue(
            &state.slot_queue,
            slot,
            timestamp,
            state,
            &mut ready,
            &mut processed_in_this_call,
            "slot",
        )
        .await;

        // Check epoch-triggered threads
        self.check_queue(
            &state.epoch_queue,
            epoch,
            timestamp,
            state,
            &mut ready,
            &mut processed_in_this_call,
            "epoch",
        )
        .await;

        ready
    }

    /// Check a specific queue for ready threads
    ///
    /// Creates ReadyThread entries (pubkey + metadata only). Full Thread data
    /// will be fetched from cache by ProcessorFactory.
    async fn check_queue(
        &self,
        queue: &Arc<Mutex<BinaryHeap<Reverse<ScheduledThread>>>>,
        current_value: u64,
        timestamp: i64,
        state: &StagingState,
        ready: &mut Vec<ReadyThread>,
        processed: &mut HashSet<Pubkey>,
        queue_name: &str,
    ) {
        let mut queue_lock = queue.lock().await;
        let mut latest_exec_count: HashMap<Pubkey, u64> = HashMap::new();

        while let Some(Reverse(scheduled)) = queue_lock.peek() {
            if scheduled.trigger_value <= current_value {
                let Reverse(scheduled) = queue_lock.pop().unwrap();

                // Skip if already processed in this call
                if processed.contains(&scheduled.thread_pubkey) {
                    continue;
                }

                // Look up tracked thread info (exec_count for dedup)
                let tracked = match state.tracked_threads.get(&scheduled.thread_pubkey) {
                    Some(t) => t.clone(),
                    None => {
                        debug!(
                            "Thread {} no longer tracked, skipping stale entry",
                            scheduled.thread_pubkey
                        );
                        continue;
                    }
                };

                // Check for stale exec_count
                if scheduled.exec_count != tracked.exec_count {
                    debug!(
                        "Stale queue entry for {} (expected exec_count={}, got={})",
                        scheduled.thread_pubkey, tracked.exec_count, scheduled.exec_count
                    );
                    continue;
                }

                // Track latest exec_count to filter duplicates
                if let Some(&latest) = latest_exec_count.get(&scheduled.thread_pubkey) {
                    if tracked.exec_count < latest {
                        debug!(
                            "Skipping stale thread {} with exec_count {} (latest: {})",
                            scheduled.thread_pubkey, tracked.exec_count, latest
                        );
                        continue;
                    }
                }
                latest_exec_count.insert(scheduled.thread_pubkey, tracked.exec_count);

                // Calculate overdue
                let overdue_seconds = if queue_name == "timestamp" {
                    timestamp - (scheduled.trigger_value as i64)
                } else {
                    0 // Slot/epoch triggers don't have overdue concept
                };

                debug!(
                    "Thread {} ready from {} queue (trigger_value={}, current={})",
                    scheduled.thread_pubkey, queue_name, scheduled.trigger_value, current_value
                );

                // Create ready thread (pubkey + metadata only)
                // ProcessorFactory will fetch full Thread from cache
                let ready_thread = ReadyThread {
                    thread_pubkey: scheduled.thread_pubkey,
                    exec_count: tracked.exec_count,
                    is_overdue: overdue_seconds > 0,
                    overdue_seconds,
                };

                ready.push(ready_thread);
                processed.insert(scheduled.thread_pubkey);
            } else {
                // No more ready threads in this queue
                break;
            }
        }
    }

    /// Classify account type
    fn classify_account(&self, data: &[u8], pubkey: &Pubkey) -> AccountType {
        use anchor_lang::AccountDeserialize;

        // Check if it's the clock sysvar
        if *pubkey == solana_sdk::sysvar::clock::ID {
            return AccountType::Clock;
        }

        // Check if data is empty (deleted account)
        if data.is_empty() {
            return AccountType::Deleted;
        }

        // Need at least 8 bytes for discriminator + some data
        if data.len() < 8 {
            return AccountType::Other;
        }

        // Try to deserialize as Thread
        // The Thread type uses Anchor's #[account] macro which provides try_deserialize
        if let Ok(thread) = Thread::try_deserialize(&mut &data[..]) {
            return AccountType::Thread(thread);
        }

        AccountType::Other
    }
}

#[derive(Debug)]
enum AccountType {
    Thread(Thread),
    Clock,
    Deleted,
    Other,
}
