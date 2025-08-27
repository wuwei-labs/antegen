//! Thread Queue Implementation
//! 
//! This module provides a priority-based thread scheduling system using crossbeam channels.
//! 
//! ## Architecture
//! 
//! The queue consists of three main components:
//! 
//! 1. **ThreadQueue** (Public API): 
//!    - Accepts thread scheduling requests from the service
//!    - Spawns execution tasks with concurrency control via semaphore
//!    
//! 2. **Scheduler** (Internal Thread):
//!    - Manages three priority queues (time, slot, epoch)
//!    - Evaluates trigger conditions and moves ready threads to execution channel
//!    
//! 3. **Execution Channel**: 
//!    - Lock-free channel connecting scheduler to executor
//!    - Threads flow from priority queues → channel → execution tasks
//!
//! ## Flow
//! ```text
//! schedule_thread() → Scheduler (priority queues) → check_triggers() → 
//! execution channel → spawn_execution_tasks() → thread_executor()
//! ```

use crate::clock::SharedClock;
use crate::metrics::ProcessorMetrics;
use antegen_thread_program::state::{Thread, Trigger, TriggerContext};
use anyhow::Result;
use crossbeam::channel::{unbounded, Sender, Receiver};
use dashmap::DashMap;
use log::{debug, info, warn};
use solana_sdk::{pubkey::Pubkey, signature::Signature};
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;
use std::sync::Arc;
use std::thread;
use tokio::{sync::Semaphore, task::JoinHandle};

// ============================================================================
// Public Types
// ============================================================================

/// Thread trigger type for queue selection
#[derive(Debug, Clone, Copy)]
pub enum TriggerType {
    Time,
    Slot,
    Epoch,
}

// ============================================================================
// Internal Types  
// ============================================================================

/// Thread scheduled in a priority queue, ordered by trigger value
#[derive(Clone, Debug)]
struct ScheduledThread {
    trigger_value: u64, // timestamp, slot, or epoch
    thread_pubkey: Pubkey,
    thread: Thread,
}

impl PartialEq for ScheduledThread {
    fn eq(&self, other: &Self) -> bool {
        self.trigger_value == other.trigger_value 
            && self.thread_pubkey == other.thread_pubkey
    }
}

impl Eq for ScheduledThread {}

impl Ord for ScheduledThread {
    fn cmp(&self, other: &Self) -> Ordering {
        self.trigger_value
            .cmp(&other.trigger_value)
            .then_with(|| self.thread_pubkey.cmp(&other.thread_pubkey))
    }
}

impl PartialOrd for ScheduledThread {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Commands for the scheduler thread
#[derive(Debug)]
enum SchedulerCommand {
    /// Add a thread to the appropriate priority queue
    AddThread {
        thread_pubkey: Pubkey,
        thread: Thread,
    },
    /// Check all queues for threads ready to execute
    CheckAllTriggers {
        timestamp: i64,
        slot: u64,
        epoch: u64,
    },
    /// Check a specific queue for ready threads
    CheckSingleTrigger {
        trigger_type: TriggerType,
        current_value: u64,
        timestamp: i64,
    },
    /// Shutdown the scheduler
    Shutdown,
}

/// Thread that has met its trigger condition and is ready to execute
struct ExecutableThread {
    thread_pubkey: Pubkey,
    thread: Thread,
    trigger_timestamp: i64,  // Blockchain time when trigger fired
}

// ============================================================================
// Scheduler Implementation
// ============================================================================

/// Internal scheduler that manages priority queues and evaluates trigger conditions
struct Scheduler {
    time_queue: BinaryHeap<Reverse<ScheduledThread>>,
    slot_queue: BinaryHeap<Reverse<ScheduledThread>>,
    epoch_queue: BinaryHeap<Reverse<ScheduledThread>>,
    execution_sender: Sender<ExecutableThread>,
}

impl Scheduler {
    fn new(execution_sender: Sender<ExecutableThread>) -> Self {
        Self {
            time_queue: BinaryHeap::new(),
            slot_queue: BinaryHeap::new(),
            epoch_queue: BinaryHeap::new(),
            execution_sender,
        }
    }

