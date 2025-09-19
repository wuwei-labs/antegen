# Geyser Plugin

The Antegen Geyser Plugin integrates with Solana validators to provide real-time thread monitoring and execution. It captures account updates, clock changes, and other blockchain events directly from the validator, enabling immediate thread execution without polling delays.

## Overview

The plugin implements the Agave Geyser Plugin interface to:
1. **Capture** real-time validator events (account updates, slot changes)
2. **Filter** thread-related events and clock updates
3. **Bridge** validator data to Observer/Executor services
4. **Provide** zero-latency thread execution triggers

## Architecture

```
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│   Validator     │───▶│   Geyser Plugin  │───▶│   Observer      │
│   (Agave)       │    │   (Real-time)    │    │   Service       │
└─────────────────┘    └──────────────────┘    └─────────────────┘
         │                       │                       │
         ▼                       ▼                       ▼
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│ Account Updates │    │  Event Filtering │    │  Thread         │
│ Slot Changes    │    │  Data Processing │    │  Execution      │
│ Clock Updates   │    │  Event Bridging  │    │  Queue          │
└─────────────────┘    └──────────────────┘    └─────────────────┘
```

## Plugin Components

### AntegenPlugin
Main plugin struct implementing the Geyser interface:

```rust
pub struct AntegenPlugin {
    pub inner: Option<Arc<Inner>>,
}

struct Inner {
    pub config: PluginConfig,
    pub worker: Arc<PluginWorker>,
    pub runtime: Arc<Runtime>,
    pub block_height: Arc<AtomicU64>,
}
```

### PluginWorker
Coordinates Observer, Executor, and Submitter services within the plugin:

```rust
pub struct PluginWorker {
    event_sender: Sender<ObservedEvent>,
    observer_service: Option<ObserverService>,
    executor_service: Option<ExecutorService>,
    submitter_service: Option<Arc<SubmitterService>>,
}
```

### Event Processing
Handles different types of validator events:

```rust
pub enum AccountUpdateEvent {
    Clock { clock: Clock },
    Thread { thread: Thread },
}
```

## Configuration

### Plugin Configuration File
```json
{
    "name": "antegen",
    "keypath": "/path/to/executor/keypair.json",
    "libpath": "/path/to/plugin/library.so",
    "thread_count": 10,
    "transaction_timeout_threshold": 150,
    "rpc_url": "http://localhost:8899",
    "ws_url": "ws://localhost:8900",
    "data_dir": "/tmp/antegen_executor",
    "forgo_executor_commission": false,
    "enable_replay": true,
    "nats_url": "nats://localhost:4222",
    "replay_delay_ms": 30000
}
```

### Environment Variable Overrides
The plugin supports environment variable configuration:

- `ANTEGEN_KEYPATH`: Executor keypair path
- `ANTEGEN_RPC_URL`: RPC endpoint URL
- `ANTEGEN_WS_URL`: WebSocket endpoint URL
- `ANTEGEN_DATA_DIR`: Data directory for persistence
- `ANTEGEN_THREAD_COUNT`: Number of worker threads
- `ANTEGEN_TRANSACTION_TIMEOUT_THRESHOLD`: Transaction timeout in seconds
- `ANTEGEN_FORGO_EXECUTOR_COMMISSION`: Boolean flag for commission
- `ANTEGEN_ENABLE_REPLAY`: Enable/disable transaction replay
- `ANTEGEN_NATS_URL`: NATS server connection string
- `ANTEGEN_REPLAY_DELAY_MS`: Delay before replay attempts

### Plugin Configuration Structure
```rust
pub struct PluginConfig {
    pub name: String,
    pub keypath: Option<String>,
    pub libpath: Option<String>,
    pub thread_count: usize,
    pub transaction_timeout_threshold: u64,
    pub rpc_url: Option<String>,
    pub ws_url: Option<String>,
    pub data_dir: Option<String>,
    pub forgo_executor_commission: Option<bool>,
    pub enable_replay: Option<bool>,
    pub nats_url: Option<String>,
    pub replay_delay_ms: Option<u64>,
}
```

