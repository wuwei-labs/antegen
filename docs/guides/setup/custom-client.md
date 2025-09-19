# Custom Client Setup Guide

Build custom clients for Antegen using available SDKs and language bindings. This guide covers architecture patterns, SDK usage, and implementation examples in multiple languages.

## Overview

Custom clients enable:
- Integration with existing applications
- Custom business logic and workflows
- Language-specific implementations
- Specialized monitoring and management tools

## Available SDKs

| Language | Package | Installation | Documentation |
|----------|---------|--------------|---------------|
| Rust | `antegen-client` | `cargo add antegen-client` | [crates.io](https://crates.io/crates/antegen-client) |
| TypeScript | `@antegen/sdk` | `npm install @antegen/sdk` | [npm](https://www.npmjs.com/package/@antegen/sdk) |
| Python | `antegen-py` | `pip install antegen-py` | [pypi](https://pypi.org/project/antegen-py/) |
| Go | `antegen-go` | `go get github.com/antegen/antegen-go` | [pkg.go.dev](https://pkg.go.dev/github.com/antegen/antegen-go) |

## Client Architecture

### Basic Architecture

```
┌─────────────────────────────────────────┐
│         Your Application                │
├─────────────────────────────────────────┤
│         Antegen SDK/Client              │
├─────────────────────────────────────────┤
│      Solana Web3 Library                │
├─────────────────────────────────────────┤
│    RPC/WebSocket Connection             │
└─────────────────────────────────────────┘
                    │
                    ▼
            Solana Network
```

### Components

1. **Thread Manager**: Create, update, delete threads
2. **Fiber Builder**: Construct and serialize instructions
3. **Trigger Factory**: Create various trigger types
4. **Event Listener**: Subscribe to thread events
5. **Transaction Builder**: Build and sign transactions

## Rust Client Implementation

### Setup

```toml
# Cargo.toml
[dependencies]
antegen-client = "0.1"
antegen-sdk = "0.1"
solana-sdk = "2.1"
solana-client = "2.1"
tokio = { version = "1", features = ["full"] }
anyhow = "1"
```

### Basic Usage

```rust
use antegen_client::{Client, ThreadBuilder, Trigger};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    signature::{Keypair, Signer},
    commitment_config::CommitmentConfig,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize client
    let rpc_client = RpcClient::new_with_commitment(
        "https://api.mainnet-beta.solana.com",
        CommitmentConfig::confirmed(),
    );

    let authority = Keypair::from_bytes(&[...])?;
    let client = Client::new(rpc_client, authority);

    // Create a thread
    let thread = ThreadBuilder::new()
        .id("my-automation")
        .trigger(Trigger::interval(60))  // Every 60 seconds
        .build();

    let signature = client.create_thread(thread).await?;
    println!("Thread created: {}", signature);

    Ok(())
}
```

### Advanced Thread Management

```rust
use antegen_client::{Client, Fiber, SerializableInstruction};
use solana_sdk::instruction::Instruction;

impl Client {
    // Add instruction to thread
    pub async fn add_fiber_example(&self) -> Result<()> {
        let thread_pubkey = self.get_thread_address("my-automation")?;

        // Build instruction
        let instruction = spl_token::instruction::transfer(
            &spl_token::id(),
            &source_token_account,
            &destination_token_account,
            &authority_pubkey,
            &[],
            1_000_000,
        )?;

        // Create fiber
        let fiber = Fiber::new()
            .index(0)
            .instruction(instruction)
            .priority_fee(5000)
            .build();

        // Add to thread
        self.add_fiber(thread_pubkey, fiber).await?;
        Ok(())
    }

    // Monitor thread events
    pub async fn monitor_thread(&self) -> Result<()> {
        let thread_pubkey = self.get_thread_address("my-automation")?;

        let (mut stream, _unsub) = self.subscribe_thread(thread_pubkey).await?;

        while let Some(event) = stream.next().await {
            match event {
                ThreadEvent::Executed { slot, signature } => {
                    println!("Thread executed at slot {}: {}", slot, signature);
                },
                ThreadEvent::Failed { error } => {
                    eprintln!("Execution failed: {}", error);
                },
                _ => {}
            }
        }

        Ok(())
    }
}
```

### Custom Event Source

```rust
use antegen_client::{EventSource, ObservedEvent};
use async_trait::async_trait;

pub struct CustomEventSource {
    // Your custom implementation
}

#[async_trait]
impl EventSource for CustomEventSource {
    async fn next_event(&mut self) -> Result<Option<ObservedEvent>> {
        // Custom event detection logic
        Ok(Some(ObservedEvent::ClockUpdate {
            clock: self.get_clock().await?,
            slot: self.get_slot().await?,
            block_height: self.get_block_height().await?,
        }))
    }

    async fn start(&mut self) -> Result<()> {
        // Initialize your event source
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        // Cleanup
        Ok(())
    }
}
```

## TypeScript Client Implementation

### Setup

```bash
# Install dependencies
npm install @antegen/sdk @solana/web3.js

# TypeScript types
npm install -D @types/node typescript
```

### Basic Usage

```typescript
import {
    AntegenClient,
    ThreadBuilder,
    Trigger
} from '@antegen/sdk';
import {
    Connection,
    Keypair,
    PublicKey
} from '@solana/web3.js';

async function main() {
    // Initialize connection
    const connection = new Connection(
        'https://api.mainnet-beta.solana.com',
        'confirmed'
    );

    // Load keypair
    const authority = Keypair.fromSecretKey(
        Buffer.from([...])
    );

    // Create client
    const client = new AntegenClient(connection, authority);

    // Create thread
    const thread = new ThreadBuilder()
        .id('my-automation')
        .trigger(Trigger.interval(60))
        .amount(0.1 * 1e9)  // 0.1 SOL
        .build();

    const signature = await client.createThread(thread);
    console.log('Thread created:', signature);
}

main().catch(console.error);
```

### React Integration

```tsx
import React, { useState, useEffect } from 'react';
import { useWallet } from '@solana/wallet-adapter-react';
import { AntegenClient, Thread } from '@antegen/sdk';

export function ThreadManager() {
    const { publicKey, signTransaction } = useWallet();
    const [threads, setThreads] = useState<Thread[]>([]);
    const [client, setClient] = useState<AntegenClient | null>(null);

    useEffect(() => {
        if (publicKey && signTransaction) {
            const connection = new Connection(clusterApiUrl('mainnet-beta'));
            const client = new AntegenClient(
                connection,
                { publicKey, signTransaction }
            );
            setClient(client);
            loadThreads(client);
        }
    }, [publicKey, signTransaction]);

    const loadThreads = async (client: AntegenClient) => {
        const threads = await client.getThreadsByAuthority(publicKey!);
        setThreads(threads);
    };

    const createThread = async () => {
        if (!client) return;

        const thread = new ThreadBuilder()
            .id(`thread-${Date.now()}`)
            .trigger(Trigger.cron('0 0 * * *'))  // Daily
            .build();

        await client.createThread(thread);
        await loadThreads(client);
    };

    return (
        <div>
            <button onClick={createThread}>Create Thread</button>
            <ul>
                {threads.map(thread => (
                    <li key={thread.pubkey.toBase58()}>
                        {thread.id} - {thread.trigger.type}
                    </li>
                ))}
            </ul>
        </div>
    );
}
```

### WebSocket Subscriptions

```typescript
import { AntegenClient } from '@antegen/sdk';

class ThreadMonitor {
    private client: AntegenClient;
    private subscriptions: Map<string, number> = new Map();

    constructor(client: AntegenClient) {
        this.client = client;
    }

    async subscribeToThread(threadId: string) {
        const threadPubkey = await this.client.getThreadAddress(threadId);

        const subId = this.client.onThreadChange(
            threadPubkey,
            (thread, context) => {
                console.log('Thread updated:', thread);
                console.log('Slot:', context.slot);

                if (thread.execCount > 0) {
                    console.log(`Executed ${thread.execCount} times`);
                }
            }
        );

        this.subscriptions.set(threadId, subId);
    }

    async unsubscribe(threadId: string) {
        const subId = this.subscriptions.get(threadId);
        if (subId) {
            await this.client.removeThreadListener(subId);
            this.subscriptions.delete(threadId);
        }
    }
}
```

## Python Client Implementation

### Setup

```bash
# Install package
pip install antegen-py solana

# Or from source
git clone https://github.com/antegen/antegen-py
cd antegen-py
pip install -e .
```

### Basic Usage

```python
from antegen import Client, ThreadBuilder, Trigger
from solana.rpc.api import Client as SolanaClient
from solana.keypair import Keypair

def main():
    # Initialize Solana client
    solana_client = SolanaClient("https://api.mainnet-beta.solana.com")

    # Load authority keypair
    authority = Keypair.from_secret_key(bytes([...]))

    # Create Antegen client
    client = Client(solana_client, authority)

    # Build thread
    thread = (ThreadBuilder()
        .with_id("my-automation")
        .with_trigger(Trigger.interval(60))
        .with_amount(0.1 * 10**9)  # 0.1 SOL
        .build())

    # Create thread on-chain
    signature = client.create_thread(thread)
    print(f"Thread created: {signature}")

    # Add instruction
    instruction = create_transfer_instruction(...)
    client.add_fiber(thread.pubkey, 0, instruction)

if __name__ == "__main__":
    main()
```

### Async Operations

```python
import asyncio
from antegen.async_client import AsyncClient

async def manage_threads():
    async with AsyncClient("https://api.mainnet-beta.solana.com") as client:
        # Create multiple threads concurrently
        tasks = []
        for i in range(10):
            thread = ThreadBuilder()                .with_id(f"thread-{i}")
                .with_trigger(Trigger.timestamp(1234567890 + i * 3600))
                .build()

            tasks.append(client.create_thread(thread))

        # Wait for all to complete
        signatures = await asyncio.gather(*tasks)

        for sig in signatures:
            print(f"Created: {sig}")

        # Monitor threads
        async for event in client.watch_threads():
            print(f"Event: {event.type} for thread {event.thread_id}")

asyncio.run(manage_threads())
```

### Data Analysis

```python
import pandas as pd
from antegen import Client, ThreadAnalyzer

class ThreadMetrics:
    def __init__(self, client: Client):
        self.client = client
        self.analyzer = ThreadAnalyzer(client)

    def analyze_execution_patterns(self, thread_id: str) -> pd.DataFrame:
        """Analyze thread execution patterns"""
        thread = self.client.get_thread(thread_id)
        history = self.analyzer.get_execution_history(thread.pubkey)

        df = pd.DataFrame(history)
        df['timestamp'] = pd.to_datetime(df['timestamp'], unit='s')
        df['success'] = df['error'].isna()

        # Calculate metrics
        metrics = {
            'total_executions': len(df),
            'success_rate': df['success'].mean(),
            'avg_gas_used': df['gas_used'].mean(),
            'total_fees': df['fee'].sum(),
        }

        return pd.DataFrame([metrics])

    def optimize_trigger_timing(self, thread_id: str):
        """Suggest optimal trigger times based on network congestion"""
        df = self.analyze_execution_patterns(thread_id)

        # Group by hour of day
        df['hour'] = df['timestamp'].dt.hour
        hourly_stats = df.groupby('hour').agg({
            'success': 'mean',
            'gas_used': 'mean',
            'fee': 'mean'
        })

        # Find optimal hour
        optimal_hour = hourly_stats['fee'].idxmin()

        return {
            'optimal_hour': optimal_hour,
            'expected_fee_reduction':
                1 - (hourly_stats.loc[optimal_hour, 'fee'] /
                     hourly_stats['fee'].mean())
        }
```

## Go Client Implementation

### Setup

```bash
go get github.com/antegen/antegen-go
go get github.com/gagliardetto/solana-go
```

### Basic Usage

```go
package main

import (
    "context"
    "fmt"
    "log"

    "github.com/antegen/antegen-go"
    "github.com/gagliardetto/solana-go"
    "github.com/gagliardetto/solana-go/rpc"
)

func main() {
    // Create RPC client
    rpcClient := rpc.New(rpc.MainNetBeta_RPC)

    // Load keypair
    authority, err := solana.PrivateKeyFromBase58("...")
    if err != nil {
        log.Fatal(err)
    }

    // Create Antegen client
    client := antegen.NewClient(rpcClient, authority)

    // Build thread
    thread := antegen.NewThreadBuilder().
        WithID("my-automation").
        WithTrigger(antegen.IntervalTrigger(60)).
        WithAmount(0.1 * solana.LAMPORTS_PER_SOL).
        Build()

    // Create thread
    ctx := context.Background()
    sig, err := client.CreateThread(ctx, thread)
    if err != nil {
        log.Fatal(err)
    }

    fmt.Printf("Thread created: %s\n", sig)
}
```

### Concurrent Operations

```go
package main

import (
    "context"
    "sync"
    "time"

    "github.com/antegen/antegen-go"
)

type ThreadPool struct {
    client   *antegen.Client
    threads  map[string]*antegen.Thread
    mu       sync.RWMutex
    workers  int
}

func NewThreadPool(client *antegen.Client, workers int) *ThreadPool {
    return &ThreadPool{
        client:  client,
        threads: make(map[string]*antegen.Thread),
        workers: workers,
    }
}

func (p *ThreadPool) CreateBatch(count int) error {
    ctx := context.Background()
    errChan := make(chan error, count)
    sem := make(chan struct{}, p.workers)

    var wg sync.WaitGroup
    wg.Add(count)

    for i := 0; i < count; i++ {
        go func(index int) {
            defer wg.Done()

            sem <- struct{}{}
            defer func() { <-sem }()

            thread := antegen.NewThreadBuilder().
                WithID(fmt.Sprintf("batch-%d", index)).
                WithTrigger(antegen.CronTrigger("0 * * * *")).
                Build()

            sig, err := p.client.CreateThread(ctx, thread)
            if err != nil {
                errChan <- err
                return
            }

            p.mu.Lock()
            p.threads[thread.ID] = thread
            p.mu.Unlock()

            fmt.Printf("Created thread %s: %s\n", thread.ID, sig)
        }(i)
    }

    wg.Wait()
    close(errChan)

    // Check for errors
    for err := range errChan {
        if err != nil {
            return err
        }
    }

    return nil
}

func (p *ThreadPool) MonitorAll(ctx context.Context) {
    ticker := time.NewTicker(10 * time.Second)
    defer ticker.Stop()

    for {
        select {
        case <-ctx.Done():
            return
        case <-ticker.C:
            p.checkThreads()
        }
    }
}

func (p *ThreadPool) checkThreads() {
    p.mu.RLock()
    defer p.mu.RUnlock()

    for id, thread := range p.threads {
        status, err := p.client.GetThreadStatus(context.Background(), thread.Pubkey)
        if err != nil {
            fmt.Printf("Error checking thread %s: %v\n", id, err)
            continue
        }

        fmt.Printf("Thread %s: executions=%d, paused=%v\n",
            id, status.ExecCount, status.Paused)
    }
}
```

## Client Patterns and Best Practices

### Connection Management

```typescript
class ConnectionManager {
    private connections: Map<string, Connection> = new Map();
    private healthChecks: Map<string, NodeJS.Timer> = new Map();

    addEndpoint(name: string, url: string) {
        const connection = new Connection(url, {
            commitment: 'confirmed',
            wsEndpoint: url.replace('https', 'wss'),
        });

        this.connections.set(name, connection);

        // Health check every 30s
        const timer = setInterval(async () => {
            try {
                await connection.getVersion();
            } catch (error) {
                console.error(`Endpoint ${name} unhealthy:`, error);
                this.handleUnhealthyConnection(name);
            }
        }, 30000);

        this.healthChecks.set(name, timer);
    }

    getHealthyConnection(): Connection {
        // Round-robin through healthy connections
        for (const [name, conn] of this.connections) {
            // Return first healthy connection
            return conn;
        }
        throw new Error('No healthy connections available');
    }
}
```

### Error Handling

```python
from enum import Enum
from typing import Optional
import time

class RetryStrategy(Enum):
    EXPONENTIAL = "exponential"
    LINEAR = "linear"
    FIXED = "fixed"

class RobustClient:
    def __init__(self, client: Client):
        self.client = client
        self.max_retries = 3
        self.base_delay = 1.0

    def execute_with_retry(
        self,
        operation,
        strategy: RetryStrategy = RetryStrategy.EXPONENTIAL,
        **kwargs
    ):
        """Execute operation with retry logic"""

        for attempt in range(self.max_retries):
            try:
                return operation(**kwargs)
            except Exception as e:
                if attempt == self.max_retries - 1:
                    raise

                delay = self._calculate_delay(attempt, strategy)
                print(f"Attempt {attempt + 1} failed, retrying in {delay}s: {e}")
                time.sleep(delay)

    def _calculate_delay(self, attempt: int, strategy: RetryStrategy) -> float:
        if strategy == RetryStrategy.EXPONENTIAL:
            return self.base_delay * (2 ** attempt)
        elif strategy == RetryStrategy.LINEAR:
            return self.base_delay * (attempt + 1)
        else:  # FIXED
            return self.base_delay
```

### Event Handling

```rust
use futures::StreamExt;
use tokio::sync::mpsc;

pub struct EventProcessor {
    event_tx: mpsc::Sender<ThreadEvent>,
    handlers: Vec<Box<dyn EventHandler>>,
}

#[async_trait]
pub trait EventHandler: Send + Sync {
    async fn handle(&self, event: &ThreadEvent) -> Result<()>;
}

impl EventProcessor {
    pub async fn process_events(&mut self) {
        let (tx, mut rx) = mpsc::channel(100);

        // Spawn event collection task
        tokio::spawn(async move {
            let mut event_stream = self.subscribe_events().await?;

            while let Some(event) = event_stream.next().await {
                if tx.send(event).await.is_err() {
                    break;
                }
            }
        });

        // Process events
        while let Some(event) = rx.recv().await {
            for handler in &self.handlers {
                if let Err(e) = handler.handle(&event).await {
                    error!("Handler error: {}", e);
                }
            }
        }
    }
}
```

### Batch Operations

```typescript
class BatchProcessor {
    constructor(
        private client: AntegenClient,
        private batchSize: number = 20
    ) {}

    async processBatch<T, R>(
        items: T[],
        operation: (item: T) => Promise<R>
    ): Promise<R[]> {
        const results: R[] = [];

        for (let i = 0; i < items.length; i += this.batchSize) {
            const batch = items.slice(i, i + this.batchSize);
            const batchResults = await Promise.all(
                batch.map(item => operation(item))
            );
            results.push(...batchResults);

            // Rate limiting between batches
            if (i + this.batchSize < items.length) {
                await new Promise(resolve => setTimeout(resolve, 1000));
            }
        }

        return results;
    }
}
```

## Testing Your Client

### Unit Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::signature::Keypair;
    use solana_program_test::*;

    #[tokio::test]
    async fn test_thread_creation() {
        let program_test = ProgramTest::new(
            "antegen_thread",
            thread_program::ID,
            processor!(thread_program::process_instruction),
        );

        let (mut banks_client, payer, recent_blockhash) =
            program_test.start().await;

        let client = TestClient::new(banks_client, payer);

        let thread = ThreadBuilder::new()
            .id("test-thread")
            .trigger(Trigger::Now)
            .build();

        let result = client.create_thread(thread).await;
        assert!(result.is_ok());
    }
}
```

### Integration Testing

```python
import pytest
from antegen.test import TestClient, MockRPCClient

@pytest.fixture
def client():
    mock_rpc = MockRPCClient()
    authority = Keypair()
    return TestClient(mock_rpc, authority)

def test_thread_lifecycle(client):
    # Create thread
    thread = client.create_thread("test", Trigger.now())
    assert thread.id == "test"

    # Add fiber
    instruction = create_test_instruction()
    client.add_fiber(thread.pubkey, 0, instruction)

    # Execute
    client.execute_thread(thread.pubkey)

    # Verify execution
    status = client.get_thread_status(thread.pubkey)
    assert status.exec_count == 1
```

## Deployment Considerations

### Configuration Management

```yaml
# config.yaml
development:
  rpc_url: http://localhost:8899
  commitment: processed
  max_retries: 5

staging:
  rpc_url: https://api.devnet.solana.com
  commitment: confirmed
  max_retries: 3

production:
  rpc_url: https://api.mainnet-beta.solana.com
  commitment: finalized
  max_retries: 3
  backup_urls:
    - https://solana-api.projectserum.com
    - https://rpc.ankr.com/solana
```

### Security

```typescript
// Secure key management
import { SecretManager } from '@cloud/secret-manager';

class SecureClient {
    private secretManager = new SecretManager();

    async getKeypair(): Promise<Keypair> {
        const secret = await this.secretManager.getSecret('executor-key');
        return Keypair.fromSecretKey(Buffer.from(secret, 'base64'));
    }

    async createClient(): Promise<AntegenClient> {
        const keypair = await this.getKeypair();
        const connection = new Connection(process.env.RPC_URL!);
        return new AntegenClient(connection, keypair);
    }
}
```

## Support Resources

- SDK Documentation: [docs.antegen.io/sdk](https://docs.antegen.io/sdk)
- Example Projects: [github.com/antegen/examples](https://github.com/antegen/examples)
- Discord Community: [discord.gg/antegen](https://discord.gg/antegen)
- Stack Overflow: [stackoverflow.com/questions/tagged/antegen](https://stackoverflow.com/questions/tagged/antegen)