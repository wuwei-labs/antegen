# Durability and Reliability

## Overview

Antegen implements multiple layers of durability and reliability mechanisms to ensure automated transactions execute successfully even under adverse network conditions. This document details the differences between durable and non-durable threads, nonce account management, transaction replay systems, and failure recovery strategies.

## Thread Durability Models

### Non-Durable Threads (Default)

Non-durable threads use standard Solana transaction mechanics with recent blockhashes:

```rust
pub struct NonDurableThread {
    pub thread: Pubkey,
    pub recent_blockhash: Hash,  // Expires after ~150 slots
    pub max_retries: u8,         // Limited retry window
}
```

**Characteristics:**
- Uses recent blockhash (valid for ~60-90 seconds)
- Lower complexity and cost
- Suitable for frequent executions
- No additional account management

**Use Cases:**
- High-frequency trading bots
- Regular interval tasks (< 2 minutes)
- Time-sensitive operations
- Low-value transactions

**Limitations:**
- Transactions expire quickly
- Network congestion can cause misses
- No long-term replay capability
- Requires active monitoring

### Durable Threads

Durable threads leverage nonce accounts for long-lived transaction validity:

```rust
pub struct DurableThread {
    pub thread: Pubkey,          // controls nonce account
    pub nonce_account: Pubkey,
    pub nonce_value: Hash,       // Current nonce value
}
```

**Characteristics:**
- Transactions valid indefinitely
- Survives network outages
- Replay capability (pending implementation)
- Higher reliability guarantees

**Use Cases:**
- Critical financial operations
- Infrequent executions (hourly/daily)
- High-value transactions
- Cross-timezone operations
- Disaster recovery scenarios

**Implementation:**
```rust
// Create durable thread
let nonce_account = Keypair::new();
let thread_id = ThreadId::from("critical-operation");

// Initialize nonce account
let init_nonce_ix = system_instruction::create_account(
    &payer.pubkey(),
    &nonce_account.pubkey(),
    rent_exemption,
    NonceAccount::LEN as u64,
    &system_program::ID,
);

let init_nonce_ix = system_instruction::initialize_nonce_account(
    &nonce_account.pubkey(),
    &thread_authority,
);

// Create thread with nonce
thread_program::create_thread_with_nonce(
    ctx,
    thread_id,
    trigger,
    nonce_account.pubkey(),
)?;
```

## When to Use Each Type

### Decision Matrix

| Criteria | Non-Durable | Durable |
|----------|-------------|---------|
| **Execution Frequency** | < 2 minutes | > 2 minutes |
| **Transaction Value** | Low-Medium | High |
| **Network Reliability Required** | Standard | Critical |
| **Complexity Tolerance** | Low | High |
| **Cost Sensitivity** | High | Low |
| **Replay Requirement** | No | Yes |
| **Long-term Storage** | No | Yes |

### Decision Flow

```
Start
  │
  ├─ Is execution critical?
  │   ├─ Yes → Use Durable
  │   └─ No → Continue
  │
  ├─ Execution interval > 5 minutes?
  │   ├─ Yes → Use Durable
  │   └─ No → Continue
  │
  ├─ Need replay capability?
  │   ├─ Yes → Use Durable
  │   └─ No → Continue
  │
  └─ Use Non-Durable (Default)
```

## Nonce Account Management

### Nonce Account Lifecycle

```
Create → Initialize → Use → Advance → Close
   │         │         │       │         │
   │         │         │       │         └─ Reclaim rent
   │         │         │       └─ Update nonce value
   │         │         └─ Sign transactions
   │         └─ Set authority
   └─ Allocate account space
```

### Nonce Account Structure

```rust
pub struct NonceAccount {
    pub version: u32,
    pub state: NonceState,
}

pub enum NonceState {
    Uninitialized,
    Initialized(NonceData),
}

pub struct NonceData {
    pub authority: Pubkey,      // Who can advance nonce
    pub blockhash: Hash,        // Nonce value
    pub fee_calculator: FeeCalculator, // Historical
}
```

### Nonce Operations

