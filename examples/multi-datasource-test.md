# Multi-Datasource Testing with Builder Pattern

This document describes how to test the multi-datasource functionality with exec_count-based deduplication.

## Overview

The new builder pattern allows multiple datasources to feed into a single adapter, with automatic deduplication based on Thread exec_count. This ensures that even if multiple datasources report the same Thread update, it will only be processed once.

## Testing with Carbon Client

### 1. Single RPC Datasource (Baseline)

```bash
# Run with single RPC datasource
cargo run --bin antegen-carbon -- \
  --rpc-url http://localhost:8899 \
  --thread-program-id <YOUR_THREAD_PROGRAM_ID> \
  --keypair ~/.config/solana/id.json \
  --datasource rpc
```

### 2. Multiple RPC Datasources (Modified Carbon)

To test multiple datasources, you would need to modify the Carbon pipeline_builder.rs to create multiple datasources:

```rust
// In pipeline_builder.rs
let datasources = vec![
    Box::new(CarbonDatasourceBuilder::new(
        create_datasource_with_url(&config, "http://rpc1.example.com").await?,
        config.thread_program_id
    )),
    Box::new(CarbonDatasourceBuilder::new(
        create_datasource_with_url(&config, "http://rpc2.example.com").await?,
        config.thread_program_id
    )),
];

let client = AntegenClient::builder()
    .datasources(datasources)
    .adapter(AdapterBuilder::carbon())
    // ... rest of config
```

## Testing with Geyser Plugin

### 1. Configure the Plugin

Update your Geyser plugin config.json:

```json
{
  "name": "antegen",
  "keypath": "/path/to/keypair.json",
  "rpc_url": "http://localhost:8899",
  "ws_url": "ws://localhost:8900",
  "enable_replay": true,
  "nats_url": "nats://localhost:4222"
}
```

### 2. Run Validator with Plugin

```bash
solana-test-validator \
  --geyser-plugin-config /path/to/config.json \
  --geyser-plugin /path/to/libantegen_client_geyser.so
```

## Expected Behavior

### With exec_count Deduplication:

1. **Multiple Updates, Same exec_count**: Only the first update is processed
   - Datasource 1 sends Thread A with exec_count=100 → Processed ✓
   - Datasource 2 sends Thread A with exec_count=100 → Skipped (duplicate)
   - Datasource 3 sends Thread A with exec_count=100 → Skipped (duplicate)

2. **New Execution**: All datasources report it, but only one is processed
   - Datasource 2 sends Thread A with exec_count=101 (fastest) → Processed ✓
   - Datasource 1 sends Thread A with exec_count=101 → Skipped
   - Datasource 3 sends Thread A with exec_count=101 → Skipped

3. **Lagging Datasource**: Old updates are ignored
   - Current state: Thread A has exec_count=105
   - Datasource 3 (lagging) sends exec_count=103 → Skipped (old)

## Monitoring Metrics

When metrics are enabled, you can monitor:

```
# Cache hits (duplicates filtered)
adapter.cache_hits{type="exec_count_unchanged"}

# Cache misses (new executions)
adapter.cache_misses{type="exec_count_increased"}

# New threads
adapter.cache_misses{type="new_thread"}
```

## Testing Script

Create a test that spawns multiple Carbon instances pointing to different RPCs:

```bash
#!/bin/bash

# Start multiple Carbon clients with different datasources
# All feeding into the same processor/submitter

# Instance 1: Primary RPC
cargo run --bin antegen-carbon -- \
  --rpc-url http://rpc1.example.com:8899 &

# Instance 2: Secondary RPC  
cargo run --bin antegen-carbon -- \
  --rpc-url http://rpc2.example.com:8899 &

# Instance 3: Tertiary RPC
cargo run --bin antegen-carbon -- \
  --rpc-url http://rpc3.example.com:8899 &

wait
```

## Verification

To verify deduplication is working:

1. **Check Logs**: Look for "Skipping duplicate Thread update" messages
2. **Check Metrics**: Monitor cache hit/miss ratios
3. **Check Execution Count**: Verify each thread executes exactly once per exec_count increment
4. **Check Performance**: Multiple datasources should not increase processing load significantly

## Benefits

1. **Redundancy**: If one datasource fails, others continue
2. **Performance**: Fastest datasource naturally serves most updates
3. **Reliability**: No duplicate executions even with multiple sources
4. **Simplicity**: No configuration needed for deduplication