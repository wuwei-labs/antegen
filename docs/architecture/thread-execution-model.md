# Thread Execution Model

## Overview

The Antegen thread execution model provides a flexible, reliable framework for automated transaction execution on Solana. This document details the complete lifecycle of a thread, from creation through execution, including the fiber system, trigger mechanics, and state management.

## Thread Lifecycle

### Lifecycle Stages

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   Created   │────▶│   Active    │────▶│  Executing  │────▶│  Complete   │
└─────────────┘     └─────────────┘     └─────────────┘     └─────────────┘
       │                    │                    │                    │
       │                    ▼                    ▼                    │
       │            ┌─────────────┐     ┌─────────────┐              │
       └───────────▶│   Paused    │     │   Failed    │◀─────────────┘
                    └─────────────┘     └─────────────┘
```

### 1. Creation Phase

Thread creation establishes the automation framework:

```rust
// Thread creation transaction
let thread_id = ThreadId::from("my-automation");
let trigger = Trigger::Interval {
    seconds: 3600,
    skippable: false
};

thread_program::create_thread(
    ctx,
    rent_exemption_amount,
    thread_id,
    trigger,
)?;
```

**Creation Requirements:**
- Unique thread ID (string or pubkey)
- Valid authority (thread owner)
- Sufficient SOL for rent exemption
- Valid trigger configuration
- Optional nonce account for durability

**PDA Derivation:**
```rust
let thread_pda = Pubkey::find_program_address(
    &[
        b"thread",
        thread.authority.as_ref(),
        thread.id.as_ref(),
    ],
    &thread_program::ID,
);
```

### 2. Configuration Phase

After creation, threads are configured with fibers (instructions):

```rust
// Add fiber to thread
thread_program::create_fiber(
    ctx,
    index,           // Execution order (0-255)
    instruction,     // Serialized instruction
    signer_seeds,    // Optional PDA seeds
    priority_fee,    // Optional priority
)?;
```

**Fiber Configuration:**
- Sequential execution (index 0 → 255)
- Each fiber is independent instruction
- Optional signer seeds for PDAs
- Priority fees for congestion

### 3. Active Phase

Threads in active state await trigger conditions:

**Trigger Evaluation:**
```rust
match thread.trigger {
    Trigger::Now => {
        // Execute immediately
        ready = true;
    },
    Trigger::Timestamp { unix_ts } => {
        // Check if current time >= target
        ready = clock.unix_timestamp >= unix_ts;
    },
    Trigger::Interval { seconds, skippable } => {
        // Check if interval elapsed
        let elapsed = clock.unix_timestamp - thread.last_exec;
        ready = elapsed >= seconds;
    },
    Trigger::Account { address, offset, size } => {
        // Monitor account data changes
        ready = account_data_changed(address, offset, size);
    },
    // Additional trigger types...
}
```

### 4. Execution Phase

When triggered, the thread enters execution:

```rust
// Executor builds transaction
let mut instructions = vec![];

// Add each fiber as instruction
for fiber in thread.fibers.iter() {
    let ix = build_instruction(fiber)?;
    instructions.push(ix);
}

// Add thread_exec instruction
instructions.push(
    thread_program::thread_exec(
        thread_pubkey,
        executor_pubkey,
    )
);

// Submit transaction
let tx = Transaction::new_signed_with_payer(
    &instructions,
    Some(&executor_pubkey),
    &[executor_keypair],
    recent_blockhash,
);
```

### 5. Completion Phase

Post-execution state updates:

```rust
// Thread state after execution
thread.exec_count += 1;
thread.last_exec = clock.unix_timestamp;
thread.next_exec = calculate_next_exec(thread.trigger);