    /// Add a thread to the appropriate priority queue based on its trigger type
    fn add_thread(&mut self, thread_pubkey: Pubkey, thread: Thread) {
        // Extract values needed for logging before consuming thread
        let (queue_type, trigger_value) = match &thread.trigger_context {
            TriggerContext::Timestamp { next, .. } => {
                let value = (*next).max(0) as u64;
                ("timestamp", value)
            }
            TriggerContext::Block { next, .. } => {
                match &thread.trigger {
                    Trigger::Slot { .. } => ("slot", *next),
                    Trigger::Epoch { .. } => ("epoch", *next),
                    _ => {
                        warn!("QUEUE: Thread {} has Block context but non-block trigger: {:?}",
                              thread_pubkey, thread.trigger);
                        return;
                    }
                }
            }
            TriggerContext::Account { .. } => {
                warn!("QUEUE: Thread {} has Account trigger which is not yet supported",
                      thread_pubkey);
                return;
            }
        };

        // Now consume thread and add to appropriate queue
        let scheduled = ScheduledThread {
            trigger_value,
            thread_pubkey,
            thread,
        };

        match queue_type {
            "timestamp" => {
                self.time_queue.push(Reverse(scheduled));
                info!("QUEUE: Thread {} scheduled for timestamp {}", thread_pubkey, trigger_value);
            }
            "slot" => {
                self.slot_queue.push(Reverse(scheduled));
                debug!("QUEUE: Thread {} scheduled for slot {}", thread_pubkey, trigger_value);
            }
            "epoch" => {
                self.epoch_queue.push(Reverse(scheduled));
                debug!("QUEUE: Thread {} scheduled for epoch {}", thread_pubkey, trigger_value);
            }
            _ => unreachable!(),
        }
    }

    /// Move threads that have met their trigger condition from queue to execution channel
    fn move_triggered_threads(
        queue: &mut BinaryHeap<Reverse<ScheduledThread>>,
        max_value: u64,
        execution_sender: &Sender<ExecutableThread>,
        trigger_timestamp: i64,
    ) {
        while let Some(&Reverse(ref entry)) = queue.peek() {
            if entry.trigger_value > max_value {
                break;
            }

            let Reverse(scheduled) = queue.pop().unwrap();
            let _ = execution_sender.send(ExecutableThread {
                thread_pubkey: scheduled.thread_pubkey,
                thread: scheduled.thread,
                trigger_timestamp,
            });
        }
    }

    /// Evaluate all queues and move threads with met trigger conditions to execution
    fn evaluate_all_triggers(&mut self, timestamp: i64, slot: u64, epoch: u64) {
        let timestamp_u64 = timestamp.max(0) as u64;
        
        Self::move_triggered_threads(
            &mut self.time_queue,
            timestamp_u64,
            &self.execution_sender,
            timestamp,
        );
        
        Self::move_triggered_threads(
            &mut self.slot_queue,
            slot,
            &self.execution_sender,
            timestamp,
        );
        
        Self::move_triggered_threads(
            &mut self.epoch_queue,
            epoch,
            &self.execution_sender,
            timestamp,
        );
    }

    /// Evaluate a specific queue and move ready threads to execution
    fn evaluate_single_trigger(&mut self, trigger_type: TriggerType, current_value: u64, timestamp: i64) {
        let queue = match trigger_type {
            TriggerType::Time => &mut self.time_queue,
            TriggerType::Slot => &mut self.slot_queue,
            TriggerType::Epoch => &mut self.epoch_queue,
        };
        
        Self::move_triggered_threads(queue, current_value, &self.execution_sender, timestamp);
    }