#### Creating Nonce Account
```rust
pub async fn create_nonce_account(
    rpc: &RpcClient,
    payer: &Keypair,
    nonce: &Keypair,
    authority: &Pubkey,
) -> Result<Signature> {
    let rent = rpc.get_minimum_balance_for_rent_exemption(
        NonceAccount::LEN
    ).await?;

    let instructions = vec![
        system_instruction::create_account(
            &payer.pubkey(),
            &nonce.pubkey(),
            rent,
            NonceAccount::LEN as u64,
            &system_program::ID,
        ),
        system_instruction::initialize_nonce_account(
            &nonce.pubkey(),
            authority,
        ),
    ];

    let tx = Transaction::new_signed_with_payer(
        &instructions,
        Some(&payer.pubkey()),
        &[payer, nonce],
        recent_blockhash,
    );

    rpc.send_and_confirm_transaction(&tx).await
}
```

#### Using Nonce in Thread
```rust
pub fn build_durable_transaction(
    thread: &Thread,
    nonce_account: &NonceAccount,
    instructions: Vec<Instruction>,
) -> Transaction {
    // Advance nonce as first instruction
    let mut all_instructions = vec![
        system_instruction::advance_nonce_account(
            &thread.nonce_account,
            &thread.authority,
        )
    ];

    // Add thread instructions
    all_instructions.extend(instructions);

    // Use nonce as blockhash
    Transaction::new_signed_with_payer(
        &all_instructions,
        Some(&executor.pubkey()),
        &[executor],
        nonce_account.blockhash,  // Durable blockhash
    )
}
```

### Nonce Best Practices

1. **Authority Management**
   - Use thread authority as nonce authority
   - Implement multi-sig for critical threads
   - Regular authority rotation for security

2. **Nonce Advancement**
   - Advance only when executing
   - Handle advance failures gracefully
   - Monitor nonce account balance

3. **Account Hygiene**
   - Close unused nonce accounts
   - Monitor rent exemption status
   - Regular account audits

## Transaction Replay System

### Message Queue-Based Replay Architecture (Future)

```
┌─────────────────────────────────────────────────────────┐
│                   Transaction Flow                       │
├─────────────────────────────────────────────────────────┤
│                                                          │
│  Submit      Success                                    │
│    ↓           ↑                                        │
│  ┌──────────────┐                                      │
│  │   Submitter  │                                      │
│  │   Service    │                                      │
│  └──────┬───────┘                                      │
│         │ Failure                                       │
│         ↓                                               │
│  ┌──────────────┐     ┌──────────────┐                │
│  │ Message Queue│────▶│    Replay    │                │
│  │  (e.g. NATS) │     │   Consumer   │                │
│  └──────────────┘     └──────────────┘                │
│         ↑                     │                        │
│         └─────────────────────┘                        │
│              After Delay                               │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

### Replay Message Format

```rust
#[derive(Serialize, Deserialize)]
pub struct DurableTransactionMessage {
    pub thread_id: String,
    pub transaction: Vec<u8>,      // Serialized transaction
    pub nonce_account: Pubkey,
    pub nonce_value: Hash,
    pub created_at: i64,
    pub retry_count: u8,
    pub max_retries: u8,
    pub priority: TransactionPriority,
}

#[derive(Serialize, Deserialize)]
pub enum TransactionPriority {
    Low,      // Replay after 60s
    Medium,   // Replay after 30s
    High,     // Replay after 10s
    Critical, // Replay after 5s
}
```

### Replay Consumer Implementation

```rust
pub struct ReplayConsumer {
    nats_client: async_nats::Client,
    submission_service: Arc<SubmitterService>,
    config: ReplayConfig,
}

impl ReplayConsumer {
    pub async fn process_replay_queue(&mut self) -> Result<()> {
        let jetstream = async_nats::jetstream::new(
            self.nats_client.clone()
        );

        // Create durable consumer
        let consumer = stream.create_consumer(
            async_nats::jetstream::consumer::pull::Config {
                durable_name: Some("replay_consumer".into()),
                ack_policy: AckPolicy::Explicit,
                max_deliver: 3,
                ack_wait: Duration::from_secs(30),
                ..Default::default()
            }
        ).await?;

        // Process messages
        let mut messages = consumer.messages().await?;
        while let Some(msg) = messages.next().await {
            self.handle_replay_message(msg).await?;
        }

        Ok(())
    }

