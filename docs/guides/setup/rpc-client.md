# RPC Client Setup Guide

This guide covers setting up Antegen with RPC-based event monitoring, suitable for environments without validator access.

## Overview

The RPC client setup provides:
- Standalone operation without validator
- Polling-based event detection
- Flexible deployment options
- Cloud-friendly architecture

## When to Use RPC Client

Choose RPC setup when:
- You don't run a validator
- Testing/development environments
- Cloud deployments (AWS, GCP, Azure)
- Need geographic distribution
- Lower operational complexity required

## Architecture

```
┌────────────────────────────────────────┐
│          RPC/WebSocket Endpoint        │
│         (Helius, Triton, QuickNode)    │
└────────────────────────────────────────┘
                    │
                    ▼
┌────────────────────────────────────────┐
│            Observer Service            │
│         (Polling & Subscriptions)      │
└────────────────────────────────────────┘
                    │
                    ▼
┌────────────────────────────────────────┐
│            Executor Service            │
│         (Queue & Retry Logic)          │
└────────────────────────────────────────┘
                    │
                    ▼
┌────────────────────────────────────────┐
│           Submitter Service            │
│            (RPC Submission)            │
└────────────────────────────────────────┘
```

## Prerequisites

### System Requirements

- **CPU**: 2+ cores
- **RAM**: 4GB minimum, 8GB recommended
- **Storage**: 20GB for queues and logs
- **Network**: Stable internet connection

### RPC Provider Requirements

- Mainnet/Devnet/Testnet endpoint
- WebSocket support (optional but recommended)
- Sufficient rate limits for your usage
- GetProgramAccounts method access

## Installation

### Step 1: Install Antegen

```bash
# From source
git clone https://github.com/wuwei-labs/antegen
cd antegen
cargo build --release

# Install binaries
cargo install --path crates/cli
cargo install --path crates/processor

# Or download pre-built binaries
curl -L https://github.com/wuwei-labs/antegen/releases/latest/download/antegen-linux-amd64.tar.gz | tar xz
sudo mv antegen /usr/local/bin/
```

### Step 2: Configure RPC Endpoints

Create configuration file `~/.antegen/config.toml`:

```toml
[network]
cluster = "mainnet-beta"  # or "devnet", "testnet"

[rpc]
# Primary RPC endpoint
primary_url = "https://api.mainnet-beta.solana.com"

# Backup endpoints for failover
backup_urls = [
    "https://solana-api.projectserum.com",
    "https://rpc.ankr.com/solana"
]

# WebSocket endpoint (optional but recommended)
ws_url = "wss://api.mainnet-beta.solana.com"

# Rate limiting
max_requests_per_second = 10
request_timeout_ms = 30000

[executor]
# Keypair for transaction signing
keypair_path = "~/.antegen/executor-keypair.json"

# Execution settings
max_concurrent_threads = 100
retry_attempts = 3
retry_delay_ms = 1000

# Commission settings
forgo_commission = false

[observer]
# Polling configuration
poll_interval_ms = 2000
batch_size = 100

# Subscription settings (if WebSocket available)
enable_subscriptions = true
max_subscriptions = 1000

[submitter]
# Submission strategy
mode = "rpc"  # "rpc", "tpu", or "tpu_with_fallback"
preflight_commitment = "processed"
skip_preflight = false

[storage]
# Data directory for queues and state
data_dir = "~/.antegen/data"

# Queue settings
max_queue_size = 10000
enable_persistence = true
```

### Step 3: Setup Executor Keypair

```bash
# Generate new keypair
solana-keygen new --outfile ~/.antegen/executor-keypair.json

# Or use existing keypair
cp ~/.config/solana/id.json ~/.antegen/executor-keypair.json

# Fund the executor account
solana transfer ~/.antegen/executor-keypair.json 1.0 --url https://api.mainnet-beta.solana.com

# Secure the keypair
chmod 600 ~/.antegen/executor-keypair.json
```

