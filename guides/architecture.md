# Architecture Overview

Antegen is a Solana program automation platform that provides scheduled transaction execution through an event-driven architecture. The system consists of three main layers working together to enable reliable, decentralized thread execution.

## System Components

### Layer 1: Thread Program (On-Chain)
The Solana program that manages thread lifecycle and execution logic.

**Key Features:**
- Thread creation with configurable triggers
- Fiber-based instruction sequences
- Trigger validation and execution
- Fee distribution with time-based decay
- Durable nonce support for reliability

**Location:** `programs/thread/`

### Layer 2: Observer & Executor (Off-Chain Services)
Event-driven services that monitor and execute threads.

**Observer Service (`crates/observer/`):**
- Monitors Solana network for thread and account events
- Filters executable threads based on trigger conditions
- Forwards execution events to Executor

**Executor Service (`crates/executor/`):**
- Receives thread execution requests from Observer
- Manages retry queue with exponential backoff
- Coordinates with Submitter for transaction submission
- Handles failed executions with dead letter queue

**Submitter Service (`crates/submitter/`):**
- Handles transaction submission via RPC and TPU clients
- Supports durable transaction replay via NATS messaging
- Optimizes submission paths (TPU with RPC fallback)
- Manages transaction durability and replay mechanisms

### Layer 3: Geyser Plugin (Validator Integration)
Real-time data pipeline for validator-level thread monitoring.

**Plugin (`plugin/`):**
- Integrates with Solana validators via Geyser interface
- Captures account updates and clock changes in real-time
- Bridges validator events to Observer/Executor services

## Data Flow

```
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   Validator     │───▶│   Geyser         │───▶│   Observer      │───▶│   Executor      │
│   Events        │    │   Plugin         │    │   Service       │    │   Service       │
└─────────────────┘    └──────────────────┘    └─────────────────┘    └─────────────────┘
         │                       │                       │                       │
         ▼                       ▼                       ▼                       ▼
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│ Raw Account &   │    │  Account/Clock   │    │  Filter & Queue │    │  Execute &      │
│ Clock Data      │    │  Updates         │    │  Ready Threads  │    │  Retry Logic    │
└─────────────────┘    └──────────────────┘    └─────────────────┘    └─────────────────┘
                                                                                │
                                                                                ▼
                                                                       ┌─────────────────┐
                                                                       │   Submitter     │
                                                                       │   Service       │
                                                                       └─────────────────┘
                                                                                │
                                                                                ▼
                                                                       ┌─────────────────┐
                                                                       │  TPU/RPC Submit │
                                                                       │  + NATS Replay  │
                                                                       └─────────────────┘
                                                                                │
                                                                                ▼
                                                                       ┌─────────────────┐
                                                                       │  Solana Thread │
                                                                       │    Program      │
                                                                       └─────────────────┘
```

## Component Interactions

### 1. Thread Creation
- Users create threads on-chain with specified triggers
- Thread state includes fibers (instructions), triggers, and configuration
- Thread PDAs are deterministic based on authority and thread ID

### 2. Event Detection
- **Geyser Plugin** captures real-time account updates from validator
- **Observer** receives events and evaluates trigger conditions
- Clock updates, account changes, and thread updates are processed

### 3. Execution Pipeline
- **Observer** identifies ready threads and forwards to Executor
- **Executor** queues execution tasks with retry logic
- Failed executions use exponential backoff before dead letter queue
- Successful executions trigger fee distribution

### 4. Persistence & Reliability
- **Sled Database** provides persistent task queuing across restarts
- **Durable Nonces** ensure transaction reliability over time
- **Retry Queue** handles network failures and congestion

## Trigger System

Threads support multiple trigger types for flexible automation:

- **Now**: Immediate execution
- **Timestamp**: Execute at specific Unix timestamp
- **Interval**: Recurring execution with fixed intervals
- **Cron**: Cron-expression based scheduling
- **Account**: Execute when account data changes
- **Slot/Epoch**: Execute at specific blockchain milestones

## Fee Economics

The system implements time-based commission decay to incentivize prompt execution:

1. **Grace Period**: Full commission for timely execution
2. **Decay Period**: Linear reduction from 100% to 0%
3. **Late Execution**: No commission after decay period

Fee distribution splits between:
- Executor commission (configurable, can be forgone)
- Core team fee (protocol development)
- Thread authority retains remainder

## Scalability Features

- **Parallel Processing**: Multiple executor threads handle concurrent executions
- **Event Batching**: Efficient processing of multiple thread updates
- **TPU Integration**: Direct validator submission for lower latency
- **Configurable Retry**: Exponential backoff with dead letter handling

## Security Considerations

- **PDA Authorization**: Thread execution requires valid authority signatures
- **Trigger Validation**: On-chain verification of trigger conditions
- **Nonce Management**: Prevents transaction replay attacks
- **Commission Limits**: Bounded fee extraction prevents exploitation

This architecture ensures reliable, scalable thread execution while maintaining decentralization and security properties essential for production Solana applications.