# Observer Service

The Observer Service is a critical component of the Antegen automation platform that monitors Solana network events and identifies executable threads. It acts as the intelligence layer between raw blockchain data and thread execution requests.

## Architecture Overview

The Observer Service implements an event-driven architecture that:
1. **Monitors** Solana network for thread-related events
2. **Filters** threads based on trigger readiness
3. **Forwards** executable threads to the Executor Service
4. **Tracks** thread state and trigger contexts

## Core Components

### Event Sources
Event sources provide different mechanisms for monitoring blockchain state:

#### Implemented Sources
- **Geyser Plugin Source**: Real-time events from validator plugins
- **RPC Source**: Polling-based monitoring via RPC endpoints

#### Event Source Trait
```rust
#[async_trait]
pub trait EventSource: Send + Sync {
    async fn start(&mut self) -> Result<()>;
    async fn stop(&mut self) -> Result<()>;
    async fn next_event(&mut self) -> Result<Option<ObservedEvent>>;
    async fn subscribe_thread(&mut self, thread_pubkey: Pubkey) -> Result<()>;
    async fn unsubscribe_thread(&mut self, thread_pubkey: Pubkey) -> Result<()>;
    async fn get_current_slot(&self) -> Result<u64>;
    fn name(&self) -> &str;
}
```

### Event Types
The service processes several types of blockchain events:

```rust
pub enum ObservedEvent {
    ThreadExecutable {
        thread_pubkey: Pubkey,
        thread: Thread,
        slot: u64,
    },
    ClockUpdate {
        slot: u64,
        epoch: u64,
        unix_timestamp: i64,
    },
    AccountUpdate {
        pubkey: Pubkey,
        account: Account,
        slot: u64,
    },
}
```

### Observer Service
The main service coordinates event processing and thread analysis:

```rust
pub struct ObserverService {
    event_source: Box<dyn EventSource>,
    observer_pubkey: Pubkey,
    rpc_client: Arc<RpcClient>,
    executor_tx: Sender<ExecutorEvent>,
    thread_cache: HashMap<Pubkey, CachedThread>,
    // ... additional fields
}
```

## Event Processing Pipeline

### 1. Event Reception
- Receives events from configured event source
- Handles connection failures and reconnection logic
- Maintains event ordering and deduplication

### 2. Thread State Management
- Caches thread account data for efficient trigger evaluation
- Tracks trigger contexts and execution history
- Manages thread subscription lifecycle

### 3. Trigger Evaluation
The service evaluates various trigger types:

#### Timestamp-Based Triggers
- **Now**: Always executable
- **Timestamp**: Ready when current time >= target time
- **Interval**: Ready when time since last execution >= interval
- **Cron**: Ready when cron schedule indicates next execution

#### Blockchain-Based Triggers
- **Slot**: Ready when current slot >= target slot
- **Epoch**: Ready when current epoch >= target epoch
- **Account**: Ready when monitored account data changes

### 4. Execution Forwarding
Ready threads are forwarded to the Executor Service:

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

## Configuration

### Service Creation
```rust
// Create observer with Geyser plugin source
let (service, executor_rx) = ObserverService::new(
    geyser_source,
    observer_pubkey,
    rpc_client
);

// Create observer with RPC polling source
let (service, executor_rx) = ObserverService::new_with_rpc(
    rpc_client,
    observer_pubkey,
    poll_interval
);
```

### Thread Subscription
```rust
// Subscribe to specific thread
observer.subscribe_thread(thread_pubkey).await?;

// Unsubscribe when no longer needed
observer.unsubscribe_thread(thread_pubkey).await?;
```

## Caching Strategy

### Thread Cache
The service maintains an in-memory cache for efficient trigger evaluation:

```rust
struct CachedThread {
    thread: Thread,
    last_updated: i64,
    trigger_ready_time: Option<i64>,
    subscription_active: bool,
}
```

**Benefits:**
- Reduces RPC calls for repeated trigger evaluations
- Enables fast filtering of non-executable threads
- Tracks subscription state for cleanup

### Cache Management
- **TTL-based expiration**: Prevents stale data usage
- **Event-driven updates**: Refreshes on account changes
- **Memory bounds**: Limits cache size to prevent memory leaks

## Error Handling

### Connection Resilience
- **Automatic Reconnection**: Recovers from network failures
- **Exponential Backoff**: Prevents overwhelming failed services
- **Event Source Fallback**: Switches between monitoring methods

### Data Validation
- **Thread State Validation**: Ensures thread accounts are valid
- **Trigger Condition Checking**: Validates trigger logic before forwarding
- **Account Data Parsing**: Handles malformed account data gracefully

## Performance Considerations

### Batching
- **Event Batching**: Processes multiple events per cycle
- **Subscription Batching**: Groups thread subscriptions for efficiency
- **Cache Updates**: Batches cache refreshes

### Filtering
- **Early Filtering**: Eliminates non-executable threads quickly
- **Trigger Pre-evaluation**: Caches trigger readiness calculations
- **Subscription Management**: Only monitors subscribed threads

### Memory Management
- **Bounded Caches**: Prevents unbounded memory growth
- **Periodic Cleanup**: Removes expired cache entries
- **Efficient Data Structures**: Uses appropriate containers for performance

## Integration Patterns

### With Geyser Plugin
```rust
let event_source = Box::new(GeyserPluginEventSource::new(event_rx));
let (observer, executor_rx) = ObserverService::new(
    event_source,
    observer_pubkey,
    rpc_client
);
```

### With RPC Polling
```rust
let (observer, executor_rx) = ObserverService::new_with_rpc(
    rpc_client,
    observer_pubkey,
    Duration::from_secs(1) // Poll interval
);
```

### Standalone Operation
```rust
// Start observer service
let mut observer = ObserverService::new(/*...*/);

// Run event processing loop
tokio::spawn(async move {
    observer.run().await
});

// Consume executor events
while let Some(event) = executor_rx.recv().await {
    match event {
        ExecutorEvent::ThreadReady { thread_pubkey, .. } => {
            println!("Thread {} ready for execution", thread_pubkey);
        }
        _ => {}
    }
}
```

## Monitoring and Observability

### Metrics
The service exposes metrics for monitoring:
- **Events Processed**: Total events received and processed
- **Threads Evaluated**: Number of threads checked for readiness
- **Cache Hit Rate**: Efficiency of thread cache
- **Execution Forwards**: Threads forwarded to executor

### Logging
Comprehensive logging for debugging and monitoring:
- **Event Reception**: Logs incoming events with metadata
- **Trigger Evaluation**: Details trigger condition checking
- **Cache Operations**: Tracks cache hits, misses, and updates
- **Error Conditions**: Detailed error logging with context

The Observer Service provides the critical intelligence layer that enables efficient, accurate thread execution scheduling in the Antegen automation platform.