## Running the Services

### Option 1: All-in-One Process

```bash
# Run combined Observer + Executor + Submitter
antegen run \
  --config ~/.antegen/config.toml \
  --log-level info
```

### Option 2: Separate Services

```bash
# Terminal 1: Observer Service
antegen observer \
  --rpc-url https://api.mainnet-beta.solana.com \
  --ws-url wss://api.mainnet-beta.solana.com \
  --poll-interval 2000

# Terminal 2: Executor Service
antegen executor \
  --keypair ~/.antegen/executor-keypair.json \
  --data-dir ~/.antegen/data \
  --max-threads 100

# Terminal 3: Submitter Service
antegen submitter \
  --rpc-url https://api.mainnet-beta.solana.com \
  --mode rpc
```

### Option 3: Docker Deployment

```yaml
# docker-compose.yml
version: '3.8'

services:
  antegen:
    image: antegen/antegen:latest
    volumes:
      - ./config.toml:/app/config.toml
      - ./executor-keypair.json:/app/executor-keypair.json
      - antegen-data:/app/data
    environment:
      - RUST_LOG=info
      - ANTEGEN_CONFIG=/app/config.toml
    restart: unless-stopped
    networks:
      - antegen-net

  # Optional: NATS for replay
  nats:
    image: nats:2.10-alpine
    command: "-js -sd /data"
    volumes:
      - nats-data:/data
    ports:
      - "4222:4222"
      - "8222:8222"
    networks:
      - antegen-net

volumes:
  antegen-data:
  nats-data:

networks:
  antegen-net:
```

Run with Docker Compose:

```bash
docker-compose up -d
docker-compose logs -f antegen
```

## RPC Provider Configuration

### Helius

```toml
[rpc]
primary_url = "https://mainnet.helius-rpc.com/?api-key=YOUR_API_KEY"
ws_url = "wss://mainnet.helius-rpc.com/?api-key=YOUR_API_KEY"
max_requests_per_second = 100  # Depends on plan
```

### QuickNode

```toml
[rpc]
primary_url = "https://YOUR-ENDPOINT.quiknode.pro/YOUR-KEY/"
ws_url = "wss://YOUR-ENDPOINT.quiknode.pro/YOUR-KEY/"
max_requests_per_second = 150  # Depends on plan
```

### Triton One

```toml
[rpc]
primary_url = "https://YOUR-PROJECT.triton.one/solana-mainnet/YOUR-KEY"
ws_url = "wss://YOUR-PROJECT.triton.one/solana-mainnet/YOUR-KEY"
max_requests_per_second = 200  # Depends on plan
```

### GenesysGo

```toml
[rpc]
primary_url = "https://ssc-dao.genesysgo.net"
ws_url = "wss://ssc-dao.genesysgo.net/ws"
max_requests_per_second = 50
```

## Performance Optimization

### Polling Optimization

```toml
[observer]
# Adjust based on your needs and RPC limits
poll_interval_ms = 1000  # Faster polling for time-sensitive
batch_size = 200         # Larger batches for efficiency

# Filter unnecessary accounts
filter_programs = [
    "AgThdyi1P5RkVeZD2rQahTvs8HePJoGFFxKtvok5s2J1"  # Thread program only
]

# Cache configuration
enable_cache = true
cache_ttl_seconds = 60
cache_size_mb = 100
```

### WebSocket Subscriptions

```toml
[observer]
# Prefer WebSocket over polling when available
enable_subscriptions = true
subscription_commitment = "confirmed"

# Reconnection settings
reconnect_delay_ms = 1000
max_reconnect_attempts = 10

# Heartbeat to keep connection alive
heartbeat_interval_ms = 30000
```

### Connection Pooling

```toml
[rpc]
# Connection pool settings
connection_pool_size = 10
keepalive_interval_ms = 60000
connect_timeout_ms = 10000

# HTTP/2 settings (if supported)
enable_http2 = true
max_concurrent_streams = 100
```

