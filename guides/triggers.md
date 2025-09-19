# Thread Triggers

Thread triggers determine when your automated threads should execute. Antegen supports multiple trigger types to accommodate different automation patterns, from simple one-time executions to complex scheduling requirements.

## Trigger Types Overview

```rust
pub enum Trigger {
    Now,
    Timestamp { unix_ts: i64 },
    Interval { seconds: i64, skippable: bool },
    Cron { schedule: String, skippable: bool },
    Account { address: Pubkey, offset: u64, size: u64 },
    Slot { slot: u64 },
    Epoch { epoch: u64 },
}
```

Each trigger type serves specific automation use cases and has unique validation requirements.

## Now Trigger

Execute immediately when the thread is processed by an executor.

### Definition
```rust
Trigger::Now
```

### Use Cases
- Manual thread execution
- One-time setup operations
- Testing and development

### Behavior
- Always ready for execution
- No scheduling delay
- Executes once when processed
- Thread can be re-triggered by updating to another Now trigger

### Example
```rust
// Create immediate execution thread
let trigger = Trigger::Now;

thread_program::create_thread(
    ctx,
    amount,
    ThreadId::from("immediate-task"),
    trigger
)?;
```

### CLI Usage
```bash
cargo run --bin cli -- create-thread \
  --id "immediate-task" \
  --amount 10000000 \
  --trigger-type now
```

## Timestamp Trigger

Execute at a specific Unix timestamp (seconds since epoch).

### Definition
```rust
Trigger::Timestamp {
    unix_ts: i64, // Unix timestamp in seconds
}
```

### Use Cases
- One-time future execution
- Event scheduling for specific dates
- Delayed task execution
- Scheduled announcements or releases

### Behavior
- Executes when current time >= target timestamp
- Single execution only
- Thread remains after execution (can be updated with new trigger)

### Examples

#### Future Date Execution
```rust
use chrono::{DateTime, Utc};

// Schedule for New Year 2025
let target_time = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
    .unwrap()
    .timestamp();

let trigger = Trigger::Timestamp {
    unix_ts: target_time,
};
```

#### Delayed Execution
```rust
use chrono::Utc;

// Execute in 24 hours
let delay_hours = 24;
let future_time = Utc::now().timestamp() + (delay_hours * 3600);

let trigger = Trigger::Timestamp {
    unix_ts: future_time,
};
```

### CLI Usage
```bash
# Schedule for specific timestamp
cargo run --bin cli -- create-thread \
  --id "future-task" \
  --amount 10000000 \
  --trigger-type timestamp \
  --timestamp 1735689600

# Schedule for specific date (using date command)
TIMESTAMP=$(date -d "2025-01-01 00:00:00 UTC" +%s)
cargo run --bin cli -- create-thread \
  --id "new-year-task" \
  --amount 10000000 \
  --trigger-type timestamp \
  --timestamp $TIMESTAMP
```

## Interval Trigger

Execute repeatedly with fixed time intervals between executions.

### Definition
```rust
Trigger::Interval {
    seconds: i64,    // Interval between executions
    skippable: bool, // Whether to skip missed executions
}
```

### Parameters
- **seconds**: Time between executions (minimum 1 second)
- **skippable**: Behavior when execution is delayed
  - `true`: Use current time for next calculation (skip missed windows)
  - `false`: Use scheduled time for next calculation (catch up)

### Use Cases
- Recurring payments or transfers
- Regular health checks
- Periodic data updates
- Automated maintenance tasks

### Behavior
- First execution starts immediately (if thread is ready)
- Subsequent executions based on last execution time + interval
- Skippable affects catch-up behavior for delayed executions

### Examples

#### Hourly Recurring Task
```rust
let trigger = Trigger::Interval {
    seconds: 3600,     // Every hour
    skippable: false,  // Catch up missed executions
};
```

#### Daily Maintenance (Skippable)
```rust
let trigger = Trigger::Interval {
    seconds: 86400,    // Every 24 hours
    skippable: true,   // Skip if delayed past next window
};
```

#### High Frequency Updates
```rust
let trigger = Trigger::Interval {
    seconds: 30,       // Every 30 seconds
    skippable: true,   // Skip to current time if delayed
};
```

### CLI Usage
```bash
# Every 5 minutes, non-skippable
cargo run --bin cli -- create-thread \
  --id "frequent-task" \
  --amount 10000000 \
  --trigger-type interval \
  --interval-seconds 300 \
  --skippable false

# Daily skippable task
cargo run --bin cli -- create-thread \
  --id "daily-maintenance" \
  --amount 10000000 \
  --trigger-type interval \
  --interval-seconds 86400 \
  --skippable true
```

## Cron Trigger

Execute based on cron expression scheduling for complex timing patterns.

### Definition
```rust
Trigger::Cron {
    schedule: String, // Cron expression (5 or 6 field format)
    skippable: bool,  // Whether to skip missed executions
}
```

