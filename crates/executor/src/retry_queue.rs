use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

/// Generic retry queue using HashMap for simplicity and efficiency
pub struct RetryQueue<K, T> 
where 
    K: Hash + Eq + Clone,
    T: Clone,
{
    /// Primary storage for tasks
    tasks: HashMap<K, QueuedTask<T>>,
    /// Track minimum retry time to avoid unnecessary iterations
    next_retry_time: Option<Instant>,
    /// Retry configuration
    config: RetryConfig,
}

/// A task with retry metadata
#[derive(Clone)]
pub struct QueuedTask<T> {
    pub task: T,
    pub attempts: u32,
    pub next_retry: Instant,
    pub delay_ms: u64,
}

/// Configuration for retry behavior
#[derive(Clone)]
pub struct RetryConfig {
    pub initial_delay_ms: u64,      // Starting delay between retries
    pub max_delay_ms: u64,          // Maximum delay between retries
    pub backoff_multiplier: f64,    // Multiplier for exponential backoff
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            initial_delay_ms: 100,
            max_delay_ms: 300_000, // 5 minutes
            backoff_multiplier: 2.0,
        }
    }
}

/// Result of processing a task
#[derive(Debug, Clone, PartialEq)]
pub enum TaskResult {
    /// Task completed successfully - remove from queue
    Success,
    /// Task failed but should retry - apply backoff
    Retry,
    /// Task failed permanently - remove from queue
    Failed,
}

impl<K, T> RetryQueue<K, T>
where
    K: Hash + Eq + Clone,
    T: Clone,
{
    /// Create a new retry queue with default configuration
    pub fn new() -> Self {
        Self::with_config(RetryConfig::default())
    }

    /// Create a new retry queue with custom configuration
    pub fn with_config(config: RetryConfig) -> Self {
        Self {
            tasks: HashMap::new(),
            next_retry_time: None,
            config,
        }
    }

    /// Queue or replace a task
    pub fn queue_task(&mut self, key: K, task: T) -> bool {
        let was_replaced = self.tasks.contains_key(&key);
        
        let queued_task = QueuedTask {
            task,
            attempts: 0,
            next_retry: Instant::now(),
            delay_ms: self.config.initial_delay_ms,
        };
        
        self.tasks.insert(key, queued_task);
        self.update_next_retry_time();
        
        was_replaced
    }

    /// Queue a task with custom initial state
    pub fn queue_task_with_state(&mut self, key: K, task: T, attempts: u32, delay_ms: u64) -> bool {
        let was_replaced = self.tasks.contains_key(&key);
        
        let queued_task = QueuedTask {
            task,
            attempts,
            next_retry: Instant::now(),
            delay_ms,
        };
        
        self.tasks.insert(key, queued_task);
        self.update_next_retry_time();
        
        was_replaced
    }

    /// Get all keys of tasks ready for processing
    pub fn get_ready_keys(&self) -> Vec<K> {
        let now = Instant::now();
        
        // Quick check optimization
        if let Some(next) = self.next_retry_time {
            if now < next {
                return vec![];
            }
        }
        
        // Collect ready task keys
        self.tasks
            .iter()
            .filter(|(_, task)| now >= task.next_retry)
            .map(|(k, _)| k.clone())
            .collect()
    }
    
    /// Get all keys in the queue (regardless of readiness)
    pub fn get_all_keys(&self) -> Vec<K> {
        self.tasks.keys().cloned().collect()
    }
    
    /// Process a ready task with a closure
    pub fn process_task<F>(&mut self, key: &K, mut processor: F) -> Option<TaskResult>
    where
        F: FnMut(&mut QueuedTask<T>) -> TaskResult,
    {
        if let Some(task) = self.tasks.get_mut(key) {
            let result = processor(task);
            self.handle_task_result(key, result.clone());
            Some(result)
        } else {
            None
        }
    }

    /// Handle task result and update retry state
    pub fn handle_task_result(&mut self, key: &K, result: TaskResult) {
        match result {
            TaskResult::Success | TaskResult::Failed => {
                self.tasks.remove(key);
            }
            TaskResult::Retry => {
                if let Some(task) = self.tasks.get_mut(key) {
                    task.attempts += 1;
                    // Exponential backoff
                    task.delay_ms = ((task.delay_ms as f64 * self.config.backoff_multiplier) as u64)
                        .min(self.config.max_delay_ms);
                    task.next_retry = Instant::now() + Duration::from_millis(task.delay_ms);
                }
            }
        }
        
        self.update_next_retry_time();
    }

    /// Remove a task from the queue
    pub fn remove(&mut self, key: &K) -> Option<QueuedTask<T>> {
        let task = self.tasks.remove(key);
        self.update_next_retry_time();
        task
    }

    /// Check if a task exists in the queue
    pub fn contains(&self, key: &K) -> bool {
        self.tasks.contains_key(key)
    }

    /// Get the number of tasks in the queue
    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    /// Check if the queue is empty
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Get queue statistics
    pub fn stats(&self) -> QueueStats {
        let now = Instant::now();
        let ready_count = self.tasks
            .values()
            .filter(|task| now >= task.next_retry)
            .count();
        
        let max_attempts = self.tasks
            .values()
            .map(|task| task.attempts)
            .max()
            .unwrap_or(0);
        
        QueueStats {
            total_tasks: self.tasks.len(),
            ready_tasks: ready_count,
            max_attempts,
            next_retry_time: self.next_retry_time,
        }
    }

    /// Update the next retry time hint
    fn update_next_retry_time(&mut self) {
        self.next_retry_time = self.tasks
            .values()
            .map(|task| task.next_retry)
            .min();
    }

    /// Clear all tasks from the queue
    pub fn clear(&mut self) {
        self.tasks.clear();
        self.next_retry_time = None;
    }

    /// Get a reference to a task
    pub fn get(&self, key: &K) -> Option<&QueuedTask<T>> {
        self.tasks.get(key)
    }

    /// Get a mutable reference to a task
    pub fn get_mut(&mut self, key: &K) -> Option<&mut QueuedTask<T>> {
        self.tasks.get_mut(key)
    }
}

