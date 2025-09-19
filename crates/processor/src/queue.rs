use anyhow::Result;
use dashmap::{DashMap, DashSet};
use log::{debug, warn};
use solana_program::pubkey::Pubkey;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use antegen_thread_program::state::{Thread, Trigger, TriggerContext};
use crate::load_balancer::{LoadBalancer, ProcessDecision};
use crate::metrics::ProcessorMetrics;
use crate::types::ExecutableThread;

/// Represents a scheduled thread in the priority queue
#[derive(Debug, Clone)]
struct ScheduledThread {
    /// The trigger value (timestamp, slot, or epoch) when this thread should execute
    trigger_value: u64,
    /// The thread's public key
    thread_pubkey: Pubkey,
    /// The thread state
    thread: Thread,
}

impl PartialEq for ScheduledThread {
    fn eq(&self, other: &Self) -> bool {
        self.trigger_value == other.trigger_value
    }
}

impl Eq for ScheduledThread {}

impl PartialOrd for ScheduledThread {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledThread {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.trigger_value.cmp(&other.trigger_value)
    }
}

/// Simple async thread queue with priority-based scheduling
pub struct ThreadQueue {
    /// Priority queue for timestamp-based triggers (min-heap via Reverse)
    time_queue: Arc<Mutex<BinaryHeap<Reverse<ScheduledThread>>>>,
    /// Priority queue for slot-based triggers
    slot_queue: Arc<Mutex<BinaryHeap<Reverse<ScheduledThread>>>>,
    /// Priority queue for epoch-based triggers
    epoch_queue: Arc<Mutex<BinaryHeap<Reverse<ScheduledThread>>>>,
    /// Track currently executing tasks
    active_tasks: Arc<DashMap<Pubkey, JoinHandle<()>>>,
    /// Track which threads are currently queued
    queued_threads: Arc<DashSet<Pubkey>>,
    /// Optional metrics collection
    metrics: Option<Arc<ProcessorMetrics>>,
    /// Load balancer for ownership decisions
    load_balancer: Option<Arc<LoadBalancer>>,
}

impl ThreadQueue {
    /// Create a new thread queue
    pub fn new() -> Self {
        Self {
            time_queue: Arc::new(Mutex::new(BinaryHeap::new())),
            slot_queue: Arc::new(Mutex::new(BinaryHeap::new())),
            epoch_queue: Arc::new(Mutex::new(BinaryHeap::new())),
            active_tasks: Arc::new(DashMap::new()),
            queued_threads: Arc::new(DashSet::new()),
            metrics: None,
            load_balancer: None,
        }
    }

    /// Create with metrics
    pub fn with_metrics(metrics: Arc<ProcessorMetrics>) -> Self {
        let mut queue = Self::new();
        queue.metrics = Some(metrics);
        queue
    }
    
    /// Set the load balancer
    pub fn set_load_balancer(&mut self, load_balancer: Arc<LoadBalancer>) {
        self.load_balancer = Some(load_balancer);
    }

    /// Schedule a thread for execution when its trigger condition is met
    pub async fn schedule_thread(&self, thread_pubkey: Pubkey, thread: Thread) -> Result<()> {
        debug!("Scheduling thread {} with exec_count {}", thread_pubkey, thread.exec_count);
        
        // Cancel any existing execution for this thread
        if let Some((_, task)) = self.active_tasks.remove(&thread_pubkey) {
            task.abort();
            debug!("Cancelled existing execution for thread {} due to update", thread_pubkey);
        }

        // Mark thread as queued (or update if already present)
        // We allow duplicates in the heap - they'll be filtered during processing
        self.queued_threads.insert(thread_pubkey);

        // Determine which queue to add to based on trigger type
        let (queue_type, trigger_value) = match &thread.trigger_context {
            TriggerContext::Timestamp { next, .. } => {
                ("timestamp", *next as u64)
            }
            TriggerContext::Block { next, .. } => {
                // Check trigger type to determine if slot or epoch
                match &thread.trigger {
                    Trigger::Slot { .. } => ("slot", *next),
                    Trigger::Epoch { .. } => ("epoch", *next),
                    _ => {
                        warn!("Unexpected trigger type for Block context");
                        self.queued_threads.remove(&thread_pubkey);
                        return Ok(());
                    }
                }
            }
            TriggerContext::Account { .. } => {
                warn!("Account triggers not yet supported for thread {}", thread_pubkey);
                self.queued_threads.remove(&thread_pubkey);
                return Ok(());
            }
        };

        let scheduled = ScheduledThread {
            trigger_value,
            thread_pubkey,
            thread,
        };

        // Add to appropriate queue
        match queue_type {
            "timestamp" => {
                let mut queue = self.time_queue.lock().await;
                queue.push(Reverse(scheduled));
                debug!("Thread {} scheduled for timestamp {}", thread_pubkey, trigger_value);
            }
            "slot" => {
                let mut queue = self.slot_queue.lock().await;
                queue.push(Reverse(scheduled));
                debug!("Thread {} scheduled for slot {}", thread_pubkey, trigger_value);
            }
            "epoch" => {
                let mut queue = self.epoch_queue.lock().await;
                queue.push(Reverse(scheduled));
                debug!("Thread {} scheduled for epoch {}", thread_pubkey, trigger_value);
            }
            _ => unreachable!(),
        }

        self.update_metrics();
        Ok(())
    }