    async fn handle_replay_message(
        &self,
        msg: Message
    ) -> Result<()> {
        let tx_msg: DurableTransactionMessage =
            serde_json::from_slice(&msg.payload)?;

        // Check if still valid
        if !self.should_replay(&tx_msg) {
            msg.ack().await?;
            return Ok(());
        }

        // Update nonce if needed
        let tx = self.refresh_transaction_nonce(tx_msg).await?;

        // Attempt resubmission
        match self.submission_service.submit(tx).await {
            Ok(_) => {
                msg.ack().await?;
                info!("Replay successful: {}", tx_msg.thread_id);
            },
            Err(e) if tx_msg.retry_count < tx_msg.max_retries => {
                // Requeue with incremented count
                let mut new_msg = tx_msg;
                new_msg.retry_count += 1;
                self.requeue_message(new_msg).await?;
                msg.ack().await?;
            },
            Err(e) => {
                // Move to dead letter queue
                self.dead_letter(tx_msg, e).await?;
                msg.ack().await?;
            }
        }

        Ok(())
    }
}
```

### Replay Configuration

```rust
pub struct ReplayConfig {
    pub enabled: bool,
    pub nats_url: String,
    pub replay_delays: ReplayDelays,
    pub max_retries: u8,
    pub batch_size: usize,
}

pub struct ReplayDelays {
    pub low: Duration,      // 60 seconds
    pub medium: Duration,   // 30 seconds
    pub high: Duration,     // 10 seconds
    pub critical: Duration, // 5 seconds
}
```

## Failure Recovery Strategies

### Retry Logic with Exponential Backoff

```rust
pub struct RetryStrategy {
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub multiplier: f64,
    pub max_attempts: u32,
}

impl RetryStrategy {
    pub async fn execute<F, T, E>(
        &self,
        mut operation: F
    ) -> Result<T, E>
    where
        F: FnMut() -> Future<Output = Result<T, E>>,
    {
        let mut delay = self.initial_delay;
        let mut attempts = 0;

        loop {
            match operation().await {
                Ok(result) => return Ok(result),
                Err(e) if attempts < self.max_attempts => {
                    attempts += 1;
                    tokio::time::sleep(delay).await;

                    // Calculate next delay with jitter
                    delay = Duration::from_secs_f64(
                        (delay.as_secs_f64() * self.multiplier)
                            .min(self.max_delay.as_secs_f64())
                    );

                    // Add random jitter (±10%)
                    let jitter = delay.as_secs_f64() * 0.1 *
                        (rand::random::<f64>() - 0.5);
                    delay = Duration::from_secs_f64(
                        delay.as_secs_f64() + jitter
                    );
                },
                Err(e) => return Err(e),
            }
        }
    }
}
```

### Dead Letter Queue

```rust
pub struct DeadLetterQueue {
    storage: sled::Db,
    notifications: Option<NotificationService>,
}

impl DeadLetterQueue {
    pub async fn add(
        &self,
        thread_id: String,
        error: ExecutionError,
        transaction: Transaction,
    ) -> Result<()> {
        let entry = DeadLetterEntry {
            thread_id: thread_id.clone(),
            error: error.to_string(),
            transaction: bincode::serialize(&transaction)?,
            timestamp: Utc::now().timestamp(),
            attempts: self.get_attempt_count(&thread_id),
        };

        // Store in persistent database
        self.storage.insert(
            thread_id.as_bytes(),
            serde_json::to_vec(&entry)?,
        )?;

        // Send notification if configured
        if let Some(notifier) = &self.notifications {
            notifier.alert(
                AlertLevel::Critical,
                format!("Thread {} moved to DLQ: {}",
                    thread_id, error)
            ).await?;
        }

        Ok(())
    }

    pub async fn retry_entry(
        &self,
        thread_id: &str,
    ) -> Result<()> {
        if let Some(entry_bytes) = self.storage.get(thread_id)? {
            let entry: DeadLetterEntry =
                serde_json::from_slice(&entry_bytes)?;

            // Attempt manual retry
            let tx: Transaction =
                bincode::deserialize(&entry.transaction)?;

            // Resubmit with fresh nonce if durable
            self.resubmit_with_fresh_nonce(tx).await?;

            // Remove from DLQ on success
            self.storage.remove(thread_id)?;
        }

        Ok(())
    }
}
```

### Circuit Breaker Pattern

```rust
pub struct CircuitBreaker {
    failure_threshold: u32,
    success_threshold: u32,
    timeout: Duration,
    state: Arc<RwLock<CircuitState>>,
}

#[derive(Clone)]
enum CircuitState {
    Closed {
        failure_count: u32,
    },
    Open {
        opened_at: Instant,
    },
    HalfOpen {
        success_count: u32,
    },
}