    /// Main scheduler loop - processes commands and manages priority queues
    fn run_event_loop(command_receiver: Receiver<SchedulerCommand>, execution_sender: Sender<ExecutableThread>) {
        let mut scheduler = Self::new(execution_sender);
        info!("QUEUE: Scheduler started");

        while let Ok(command) = command_receiver.recv() {
            match command {
                SchedulerCommand::AddThread { thread_pubkey, thread } => {
                    scheduler.add_thread(thread_pubkey, thread);
                }
                SchedulerCommand::CheckAllTriggers { timestamp, slot, epoch } => {
                    scheduler.evaluate_all_triggers(timestamp, slot, epoch);
                }
                SchedulerCommand::CheckSingleTrigger { trigger_type, current_value, timestamp } => {
                    scheduler.evaluate_single_trigger(trigger_type, current_value, timestamp);
                }
                SchedulerCommand::Shutdown => {
                    info!("QUEUE: Scheduler shutting down");
                    break;
                }
            }
        }
    }
}

// ============================================================================
// Public API
// ============================================================================

/// Thread queue with priority-based scheduling and concurrent execution control
#[derive(Clone)]
pub struct ThreadQueue {
    /// Send commands to the scheduler thread
    scheduler_sender: Sender<SchedulerCommand>,
    /// Receive threads ready for execution from scheduler
    execution_receiver: Receiver<ExecutableThread>,
    /// Track currently executing tasks
    executing_tasks: Arc<DashMap<Pubkey, JoinHandle<()>>>,
    /// Limit concurrent executions (default: 50)
    concurrency_limiter: Arc<Semaphore>,
    /// Shared blockchain clock
    clock: SharedClock,
    /// Optional metrics collection
    metrics: Option<Arc<ProcessorMetrics>>,
}

impl ThreadQueue {
    /// Create a new thread queue with specified concurrency limit
    pub fn new(max_concurrent_threads: usize, clock: SharedClock) -> Result<Self> {
        let (scheduler_sender, scheduler_receiver) = unbounded();
        let (execution_sender, execution_receiver) = unbounded();

        // Spawn the scheduler in a dedicated OS thread
        thread::spawn(move || {
            Scheduler::run_event_loop(scheduler_receiver, execution_sender);
        });

        Ok(Self {
            scheduler_sender,
            execution_receiver,
            executing_tasks: Arc::new(DashMap::new()),
            concurrency_limiter: Arc::new(Semaphore::new(max_concurrent_threads)),
            clock,
            metrics: None,
        })
    }

    /// Create a new thread queue with metrics
    pub fn with_metrics(
        max_concurrent_threads: usize,
        clock: SharedClock,
        metrics: Arc<ProcessorMetrics>,
    ) -> Result<Self> {
        let mut queue = Self::new(max_concurrent_threads, clock)?;
        queue.metrics = Some(metrics);
        Ok(queue)
    }

    /// Update metrics for executing task count
    fn update_execution_metrics(&self) {
        if let Some(ref metrics) = self.metrics {
            let executing_count = self.executing_tasks.len() as u64;
            metrics.set_queue_size(executing_count, None);
        }
    }

    /// Schedule a thread for execution when its trigger condition is met
    pub async fn schedule_thread(&self, thread_pubkey: Pubkey, thread: Thread) -> Result<()> {
        // Cancel any existing execution for this thread
        if let Some((_, task)) = self.executing_tasks.remove(&thread_pubkey) {
            task.abort();
            info!("QUEUE: Cancelled existing execution for thread {} due to update", thread_pubkey);
        }

        // Send thread to scheduler for queuing
        self.scheduler_sender
            .send(SchedulerCommand::AddThread { thread_pubkey, thread })
            .map_err(|e| anyhow::anyhow!("Failed to send thread to scheduler: {}", e))?;

        self.update_execution_metrics();
        Ok(())
    }