    /// Get all threads ready for execution based on current time/slot/epoch
    pub async fn get_ready_threads(
        &self,
        timestamp: i64,
        slot: u64,
        epoch: u64,
    ) -> Vec<ExecutableThread> {
        let mut ready = Vec::new();
        let timestamp_u64 = timestamp.max(0) as u64;

        // Check timestamp-triggered threads
        {
            let mut queue = self.time_queue.lock().await;
            let mut to_requeue = Vec::new();
            let mut latest_exec_count: HashMap<Pubkey, u64> = HashMap::new();
            
            // Pop all ready threads
            while let Some(Reverse(scheduled)) = queue.peek() {
                if scheduled.trigger_value <= timestamp_u64 {
                    let Reverse(scheduled) = queue.pop().unwrap();
                    
                    // Check if this is a stale version
                    if let Some(&latest) = latest_exec_count.get(&scheduled.thread_pubkey) {
                        if scheduled.thread.exec_count < latest {
                            debug!("Skipping stale thread {} with exec_count {} (latest: {})", 
                                  scheduled.thread_pubkey, scheduled.thread.exec_count, latest);
                            continue; // Don't process or re-queue stale versions
                        }
                    }
                    latest_exec_count.insert(scheduled.thread_pubkey, scheduled.thread.exec_count);
                    
                    // Calculate overdue seconds for timestamp triggers
                    let overdue_seconds = timestamp - (scheduled.trigger_value as i64);
                    
                    // Check with load balancer
                    let should_process = if let Some(ref load_balancer) = self.load_balancer {
                        match load_balancer.should_process(
                            &scheduled.thread_pubkey,
                            &scheduled.thread.last_executor,
                            true, // is_overdue
                            overdue_seconds,
                        ).await {
                            Ok(ProcessDecision::Process) => {
                                debug!("Thread {} ready at timestamp {} (overdue by {}s, will process)", 
                                      scheduled.thread_pubkey, timestamp, overdue_seconds);
                                true
                            }
                            Ok(ProcessDecision::Skip) | Ok(ProcessDecision::AtCapacity) => {
                                debug!("Thread {} ready but skipped by load balancer (overdue by {}s)", 
                                      scheduled.thread_pubkey, overdue_seconds);
                                false
                            }
                            Err(e) => {
                                warn!("Load balancer error for thread {}: {}", scheduled.thread_pubkey, e);
                                false
                            }
                        }
                    } else {
                        // No load balancer, process all ready threads
                        debug!("Thread {} ready at timestamp {} (no load balancer)", 
                              scheduled.thread_pubkey, timestamp);
                        true
                    };
                    
                    if should_process {
                        // Remove from queued set since we're processing it
                        self.queued_threads.remove(&scheduled.thread_pubkey);
                        ready.push(ExecutableThread {
                            thread_pubkey: scheduled.thread_pubkey,
                            thread: scheduled.thread,
                            slot,
                        });
                    } else {
                        // Re-queue for next check (only latest versions)
                        to_requeue.push(scheduled);
                    }
                } else {
                    // Not ready yet, stop checking
                    break;
                }
            }
            
            // Re-queue threads we're not processing
            for scheduled in to_requeue {
                queue.push(Reverse(scheduled));
            }
        }

        // For slot and epoch triggers, we don't have a good way to calculate overdue seconds
        // So for now, just process them without load balancing
        // TODO: Implement proper overdue calculation for slot/epoch triggers
        
        // Check slot-triggered threads
        {
            let mut queue = self.slot_queue.lock().await;
            while let Some(Reverse(scheduled)) = queue.peek() {
                if scheduled.trigger_value <= slot {
                    let Reverse(scheduled) = queue.pop().unwrap();
                    debug!("Thread {} ready at slot {}", 
                           scheduled.thread_pubkey, slot);
                    // Remove from queued set since we're processing it
                    self.queued_threads.remove(&scheduled.thread_pubkey);
                    ready.push(ExecutableThread {
                        thread_pubkey: scheduled.thread_pubkey,
                        thread: scheduled.thread,
                        slot,
                    });
                } else {
                    break;
                }
            }
        }

        // Check epoch-triggered threads
        {
            let mut queue = self.epoch_queue.lock().await;
            while let Some(Reverse(scheduled)) = queue.peek() {
                if scheduled.trigger_value <= epoch {
                    let Reverse(scheduled) = queue.pop().unwrap();
                    debug!("Thread {} ready at epoch {}", 
                           scheduled.thread_pubkey, epoch);
                    // Remove from queued set since we're processing it
                    self.queued_threads.remove(&scheduled.thread_pubkey);
                    ready.push(ExecutableThread {
                        thread_pubkey: scheduled.thread_pubkey,
                        thread: scheduled.thread,
                        slot,
                    });
                } else {
                    break;
                }
            }
        }

        ready
    }