### Cron Expression Format
Supports standard 5-field cron format:
```
┌─────────────── minute (0 - 59)
│ ┌────────────── hour (0 - 23)  
│ │ ┌──────────── day of month (1 - 31)
│ │ │ ┌────────── month (1 - 12)
│ │ │ │ ┌──────── day of week (0 - 6) (Sunday=0)
│ │ │ │ │
* * * * *
```

### Special Characters
- `*`: Any value
- `,`: Value list separator  
- `-`: Value range
- `/`: Step values
- `@yearly`, `@monthly`, `@weekly`, `@daily`, `@hourly`: Shortcuts

### Use Cases
- Business hour operations
- Weekly/monthly reports
- Market opening/closing actions
- Backup schedules
- Complex recurring patterns

### Examples

#### Business Hours Execution
```rust
// Every weekday at 9:00 AM
let trigger = Trigger::Cron {
    schedule: "0 9 * * 1-5".to_string(),
    skippable: false,
};
```

#### Weekly Reports
```rust
// Every Monday at 8:00 AM
let trigger = Trigger::Cron {
    schedule: "0 8 * * 1".to_string(),
    skippable: true,
};
```

#### Market Hours
```rust
// Every 15 minutes during market hours (9:30 AM - 4:00 PM, weekdays)
let trigger = Trigger::Cron {
    schedule: "*/15 9-15 * * 1-5".to_string(),
    skippable: true,
};
```

#### End of Month
```rust
// Last day of every month at 11:59 PM
let trigger = Trigger::Cron {
    schedule: "59 23 28-31 * *".to_string(), // Note: needs validation logic
    skippable: false,
};
```

### CLI Usage
```bash
# Daily at 6:00 AM
cargo run --bin cli -- create-thread \
  --id "daily-report" \
  --amount 10000000 \
  --trigger-type cron \
  --cron-schedule "0 6 * * *" \
  --skippable false

# Every 5 minutes
cargo run --bin cli -- create-thread \
  --id "frequent-check" \
  --amount 10000000 \
  --trigger-type cron \
  --cron-schedule "*/5 * * * *" \
  --skippable true
```

## Account Trigger

Execute when monitored account data changes within specified range.

### Definition
```rust
Trigger::Account {
    address: Pubkey, // Account to monitor
    offset: u64,     // Byte offset to start monitoring
    size: u64,       // Number of bytes to monitor
}
```

### Parameters
- **address**: The Solana account to monitor for changes
- **offset**: Starting byte position in account data (0-based)
- **size**: Number of bytes to monitor from offset

### Use Cases
- React to token balance changes
- Monitor program state updates
- Trigger on oracle price updates
- Respond to governance proposals
- React to NFT transfers

### Behavior
- Monitors specific byte range in account data
- Uses hash comparison to detect changes
- Executes when data hash differs from last known hash
- Requires account to be provided in remaining_accounts during execution

### Data Range Considerations
- **Full Account**: `offset=0, size=account.data.len()`
- **Skip Discriminator**: `offset=8, size=data_size` (for Anchor programs)
- **Specific Field**: Target exact struct field location
- **Dynamic Size**: If `offset + size > data.len()`, monitors from offset to end

### Examples

#### Token Balance Monitor
```rust
// Monitor SPL token account balance (bytes 64-72)
let trigger = Trigger::Account {
    address: token_account_pubkey,
    offset: 64, // Balance field in SPL token account
    size: 8,    // u64 balance
};
```

#### Program State Monitor
```rust
// Monitor entire program data account (skip discriminator)
let trigger = Trigger::Account {
    address: program_data_account,
    offset: 8,  // Skip Anchor discriminator
    size: 256,  // Monitor specific data size
};
```

#### Oracle Price Feed
```rust
// Monitor Pyth price account
let trigger = Trigger::Account {
    address: pyth_price_account,
    offset: 208, // Price field offset in Pyth account
    size: 8,     // Price value size
};
```

#### NFT Ownership Changes
```rust
// Monitor NFT token account for ownership changes
let trigger = Trigger::Account {
    address: nft_token_account,
    offset: 32, // Owner field offset
    size: 32,   // Pubkey size
};
```

### CLI Usage
```bash
# Monitor token account balance changes
cargo run --bin cli -- create-thread \
  --id "balance-monitor" \
  --amount 10000000 \
  --trigger-type account \
  --account-address "TOKEN_ACCOUNT_PUBKEY" \
  --account-offset 64 \
  --account-size 8

# Monitor full program data account
cargo run --bin cli -- create-thread \
  --id "state-monitor" \
  --amount 10000000 \
  --trigger-type account \
  --account-address "PROGRAM_DATA_ACCOUNT" \
  --account-offset 8 \
  --account-size 1000
```

### Execution Requirements
When executing account-triggered threads, the monitored account must be included:

```rust
// In thread execution
let accounts = vec![
    // Regular thread execution accounts
    AccountMeta::new(executor.pubkey(), true),
    AccountMeta::new(thread.pubkey(), false),
    // ... other accounts
    
    // REQUIRED: Monitored account in remaining_accounts
    AccountMeta::new_readonly(monitored_account, false),
];
```

