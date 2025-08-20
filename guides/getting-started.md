# Getting Started

This guide walks you through setting up and using the Antegen thread automation platform on Solana. You'll learn how to create, configure, and execute automated threads for your applications.

## Prerequisites

### Development Environment
- **Rust**: Latest stable version
- **Solana CLI**: Version 1.18+
- **Node.js**: Version 16+ (for TypeScript SDK examples)

### Knowledge Requirements
- Basic understanding of Solana programs and accounts
- Familiarity with Rust and/or TypeScript
- Understanding of PDAs (Program Derived Addresses)

## Installation

### 1. Clone the Repository
```bash
git clone https://github.com/wuwei-labs/antegen
cd antegen
```

### 2. Build the Project
```bash
# Build all components
cargo build --release

# Build specific components
cargo build -p thread-program --release
cargo build -p antegen-observer --release  
cargo build -p antegen-executor --release
cargo build -p plugin --release
```

### 3. Install Solana CLI (if needed)
```bash
sh -c "$(curl -sSfL https://release.solana.com/v1.18.0/install)"
```

## Quick Start

### 1. Start Local Validator
```bash
# Start Solana test validator
solana-test-validator
```

### 2. Configure Solana CLI
```bash
# Set to localnet
solana config set --url localhost

# Create or use existing keypair
solana-keygen new --outfile ~/.config/solana/id.json
```

### 3. Deploy Thread Program
```bash
# Build and deploy the program
anchor build
anchor deploy
```

### 4. Initialize Configuration
```bash
# Initialize global thread configuration (admin only)
cargo run --bin cli -- init-config \
  --admin $(solana-keygen pubkey ~/.config/solana/id.json) \
  --commission-fee 1000000 \
  --executor-fee-bps 8000 \
  --core-team-bps 1000
```

## Creating Your First Thread

### 1. Simple Transfer Thread
Let's create a thread that transfers SOL every 60 seconds:

```rust
use antegen_thread_program::{
    state::{Trigger, SerializableInstruction},
    ThreadId,
};
use solana_sdk::{
    system_instruction,
    instruction::Instruction,
};

// Define thread parameters
let thread_id = ThreadId::from("my-transfer-thread");
let trigger = Trigger::Interval {
    seconds: 60,
    skippable: false,
};

// Create transfer instruction
let transfer_ix = system_instruction::transfer(
    &source_pubkey,
    &destination_pubkey,
    1_000_000, // 0.001 SOL
);

// Convert to serializable format
let serializable_ix = SerializableInstruction::from(transfer_ix);
```

### 2. Create Thread Account
```bash
# Create thread with 0.01 SOL funding
cargo run --bin cli -- create-thread \
  --id "my-transfer-thread" \
  --amount 10000000 \
  --trigger-type interval \
  --interval-seconds 60
```

### 3. Add Fiber (Instruction)
```bash
# Add transfer instruction as first fiber
cargo run --bin cli -- create-fiber \
  --thread-id "my-transfer-thread" \
  --index 0 \
  --program-id 11111111111111111111111111111112 \
  --accounts "[source],[destination],[system_program]" \
  --data "base64_encoded_instruction_data"
```

## Running the Executor

### 1. Standalone Executor
```bash
# Run executor service standalone
cargo run --bin executor -- \
  --rpc-url http://localhost:8899 \
  --keypair-path ~/.config/solana/id.json \
  --data-dir /tmp/antegen_executor
```

### 2. With Observer Integration
```bash
# Run observer + executor together
cargo run --bin localnet-processor -- \
  --rpc-url http://localhost:8899 \
  --ws-url ws://localhost:8900 \
  --keypair-path ~/.config/solana/id.json \
  --data-dir /tmp/antegen_data
```

## Thread Trigger Types

### Timestamp Trigger
Execute at a specific time:
```rust
let trigger = Trigger::Timestamp {
    unix_ts: 1640995200, // January 1, 2022
};
```

### Interval Trigger
Execute every N seconds:
```rust
let trigger = Trigger::Interval {
    seconds: 3600, // Every hour
    skippable: false,
};
```

### Cron Trigger
Execute based on cron schedule:
```rust
let trigger = Trigger::Cron {
    schedule: "0 0 * * *".to_string(), // Daily at midnight
    skippable: true,
};
```

### Account Trigger
Execute when account data changes:
```rust
let trigger = Trigger::Account {
    address: account_pubkey,
    offset: 0,
    size: 32,
};
```

## Common Patterns

### 1. Token Transfer Automation
```rust
// Create recurring token transfer
let transfer_ix = spl_token::instruction::transfer(
    &spl_token::id(),
    &source_token_account,
    &destination_token_account,
    &owner,
    &[],
    amount,
)?;

// Add to thread as fiber
thread_program::create_fiber(
    ctx,
    0,
    transfer_ix.into(),
    vec![] // No additional signer seeds
)?;
```

