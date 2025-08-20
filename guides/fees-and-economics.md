# Fees and Economics

Antegen implements a sophisticated fee system designed to incentivize timely thread execution while ensuring sustainable economics for all participants. This guide explains the fee structure, timing economics, and optimization strategies.

## Fee Structure Overview

The Antegen fee system operates on three key principles:
1. **Time-based Commission Decay**: Earlier execution = higher rewards
2. **Distributed Fee Model**: Fees split between multiple stakeholders
3. **Configurable Parameters**: Adjustable for network conditions

## Commission Components

### Base Commission Fee
The foundation of the fee system is a base commission fee configured globally:

```rust
pub struct ThreadConfig {
    pub commission_fee: u64,      // Base fee in lamports (e.g., 1,000,000 = 0.001 SOL)
    pub executor_fee_bps: u64,    // Executor's share in basis points (e.g., 8000 = 80%)
    pub core_team_bps: u64,       // Core team's share in basis points (e.g., 1000 = 10%)
    // ... remaining goes to thread authority (10% in this example)
}
```

### Fee Distribution
The base commission is distributed among three parties:

1. **Executor Fee**: Compensation for thread execution service
2. **Core Team Fee**: Protocol development and maintenance
3. **Thread Authority**: Remainder stays with thread owner

**Example Distribution:**
- Base Commission: 0.001 SOL (1,000,000 lamports)
- Executor (80%): 0.0008 SOL (800,000 lamports)  
- Core Team (10%): 0.0001 SOL (100,000 lamports)
- Thread Authority (10%): 0.0001 SOL (100,000 lamports)

## Time-Based Commission Decay

The innovative aspect of Antegen's economics is time-based commission decay, which incentivizes prompt execution.

### Timing Phases

#### 1. Grace Period
- **Duration**: Configurable (e.g., 300 seconds = 5 minutes)
- **Commission**: 100% of base commission
- **Purpose**: Allow for normal network and processing delays

#### 2. Decay Period  
- **Duration**: Configurable (e.g., 1800 seconds = 30 minutes)
- **Commission**: Linear decay from 100% to 0%
- **Formula**: `commission_multiplier = 1.0 - (time_in_decay / decay_duration)`

#### 3. Late Execution
- **Duration**: After grace + decay period
- **Commission**: 0% (no executor fee)
- **Purpose**: Discourage extremely late execution

### Commission Calculation

```rust
fn calculate_commission_multiplier(
    trigger_ready_time: i64,
    execution_time: i64,
    grace_period: i64,
    decay_period: i64,
) -> f64 {
    let time_since_ready = execution_time - trigger_ready_time;
    
    if time_since_ready <= grace_period {
        // Within grace period: full commission
        1.0
    } else if time_since_ready <= grace_period + decay_period {
        // Within decay period: linear decay
        let time_into_decay = (time_since_ready - grace_period) as f64;
        let decay_progress = time_into_decay / decay_period as f64;
        1.0 - decay_progress
    } else {
        // Late execution: no commission
        0.0
    }
}
```

### Example Timeline

**Configuration:**
- Grace Period: 300 seconds (5 minutes)
- Decay Period: 1800 seconds (30 minutes)  
- Base Commission: 0.001 SOL

**Execution Scenarios:**

| Execution Time | Phase | Commission Multiplier | Executor Fee |
|---|---|---|---|
| 0-300s | Grace | 100% | 0.0008 SOL |
| 600s | Decay | 85.7% | 0.000686 SOL |
| 1200s | Decay | 57.1% | 0.000457 SOL |
| 1800s | Decay | 28.6% | 0.000229 SOL |
| 2100s+ | Late | 0% | 0 SOL |

## Executor Commission Options

### Standard Commission
Executors receive their configured percentage of the effective commission:

```rust
let executor_fee = (effective_commission * config.executor_fee_bps) / 10_000;
```

### Forgo Commission
Executors can choose to forgo their commission, with benefits going to the thread authority:

```rust
pub fn exec_thread(ctx: Context<ThreadExec>, forgo_commission: bool) -> Result<()> {
    let executor_fee = if forgo_commission {
        0 // Executor forgoes commission
    } else {
        (effective_commission * config.executor_fee_bps) / 10_000
    };
    
    if forgo_commission && effective_commission > 0 {
        msg!("Executor forgoing commission: {} lamports retained by thread", 
            (effective_commission * config.executor_fee_bps) / 10_000);
    }
}
```

### Strategic Use of Forgo Commission
- **Ecosystem Building**: Support early adopters by reducing their costs
- **Competitive Advantage**: Attract users with lower execution fees
- **Network Participation**: Contribute to network health during low-activity periods
- **Testing**: Reduce costs during development and testing

## Thread Funding Economics

### Minimum Funding Requirements

Threads must maintain sufficient balance for:
1. **Rent Exemption**: Account storage costs (~0.00203 SOL for thread accounts)
2. **Transaction Fees**: Network transaction costs (~0.000005 SOL per transaction)
3. **Commission Fees**: Execution commission costs (configurable)
4. **Buffer**: Additional funds for multiple executions

### Funding Calculation Example
```rust
// Calculate minimum thread funding
let rent_exemption = 2_039_280; // ~0.00204 SOL
let tx_fee_per_execution = 5_000; // ~0.000005 SOL  
let commission_per_execution = 1_000_000; // ~0.001 SOL
let expected_executions = 100;

let minimum_funding = rent_exemption + 
    (tx_fee_per_execution + commission_per_execution) * expected_executions;

// Result: ~0.102 SOL for 100 executions
```

