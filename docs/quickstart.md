# Antegen Quick Start Guide

Get started with Antegen in 5 minutes! This guide will help you create your first automated thread on Solana.

## Prerequisites

- Rust (latest stable)
- Solana CLI 2.1+
- 1 SOL for testing (use devnet for free SOL)

## Installation

```bash
# Clone and build Antegen
git clone https://github.com/wuwei-labs/antegen
cd antegen
cargo build --release

# Install the CLI
cargo install --path crates/cli
```

## Step 1: Start Local Environment

```bash
# Start a local test validator
solana-test-validator

# In a new terminal, configure Solana CLI
solana config set --url localhost

# Create a keypair (or use existing)
solana-keygen new --outfile ~/.config/solana/id.json

# Airdrop SOL for testing
solana airdrop 10
```

## Step 2: Deploy Thread Program

```bash
# Build the program
anchor build

# Deploy to localnet
anchor deploy

# Note the program ID from output
export THREAD_PROGRAM_ID=<YOUR_PROGRAM_ID>
```

## Step 3: Start Antegen Services

```bash
# Run the local processor (combines Observer + Executor)
antegen localnet --dev
```

## Step 4: Create Your First Thread

Create a simple recurring transfer that sends 0.001 SOL every 60 seconds:

```bash
# Create a thread that transfers SOL every minute
antegen thread create \
  --id "my-first-thread" \
  --trigger "interval:60" \
  --amount 0.1

# Add the transfer instruction
antegen thread add-instruction \
  --id "my-first-thread" \
  --program "11111111111111111111111111111112" \
  --accounts "[YOUR_WALLET],[RECIPIENT_WALLET]" \
  --data "transfer:0.001"

# Fund the thread for execution fees
antegen thread fund \
  --id "my-first-thread" \
  --amount 0.05

# Start the thread
antegen thread start --id "my-first-thread"
```

## Step 5: Monitor Your Thread

```bash
# Check thread status
antegen thread status --id "my-first-thread"

# Watch execution logs
antegen thread logs --id "my-first-thread" --follow

# View thread details
antegen thread get --id "my-first-thread"
```

## What Just Happened?

1. **Thread Created**: You created an automation unit with a 60-second interval trigger
2. **Instruction Added**: You added a SOL transfer instruction to execute
3. **Thread Funded**: You provided SOL for execution fees
4. **Automation Started**: The thread began executing every 60 seconds
5. **Monitoring Active**: You're watching the thread execute in real-time

## Next Steps

### Try Different Triggers

```bash
# Execute at specific time
antegen thread create --id "scheduled" --trigger "timestamp:1735689600"

# Execute on account changes
antegen thread create --id "monitor" --trigger "account:PUBKEY"

# Execute with cron schedule (daily at noon)
antegen thread create --id "daily" --trigger "cron:0 12 * * *"
```

### Explore Advanced Features

- **[Architecture Overview](architecture/overview.md)** - Understand how Antegen works
- **[Thread Execution Model](architecture/thread-execution-model.md)** - Deep dive into threads
- **[Setup Guides](guides/setup/)** - Production deployment options
- **[API Reference](api/)** - Complete command reference

### Get Help

```bash
# View all commands
antegen --help

# Get help for specific command
antegen thread create --help

# Check version
antegen --version
```

## Common Commands

```bash
# List all your threads
antegen thread list

# Pause a thread
antegen thread pause --id "my-first-thread"

# Resume a thread
antegen thread resume --id "my-first-thread"

# Delete a thread
antegen thread delete --id "my-first-thread"

# Withdraw funds from thread
antegen thread withdraw --id "my-first-thread" --amount 0.01
```

## Troubleshooting

### Thread not executing?
- Check trigger conditions: `antegen thread status --id "thread-id"`
- Ensure sufficient funding: `antegen thread balance --id "thread-id"`
- Verify thread is active: `antegen thread get --id "thread-id"`

### Transaction failures?
- Check logs: `antegen thread logs --id "thread-id"`
- Verify instruction accounts exist
- Ensure proper account permissions

### Connection issues?
- Confirm Solana RPC is accessible: `solana cluster-version`
- Check Antegen services are running: `ps aux | grep antegen`

## Example: Token Distribution Bot

Here's a complete example of a token distribution thread:

```bash
# Create thread for weekly token distribution
antegen thread create \
  --id "token-distributor" \
  --trigger "cron:0 0 * * MON" \
  --amount 0.1

# Add SPL token transfer instruction
antegen thread add-instruction \
  --id "token-distributor" \
  --program "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA" \
  --accounts "[token_account],[recipient],[owner]" \
  --data "transfer:1000000"

# Start the distribution
antegen thread start --id "token-distributor"
```

Ready to build more complex automations? Check out our [comprehensive guides](guides/) or join our [Discord community](https://discord.gg/antegen) for support!

## Quick Reference

| Command | Description |
|---------|-------------|
| `antegen thread create` | Create new thread |
| `antegen thread list` | List all threads |
| `antegen thread status` | Check thread status |
| `antegen thread start` | Start thread execution |
| `antegen thread pause` | Pause thread |
| `antegen thread logs` | View execution logs |
| `antegen localnet` | Start local services |

🚀 **You're now ready to automate on Solana with Antegen!**