// Fee distribution
let total_fee = thread.fee_account.lamports();
let executor_fee = calculate_commission(total_fee, elapsed_time);
let core_fee = total_fee * CORE_FEE_BPS / 10000;
let remainder = total_fee - executor_fee - core_fee;
```

## Fiber System

### Fiber Architecture

Fibers are the building blocks of thread automation - individual instructions executed sequentially:

```rust
pub struct Fiber {
    pub thread: Pubkey,              // Parent thread
    pub index: u8,                   // Execution order
    pub instruction: Instruction,   // Actual instruction
    pub signer_seeds: Vec<Vec<Vec<u8>>>, // PDA signing
    pub priority_fee: u64,          // Transaction priority
}
```

### Fiber Composition Patterns

#### Sequential Execution
```
Fiber 0: Check balance
Fiber 1: Transfer if sufficient
Fiber 2: Log transaction
```

#### Conditional Logic (via CPI)
```
Fiber 0: Call decision program
Fiber 1: Execute based on return
```

#### Multi-Program Integration
```
Fiber 0: Swap tokens (DEX program)
Fiber 1: Stake LP tokens (Staking program)
Fiber 2: Update records (Custom program)
```

### Instruction Serialization

Fibers store instructions in serialized format:

```rust
#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct SerializableInstruction {
    pub program_id: Pubkey,
    pub accounts: Vec<SerializableAccountMeta>,
    pub data: Vec<u8>,
}

impl From<Instruction> for SerializableInstruction {
    fn from(ix: Instruction) -> Self {
        SerializableInstruction {
            program_id: ix.program_id,
            accounts: ix.accounts.into_iter()
                .map(|acc| SerializableAccountMeta {
                    pubkey: acc.pubkey,
                    is_signer: acc.is_signer,
                    is_writable: acc.is_writable,
                })
                .collect(),
            data: ix.data,
        }
    }
}
```

### Signer Seed Management

For PDA signing within fibers:

```rust
// Define signer seeds for PDA
let signer_seeds = vec![
    vec![b"vault".to_vec()],
    vec![thread_pubkey.to_bytes().to_vec()],
    vec![bump.to_le_bytes().to_vec()],
];

// Thread signs on behalf of PDA
thread_program::create_fiber(
    ctx,
    0,
    transfer_instruction,
    signer_seeds,  // Thread will sign as PDA
    0,
)?;
```

## Trigger Mechanics

### Trigger Types

#### 1. Immediate (Now)
```rust
Trigger::Now
```
- Executes as soon as possible
- No conditions to evaluate
- Useful for one-time operations

#### 2. Timestamp
```rust
Trigger::Timestamp { unix_ts: i64 }
```
- Executes at specific Unix timestamp
- One-time execution
- Timezone-independent

#### 3. Interval
```rust
Trigger::Interval {
    seconds: i64,
    skippable: bool
}
```
- Recurring execution pattern
- Fixed interval between executions
- Skippable flag for missed executions

#### 4. Cron Expression
```rust
Trigger::Cron {
    schedule: String,  // "0 0 * * *"
    skippable: bool
}
```
- Complex scheduling patterns
- Standard cron syntax support
- Timezone considerations

#### 5. Account Monitor
```rust
Trigger::Account {
    address: Pubkey,
    offset: u64,
    size: u64
}
```
- Monitors specific account data
- Triggers on data changes
- Efficient partial monitoring

#### 6. Slot-Based
```rust
Trigger::Slot { slot: u64 }
```
- Blockchain-native scheduling
- Executes at specific slot
- Precise timing control

#### 7. Epoch-Based
```rust
Trigger::Epoch { epoch: u64 }
```
- Long-term scheduling
- Epoch boundary execution
- Validator-aligned timing

### Trigger Context

Each execution includes context about the triggering event:

```rust
pub enum TriggerContext {
    Account {
        hash: u64  // Hash of monitored data
    },
    Timestamp {
        prev: i64,  // Previous execution
        next: i64   // Next scheduled
    },
    Block {
        prev: u64,  // Previous slot/epoch
        next: u64   // Next scheduled
    },
}
```

### Trigger Evaluation Algorithm

```rust
impl Thread {
    pub fn is_ready(&self, clock: &Clock) -> bool {
        // Check rate limiting
        if !self.check_rate_limit(clock) {
            return false;
        }

        // Evaluate trigger condition
        match &self.trigger {
            Trigger::Now => true,

            Trigger::Timestamp { unix_ts } => {
                clock.unix_timestamp >= *unix_ts
            },

            Trigger::Interval { seconds, skippable } => {
                let elapsed = clock.unix_timestamp - self.last_exec;
                if *skippable {
                    // Skip missed intervals
                    elapsed >= *seconds
                } else {
                    // Execute all missed intervals
                    elapsed >= *seconds || self.exec_count == 0
                }
            },

            Trigger::Cron { schedule, skippable } => {
                let next = calculate_next_cron(schedule, self.last_exec)?;
                if *skippable {
                    clock.unix_timestamp >= next
                } else {
                    // Execute if we've passed scheduled time
                    clock.unix_timestamp >= next
                }
            },

            // Additional trigger evaluations...
        }
    }
}
```

## Execution Guarantees

### Reliability Guarantees

1. **At-Least-Once Execution**: Retry mechanisms ensure execution
2. **Ordered Fiber Execution**: Fibers execute in index order
3. **Atomic Thread Updates**: State changes are atomic
4. **Deterministic PDA Signing**: Consistent signing behavior

### Ordering Guarantees

```
Thread State: Active
     ↓