## Slot Trigger

Execute when blockchain reaches a specific slot number.

### Definition
```rust
Trigger::Slot {
    slot: u64, // Target slot number
}
```

### Use Cases
- Precise blockchain timing
- Slot-based auctions
- Network milestone events
- Block-synchronized operations

### Behavior
- Executes when `current_slot >= target_slot`
- Single execution per trigger
- Slot timing approximated for fee calculations (~400ms per slot)

### Example
```rust
// Execute at slot 150,000,000
let trigger = Trigger::Slot {
    slot: 150_000_000,
};
```

### CLI Usage
```bash
cargo run --bin cli -- create-thread \
  --id "slot-trigger" \
  --amount 10000000 \
  --trigger-type slot \
  --slot 150000000
```

## Epoch Trigger

Execute when blockchain reaches a specific epoch number.

### Definition
```rust
Trigger::Epoch {
    epoch: u64, // Target epoch number
}
```

### Use Cases
- Epoch-based rewards distribution
- Validator rotation events
- Long-term scheduled operations
- Network upgrade coordination

### Behavior
- Executes when `current_epoch >= target_epoch`
- Single execution per trigger
- Epoch timing varies (~2.5 days per epoch typically)

### Example
```rust
// Execute at epoch 500
let trigger = Trigger::Epoch {
    epoch: 500,
};
```

### CLI Usage
```bash
cargo run --bin cli -- create-thread \
  --id "epoch-trigger" \
  --amount 10000000 \
  --trigger-type epoch \
  --epoch 500
```

## Trigger Context

Each trigger maintains context for state tracking between executions:

### TriggerContext Types
```rust
pub enum TriggerContext {
    Timestamp { prev: i64, next: i64 },
    Block { prev: u64, next: u64 },
    Account { hash: u64 },
}
```

### Context Updates
- **Timestamp triggers**: Track previous and next execution times
- **Block triggers**: Track previous and next slot/epoch
- **Account triggers**: Track data hash for change detection

## Advanced Patterns

### Chaining Triggers
```rust
// Create sequence of timed operations
let triggers = vec![
    Trigger::Timestamp { unix_ts: start_time },
    Trigger::Interval { seconds: 3600, skippable: false },
    Trigger::Timestamp { unix_ts: end_time },
];

// Create separate threads for each phase
for (i, trigger) in triggers.iter().enumerate() {
    thread_program::create_thread(
        ctx.clone(),
        amount,
        ThreadId::from(format!("phase-{}", i)),
        trigger.clone()
    )?;
}
```

### Conditional Execution
```rust
// Use account trigger to react to state changes
let state_monitor = Trigger::Account {
    address: program_state_account,
    offset: 8,
    size: 1, // Monitor single byte flag
};

// Combine with interval for periodic checks
let periodic_check = Trigger::Interval {
    seconds: 300, // Every 5 minutes
    skippable: true,
};
```

### Multi-Trigger Patterns
```rust
// Create multiple threads for different conditions
let triggers = HashMap::from([
    ("hourly", Trigger::Interval { seconds: 3600, skippable: true }),
    ("daily", Trigger::Cron { schedule: "0 0 * * *".to_string(), skippable: false }),
    ("balance", Trigger::Account { address: token_account, offset: 64, size: 8 }),
]);

for (name, trigger) in triggers {
    thread_program::create_thread(
        ctx.clone(),
        amount,
        ThreadId::from(name),
        trigger
    )?;
}
```

## Performance Considerations

### Trigger Evaluation Efficiency
- **Account triggers**: Most resource-intensive (requires account data fetching)
- **Time-based triggers**: Lightweight comparison operations
- **Block triggers**: Minimal overhead using cached block info

### Monitoring Frequency
- High-frequency intervals (< 30 seconds) may increase execution costs
- Account triggers with large monitored ranges increase processing time
- Consider using skippable flags for non-critical timing

### Fee Implications
- Frequent executions accumulate more fees
- Late executions receive reduced commission (fee decay)
- Account triggers may have unpredictable execution timing

## Best Practices

### Trigger Selection
1. **Use simplest trigger** that meets requirements
2. **Prefer intervals over cron** for simple recurring tasks
3. **Use account triggers sparingly** for critical state changes
4. **Set skippable=true** for non-critical timing requirements

### Timing Considerations
1. **Account for network delays** in time-critical applications
2. **Use grace periods** for important executions
3. **Consider fee decay** when setting execution timing
4. **Test trigger behavior** on devnet before mainnet

### Error Handling
1. **Monitor trigger conditions** before creating threads
2. **Validate cron expressions** before deployment
3. **Test account trigger ranges** with sample data
4. **Handle trigger condition failures** gracefully

Understanding trigger types and their behaviors is essential for building reliable automated systems with Antegen. Choose the appropriate trigger type for your use case and consider the trade-offs between execution frequency, reliability, and cost.