/// Statistics about the queue
#[derive(Debug, Clone)]
pub struct QueueStats {
    pub total_tasks: usize,
    pub ready_tasks: usize,
    pub max_attempts: u32,
    pub next_retry_time: Option<Instant>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn test_queue_and_retrieve() {
        let mut queue: RetryQueue<String, String> = RetryQueue::new();
        
        assert!(queue.is_empty());
        
        let was_replaced = queue.queue_task("task1".to_string(), "data1".to_string());
        assert!(!was_replaced);
        assert_eq!(queue.len(), 1);
        
        let ready_keys = queue.get_ready_keys();
        assert_eq!(ready_keys.len(), 1);
        
        let task = queue.get(&"task1".to_string()).unwrap();
        assert_eq!(task.task, "data1");
    }

    #[test]
    fn test_replacement() {
        let mut queue: RetryQueue<String, String> = RetryQueue::new();
        
        queue.queue_task("task1".to_string(), "data1".to_string());
        let was_replaced = queue.queue_task("task1".to_string(), "data2".to_string());
        
        assert!(was_replaced);
        assert_eq!(queue.len(), 1);
        
        let task = queue.get(&"task1".to_string()).unwrap();
        assert_eq!(task.task, "data2");
        assert_eq!(task.attempts, 0); // Reset on replacement
    }

    #[test]
    fn test_retry_backoff() {
        let config = RetryConfig {
            initial_delay_ms: 10,
            max_delay_ms: 100,
            backoff_multiplier: 2.0,
        };
        
        let mut queue: RetryQueue<String, String> = RetryQueue::with_config(config);
        
        queue.queue_task("task1".to_string(), "data1".to_string());
        
        // First attempt
        let ready_keys = queue.get_ready_keys();
        assert_eq!(ready_keys.len(), 1);
        queue.handle_task_result(&"task1".to_string(), TaskResult::Retry);
        
        // Should not be ready immediately
        let ready_keys = queue.get_ready_keys();
        assert_eq!(ready_keys.len(), 0);
        
        // Wait for retry delay
        sleep(Duration::from_millis(15));
        let ready_keys = queue.get_ready_keys();
        assert_eq!(ready_keys.len(), 1);
        
        // Check backoff increased
        let task = queue.get(&"task1".to_string()).unwrap();
        assert_eq!(task.attempts, 1);
        assert_eq!(task.delay_ms, 20); // 10 * 2
    }

    #[test]
    fn test_task_removal() {
        let mut queue: RetryQueue<String, String> = RetryQueue::new();
        
        queue.queue_task("task1".to_string(), "data1".to_string());
        queue.queue_task("task2".to_string(), "data2".to_string());
        
        queue.handle_task_result(&"task1".to_string(), TaskResult::Success);
        assert_eq!(queue.len(), 1);
        assert!(!queue.contains(&"task1".to_string()));
        
        queue.handle_task_result(&"task2".to_string(), TaskResult::Failed);
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_stats() {
        let mut queue: RetryQueue<String, String> = RetryQueue::new();
        
        queue.queue_task("task1".to_string(), "data1".to_string());
        queue.queue_task("task2".to_string(), "data2".to_string());
        
        let stats = queue.stats();
        assert_eq!(stats.total_tasks, 2);
        assert_eq!(stats.ready_tasks, 2);
        assert_eq!(stats.max_attempts, 0);
        assert!(stats.next_retry_time.is_some());
    }
}