## Multi-RPC Configuration

### Load Balancing

```toml
[rpc]
# Round-robin across multiple endpoints
urls = [
    "https://api.mainnet-beta.solana.com",
    "https://solana-api.projectserum.com",
    "https://rpc.ankr.com/solana"
]

strategy = "round_robin"  # or "least_latency", "weighted"

# Health checking
health_check_interval_ms = 30000
unhealthy_threshold = 3
```

### Failover Configuration

```toml
[rpc.failover]
enabled = true
primary_url = "https://primary-rpc.example.com"

# Fallback endpoints in priority order
fallback_urls = [
    "https://backup1-rpc.example.com",
    "https://backup2-rpc.example.com"
]

# Failover triggers
max_consecutive_errors = 3
error_rate_threshold = 0.1  # 10% error rate
latency_threshold_ms = 5000

# Recovery settings
recovery_check_interval_ms = 60000
```

## Monitoring

### Metrics Collection

```toml
[metrics]
enabled = true
port = 9090
endpoint = "/metrics"

# Prometheus format
format = "prometheus"

# Custom labels
labels = {
    environment = "production",
    region = "us-east-1"
}
```

Key metrics to monitor:

```bash
# RPC metrics
antegen_rpc_requests_total
antegen_rpc_request_duration_seconds
antegen_rpc_errors_total
antegen_rpc_rate_limit_hits_total

# Observer metrics
antegen_observer_polls_total
antegen_observer_threads_discovered_total
antegen_observer_events_queued_total

# Executor metrics
antegen_executor_threads_processed_total
antegen_executor_queue_depth
antegen_executor_processing_duration_seconds

# Submitter metrics
antegen_submitter_transactions_sent_total
antegen_submitter_confirmations_total
antegen_submitter_failures_total
```

### Logging Configuration

```toml
[logging]
# Log level: trace, debug, info, warn, error
level = "info"

# Output format
format = "json"  # or "pretty", "compact"

# File output
file_path = "/var/log/antegen/antegen.log"
max_file_size_mb = 100
max_files = 10

# Filters
filters = [
    "antegen=info",
    "solana_client=warn"
]
```

### Health Checks

Create health check endpoint:

```bash
# health-check.sh
#!/bin/bash

# Check RPC connection
if ! curl -s https://api.mainnet-beta.solana.com/health > /dev/null; then
    echo "RPC unreachable"
    exit 1
fi

# Check Antegen process
if ! pgrep -x antegen > /dev/null; then
    echo "Antegen not running"
    exit 1
fi

# Check queue depth
QUEUE_DEPTH=$(curl -s localhost:9090/metrics | grep queue_depth | awk '{print $2}')
if [ "$QUEUE_DEPTH" -gt 1000 ]; then
    echo "Queue backlog detected: $QUEUE_DEPTH"
    exit 1
fi

echo "All systems operational"
```

## Cost Optimization

### RPC Usage Reduction

```toml
[observer]
# Intelligent caching
cache_strategy = "aggressive"
cache_invalidation = "smart"

# Batch operations
batch_requests = true
max_batch_size = 100

# Skip unchanged data
enable_change_detection = true
change_detection_interval_ms = 5000

# Use compression
enable_compression = true
compression_type = "gzip"
```

### Selective Monitoring

```toml
[observer.filters]
# Only monitor active threads
active_only = true

# Filter by trigger type
trigger_types = ["interval", "cron", "timestamp"]

# Filter by authority (monitor your own threads)
authorities = ["YourPubkey..."]

# Minimum thread balance to monitor
min_balance_lamports = 1000000
```

## Troubleshooting

### High RPC Costs