### 2. Account State Monitoring
```rust
// Monitor account for changes
let trigger = Trigger::Account {
    address: monitored_account,
    offset: 8,  // Skip discriminator
    size: 32,   // Monitor specific field
};

// Create thread with monitoring trigger
thread_program::create_thread(
    ctx,
    rent_exemption_amount,
    ThreadId::from("account-monitor"),
    trigger
)?;
```

### 3. Scheduled Program Calls
```rust
// Schedule custom program instruction
let custom_ix = Instruction {
    program_id: your_program_id,
    accounts: vec![
        AccountMeta::new(account1, false),
        AccountMeta::new_readonly(account2, false),
    ],
    data: your_instruction_data,
};

// Add to thread
thread_program::create_fiber(
    ctx,
    0,
    custom_ix.into(),
    vec![] // Thread will sign as authority
)?;
```

## Managing Threads

### Check Thread Status
```bash
# View thread account details
solana account [THREAD_PUBKEY]

# List all threads for authority
cargo run --bin cli -- list-threads \
  --authority $(solana-keygen pubkey ~/.config/solana/id.json)
```

### Pause/Resume Threads
```bash
# Pause thread execution
cargo run --bin cli -- toggle-thread \
  --thread-id "my-transfer-thread"

# Resume thread execution
cargo run --bin cli -- toggle-thread \
  --thread-id "my-transfer-thread"
```

### Update Thread Trigger
```bash
# Change interval from 60s to 120s
cargo run --bin cli -- update-thread \
  --thread-id "my-transfer-thread" \
  --trigger-type interval \
  --interval-seconds 120
```

### Withdraw from Thread
```bash
# Withdraw 0.005 SOL from thread balance
cargo run --bin cli -- withdraw-thread \
  --thread-id "my-transfer-thread" \
  --amount 5000000
```

## Monitoring and Debugging

### Thread Execution Logs
```bash
# Monitor executor logs
cargo run --bin executor -- --log-level debug

# Filter for specific thread
cargo run --bin executor -- --log-level debug | grep "THREAD_PUBKEY"
```

### Queue Status
```bash
# Check executor queue status
cargo run --bin cli -- queue-status \
  --data-dir /tmp/antegen_executor

# View dead letter queue
cargo run --bin cli -- dead-letter-queue \
  --data-dir /tmp/antegen_executor
```

### Transaction History
```bash
# View recent transactions for thread
solana transaction-history [THREAD_PUBKEY] --limit 10
```

## Production Deployment

### 1. Validator Integration
For production deployment with real-time event processing:

```json
{
    "libpath": "/path/to/libantegen_plugin.so",
    "keypath": "/path/to/executor-keypair.json",
    "rpc_url": "https://api.mainnet-beta.solana.com",
    "data_dir": "/var/lib/antegen",
    "forgo_executor_commission": false
}
```

### 2. Service Configuration
```bash
# Create systemd service
sudo systemctl enable antegen-executor
sudo systemctl start antegen-executor

# Monitor service status
sudo systemctl status antegen-executor
```

### 3. Monitoring Setup
```bash
# Set up log rotation
sudo logrotate -d /etc/logrotate.d/antegen

# Configure metrics collection
export ANTEGEN_METRICS_ENDPOINT="http://prometheus:9090"
```

## Troubleshooting

### Common Issues

1. **"Thread not found" error**: Verify thread PDA derivation
2. **"Trigger not ready" error**: Check trigger conditions and timing
3. **"Insufficient funds" error**: Ensure thread has enough SOL for fees
4. **"Nonce account invalid" error**: Verify durable nonce setup

### Debug Commands
```bash
# Validate thread PDA
cargo run --bin cli -- derive-thread-pda \
  --authority AUTHORITY_PUBKEY \
  --id "thread-id"

# Check trigger readiness
cargo run --bin cli -- check-trigger \
  --thread-id "my-thread"

# Validate fiber instruction
cargo run --bin cli -- validate-fiber \
  --thread-id "my-thread" \
  --index 0
```

## Next Steps

- Explore [Trigger Types](triggers.md) for advanced scheduling
- Learn about [Fee Economics](fees-and-economics.md) for cost optimization
- Review [Architecture](architecture.md) for system understanding
- Check component READMEs for detailed API documentation

## Support

- **GitHub Issues**: Report bugs and feature requests
- **Documentation**: Comprehensive guides in `/guides`
- **Examples**: Sample code in `/examples`
- **Community**: Join our Discord for discussions

You're now ready to build automated applications with Antegen! Start with simple examples and gradually explore more complex automation patterns.