impl CircuitBreaker {
    pub async fn execute<F, T>(&self, operation: F) -> Result<T>
    where
        F: Future<Output = Result<T>>,
    {
        let state = self.state.read().await;

        match &*state {
            CircuitState::Open { opened_at } => {
                if opened_at.elapsed() > self.timeout {
                    // Transition to half-open
                    drop(state);
                    let mut state = self.state.write().await;
                    *state = CircuitState::HalfOpen {
                        success_count: 0
                    };
                } else {
                    return Err(anyhow!("Circuit breaker is open"));
                }
            },
            _ => {},
        }

        // Attempt operation
        match operation.await {
            Ok(result) => {
                self.record_success().await;
                Ok(result)
            },
            Err(e) => {
                self.record_failure().await;
                Err(e)
            }
        }
    }

    async fn record_success(&self) {
        let mut state = self.state.write().await;

        match &*state {
            CircuitState::HalfOpen { success_count } => {
                if success_count + 1 >= self.success_threshold {
                    *state = CircuitState::Closed {
                        failure_count: 0
                    };
                } else {
                    *state = CircuitState::HalfOpen {
                        success_count: success_count + 1
                    };
                }
            },
            CircuitState::Closed { .. } => {
                *state = CircuitState::Closed {
                    failure_count: 0
                };
            },
            _ => {},
        }
    }

    async fn record_failure(&self) {
        let mut state = self.state.write().await;

        match &*state {
            CircuitState::Closed { failure_count } => {
                if failure_count + 1 >= self.failure_threshold {
                    *state = CircuitState::Open {
                        opened_at: Instant::now()
                    };
                } else {
                    *state = CircuitState::Closed {
                        failure_count: failure_count + 1
                    };
                }
            },
            CircuitState::HalfOpen { .. } => {
                *state = CircuitState::Open {
                    opened_at: Instant::now()
                };
            },
            _ => {},
        }
    }
}
```

## High Availability Patterns

### Multi-Region Deployment

```yaml
# Architecture for multi-region HA
regions:
  us-east:
    executors: 3
    submitters: 2
    nats_cluster: 3
    rpc_endpoints:
      - https://api.mainnet-beta.solana.com
      - https://solana-api.projectserum.com

  eu-west:
    executors: 3
    submitters: 2
    nats_cluster: 3
    rpc_endpoints:
      - https://api.mainnet-beta.solana.com
      - https://solana-api.projectserum.com

  asia-pacific:
    executors: 2
    submitters: 1
    nats_cluster: 3
    rpc_endpoints:
      - https://api.mainnet-beta.solana.com
      - https://solana-api.projectserum.com

coordination:
  leader_election: etcd
  state_sync: nats_jetstream
  monitoring: prometheus_federation
```

### Leader Election for Executors

```rust
pub struct ExecutorCoordinator {
    etcd_client: etcd_client::Client,
    executor_id: String,
    lease_ttl: i64,
}

impl ExecutorCoordinator {
    pub async fn acquire_leadership(&self) -> Result<bool> {
        // Try to acquire distributed lock
        let lease = self.etcd_client
            .lease_grant(self.lease_ttl, None)
            .await?;

        let key = format!("/antegen/executor/leader");
        let txn = self.etcd_client
            .txn()
            .when([Compare::version(key.clone(), CompareOp::Equal, 0)])
            .and_then([TxnOp::put(
                key.clone(),
                self.executor_id.clone(),
                Some(PutOptions::new().with_lease(lease.id()))
            )])
            .or_else([TxnOp::get(key.clone(), None)]);

        let response = txn.commit().await?;
        Ok(response.succeeded())
    }

    pub async fn maintain_leadership(&self) -> Result<()> {
        // Periodically refresh lease
        loop {
            tokio::time::sleep(
                Duration::from_secs(self.lease_ttl as u64 / 3)
            ).await;

            if !self.refresh_lease().await? {
                // Lost leadership, restart election
                break;
            }
        }

        Ok(())
    }
}
```

### Graceful Degradation

```rust
pub struct DegradationStrategy {
    pub modes: Vec<OperatingMode>,
    pub current_mode: Arc<RwLock<OperatingMode>>,
}

#[derive(Clone)]
pub enum OperatingMode {
    Full {
        all_features: bool,
    },
    Degraded {
        essential_only: bool,
        reduced_throughput: f64,
    },
    Maintenance {
        read_only: bool,
    },
}

