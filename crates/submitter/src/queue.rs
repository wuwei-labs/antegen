use antegen_thread_program::state::{Thread, Trigger, TriggerContext};
use anyhow::Result;
use dashmap::DashMap;
use log::{debug, info, warn};
use solana_sdk::{pubkey::Pubkey, signature::Signature};
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::{sync::Semaphore, task::JoinHandle};

/// Entry in the priority queue with version tracking
#[derive(Clone, Debug, Eq, PartialEq)]
struct ThreadEntry {
    trigger_value: u64, // timestamp, slot, or epoch
    thread_pubkey: Pubkey,
    version: u64, // Version to detect stale entries
}

// Implement Ord for min-heap behavior (lowest trigger_value first)
impl Ord for ThreadEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Only compare by trigger_value, not version
        self.trigger_value
            .cmp(&other.trigger_value)
            .then_with(|| self.thread_pubkey.cmp(&other.thread_pubkey))
    }
}

impl PartialOrd for ThreadEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Ephemeral thread queue with version tracking
#[derive(Clone)]
pub struct ThreadQueue {
    // Priority queues (min-heap via Reverse)
    scheduled_by_time: Arc<Mutex<BinaryHeap<Reverse<ThreadEntry>>>>,
    scheduled_by_slot: Arc<Mutex<BinaryHeap<Reverse<ThreadEntry>>>>,
    scheduled_by_epoch: Arc<Mutex<BinaryHeap<Reverse<ThreadEntry>>>>,

    // Version tracking for each thread
    thread_versions: Arc<DashMap<Pubkey, u64>>,

    // Thread data storage
    threads: Arc<DashMap<Pubkey, Thread>>,

    // Active task tracking
    active_tasks: Arc<DashMap<Pubkey, JoinHandle<()>>>,

    // Limit concurrent processing tasks
    task_semaphore: Arc<Semaphore>,
}

impl ThreadQueue {
    /// Create a new ephemeral thread queue
    pub fn new(max_concurrent_threads: usize) -> Result<Self> {
        Ok(Self {
            scheduled_by_time: Arc::new(Mutex::new(BinaryHeap::new())),
            scheduled_by_slot: Arc::new(Mutex::new(BinaryHeap::new())),
            scheduled_by_epoch: Arc::new(Mutex::new(BinaryHeap::new())),
            thread_versions: Arc::new(DashMap::new()),
            threads: Arc::new(DashMap::new()),
            active_tasks: Arc::new(DashMap::new()),
            task_semaphore: Arc::new(Semaphore::new(max_concurrent_threads)),
        })
    }

    /// Schedule a thread
    pub fn schedule_thread(&self, thread_pubkey: Pubkey, thread: Thread) -> Result<()> {
        // Cancel any existing task for this thread
        if let Some((_, task)) = self.active_tasks.remove(&thread_pubkey) {
            task.abort();
            info!(
                "QUEUE: Cancelled old task for thread {} due to update",
                thread_pubkey
            );
        }

        // Increment version for this thread
        let version = self
            .thread_versions
            .entry(thread_pubkey)
            .and_modify(|v| *v += 1)
            .or_insert(0)
            .clone();

        // Store/update thread data
        self.threads.insert(thread_pubkey, thread.clone());

        // Add to appropriate priority queue based on trigger context
        match &thread.trigger_context {
            TriggerContext::Timestamp { next, .. } => {
                let entry = ThreadEntry {
                    trigger_value: (*next).max(0) as u64,
                    thread_pubkey,
                    version,
                };
                let mut queue = self.scheduled_by_time.lock().unwrap();
                queue.push(Reverse(entry));
                info!(
                    "QUEUE: Thread {} scheduled for timestamp {} (version: {})",
                    thread_pubkey, next, version
                );
            }
            TriggerContext::Block { next, .. } => {
                // Determine if this is slot or epoch based on trigger type
                match &thread.trigger {
                    Trigger::Slot { .. } => {
                        let entry = ThreadEntry {
                            trigger_value: *next,
                            thread_pubkey,
                            version,
                        };
                        let mut queue = self.scheduled_by_slot.lock().unwrap();
                        queue.push(Reverse(entry));
                        debug!(
                            "QUEUE: Thread {} scheduled for slot {} (version: {})",
                            thread_pubkey, next, version
                        );
                    }
                    Trigger::Epoch { .. } => {
                        let entry = ThreadEntry {
                            trigger_value: *next,
                            thread_pubkey,
                            version,
                        };
                        let mut queue = self.scheduled_by_epoch.lock().unwrap();
                        queue.push(Reverse(entry));
                        debug!(
                            "QUEUE: Thread {} scheduled for epoch {} (version: {})",
                            thread_pubkey, next, version
                        );
                    }
                    _ => {
                        // Shouldn't happen, but handle gracefully
                        warn!(
                            "QUEUE: Thread {} has Block context but non-block trigger: {:?}",
                            thread_pubkey, thread.trigger
                        );
                    }
                }
            }
            TriggerContext::Account { .. } => {
                // Account-based triggers not supported in ephemeral queue yet
                warn!(
                    "QUEUE: Thread {} has Account trigger which is not yet supported",
                    thread_pubkey
                );
            }
        }

        Ok(())
    }