Check Trigger: Ready
     ↓
Build Transaction:
  - Fiber 0 (index=0)
  - Fiber 1 (index=1)
  - Fiber N (index=N)
  - ThreadExec instruction
     ↓
Submit Transaction
     ↓
Update Thread State
```

### Failure Handling

#### Execution Failures

```rust
// Executor retry logic
let mut retry_count = 0;
let max_retries = 5;
let mut backoff = Duration::from_secs(1);

loop {
    match submit_transaction(&tx).await {
        Ok(_) => break,
        Err(e) if retry_count < max_retries => {
            retry_count += 1;
            tokio::time::sleep(backoff).await;
            backoff *= 2;  // Exponential backoff
        },
        Err(e) => {
            // Move to dead letter queue
            dead_letter_queue.push(thread_id, e);
            break;
        }
    }
}
```

#### Partial Execution

If a fiber fails mid-execution:
- Previous fibers may have succeeded
- Thread state remains unchanged
- Next execution attempts all fibers
- Consider idempotent operations

## State Management

### Thread Account State

```rust
pub struct Thread {
    // Identity
    pub authority: Pubkey,       // Owner/controller
    pub id: String,             // Unique identifier

    // Execution control
    pub paused: bool,           // Pause flag
    pub trigger: Trigger,       // Execution condition

    // Durability
    pub nonce_account: Pubkey,  // Optional nonce

    // Execution tracking
    pub created_at: i64,        // Creation timestamp
    pub exec_count: u64,        // Total executions
    pub last_exec: i64,         // Last execution time
    pub next_exec: i64,         // Next scheduled time

    // Rate limiting
    pub rate_limit: u64,        // Min seconds between

    // Fee management
    pub fee_account: Pubkey,    // Fee collection

    // Reserved space
    pub _reserved: [u8; 256],   // Future use
}
```

### State Transitions

```rust
impl Thread {
    pub fn execute(&mut self, clock: &Clock) -> Result<()> {
        // Pre-execution checks
        require!(!self.paused, ThreadError::ThreadPaused);
        require!(self.is_ready(clock), ThreadError::TriggerNotReady);

        // Update execution state
        self.exec_count += 1;
        self.last_exec = clock.unix_timestamp;

        // Calculate next execution
        self.next_exec = match &self.trigger {
            Trigger::Interval { seconds, .. } => {
                self.last_exec + seconds
            },
            Trigger::Cron { schedule, .. } => {
                calculate_next_cron(schedule, self.last_exec)?
            },
            _ => i64::MAX,  // One-time triggers
        };

        Ok(())
    }

    pub fn pause(&mut self) -> Result<()> {
        self.paused = true;
        Ok(())
    }

    pub fn resume(&mut self) -> Result<()> {
        self.paused = false;
        Ok(())
    }
}
```

### Fiber State Management

```rust
pub struct FiberAccount {
    pub thread: Pubkey,
    pub index: u8,
    pub instruction_data: Vec<u8>,  // Serialized instruction
    pub signer_seeds: Vec<Vec<Vec<u8>>>,
    pub priority_fee: u64,
}

impl FiberAccount {
    pub fn update_instruction(&mut self, ix: Instruction) -> Result<()> {
        self.instruction_data = SerializableInstruction::from(ix)
            .try_to_vec()?;
        Ok(())
    }

