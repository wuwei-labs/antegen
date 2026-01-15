# Thread Program

The Antegen Thread Program is a Solana program that enables scheduled transaction execution through configurable triggers and instruction sequences (fibers). It provides a decentralized automation layer for Solana applications.

**Program ID:** `AgThdyi1P5RkVeZD2rQahTvs8HePJoGFFxKtvok5s2J1`

## Key Concepts

### Threads
A thread is an on-chain automation unit that executes instruction sequences when trigger conditions are met. Each thread contains:
- **Authority**: The account that owns and controls the thread
- **ID**: Unique identifier (bytes or Pubkey) for the thread
- **Trigger**: Condition that determines when execution should occur
- **Fibers**: Ordered sequence of instructions to execute
- **State**: Current execution index, pause status, nonce info

### Fibers
Fibers are individual instructions within a thread's execution sequence. They are stored as compiled instructions and executed in order:
- **Index**: Position in the execution sequence (0-255)
- **Compiled Instruction**: Serialized instruction data
- **Execution Tracking**: Last executed timestamp and count

### Triggers
Triggers define when a thread should execute. Supported types:
- **Now**: Execute immediately when called
- **Timestamp**: Execute at specific Unix timestamp
- **Interval**: Recurring execution with fixed second intervals
- **Cron**: Schedule-based execution using cron expressions
- **Account**: Execute when specified account data changes
- **Slot**: Execute when blockchain reaches target slot
- **Epoch**: Execute when blockchain reaches target epoch

## Program Instructions

### Configuration Management

#### `init_config`
Initialize the global thread configuration (admin only).
```rust
pub fn init_config(ctx: Context<ConfigInit>) -> Result<()>
```

#### `update_config`
Update global configuration parameters.
```rust
pub fn update_config(ctx: Context<ConfigUpdate>, params: ConfigUpdateParams) -> Result<()>
```

### Thread Lifecycle

#### `create_thread`
Create a new thread with specified trigger and funding.
```rust
pub fn create_thread(
    ctx: Context<ThreadCreate>,
    amount: u64,           // Initial funding in lamports
    id: ThreadId,          // Unique thread identifier
    trigger: Trigger,      // Execution trigger condition
) -> Result<()>
```

#### `close_thread`
Close thread account and return lamports to authority. Requires authority (owner) or thread itself to sign.
```rust
pub fn close_thread(ctx: Context<ThreadClose>) -> Result<()>
```

#### `delete_thread`
Admin-only instruction to delete a thread, skipping all checks. Used for cleaning up stuck/broken threads.
```rust
pub fn delete_thread(ctx: Context<ThreadDelete>) -> Result<()>
```

#### `update_thread`
Modify thread properties (pause state, trigger). Only provided fields are updated.
```rust
pub fn update_thread(
    ctx: Context<ThreadUpdate>,
    params: ThreadUpdateParams
) -> Result<()>

pub struct ThreadUpdateParams {
    pub paused: Option<bool>,    // Explicitly set pause state
    pub trigger: Option<Trigger>, // Update trigger condition
}
```

#### `withdraw_thread`
Withdraw lamports from thread's balance.
```rust
pub fn withdraw_thread(
    ctx: Context<ThreadWithdraw>,
    amount: u64
) -> Result<()>
```

### Fiber Management

#### `create_fiber`
Add instruction to thread's execution sequence.
```rust
pub fn create_fiber(
    ctx: Context<FiberCreate>,
    index: u8,                          // Position in sequence
    instruction: SerializableInstruction, // Instruction to execute
    signer_seeds: Vec<Vec<Vec<u8>>>,    // PDA seeds for signing
) -> Result<()>
```

#### `close_fiber`
Remove instruction from thread's sequence.
```rust
pub fn close_fiber(
    ctx: Context<FiberClose>,
    index: u8
) -> Result<()>
```

### Thread Execution

#### `exec_thread`
Execute the next fiber in thread's sequence with trigger validation.
```rust
pub fn exec_thread(
    ctx: Context<ThreadExec>,
    forgo_commission: bool  // Executor can forgo their commission
) -> Result<()>
```

