# Submitter Service

The Submitter Service is a high-performance transaction submission layer that handles the delivery of thread execution transactions to the Solana network. It provides optimized submission paths, durable transaction replay capabilities, and fallback mechanisms to ensure reliable execution.

## Architecture Overview

The Submitter Service implements a multi-layered approach to transaction submission:
1. **Optimized Submission**: TPU direct submission with RPC fallback
2. **Durable Replay**: NATS-based transaction replay for reliability
3. **Connection Management**: Efficient connection pooling and caching
4. **Error Handling**: Comprehensive retry logic and failure classification

## Core Components

### SubmitterService
Main service that orchestrates transaction submission and replay:

```rust
pub struct SubmitterService {
    submitter: Arc<TransactionSubmitter>,
    rpc_client: Arc<RpcClient>,
    nats_client: Option<async_nats::Client>,
    config: SubmitterConfig,
    replay_handle: Option<JoinHandle<Result<()>>>,
}
```

### TransactionSubmitter
Core submission engine with multiple delivery paths:

```rust
pub struct TransactionSubmitter {
    rpc_client: Arc<RpcClient>,
    tpu_client: Option<TpuClient>,
    config: TpuConfig,
    connection_cache: Arc<ConnectionCache>,
}
```

### SubmitterConfig
Configuration for submission behavior and replay settings:

```rust
pub struct SubmitterConfig {
    pub tpu_config: Option<TpuConfig>,
    pub submission_mode: SubmissionMode,
    pub enable_replay: bool,
    pub nats_url: Option<String>,
    pub replay_delay_ms: u64,
    pub max_retries: u32,
    pub retry_delay_ms: u64,
}
```

## Submission Modes

### TPU with RPC Fallback (Recommended)
```rust
SubmissionMode::TpuWithFallback
```
- **Primary**: Direct TPU submission for lowest latency
- **Fallback**: RPC submission if TPU fails or unavailable
- **Best For**: Production environments requiring optimal performance

### RPC Only
```rust
SubmissionMode::RpcOnly
```
- **Method**: Traditional RPC-based submission
- **Latency**: Higher but more predictable
- **Best For**: Testing, development, or when TPU is unavailable

### TPU Only
```rust
SubmissionMode::TpuOnly
```
- **Method**: Direct TPU submission without fallback
- **Performance**: Lowest latency when available
- **Best For**: Specialized high-performance scenarios

## TPU Client Configuration

### TpuConfig
```rust
pub struct TpuConfig {
    pub fanout_slots: u64,           // Leader slots to target
    pub max_connections: usize,       // Connection pool size
    pub connection_timeout_ms: u64,   // Connection timeout
    pub confirm_transaction: bool,    // Wait for confirmation
}
```

### Default Configuration
```rust
impl Default for TpuConfig {
    fn default() -> Self {
        Self {
            fanout_slots: 12,
            max_connections: 8,
            connection_timeout_ms: 5000,
            confirm_transaction: true,
        }
    }
}
```

## Durable Transaction Replay

### NATS Integration
The service integrates with NATS messaging for transaction replay:

- **Publish**: Durable transactions are published to NATS
- **Replay**: Consumer re-submits transactions after configured delay
- **Durability**: Ensures critical transactions eventually execute

### DurableTransactionMessage
```rust
pub struct DurableTransactionMessage {
    pub transaction_base64: String,
    pub thread_pubkey: String,
    pub original_signature: String,
    pub executor_pubkey: String,
    pub timestamp: i64,
    pub replay_count: u32,
}
```

### Replay Process
1. **Detection**: Identify durable transactions (with nonce accounts)
2. **Publication**: Publish to NATS subject `antegen.durable_txs`
3. **Delay**: Wait configured replay delay (default 30 seconds)
4. **Re-submission**: Attempt transaction re-submission
5. **Confirmation**: Verify transaction didn't already execute

## Service Integration

### With Executor Service
The Submitter integrates directly with the Executor Service:

```rust
// Create submitter service
let submitter_config = SubmitterConfig {
    enable_replay: true,
    nats_url: Some("nats://localhost:4222".to_string()),
    replay_delay_ms: 30_000,
    tpu_config: Some(TpuConfig::default()),
    submission_mode: SubmissionMode::TpuWithFallback,
    ..Default::default()
};

let submitter_service = Arc::new(
    SubmitterService::new(rpc_client.clone(), submitter_config).await?
);

// Create executor with submitter
let executor = ExecutorService::new_with_observer(
    rpc_client,
    keypair,
    executor_event_rx,
    submitter_service,  // Shared submitter service
    data_dir,
    forgo_commission,
).await?;
```

### With Plugin Worker
In the Geyser Plugin, all three services work together:

```rust
impl PluginWorker {
    pub async fn new(
        rpc_url: String,
        ws_url: String,
        keypair_path: String,
        data_dir: Option<String>,
        forgo_executor_commission: bool,
        enable_replay: bool,
        nats_url: Option<String>,
        replay_delay_ms: Option<u64>,
    ) -> Result<Self> {
        // Create submitter service
        let submitter_service = Arc::new(SubmitterService::new(
            rpc_client.clone(),
            SubmitterConfig {
                enable_replay,
                nats_url,
                replay_delay_ms: replay_delay_ms.unwrap_or(30_000),
                tpu_config: Some(TpuConfig::default()),
                submission_mode: SubmissionMode::TpuWithFallback,
                ..Default::default()
            },
        ).await?);
        
        // Create executor with shared submitter
        let executor_service = ExecutorService::new_with_observer(
            rpc_client,
            keypair,
            executor_event_rx,
            submitter_service.clone(),
            data_dir,
            forgo_executor_commission,
        ).await?;
        
        // Services share the submitter
        Ok(Self {
            observer_service: Some(observer_service),
            executor_service: Some(executor_service),
            submitter_service: Some(submitter_service),
        })
    }
}
```