    pub fn to_instruction(&self) -> Result<Instruction> {
        let serializable = SerializableInstruction::try_from_slice(
            &self.instruction_data
        )?;
        Ok(serializable.into())
    }
}
```

## Advanced Execution Patterns

### Dynamic Thread Updates

Threads can modify themselves during execution:

```rust
pub struct ThreadResponse {
    pub close_to: Option<Pubkey>,      // Close thread
    pub next_instruction: Option<u8>,  // Jump to fiber
    pub trigger: Option<Trigger>,      // Update trigger
}

// Target program returns response
fn process_with_response() -> ThreadResponse {
    ThreadResponse {
        close_to: None,
        next_instruction: Some(2),  // Skip to fiber 2
        trigger: Some(Trigger::Interval {
            seconds: 7200,  // Change to 2 hours
            skippable: true
        }),
    }
}
```

### Conditional Execution

Using decision programs:

```rust
// Decision fiber (index 0)
let decision_ix = decision_program::evaluate_condition(
    condition_account,
    thread_pubkey,
);

// Action fiber (index 1)
// Only executes if decision fiber succeeds
let action_ix = target_program::execute_action(
    target_account,
);
```

### Batch Operations

Multiple operations in single thread:

```rust
// Token distribution thread
for (index, recipient) in recipients.iter().enumerate() {
    thread_program::create_fiber(
        ctx,
        index as u8,
        spl_token::transfer(
            token_account,
            recipient,
            amount,
        ),
        vec![],
        0,
    )?;
}
```

## Performance Optimization

### Fiber Optimization

1. **Minimize Fiber Count**: Combine operations where possible
2. **Order by Dependency**: Dependent operations first
3. **Use Priority Fees**: For congested conditions
4. **Batch Similar Operations**: Reduce overhead

### Trigger Optimization

1. **Choose Appropriate Type**: Match use case
2. **Set Reasonable Intervals**: Avoid too frequent
3. **Use Skippable Flag**: For non-critical timing
4. **Monitor Account Efficiently**: Minimal data ranges

### State Access Patterns

1. **Cache Thread State**: Reduce RPC calls
2. **Batch Fiber Fetches**: Get all at once
3. **Use Geyser for Updates**: Real-time notifications
4. **Implement Local Validation**: Pre-flight checks

## Security Considerations

### Thread Security

1. **Authority Validation**: Only owner can modify
2. **Fiber Validation**: Instructions verified on creation
3. **Signer Verification**: Proper authorization checked
4. **Rate Limiting**: Prevents execution spam

### Execution Security

1. **Executor Authorization**: Valid keypair required
2. **Fee Bounds**: Limited commission extraction
3. **State Consistency**: Atomic updates only
4. **Replay Protection**: Nonce accounts for durability

## Monitoring and Debugging

### Key Metrics

```rust
// Thread health metrics
pub struct ThreadMetrics {
    pub total_executions: u64,
    pub successful_executions: u64,
    pub failed_executions: u64,
    pub average_latency_ms: u64,
    pub last_execution_time: i64,
    pub next_scheduled_time: i64,
    pub fee_balance: u64,
}
```

### Debug Information

```rust
// Execution trace
pub struct ExecutionTrace {
    pub thread_id: String,
    pub trigger_type: String,
    pub trigger_context: TriggerContext,
    pub fibers_executed: Vec<u8>,
    pub transaction_signature: Signature,
    pub execution_time_ms: u64,
    pub fee_paid: u64,
    pub error: Option<String>,
}
```

### Common Issues

1. **Thread Not Executing**
   - Check trigger conditions
   - Verify not paused
   - Ensure sufficient fees
   - Check rate limiting

2. **Fiber Failures**
   - Validate instruction data
   - Check account permissions
   - Verify signer seeds
   - Ensure accounts exist

3. **Timing Issues**
   - Clock synchronization
   - Network latency
   - Trigger evaluation logic
   - Rate limit conflicts

## Conclusion

The Antegen thread execution model provides a comprehensive framework for automated transaction execution on Solana. Through its combination of flexible triggers, sequential fiber execution, and robust state management, developers can build sophisticated automation patterns while maintaining reliability and security. The model's emphasis on deterministic execution, proper error handling, and performance optimization ensures that threads execute efficiently and predictably in production environments.