**Execution Flow:**
1. Validate trigger conditions are met
2. Advance durable nonce (required for all threads)
3. Update trigger context for next execution
4. Execute current fiber instruction via CPI
5. Advance to next fiber in sequence (wraps around)
6. Calculate and distribute execution fees

## Fee Economics

The program implements time-based commission decay to incentivize prompt execution:

### Commission Calculation
- **Base Commission**: Configured per-execution fee
- **Grace Period**: Full commission for timely execution
- **Decay Period**: Linear reduction from 100% to 0% over time
- **Late Penalty**: No commission after grace + decay period

### Fee Distribution
Total effective commission is split between:
- **Executor Fee**: Configurable percentage (can be forgone)
- **Core Team Fee**: Protocol development funding
- **Thread Authority**: Retains remainder

## Account Structure

### Thread Account
```rust
pub struct Thread {
    pub authority: Pubkey,           // Thread owner
    pub id: Vec<u8>,                // Thread identifier  
    pub trigger: Trigger,           // Execution condition
    pub trigger_context: TriggerContext, // Trigger state
    pub exec_index: u8,             // Current fiber index
    pub fibers: Vec<Pubkey>,        // Fiber account addresses
    pub paused: bool,               // Execution status
    pub created_at: i64,           // Creation timestamp
    pub bump: u8,                  // PDA bump
    pub version: u8,               // Version for upgrades
}
```

### Fiber Account
```rust
pub struct FiberState {
    pub compiled_instruction: Vec<u8>, // Serialized instruction
    pub last_executed: i64,           // Last execution time
    pub execution_count: u64,         // Total executions
    pub bump: u8,                     // PDA bump
}
```

### Configuration Account
```rust
pub struct ThreadConfig {
    pub admin: Pubkey,                 // Config admin
    pub paused: bool,                  // Global pause
    pub commission_fee: u64,           // Base execution fee
    pub executor_fee_bps: u64,         // Executor share (basis points)
    pub core_team_bps: u64,           // Core team share (basis points)
    pub grace_period_seconds: i64,     // Full commission window
    pub fee_decay_seconds: i64,        // Decay period length
    pub bump: u8,                     // PDA bump
}
```

## PDA Seeds

The program uses deterministic PDAs for account addressing:

- **Config**: `["config"]`
- **Thread**: `["thread", authority, thread_id]`
- **Fiber**: `["thread_fiber", thread_pubkey, fiber_index]`

## Error Handling

The program defines comprehensive error types for validation and execution:

```rust
pub enum AntegenThreadError {
    InvalidThreadAuthority,
    InvalidConfigAdmin,
    ThreadPaused,
    GlobalPauseActive,
    TriggerConditionFailed,
    TriggerNotReady,
    InvalidThreadState,
    NonceRequired,
    InvalidNonceAccount,
    // ... additional errors
}
```

## Durable Nonces

All threads must use durable nonces for transaction reliability:
- Prevents replay attacks over long time periods
- Enables offline transaction preparation
- Thread PDA serves as nonce authority
- Nonce is advanced before each execution

## Integration Examples

### Creating a Simple Recurring Thread
```rust
// Create thread that executes every 60 seconds
let trigger = Trigger::Interval { 
    seconds: 60, 
    skippable: false 
};

// Fund thread with 0.01 SOL
let amount = 10_000_000;
let id = ThreadId::from("my-recurring-task");

thread_program::create_thread(
    ctx,
    amount,
    id,
    trigger
)?;
```

### Adding Instruction Fiber
```rust
// Add transfer instruction as fiber
let instruction = system_instruction::transfer(
    &source,
    &destination,
    1_000_000
);

thread_program::create_fiber(
    ctx,
    0, // First fiber
    instruction.into(),
    vec![] // No additional seeds
)?;
```

This program provides the foundation for decentralized automation on Solana, enabling applications to schedule reliable transaction execution without centralized infrastructure.