## Configuration Examples

### Production Configuration
```rust
let config = SubmitterConfig {
    tpu_config: Some(TpuConfig {
        fanout_slots: 16,
        max_connections: 12,
        connection_timeout_ms: 3000,
        confirm_transaction: true,
    }),
    submission_mode: SubmissionMode::TpuWithFallback,
    enable_replay: true,
    nats_url: Some("nats://nats.production.com:4222".to_string()),
    replay_delay_ms: 30_000,  // 30 seconds
    max_retries: 3,
    retry_delay_ms: 1000,
};
```

### Development Configuration
```rust
let config = SubmitterConfig {
    tpu_config: Some(TpuConfig::default()),
    submission_mode: SubmissionMode::RpcOnly,  // Simpler for testing
    enable_replay: false,  // Disable for local testing
    nats_url: None,
    replay_delay_ms: 5_000,  // Shorter delay for testing
    max_retries: 1,
    retry_delay_ms: 500,
};
```

### High-Performance Configuration
```rust
let config = SubmitterConfig {
    tpu_config: Some(TpuConfig {
        fanout_slots: 20,      // More aggressive targeting
        max_connections: 16,    // Higher connection pool
        connection_timeout_ms: 2000,  // Faster timeouts
        confirm_transaction: false,   // Skip confirmation for speed
    }),
    submission_mode: SubmissionMode::TpuOnly,  // TPU-only for max speed
    enable_replay: true,       // Still maintain durability
    nats_url: Some("nats://localhost:4222".to_string()),
    replay_delay_ms: 15_000,   // Faster replay
    max_retries: 5,            // More retries for reliability
    retry_delay_ms: 200,       // Faster retry cycles
};
```

## Environment Variables

The service supports environment-based configuration:

- `ANTEGEN_ENABLE_REPLAY`: Enable/disable transaction replay
- `ANTEGEN_NATS_URL`: NATS server connection string
- `ANTEGEN_REPLAY_DELAY_MS`: Delay before replay attempts
- `ANTEGEN_TPU_FANOUT_SLOTS`: TPU client fanout slots
- `ANTEGEN_MAX_TPU_CONNECTIONS`: Maximum TPU connections

## Performance Optimizations

### Connection Pooling
- **Persistent Connections**: Reuse TPU/RPC connections
- **Connection Caching**: Cache connections to validators
- **Pool Management**: Automatic connection lifecycle management

### Batch Operations
- **Transaction Batching**: Group similar transactions when possible
- **Parallel Submission**: Submit multiple transactions concurrently
- **Load Balancing**: Distribute across available validators

### Retry Logic
- **Intelligent Backoff**: Exponential backoff for transient failures
- **Failure Classification**: Different handling for different error types
- **Circuit Breaker**: Temporarily disable failing submission paths

## Monitoring and Metrics

### Submission Metrics
- **Success Rate**: Percentage of successful submissions
- **Latency Distribution**: P50, P95, P99 submission latencies
- **TPU vs RPC Usage**: Breakdown of submission methods
- **Retry Rate**: Percentage of transactions requiring retries

### Replay Metrics
- **Replay Success Rate**: Percentage of successful replay attempts
- **Replay Delay Distribution**: Actual vs configured replay delays
- **Duplicate Prevention**: Transactions skipped due to existing confirmation
- **NATS Health**: Connection status and message processing rate

### Error Tracking
- **Network Errors**: Connection failures and timeouts
- **Transaction Errors**: Invalid signatures, insufficient funds, etc.
- **Replay Errors**: NATS connection issues, deserialization errors
- **Performance Degradation**: Latency increases, connection pool exhaustion

## Integration Examples

### Standalone Usage
```rust
// Create and configure submitter
let config = SubmitterConfig::default();
let submitter = SubmitterService::new(rpc_client, config).await?;

// Start replay consumer if enabled
submitter.start_replay_consumer().await?;

// Submit transactions
let signature = submitter.submit(&transaction).await?;
println!("Transaction submitted: {}", signature);
```

### With Custom Configuration
```rust
// Create custom config
let config = SubmitterConfig {
    submission_mode: SubmissionMode::TpuWithFallback,
    enable_replay: true,
    nats_url: Some("nats://localhost:4222".to_string()),
    replay_delay_ms: 45_000,
    ..Default::default()
};

// Create service
let mut submitter = SubmitterService::new(rpc_client, config).await?;

// Start all services
submitter.start().await?;

// Use service
let signature = submitter.submit_with_retries(&transaction, 3).await?;
```

## Error Handling

### Common Error Types
- **NetworkError**: Connection failures, timeouts
- **TransactionError**: Invalid transaction, insufficient funds
- **ReplayError**: NATS connection issues, replay failures
- **ConfigurationError**: Invalid TPU config, missing parameters

### Error Recovery
- **Automatic Retry**: Transient errors automatically retried
- **Fallback Paths**: TPU failures fall back to RPC
- **Circuit Breaker**: Failing paths temporarily disabled
- **Graceful Degradation**: Service continues with reduced functionality

The Submitter Service provides the critical infrastructure for reliable, high-performance transaction delivery in the Antegen automation platform, ensuring that thread executions reach the Solana network efficiently and reliably.