```bash
# Monitor RPC usage
curl localhost:9090/metrics | grep rpc_requests

# Reduce polling frequency
sed -i 's/poll_interval_ms = 1000/poll_interval_ms = 5000/' config.toml

# Enable aggressive caching
sed -i 's/enable_cache = false/enable_cache = true/' config.toml
```

### Connection Issues

```bash
# Test RPC endpoint
curl -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' \
  https://api.mainnet-beta.solana.com

# Check DNS resolution
nslookup api.mainnet-beta.solana.com

# Test WebSocket
wscat -c wss://api.mainnet-beta.solana.com
```

### Slow Performance

```bash
# Increase batch size
antegen config set observer.batch_size 500

# Enable connection pooling
antegen config set rpc.connection_pool_size 20

# Use geographically closer RPC
antegen config set rpc.primary_url "https://closest-rpc.example.com"
```

### Queue Backlog

```bash
# Check queue status
antegen queue status

# Clear stuck items
antegen queue clear --older-than 1h

# Increase executor threads
antegen config set executor.max_concurrent_threads 200
```

## Security Best Practices

### Keypair Security

```bash
# Encrypt keypair at rest
gpg --symmetric --cipher-algo AES256 executor-keypair.json

# Use environment variable for keypair
export ANTEGEN_KEYPAIR=$(cat executor-keypair.json | base64)

# Rotate keypairs regularly
antegen keypair rotate --old old-keypair.json --new new-keypair.json
```

### Network Security

```toml
[security]
# Use TLS for all connections
require_tls = true
tls_verify = true

# API key management
api_key_env_var = "ANTEGEN_API_KEY"

# Rate limiting per IP
enable_rate_limiting = true
rate_limit_per_minute = 100

# IP whitelisting (optional)
allowed_ips = ["192.168.1.0/24"]
```

### Monitoring Security

```bash
# Set up alerts
cat > alerts.yaml << EOF
alerts:
  - name: unauthorized_access
    condition: http_403_errors > 0
    action: notify_security

  - name: high_error_rate
    condition: error_rate > 0.05
    action: page_oncall

  - name: keypair_balance_low
    condition: executor_balance < 0.1
    action: notify_finance
EOF
```

## Production Deployment Checklist

- [ ] RPC endpoint configured and tested
- [ ] WebSocket endpoint configured (if available)
- [ ] Executor keypair created and funded
- [ ] Configuration file validated
- [ ] Systemd service configured (Linux)
- [ ] Docker containers built (if using Docker)
- [ ] Monitoring endpoints exposed
- [ ] Health checks configured
- [ ] Log rotation set up
- [ ] Backup RPC endpoints configured
- [ ] Rate limiting configured appropriately
- [ ] Security measures implemented
- [ ] Alert rules configured
- [ ] Documentation updated

## Maintenance

### Regular Tasks

```bash
# Check executor balance daily
0 0 * * * /usr/local/bin/check-balance.sh

# Clean old queue data weekly
0 2 * * 0 antegen queue clean --older-than 7d

# Rotate logs monthly
0 3 1 * * /usr/sbin/logrotate -f /etc/logrotate.d/antegen

# Update RPC endpoints quarterly
0 0 1 */3 * /usr/local/bin/update-rpc-endpoints.sh
```

### Upgrading Antegen

```bash
# Stop service
systemctl stop antegen

# Backup configuration
cp -r ~/.antegen ~/.antegen.bak

# Update binaries
cargo install --force --path crates/cli
cargo install --force --path crates/processor

# Migrate configuration if needed
antegen migrate-config

# Start service
systemctl start antegen

# Verify version
antegen --version
```

## Support Resources

- GitHub: [github.com/wuwei-labs/antegen](https://github.com/wuwei-labs/antegen)
- Discord: [discord.gg/antegen](https://discord.gg/antegen)
- RPC Provider Docs:
  - [Helius](https://docs.helius.xyz)
  - [QuickNode](https://www.quicknode.com/docs)
  - [Triton](https://docs.triton.one)