## Geyser Interface Implementation

### Account Updates
Monitors all account updates and filters for relevant events:

```rust
fn update_account(
    &self,
    account: ReplicaAccountInfoVersions,
    slot: u64,
    is_startup: bool,
) -> PluginResult<()> {
    // Parse account info from different versions
    let account_info = match account {
        ReplicaAccountInfoVersions::V0_0_1(info) => info,
        ReplicaAccountInfoVersions::V0_0_2(info) => info, 
        ReplicaAccountInfoVersions::V0_0_3(info) => info,
    };
    
    // Filter for thread and clock accounts
    let event = AccountUpdateEvent::try_from(account_info);
    
    // Process relevant events
    match event {
        AccountUpdateEvent::Clock { clock } => {
            self.worker.send_clock_event(clock, slot, block_height).await;
        }
        AccountUpdateEvent::Thread { thread } => {
            self.worker.send_thread_event(thread, account_pubkey, slot).await;
        }
    }
}
```

### Slot Status Updates
Tracks confirmed and finalized slots for block height:

```rust
fn update_slot_status(
    &self,
    slot: u64,
    _parent: Option<u64>,
    status: &SlotStatus,
) -> PluginResult<()> {
    match status {
        SlotStatus::Confirmed | SlotStatus::Rooted => {
            // Update block height counter
            let new_height = self.inner.block_height
                .fetch_add(1, Ordering::SeqCst) + 1;
        }
        _ => {}
    }
}
```

### Event Filtering
The plugin filters events to only process relevant updates:

#### Thread Account Filtering
```rust
// Check if account is owned by thread program
if account_info.owner == THREAD_PROGRAM_ID {
    // Parse as thread account
    let thread = Thread::try_deserialize(&mut &account_info.data[8..])?;
    
    // Skip paused threads
    if !thread.paused {
        return AccountUpdateEvent::Thread { thread };
    }
}
```

#### Clock Account Filtering
```rust
// Check for clock sysvar updates
if account_pubkey == solana_program::sysvar::clock::ID {
    let clock = Clock::try_deserialize(&mut &account_info.data[..])?;
    return AccountUpdateEvent::Clock { clock };
}
```

## Service Integration

### Observer Service Integration
The plugin creates and manages an Observer Service:

```rust
// Create event source that receives from plugin
let event_source = Box::new(GeyserPluginEventSource::new(observed_rx));

// Create observer service
let (observer_service, executor_event_rx) = ObserverService::new(
    event_source,
    observer_pubkey,
    rpc_client.clone()
);
```

### Submitter Service Integration
The plugin creates a shared Submitter Service with TPU and replay capabilities:

```rust
// Create submitter service with replay configuration
let submitter_config = SubmitterConfig {
    enable_replay,
    nats_url,
    replay_delay_ms: replay_delay_ms.unwrap_or(30_000),
    tpu_config: Some(TpuConfig::default()),
    submission_mode: SubmissionMode::TpuWithFallback,
    ..Default::default()
};

let submitter_service = Arc::new(SubmitterService::new(
    rpc_client.clone(),
    submitter_config,
).await?);
```

### Executor Service Integration
The plugin creates and manages an Executor Service with the shared Submitter:

```rust
// Create executor service with observer events and submitter
let executor_service = ExecutorService::new_with_observer(
    rpc_client,
    keypair.clone(),
    executor_event_rx,
    submitter_service.clone(), // Shared submitter service
    data_dir,
    forgo_executor_commission,
).await?;
```

### Event Flow
```
Validator Event → Plugin Filter → Observer Service → Executor Service → Thread Execution
```

## Event Source Implementation

### GeyserPluginEventSource
Custom event source that bridges plugin events to Observer:

```rust
struct GeyserPluginEventSource {
    receiver: Receiver<ObservedEvent>,
    running: bool,
}

#[async_trait]
impl EventSource for GeyserPluginEventSource {
    async fn next_event(&mut self) -> Result<Option<ObservedEvent>> {
        match self.receiver.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                self.running = false;
                Ok(None)
            }
        }
    }
    
    // Other trait methods...
}
```

## Runtime Management

### Tokio Runtime
The plugin creates a multi-threaded runtime for async operations:

```rust
fn build_runtime(config: PluginConfig) -> Arc<Runtime> {
    Arc::new(
        Builder::new_multi_thread()
            .enable_all()
            .thread_name("antegen-plugin")
            .worker_threads(config.thread_count)
            .max_blocking_threads(config.thread_count)
            .build()
            .unwrap(),
    )
}
```

### Service Lifecycle
```rust
// Initialize plugin services
pub fn start(&mut self, runtime: Handle) -> Result<()> {
    // Spawn observer service
    runtime.spawn(async move {
        observer_service.run().await
    });
    
    // Spawn executor service  
    runtime.spawn(async move {
        executor_service.run().await
    });
    
    // Start submitter service (including optional replay consumer)
    runtime.spawn(async move {
        match submitter_service.start_replay_consumer().await {
            Ok(Some(_handle)) => info!("SUBMITTER: Replay consumer started"),
            Ok(None) => info!("SUBMITTER: Replay consumer not enabled"),
            Err(e) => error!("SUBMITTER: Failed to start replay consumer: {}", e),
        }
    });
}
```

## Performance Considerations

### Event Processing
- **Non-blocking Operations**: Uses `try_recv()` to avoid blocking validator
- **Async Processing**: Offloads event processing to async tasks
- **Efficient Filtering**: Early filtering reduces unnecessary processing

### Memory Management
- **Bounded Channels**: Prevents unbounded memory growth
- **Event Batching**: Processes multiple events per cycle
- **Resource Cleanup**: Proper cleanup on plugin unload

### Threading
- **Configurable Workers**: Adjustable thread pool size
- **Thread Isolation**: Separate threads for different operations
- **Load Balancing**: Work distribution across threads

## Monitoring and Logging

### Plugin Logging
```rust
// Plugin initialization logging
info!("antegen-plugin v{} - geyser_interface_version: {}", 
    env!("CARGO_PKG_VERSION"),
    env!("GEYSER_INTERFACE_VERSION")
);

// Event processing logging
info!("GEYSER->OBSERVER: Thread event for {} at slot {}", 
    thread_pubkey, slot);
```

### Service Monitoring
```rust
// Observer service monitoring
info!("OBSERVER: Task started, entering event loop");

// Executor service monitoring  
info!("EXECUTOR: Task started, entering event loop");
```

## Installation and Deployment

### Building the Plugin
```bash
# Build the plugin library
cargo build --release --package plugin

# Plugin library location
target/release/libantegen_plugin.so
```

### Validator Configuration
Add to validator startup command:
```bash
solana-validator \
  --geyser-plugin-config /path/to/plugin-config.json \
  # ... other validator flags
```

### Plugin Configuration File
```json
{
    "libpath": "/path/to/libantegen_plugin.so",
    "keypath": "/path/to/executor-keypair.json",
    "rpc_url": "http://localhost:8899",
    "ws_url": "ws://localhost:8900",
    "data_dir": "/tmp/antegen_data"
}
```

## Error Handling

### Plugin Errors
- **Initialization Failures**: Graceful fallback and error logging
- **Service Failures**: Automatic restart attempts
- **Network Failures**: Connection retry logic

### Event Processing Errors
- **Malformed Data**: Skip invalid events, continue processing
- **Channel Failures**: Detect and handle disconnections
- **Service Unavailable**: Implement backoff strategies

The Geyser Plugin provides the critical real-time data pipeline that enables immediate thread execution in response to blockchain events, eliminating polling delays and ensuring optimal execution timing.