    /// Continuously spawn execution tasks for threads in the execution channel (event-driven)
    pub async fn spawn_execution_tasks_continuous<F, Fut>(&self, thread_executor: F)
    where
        F: Fn(Pubkey, Thread) -> Fut + Send + Sync + 'static + Clone,
        Fut: std::future::Future<Output = Result<Signature>> + Send + 'static,
    {
        let receiver = self.execution_receiver.clone();
        let executing_tasks = self.executing_tasks.clone();
        let concurrency_limiter = self.concurrency_limiter.clone();
        let metrics = self.metrics.clone();
        let clock = self.clock.clone();
        
        // Use blocking task to handle channel iterator (truly event-driven)
        tokio::task::spawn_blocking(move || {
            // Use iter() which blocks on each item - this is truly event-driven!
            for executable in receiver.iter() {
                let thread_pubkey = executable.thread_pubkey;
                let thread = executable.thread.clone();
                let trigger_timestamp = executable.trigger_timestamp;
                
                // Skip if already executing
                if executing_tasks.contains_key(&thread_pubkey) {
                    debug!("QUEUE: Thread {} already being processed, skipping", thread_pubkey);
                    continue;
                }
                
                info!("QUEUE: Moving thread {} to execution at blockchain time {} (trigger: {:?})", 
                      thread_pubkey, 
                      trigger_timestamp,
                      thread.trigger);
                
                // Clone for the spawned task
                let executing_tasks_clone = executing_tasks.clone();
                let concurrency_limiter_clone = concurrency_limiter.clone();
                let metrics_clone = metrics.clone();
                let executor_clone = thread_executor.clone();
                let clock_clone = clock.clone();
                
                // Need to spawn async task from blocking context
                let handle = tokio::runtime::Handle::current();
                let task = handle.spawn(async move {
                    let blockchain_time = clock_clone.get_timestamp().await;
                    info!("QUEUE: Task spawned for thread {} at blockchain time {}", 
                        thread_pubkey, blockchain_time);
                    
                    // Wait for execution slot
                    info!("QUEUE: Acquiring permit for thread {}", thread_pubkey);
                    let _permit = match concurrency_limiter_clone.acquire().await {
                        Ok(permit) => {
                            info!("QUEUE: Acquired permit for thread {}", thread_pubkey);
                            permit
                        },
                        Err(_) => {
                            warn!("Failed to acquire semaphore permit for thread {}", thread_pubkey);
                            return;
                        }
                    };
                    
                    let exec_time = clock_clone.get_timestamp().await;
                    info!("QUEUE: Starting execution for thread {} at blockchain time {} (slots: {})",
                          thread_pubkey, 
                          exec_time,
                          concurrency_limiter_clone.available_permits());
                    
                    // Execute thread - task handles all retries internally
                    match executor_clone(thread_pubkey, thread).await {
                        Ok(signature) => {
                            info!("Thread {} executed: {}", thread_pubkey, signature);
                        }
                        Err(e) => {
                            warn!("QUEUE: Thread {} failed after all retries: {}",
                                  thread_pubkey, e);
                        }
                    }
                    
                    // Remove from executing set
                    executing_tasks_clone.remove(&thread_pubkey);
                    
                    // Update metrics
                    if let Some(ref m) = metrics_clone {
                        let executing_count = executing_tasks_clone.len() as u64;
                        m.set_queue_size(executing_count, None);
                    }
                });
                
                // Track the executing task
                executing_tasks.insert(thread_pubkey, task);
            }
            
            warn!("QUEUE: Execution receiver closed, stopping execution processor");
        });
    }
    

    /// Check a specific trigger type (threads will be executed automatically by continuous task)
    pub async fn check_single_trigger(
        &self,
        trigger_type: TriggerType,
        current_value: u64,
    ) {
        info!("QUEUE: Checking {:?} trigger for threads <= {}", trigger_type, current_value);

        // Get current blockchain timestamp for the trigger
        let timestamp = self.clock.get_timestamp().await;

        // Request scheduler to evaluate this trigger type
        // Any threads that meet criteria will be sent to execution channel
        // and automatically picked up by the continuous execution task
        if let Err(e) = self.scheduler_sender.send(SchedulerCommand::CheckSingleTrigger {
            trigger_type,
            current_value,
            timestamp,
        }) {
            warn!("Failed to send trigger check to scheduler: {}", e);
        }
    }

    /// Check all trigger types (threads will be executed automatically by continuous task)
    pub async fn check_all_triggers(
        &self,
        current_slot: u64,
        current_epoch: u64,
        current_timestamp: i64,
    ) {
        info!("QUEUE: Checking all triggers - timestamp: {}, slot: {}, epoch: {}",
              current_timestamp, current_slot, current_epoch);

        // Request scheduler to evaluate all trigger types
        // Any threads that meet criteria will be sent to execution channel
        // and automatically picked up by the continuous execution task
        if let Err(e) = self.scheduler_sender.send(SchedulerCommand::CheckAllTriggers {
            timestamp: current_timestamp,
            slot: current_slot,
            epoch: current_epoch,
        }) {
            warn!("Failed to send trigger check to scheduler: {}", e);
        }
    }
}

impl Drop for ThreadQueue {
    fn drop(&mut self) {
        // Gracefully shutdown the scheduler thread
        let _ = self.scheduler_sender.send(SchedulerCommand::Shutdown);
    }
}