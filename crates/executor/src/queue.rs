use anyhow::Result;
use log::{error, info};
use serde::{Deserialize, Serialize};
use solana_program::pubkey::Pubkey;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use antegen_thread_program::state::{Thread, Trigger};

#[derive(Debug, Clone)]
pub struct ExecutionTask {
    pub thread_pubkey: Pubkey,
    pub thread: Thread,
}

impl ExecutionTask {
    pub fn serialize(&self) -> Result<Vec<u8>> {
        let thread_bytes = anchor_lang::AnchorSerialize::try_to_vec(&self.thread)?;
        let mut result = Vec::new();
        result.extend_from_slice(&self.thread_pubkey.to_bytes());
        result.extend_from_slice(&(thread_bytes.len() as u32).to_le_bytes());
        result.extend_from_slice(&thread_bytes);
        Ok(result)
    }

    pub fn deserialize(data: &[u8]) -> Result<Self> {
        if data.len() < 36 {
            return Err(anyhow::anyhow!("Invalid data length"));
        }

        let thread_pubkey = Pubkey::try_from(&data[0..32])?;
        let thread_len = u32::from_le_bytes(data[32..36].try_into()?) as usize;

        if data.len() < 36 + thread_len {
            return Err(anyhow::anyhow!("Invalid thread data length"));
        }

        let thread = anchor_lang::AnchorDeserialize::deserialize(&mut &data[36..36 + thread_len])?;

        Ok(Self {
            thread_pubkey,
            thread,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskMetadata {
    pub attempts: u32,
    pub last_error: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl Default for TaskMetadata {
    fn default() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            attempts: 0,
            last_error: None,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    pub backoff_multiplier: f64,
    pub max_attempts: u32,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            initial_delay_ms: 100,
            max_delay_ms: 300_000, // 5 minutes
            backoff_multiplier: 2.0,
            max_attempts: 10,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TaskResult {
    Success,
    Retry,
    Failed,
}

#[derive(Debug, Clone)]
pub struct QueueStats {
    pub scheduled: usize,
    pub processing: usize,
    pub dead_letter: usize,
}

pub struct Queue {
    db: Arc<sled::Db>,
    scheduled: Arc<sled::Tree>,
    processing: Arc<sled::Tree>,
    dead_letter: Arc<sled::Tree>,
    metadata: Arc<sled::Tree>,
    config_tree: Arc<sled::Tree>,
    config: RetryConfig,
}

impl Queue {
    pub fn new(data_dir: impl AsRef<Path>) -> Result<Self> {
        let db_path = data_dir.as_ref().join("executor_queue");
        let db = sled::open(&db_path)?;

        let scheduled = db.open_tree("scheduled")?;
        let processing = db.open_tree("processing")?;
        let dead_letter = db.open_tree("dead_letter")?;
        let metadata = db.open_tree("metadata")?;
        let config_tree = db.open_tree("config")?;

        let mut queue = Self {
            db: Arc::new(db),
            scheduled: Arc::new(scheduled),
            processing: Arc::new(processing),
            dead_letter: Arc::new(dead_letter),
            metadata: Arc::new(metadata),
            config_tree: Arc::new(config_tree),
            config: RetryConfig::default(),
        };

        queue.config = queue.get_config()?;
        Ok(queue)
    }

    pub fn with_config(data_dir: impl AsRef<Path>, config: RetryConfig) -> Result<Self> {
        let mut queue = Self::new(data_dir)?;
        queue.set_config(config)?;
        queue.config = queue.get_config()?;
        Ok(queue)
    }

    pub fn queue_task(&self, thread_pubkey: Pubkey, thread: Thread) -> Result<()> {
        let task = ExecutionTask {
            thread_pubkey,
            thread: thread.clone(),
        };

        let schedule_time = match thread.trigger {
            Trigger::Now => SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            Trigger::Timestamp { unix_ts } => (unix_ts as u64) * 1000,
            Trigger::Interval { .. } | Trigger::Cron { .. } => {
                if let antegen_thread_program::state::TriggerContext::Timestamp { next, .. } =
                    thread.trigger_context
                {
                    (next as u64) * 1000
                } else {
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64
                }
            }
            _ => SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        };

        self.schedule_task(task, schedule_time)?;
        Ok(())
    }

    fn schedule_task(&self, task: ExecutionTask, schedule_time: u64) -> Result<()> {
        let key = format!("{:020}_{}", schedule_time, task.thread_pubkey);
        let value = task.serialize()?;
        self.scheduled.insert(key.as_bytes(), value)?;

        let metadata = TaskMetadata::default();
        let metadata_key = task.thread_pubkey.to_bytes();
        let metadata_value = bincode::serialize(&metadata)?;
        self.metadata.insert(metadata_key, metadata_value)?;

        Ok(())
    }

    pub fn get_ready_tasks(
        &self,
        current_time: u64,
        _current_slot: u64,
        _current_epoch: u64,
    ) -> Result<Vec<ExecutionTask>> {
        let mut tasks = Vec::new();
        let time_key = format!("{:020}", current_time);

        for item in self.scheduled.range(..=time_key.as_bytes()) {
            let (key, value) = item?;

            if let Ok(key_str) = std::str::from_utf8(&key) {
                if !key_str.starts_with("slot_") && !key_str.starts_with("epoch_") {
                    let task = ExecutionTask::deserialize(&value)?;
                    tasks.push(task);
                }
            }
        }

        Ok(tasks)
    }

    pub fn move_to_processing(&self, task: &ExecutionTask) -> Result<()> {
        let key = task.thread_pubkey.to_bytes();
        let value = task.serialize()?;

        // Remove from scheduled
        for item in self.scheduled.scan_prefix(b"") {
            if let Ok((k, _)) = item {
                if let Ok(key_str) = std::str::from_utf8(&k) {
                    if key_str.ends_with(&task.thread_pubkey.to_string()) {
                        self.scheduled.remove(&k)?;
                        break;
                    }
                }
            }
        }

        self.processing.insert(&key, value)?;

        if let Some(metadata_bytes) = self.metadata.get(&key)? {
            let mut metadata: TaskMetadata = bincode::deserialize(&metadata_bytes)?;
            metadata.attempts += 1;
            metadata.updated_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let metadata_value = bincode::serialize(&metadata)?;
            self.metadata.insert(key, metadata_value)?;
        }

        Ok(())
    }

    pub fn handle_task_result(
        &self,
        thread_pubkey: &Pubkey,
        result: TaskResult,
        error: Option<String>,
    ) -> Result<()> {
        match result {
            TaskResult::Success => {
                info!("EXECUTOR: Task {} completed successfully", thread_pubkey);
                self.complete_task(thread_pubkey)?;
            }
            TaskResult::Failed => {
                error!("EXECUTOR: Task {} failed permanently", thread_pubkey);
                if let Some(task_bytes) = self.processing.get(thread_pubkey.to_bytes())? {
                    let task = ExecutionTask::deserialize(&task_bytes)?;
                    self.move_to_dead_letter(&task, error)?;
                }
            }
            TaskResult::Retry => {
                if let Some(task_bytes) = self.processing.get(thread_pubkey.to_bytes())? {
                    let task = ExecutionTask::deserialize(&task_bytes)?;

                    let metadata_key = thread_pubkey.to_bytes();
                    let attempts = if let Some(metadata_bytes) = self.metadata.get(&metadata_key)? {
                        let metadata: TaskMetadata = bincode::deserialize(&metadata_bytes)?;
                        metadata.attempts
                    } else {
                        0
                    };

                    let delay_ms = self.calculate_retry_delay(attempts);
                    info!(
                        "EXECUTOR: Task {} will retry in {}ms (attempt {})",
                        thread_pubkey, delay_ms, attempts
                    );
                    self.retry_task(&task, delay_ms, error)?;
                }
            }
        }

        self.flush()?;
        Ok(())
    }

    fn complete_task(&self, thread_pubkey: &Pubkey) -> Result<()> {
        let key = thread_pubkey.to_bytes();
        self.processing.remove(&key)?;
        self.metadata.remove(&key)?;
        Ok(())
    }

    fn retry_task(&self, task: &ExecutionTask, delay_ms: u64, error: Option<String>) -> Result<()> {
        let key = task.thread_pubkey.to_bytes();

        if let Some(metadata_bytes) = self.metadata.get(&key)? {
            let mut metadata: TaskMetadata = bincode::deserialize(&metadata_bytes)?;

            if metadata.attempts >= self.config.max_attempts {
                return self.move_to_dead_letter(task, error);
            }

            metadata.last_error = error;
            metadata.updated_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let metadata_value = bincode::serialize(&metadata)?;
            self.metadata.insert(key.clone(), metadata_value)?;
        }

        self.processing.remove(&key)?;

        let retry_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
            + delay_ms;

        self.schedule_task(task.clone(), retry_time)?;
        Ok(())
    }

    fn move_to_dead_letter(&self, task: &ExecutionTask, error: Option<String>) -> Result<()> {
        let key = task.thread_pubkey.to_bytes();
        let value = task.serialize()?;

        self.processing.remove(&key)?;
        self.dead_letter.insert(key.clone(), value)?;

        if let Some(metadata_bytes) = self.metadata.get(&key)? {
            let mut metadata: TaskMetadata = bincode::deserialize(&metadata_bytes)?;
            metadata.last_error = error;
            metadata.updated_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let metadata_value = bincode::serialize(&metadata)?;
            self.metadata.insert(key, metadata_value)?;
        }

        Ok(())
    }

    fn calculate_retry_delay(&self, attempts: u32) -> u64 {
        let delay = (self.config.initial_delay_ms as f64
            * self.config.backoff_multiplier.powi(attempts as i32)) as u64;
        delay.min(self.config.max_delay_ms)
    }

    pub fn get_processing_tasks(&self) -> Result<Vec<ExecutionTask>> {
        let mut tasks = Vec::new();
        for item in self.processing.iter() {
            let (_, value) = item?;
            let task = ExecutionTask::deserialize(&value)?;
            tasks.push(task);
        }
        Ok(tasks)
    }

    pub fn get_dead_letter_tasks(&self) -> Result<Vec<(ExecutionTask, TaskMetadata)>> {
        let mut tasks = Vec::new();
        for item in self.dead_letter.iter() {
            let (key, value) = item?;
            let task = ExecutionTask::deserialize(&value)?;

            let metadata = if let Some(metadata_bytes) = self.metadata.get(&key)? {
                bincode::deserialize(&metadata_bytes)?
            } else {
                TaskMetadata::default()
            };

            tasks.push((task, metadata));
        }
        Ok(tasks)
    }

    pub fn resurrect_from_dead_letter(&self, thread_pubkey: &Pubkey) -> Result<()> {
        let key = thread_pubkey.to_bytes();

        if let Some(value) = self.dead_letter.get(&key)? {
            let task = ExecutionTask::deserialize(&value)?;
            self.dead_letter.remove(&key)?;

            let mut metadata = if let Some(metadata_bytes) = self.metadata.get(&key)? {
                bincode::deserialize::<TaskMetadata>(&metadata_bytes)?
            } else {
                TaskMetadata::default()
            };
            metadata.attempts = 0;
            metadata.last_error = None;
            metadata.updated_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let metadata_value = bincode::serialize(&metadata)?;
            self.metadata.insert(key, metadata_value)?;

            let schedule_time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            self.schedule_task(task, schedule_time)?;
        }

        Ok(())
    }

    fn set_config(&self, config: RetryConfig) -> Result<()> {
        let value = bincode::serialize(&config)?;
        self.config_tree.insert(b"retry_config", value)?;
        Ok(())
    }

    fn get_config(&self) -> Result<RetryConfig> {
        if let Some(value) = self.config_tree.get(b"retry_config")? {
            Ok(bincode::deserialize(&value)?)
        } else {
            let default_config = RetryConfig::default();
            self.set_config(default_config.clone())?;
            Ok(default_config)
        }
    }

    pub fn get_stats(&self) -> Result<QueueStats> {
        let scheduled_count = self.scheduled.len();
        let processing_count = self.processing.len();
        let dead_letter_count = self.dead_letter.len();

        Ok(QueueStats {
            scheduled: scheduled_count,
            processing: processing_count,
            dead_letter: dead_letter_count,
        })
    }

    pub fn flush(&self) -> Result<()> {
        self.db.flush()?;
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        for item in self.scheduled.iter() {
            if let Ok((key, _)) = item {
                self.scheduled.remove(key)?;
            }
        }
        for item in self.processing.iter() {
            if let Ok((key, _)) = item {
                self.processing.remove(key)?;
            }
        }
        for item in self.dead_letter.iter() {
            if let Ok((key, _)) = item {
                self.dead_letter.remove(key)?;
            }
        }
        for item in self.metadata.iter() {
            if let Ok((key, _)) = item {
                self.metadata.remove(key)?;
            }
        }
        self.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_queue_and_process() {
        let temp_dir = TempDir::new().unwrap();
        let queue = Queue::new(temp_dir.path()).unwrap();

        let thread = Thread {
            version: 1,
            bump: 0,
            authority: Pubkey::new_unique(),
            id: vec![1, 2, 3],
            name: "test".to_string(),
            created_at: 123456,
            paused: false,
            fibers: vec![],
            exec_index: 0,
            trigger: Trigger::Now,
            trigger_context: antegen_thread_program::state::TriggerContext::Timestamp {
                prev: 0,
                next: 123456,
            },
            nonce_account: Pubkey::default(),
            last_nonce: String::new(),
        };

        let thread_pubkey = Pubkey::new_unique();
        queue.queue_task(thread_pubkey, thread).unwrap();

        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
            + 1000;

        let ready_tasks = queue.get_ready_tasks(current_time, 0, 0).unwrap();
        assert_eq!(ready_tasks.len(), 1);

        let task = &ready_tasks[0];
        queue.move_to_processing(task).unwrap();

        let processing_tasks = queue.get_processing_tasks().unwrap();
        assert_eq!(processing_tasks.len(), 1);

        queue
            .handle_task_result(&thread_pubkey, TaskResult::Success, None)
            .unwrap();

        let processing_tasks = queue.get_processing_tasks().unwrap();
        assert_eq!(processing_tasks.len(), 0);
    }

    #[tokio::test]
    async fn test_retry_logic() {
        let temp_dir = TempDir::new().unwrap();
        let config = RetryConfig {
            initial_delay_ms: 10,
            max_delay_ms: 100,
            backoff_multiplier: 2.0,
            max_attempts: 3,
        };
        let queue = Queue::with_config(temp_dir.path(), config).unwrap();

        let thread = Thread {
            version: 1,
            bump: 0,
            authority: Pubkey::new_unique(),
            id: vec![1, 2, 3],
            name: "test".to_string(),
            created_at: 123456,
            paused: false,
            fibers: vec![],
            exec_index: 0,
            trigger: Trigger::Now,
            trigger_context: antegen_thread_program::state::TriggerContext::Timestamp {
                prev: 0,
                next: 123456,
            },
            nonce_account: Pubkey::default(),
            last_nonce: String::new(),
        };

        let thread_pubkey = Pubkey::new_unique();
        queue.queue_task(thread_pubkey, thread).unwrap();

        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
            + 1000;

        let ready_tasks = queue.get_ready_tasks(current_time, 0, 0).unwrap();
        let task = &ready_tasks[0];
        queue.move_to_processing(task).unwrap();

        queue
            .handle_task_result(&thread_pubkey, TaskResult::Retry, None)
            .unwrap();

        let stats = queue.get_stats().unwrap();
        assert_eq!(stats.scheduled, 1);
        assert_eq!(stats.processing, 0);
    }
}