### Fund Management
```rust
// Check if thread needs refunding
fn needs_refunding(thread: &Thread, min_balance: u64) -> bool {
    thread.lamports() < min_balance
}

// Refund thread account
thread_program::fund_thread(ctx, additional_amount)?;

// Monitor thread balance
fn calculate_remaining_executions(
    current_balance: u64,
    cost_per_execution: u64
) -> u64 {
    current_balance / cost_per_execution
}
```

## Economic Optimization Strategies

### For Thread Creators

#### 1. Timing Optimization
- **Schedule during low-congestion periods** to increase execution promptness
- **Use skippable intervals** for non-critical timing to reduce late execution risk
- **Set realistic timing expectations** based on network conditions

#### 2. Fee Management
- **Monitor thread balances** regularly to prevent execution failures
- **Batch operations** when possible to reduce per-execution overhead
- **Use appropriate commission settings** based on execution criticality

#### 3. Trigger Selection
- **Choose appropriate trigger types** to balance functionality and cost
- **Avoid high-frequency triggers** unless necessary for business requirements
- **Use account triggers judiciously** as they may have higher monitoring costs

### For Executors

#### 1. Execution Timing Strategy
- **Prioritize high-value threads** within grace period for maximum commission
- **Monitor network conditions** to optimize execution timing
- **Use multiple execution threads** to handle concurrent opportunities

#### 2. Infrastructure Optimization
- **Deploy close to validators** to minimize network latency
- **Use multiple RPC endpoints** to ensure reliable connectivity
- **Implement efficient queue processing** to maximize throughput

#### 3. Commission Strategy
- **Balance forgo commission usage** to attract users while maintaining profitability
- **Monitor fee decay patterns** to optimize execution scheduling
- **Track performance metrics** to identify optimization opportunities

## Network Economics Impact

### Validator Revenue
- **Transaction fees** from thread executions provide additional validator revenue
- **Increased network activity** from automated transactions
- **Potential for premium execution services** through TPU integration

### Network Utilization
- **Predictable load patterns** from scheduled executions
- **Distributed execution timing** reduces network congestion spikes
- **Efficient resource usage** through batched operations

### Economic Sustainability
- **Self-funding ecosystem** where fees support continued operation
- **Incentive alignment** between all participants
- **Growth-oriented fee structure** that scales with network adoption

## Fee Configuration Examples

### Conservative Configuration (Low Activity Networks)
```rust
ThreadConfig {
    commission_fee: 500_000,        // 0.0005 SOL
    executor_fee_bps: 7000,         // 70%
    core_team_bps: 1500,           // 15%
    grace_period_seconds: 600,      // 10 minutes
    fee_decay_seconds: 3600,       // 60 minutes
}
```

### Standard Configuration (Mainnet)
```rust
ThreadConfig {
    commission_fee: 1_000_000,      // 0.001 SOL
    executor_fee_bps: 8000,         // 80%  
    core_team_bps: 1000,           // 10%
    grace_period_seconds: 300,      // 5 minutes
    fee_decay_seconds: 1800,       // 30 minutes
}
```

### High-Performance Configuration (Trading/DeFi)
```rust
ThreadConfig {
    commission_fee: 2_000_000,      // 0.002 SOL
    executor_fee_bps: 8500,         // 85%
    core_team_bps: 500,            // 5%
    grace_period_seconds: 60,       // 1 minute
    fee_decay_seconds: 600,        // 10 minutes
}
```

## Monitoring and Analytics

### Key Metrics to Track

#### Thread Owner Metrics
- **Average execution latency** (time from trigger ready to execution)
- **Commission multiplier distribution** (percentage of executions in each phase)
- **Cost per execution** (total fees divided by executions)
- **Thread balance utilization** (how long funding lasts)

#### Executor Metrics
- **Commission earned per period** (total fees collected)
- **Execution success rate** (successful vs. failed executions)
- **Average commission multiplier** (timing performance indicator)
- **Thread processing throughput** (executions per unit time)

#### Network Metrics
- **Total thread executions** (network activity indicator)
- **Average commission fees** (economic health indicator)
- **Fee decay distribution** (execution timing patterns)
- **Executor participation** (number of active executors)

### Economic Health Indicators

#### Healthy Economics
- High percentage of executions within grace period (>80%)
- Consistent executor participation
- Growing thread creation rate
- Stable or decreasing average execution latency

#### Warning Signs
- High percentage of late executions (>20% with 0% commission)
- Declining executor participation
- Threads frequently running out of funds
- Increasing average execution latency

## Future Economic Considerations

### Dynamic Fee Adjustment
- **Network congestion-based pricing** to optimize resource allocation
- **Seasonal fee adjustment** for predictable demand patterns
- **Performance-based executor rewards** for consistent high-quality service

### Advanced Fee Models
- **Priority execution tiers** with different fee structures
- **Volume discounts** for high-frequency thread creators
- **Staking-based fee reductions** for long-term network participants

### Cross-Chain Economics
- **Multi-chain execution cost comparison** for optimal chain selection
- **Cross-chain arbitrage opportunities** for executors
- **Unified fee tokens** across different blockchain networks

Understanding Antegen's fee structure and economic incentives is crucial for optimizing both thread creation costs and executor profitability. The time-based decay system creates a natural balance between execution quality and cost, while the distributed fee model ensures sustainable network economics for all participants.