    /// Spawn a processing task with self-contained retry logic
    fn spawn_processing_task<F, Fut>(
        &self,
        thread_pubkey: Pubkey,
        thread: Thread,
        thread_executor: F,
    ) -> JoinHandle<()>
    where
        F: Fn(Pubkey, Thread) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Signature>> + Send + 'static,
    {
        let active_tasks = self.active_tasks.clone();
        let threads = self.threads.clone();
        let thread_versions = self.thread_versions.clone();
        let semaphore = self.task_semaphore.clone();

        tokio::spawn(async move {
            // Acquire permit before processing - this will wait if at capacity
            let _permit = match semaphore.acquire().await {
                Ok(permit) => permit,
                Err(_) => {
                    warn!(
                        "Failed to acquire semaphore permit for thread {}",
                        thread_pubkey
                    );
                    return;
                }
            };

            info!(
                "QUEUE: Task started for thread {} (permits available: {})",
                thread_pubkey,
                semaphore.available_permits()
            );

            let mut backoff = Duration::from_secs(1);
            let max_backoff = Duration::from_secs(30);

            loop {
                match thread_executor(thread_pubkey, thread.clone()).await {
                    Ok(signature) => {
                        info!(
                            "QUEUE: Thread {} succeeded with signature {}",
                            thread_pubkey, signature
                        );

                        // Clean up on success
                        threads.remove(&thread_pubkey);
                        thread_versions.remove(&thread_pubkey);
                        active_tasks.remove(&thread_pubkey);
                        break;
                    }
                    Err(e) => {
                        warn!(
                            "Thread {} failed: {}, retrying in {:?}",
                            thread_pubkey, e, backoff
                        );

                        // Check if task was cancelled (thread updated)
                        if !active_tasks.contains_key(&thread_pubkey) {
                            info!("Task for thread {} was cancelled", thread_pubkey);
                            break;
                        }

                        tokio::time::sleep(backoff).await;
                        backoff = std::cmp::min(backoff * 2, max_backoff);
                    }
                }
            }
            // Permit automatically dropped here, releasing the slot
        })
    }

    /// Process all threads from a specific queue that are ready
    async fn process_queue_with_version<F, Fut>(
        &self,
        queue: &Arc<Mutex<BinaryHeap<Reverse<ThreadEntry>>>>,
        max_trigger_value: u64,
        thread_executor: F,
    ) where
        F: Fn(Pubkey, Thread) -> Fut + Send + Sync + 'static + Clone,
        Fut: std::future::Future<Output = Result<Signature>> + Send + 'static,
    {
        let mut ready_threads = Vec::new();

        // Extract ready threads while validating versions
        {
            let mut queue = queue.lock().unwrap();
            while let Some(&Reverse(ref entry)) = queue.peek() {
                if entry.trigger_value > max_trigger_value {
                    break;
                }

                let Reverse(entry) = queue.pop().unwrap();

                // Check if this is the current version
                if let Some(current_version) = self.thread_versions.get(&entry.thread_pubkey) {
                    if entry.version < *current_version {
                        // Stale entry, skip it
                        debug!(
                            "QUEUE: Skipping stale entry for thread {} (version {} < {})",
                            entry.thread_pubkey, entry.version, *current_version
                        );
                        continue;
                    }
                }

                // Skip if already being processed
                if self.active_tasks.contains_key(&entry.thread_pubkey) {
                    debug!(
                        "QUEUE: Thread {} already being processed, skipping",
                        entry.thread_pubkey
                    );
                    continue;
                }

                // Get thread data
                if let Some(thread) = self.threads.get(&entry.thread_pubkey) {
                    ready_threads.push((entry.thread_pubkey, thread.clone()));
                }
            }
        }

        // Process ready threads
        if ready_threads.is_empty() {
            debug!("QUEUE: No ready threads found in queue");
        } else {
            info!(
                "QUEUE: Found {} ready threads to process",
                ready_threads.len()
            );
        }

        for (thread_pubkey, thread) in ready_threads {
            info!(
                "QUEUE: Found ready thread {} (trigger: {:?})",
                thread_pubkey, thread.trigger
            );

            // Spawn task with retry loop
            let task = self.spawn_processing_task(thread_pubkey, thread, thread_executor.clone());

            // Track the task for potential cancellation
            self.active_tasks.insert(thread_pubkey, task);
        }
    }

    /// Process all threads whose trigger conditions are met
    /// Spawns async tasks to handle each ready thread atomically
    pub async fn process_threads<F, Fut>(
        &self,
        current_slot: u64,
        current_epoch: u64,
        current_timestamp: i64,
        thread_executor: F,
    ) where
        F: Fn(Pubkey, Thread) -> Fut + Send + Sync + 'static + Clone,
        Fut: std::future::Future<Output = Result<Signature>> + Send + 'static,
    {
        info!(
            "QUEUE: Processing threads for timestamp: {}, slot: {}, epoch: {}",
            current_timestamp, current_slot, current_epoch
        );

        // Process threads scheduled for current or past time
        let current_timestamp_u64 = current_timestamp.max(0) as u64;
        info!(
            "QUEUE: Checking scheduled_by_time queue for threads <= {}",
            current_timestamp_u64
        );
        self.process_queue_with_version(
            &self.scheduled_by_time,
            current_timestamp_u64,
            thread_executor.clone(),
        )
        .await;

        // Process threads scheduled for current or past slot
        info!(
            "QUEUE: Checking scheduled_by_slot queue for threads <= {}",
            current_slot
        );
        self.process_queue_with_version(
            &self.scheduled_by_slot,
            current_slot,
            thread_executor.clone(),
        )
        .await;

        // Process threads scheduled for current or past epoch
        info!(
            "QUEUE: Checking scheduled_by_epoch queue for threads <= {}",
            current_epoch
        );
        self.process_queue_with_version(
            &self.scheduled_by_epoch,
            current_epoch,
            thread_executor.clone(),
        )
        .await;
    }
}
