# Executor Service

The Executor Service is responsible for executing ready threads received from the Observer Service. It manages transaction submission, retry logic, and failure handling to ensure reliable thread execution on Solana.

## Architecture Overview

The Executor Service implements a robust execution pipeline that:
1. **Receives** thread execution requests from Observer
2. **Queues** tasks with persistent storage using Sled database
3. **Executes** transactions via RPC/TPU with retry logic
4. **Handles** failures with exponential backoff and dead letter queue

## Core Components

### Queue System
The service uses Sled embedded database for persistent task queuing across five trees:

```rust
pub struct Queue {
    db: Arc<sled::Db>,
    scheduled: Arc<sled::Tree>,      // Tasks scheduled for future execution
    processing: Arc<sled::Tree>,     // Currently executing tasks  
    dead_letter: Arc<sled::Tree>,    // Failed tasks after max retries
    metadata: Arc<sled::Tree>,       // Task metadata and metrics
    config_tree: Arc<sled::Tree>,    // Queue configuration
    config: RetryConfig,
}
```

#### Task States
Tasks flow through different states in the queue system:
- **Scheduled**: Waiting for execution time
- **Processing**: Currently being executed
- **Completed**: Successfully executed (removed)
- **Dead Letter**: Failed after maximum retries

### Execution Tasks
Each execution request is represented as a task:

```rust
pub struct ExecutionTask {
    pub id: String,
    pub thread_pubkey: Pubkey,
    pub thread: Thread,
    pub trigger_time: i64,
    pub scheduled_time: i64,
    pub retry_count: u32,
    pub last_error: Option<String>,
    pub created_at: i64,
}
```

### Retry Configuration
Configurable retry behavior with exponential backoff:

```rust
pub struct RetryConfig {
    pub max_retries: u32,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    pub backoff_multiplier: f64,
    pub jitter_factor: f64,
}
```

## Transaction Submission

### Submission Methods
The service supports multiple transaction submission methods:

#### RPC Submission
Traditional RPC-based transaction submission:
```rust
impl TransactionSubmitter for RpcSubmitter {
    async fn submit_transaction(
        &self,
        transaction: &Transaction,
    ) -> Result<Signature>;
}
```

#### TPU Submission (Placeholder)
Direct TPU submission for lower latency (to be implemented):
```rust
impl TransactionSubmitter for TpuSubmitter {
    async fn submit_transaction(
        &self,
        transaction: &Transaction,
    ) -> Result<Signature>;
}
```

### Transaction Building
The service constructs thread execution transactions:

1. **Load Thread State**: Fetch current thread and fiber accounts
2. **Build Instruction**: Create `exec_thread` instruction with proper accounts
3. **Add Compute Budget**: Set compute units for complex transactions
4. **Sign Transaction**: Use executor keypair for signing
5. **Set Recent Blockhash**: Ensure transaction validity

## Retry Logic

### Exponential Backoff
Failed executions are retried with increasing delays:

```rust
fn calculate_retry_delay(&self, retry_count: u32) -> Duration {
    let base_delay = self.config.initial_delay_ms as f64;
    let multiplier = self.config.backoff_multiplier;
    let max_delay = self.config.max_delay_ms as f64;
    
    let delay = base_delay * multiplier.powi(retry_count as i32);
    let bounded_delay = delay.min(max_delay);
    
    // Add jitter to prevent thundering herd
    let jitter = bounded_delay * self.config.jitter_factor;
    let final_delay = bounded_delay + (rand::random::<f64>() * jitter);
    
    Duration::from_millis(final_delay as u64)
}
```

### Failure Classification
Different failure types receive different retry treatment:

- **Transient Failures**: Network errors, RPC timeouts → Retry
- **Permanent Failures**: Invalid signatures, account errors → Dead letter
- **Trigger Failures**: Trigger not ready → Reschedule for later
- **Resource Failures**: Insufficient compute units → Increase and retry

### Dead Letter Queue
Tasks that exceed maximum retries are moved to dead letter queue for analysis:

```rust
pub async fn move_to_dead_letter(&self, task: ExecutionTask) -> Result<()> {
    let dead_task = DeadLetterTask {
        original_task: task,
        failed_at: Utc::now().timestamp(),
        final_error: "Max retries exceeded".to_string(),
    };
    
    self.dead_letter.insert(
        dead_task.id.as_bytes(),
        dead_task.serialize()?
    )?;
    
    Ok(())
}
```

## Service Configuration

### Executor Service Creation
```rust
// Create with observer integration
let executor = ExecutorService::new_with_observer(
    rpc_client,
    keypair,
    observer_rx,           // Receives ExecutorEvents
    tpu_config,           // Optional TPU configuration  
    data_dir,             // Sled database directory
    forgo_commission,     // Whether to forgo execution fees
).await?;

// Create standalone
let executor = ExecutorService::new(
    rpc_client,
    keypair,
    data_dir,
    forgo_commission,
).await?;
```