impl DegradationStrategy {
    pub async fn evaluate_health(&self) -> OperatingMode {
        let metrics = self.collect_health_metrics().await;

        if metrics.error_rate > 0.5 {
            OperatingMode::Maintenance { read_only: true }
        } else if metrics.error_rate > 0.1 {
            OperatingMode::Degraded {
                essential_only: true,
                reduced_throughput: 0.5,
            }
        } else {
            OperatingMode::Full { all_features: true }
        }
    }

    pub async fn apply_mode(&self, mode: OperatingMode) {
        let mut current = self.current_mode.write().await;
        *current = mode.clone();

        match mode {
            OperatingMode::Full { .. } => {
                info!("Operating in full mode");
            },
            OperatingMode::Degraded { reduced_throughput, .. } => {
                warn!("Degraded mode: {}% throughput",
                    reduced_throughput * 100.0);
            },
            OperatingMode::Maintenance { .. } => {
                error!("Maintenance mode: read-only operations");
            }
        }
    }
}
```

## Monitoring and Alerting

### Health Checks

```rust
pub struct HealthChecker {
    checks: Vec<Box<dyn HealthCheck>>,
}

#[async_trait]
pub trait HealthCheck {
    async fn check(&self) -> HealthStatus;
    fn name(&self) -> &str;
    fn critical(&self) -> bool;
}

pub struct HealthStatus {
    pub healthy: bool,
    pub message: String,
    pub metrics: HashMap<String, f64>,
}

// Example health checks
pub struct NonceAccountCheck;

#[async_trait]
impl HealthCheck for NonceAccountCheck {
    async fn check(&self) -> HealthStatus {
        // Check all nonce accounts
        let nonce_accounts = fetch_nonce_accounts().await;
        let invalid_count = nonce_accounts.iter()
            .filter(|n| !n.is_valid())
            .count();

        HealthStatus {
            healthy: invalid_count == 0,
            message: format!("{} invalid nonce accounts", invalid_count),
            metrics: HashMap::from([
                ("invalid_nonces".into(), invalid_count as f64),
            ]),
        }
    }

    fn name(&self) -> &str { "nonce_accounts" }
    fn critical(&self) -> bool { true }
}
```

### Durability Metrics

```rust
pub struct DurabilityMetrics {
    // Thread metrics
    pub total_threads: Counter,
    pub durable_threads: Counter,
    pub non_durable_threads: Counter,

    // Execution metrics
    pub successful_executions: Counter,
    pub failed_executions: Counter,
    pub replay_attempts: Counter,
    pub replay_successes: Counter,

    // Nonce metrics
    pub active_nonces: Gauge,
    pub nonce_advances: Counter,
    pub nonce_failures: Counter,

    // Queue metrics
    pub replay_queue_depth: Gauge,
    pub dead_letter_queue_depth: Gauge,

    // Latency metrics
    pub execution_latency: Histogram,
    pub replay_latency: Histogram,
}
```

## Best Practices

### Choosing Durability Level

1. **Start with Non-Durable**
   - Simpler implementation
   - Lower costs
   - Suitable for most use cases

2. **Upgrade to Durable When**
   - Execution intervals > 5 minutes
   - Critical financial operations
   - Need guaranteed execution
   - Cross-timezone operations

### Nonce Account Management

1. **One Nonce Per Thread**
   - Isolates failures
   - Simplifies management
   - Clear ownership

2. **Regular Maintenance**
   - Monitor account balances
   - Clean up unused nonces
   - Rotate authorities periodically

### Replay Strategy

1. **Progressive Delays**
   - Start with short delays
   - Increase exponentially
   - Cap at reasonable maximum

2. **Priority-Based Replay**
   - Critical threads first
   - Value-based ordering
   - Resource allocation

### Monitoring and Alerting

1. **Key Metrics**
   - Execution success rate
   - Replay queue depth
   - Nonce account health
   - Dead letter queue size

2. **Alert Thresholds**
   - DLQ > 10 entries
   - Success rate < 95%
   - Replay failures > 5/hour
   - Invalid nonces > 0

## Conclusion

Antegen's durability and reliability features provide multiple layers of protection against transaction failures. By choosing the appropriate durability model, implementing proper retry strategies, and maintaining robust monitoring, developers can build automation systems that operate reliably even under challenging network conditions. The combination of nonce accounts for durability, message queue-based replay for recovery (when implemented), and comprehensive failure handling ensures that critical automated transactions execute successfully.