    /// Track an active task
    pub fn track_task(&self, thread_pubkey: Pubkey, handle: JoinHandle<()>) {
        self.active_tasks.insert(thread_pubkey, handle);
        self.update_metrics();
    }

    /// Remove completed task
    pub fn task_completed(&self, thread_pubkey: &Pubkey) {
        self.active_tasks.remove(thread_pubkey);
        self.update_metrics();
    }

    /// Abort any active task for a thread without scheduling a new one
    pub fn abort_task_if_active(&self, thread_pubkey: &Pubkey) {
        if let Some((_, task)) = self.active_tasks.remove(thread_pubkey) {
            task.abort();
            debug!("Aborted active task for thread {} due to update", thread_pubkey);
            self.update_metrics();
        }
    }

    /// Update metrics
    fn update_metrics(&self) {
        if let Some(ref metrics) = self.metrics {
            let active_count = self.active_tasks.len() as u64;
            metrics.set_queue_size(active_count, None);
        }
    }

    /// Get number of active tasks
    pub fn active_task_count(&self) -> usize {
        self.active_tasks.len()
    }

    /// Get detailed queue statistics
    pub async fn get_queue_stats(&self) -> QueueStats {
        let time_queue_size = self.time_queue.lock().await.len();
        let slot_queue_size = self.slot_queue.lock().await.len();
        let epoch_queue_size = self.epoch_queue.lock().await.len();
        let active_tasks = self.active_tasks.len();
        
        QueueStats {
            timestamp_threads: time_queue_size,
            slot_threads: slot_queue_size,
            epoch_threads: epoch_queue_size,
            total_monitored: time_queue_size + slot_queue_size + epoch_queue_size,
            active_executions: active_tasks,
        }
    }
}

/// Statistics about thread queue state
#[derive(Debug, Clone)]
pub struct QueueStats {
    pub timestamp_threads: usize,
    pub slot_threads: usize,
    pub epoch_threads: usize,
    pub total_monitored: usize,
    pub active_executions: usize,
}

impl Clone for ThreadQueue {
    fn clone(&self) -> Self {
        Self {
            time_queue: self.time_queue.clone(),
            slot_queue: self.slot_queue.clone(),
            epoch_queue: self.epoch_queue.clone(),
            active_tasks: self.active_tasks.clone(),
            queued_threads: self.queued_threads.clone(),
            metrics: self.metrics.clone(),
            load_balancer: self.load_balancer.clone(),
        }
    }
}