### Runtime Configuration
```rust
let config = RetryConfig {
    max_retries: 5,
    initial_delay_ms: 1000,        // Start with 1 second
    max_delay_ms: 300_000,         // Cap at 5 minutes
    backoff_multiplier: 2.0,       // Double delay each retry
    jitter_factor: 0.1,            // 10% jitter
};
```

## Event Processing

### Observer Integration
The service processes events from the Observer Service:

```rust
pub enum ExecutorEvent {
    ThreadReady {
        thread_pubkey: Pubkey,
        thread: Thread,
        trigger_time: i64,
    },
    ClockUpdate {
        slot: u64,
        epoch: u64,
        unix_timestamp: i64,
    },
}
```

### Event Handling
```rust
match event {
    ExecutorEvent::ThreadReady { thread_pubkey, thread, trigger_time } => {
        let task = ExecutionTask {
            id: generate_task_id(),
            thread_pubkey,
            thread,
            trigger_time,
            scheduled_time: Utc::now().timestamp(),
            retry_count: 0,
            last_error: None,
            created_at: Utc::now().timestamp(),
        };
        
        self.queue.schedule_task(task).await?;
    }
    ExecutorEvent::ClockUpdate { .. } => {
        // Process scheduled tasks that are now ready
        self.process_ready_tasks().await?;
    }
}
```

## Task Processing Pipeline

### 1. Task Scheduling
- Receive thread execution requests
- Schedule for immediate or future execution
- Persist to scheduled tree in database

### 2. Task Processing
- Poll for ready tasks based on current time
- Move tasks to processing tree
- Attempt transaction execution

### 3. Result Handling
- **Success**: Remove from processing tree, update metrics
- **Retriable Failure**: Schedule retry with backoff delay
- **Permanent Failure**: Move to dead letter queue

### 4. Cleanup
- Remove completed tasks from processing tree
- Archive metrics for monitoring
- Clean up expired dead letter entries

## Error Handling

### Network Errors
- **RPC Timeouts**: Retry with exponential backoff
- **Connection Failures**: Attempt reconnection
- **Rate Limiting**: Respect RPC provider limits

### Transaction Errors
- **Insufficient Funds**: Log error, move to dead letter
- **Invalid Signatures**: Rebuild transaction and retry
- **Blockhash Expired**: Refresh blockhash and retry
- **Program Errors**: Analyze error, decide retry strategy

### Queue Errors
- **Database Corruption**: Attempt recovery, log errors
- **Disk Full**: Implement cleanup strategies
- **Serialization Errors**: Handle data format changes

## Performance Optimizations

### Concurrent Execution
- **Parallel Processing**: Multiple worker threads process tasks concurrently
- **Batch Operations**: Group database operations for efficiency
- **Connection Pooling**: Reuse RPC connections

### Memory Management
- **Streaming Processing**: Process tasks in batches to limit memory usage
- **Cache Optimization**: Cache frequently accessed data
- **Garbage Collection**: Regular cleanup of completed/expired tasks

### Database Optimization
- **Efficient Indexing**: Use appropriate keys for fast lookups
- **Compaction**: Regular database compaction for performance
- **Backup Strategy**: Periodic snapshots for disaster recovery

## Monitoring and Metrics

### Execution Metrics
- **Success Rate**: Percentage of successful executions
- **Average Latency**: Time from scheduling to completion
- **Retry Rate**: Percentage of tasks requiring retries
- **Dead Letter Rate**: Percentage of tasks failing permanently

### Queue Metrics
- **Queue Depth**: Number of scheduled tasks
- **Processing Time**: Time spent in processing state
- **Database Size**: Storage usage by tree
- **Throughput**: Tasks processed per unit time

### System Health
- **RPC Connection Status**: Monitor RPC endpoint health
- **Database Health**: Monitor Sled database operations
- **Memory Usage**: Track service memory consumption
- **Error Rates**: Monitor different error types

## Integration Examples

### Standalone Executor
```rust
// Create and start executor service
let mut executor = ExecutorService::new(
    rpc_client,
    keypair, 
    Some("/tmp/executor_data".to_string()),
    false // Don't forgo commission
).await?;

// Run the service
tokio::spawn(async move {
    if let Err(e) = executor.run().await {
        eprintln!("Executor service error: {}", e);
    }
});
```

### Observer Integration
```rust
// Create services
let (observer, executor_rx) = ObserverService::new(/*...*/);
let executor = ExecutorService::new_with_observer(
    rpc_client,
    keypair,
    executor_rx,
    None,
    Some("/tmp/executor_data".to_string()),
    false
).await?;

// Start both services
tokio::spawn(async move { observer.run().await });
tokio::spawn(async move { executor.run().await });
```

The Executor Service provides reliable, persistent thread execution with comprehensive retry logic and failure handling, ensuring thread automation continues to work even through network failures and system restarts.