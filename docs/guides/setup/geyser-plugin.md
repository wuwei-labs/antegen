# Geyser Plugin Setup Guide

This guide walks you through setting up the Antegen Geyser Plugin for real-time thread execution with your Solana validator.

## Overview

The Geyser Plugin provides:
- Zero-latency event detection directly from validator
- Reduced RPC load and costs
- Integrated Observer, Executor, and Submitter services
- Direct TPU access for transaction submission

## Prerequisites

### System Requirements

- **Solana Validator**: Running Agave 2.1+ or compatible
- **CPU**: 4+ cores recommended (for plugin threads)
- **RAM**: 8GB+ available for plugin operations
- **Storage**: 50GB+ for execution queues and logs
- **Network**: Stable connection for transaction submission

### Software Requirements

- Rust 1.75+ (for building)
- Solana CLI 2.1+
- Access to validator configuration

## Installation

### Step 1: Build the Plugin

```bash
# Clone the repository
git clone https://github.com/wuwei-labs/antegen
cd antegen

# Build the plugin with optimizations
cargo build --release --package antegen-geyser-plugin

# The plugin library will be at:
# target/release/libantegen_geyser_plugin.so
```

### Step 2: Create Plugin Configuration

Create a configuration file `antegen-plugin-config.json`:

```json
{
  "libpath": "/path/to/libantegen_geyser_plugin.so",
  "name": "antegen",
  "keypath": "/path/to/executor-keypair.json",
  "thread_count": 10,
  "transaction_timeout_threshold": 150,
  "rpc_url": "http://127.0.0.1:8899",
  "ws_url": "ws://127.0.0.1:8900",
  "data_dir": "/var/lib/antegen",
  "forgo_executor_commission": false,
  "enable_replay": true,
  "nats_url": "nats://localhost:4222",
  "replay_delay_ms": 30000
}
```

### Step 3: Configure Executor Keypair

```bash
# Generate executor keypair
solana-keygen new --outfile /path/to/executor-keypair.json

# Fund the executor account
solana transfer /path/to/executor-keypair.json 1.0

# Set appropriate permissions
chmod 600 /path/to/executor-keypair.json
chown validator:validator /path/to/executor-keypair.json
```

### Step 4: Update Validator Configuration

Add the plugin to your validator startup command:

```bash
solana-validator \
  --identity /path/to/validator-keypair.json \
  --vote-account /path/to/vote-account.json \
  --ledger /path/to/ledger \
  --rpc-port 8899 \
  --dynamic-port-range 8000-8020 \
  --geyser-plugin-config /path/to/antegen-plugin-config.json \
  # ... other validator flags
```

Or add to your existing validator startup script.

## Configuration Options

### Required Parameters

| Parameter | Description | Example |
|-----------|-------------|---------|
| `libpath` | Path to plugin library | `/opt/antegen/libantegen_geyser_plugin.so` |
| `keypath` | Executor keypair path | `/etc/antegen/executor-keypair.json` |
| `rpc_url` | Local RPC endpoint | `http://127.0.0.1:8899` |
| `data_dir` | Data storage directory | `/var/lib/antegen` |

### Optional Parameters

| Parameter | Description | Default |
|-----------|-------------|---------|
| `name` | Plugin instance name | `antegen` |
| `thread_count` | Worker thread count | `10` |
| `transaction_timeout_threshold` | TX timeout (seconds) | `150` |
| `ws_url` | WebSocket endpoint | `ws://127.0.0.1:8900` |
| `forgo_executor_commission` | Skip commission fees | `false` |
| `enable_replay` | Enable NATS replay | `false` |
| `nats_url` | NATS server URL | `nats://localhost:4222` |
| `replay_delay_ms` | Replay delay (ms) | `30000` |

### Environment Variables

You can override configuration using environment variables:

```bash
export ANTEGEN_KEYPATH=/path/to/executor-keypair.json
export ANTEGEN_RPC_URL=http://127.0.0.1:8899
export ANTEGEN_DATA_DIR=/var/lib/antegen
export ANTEGEN_THREAD_COUNT=20
export ANTEGEN_FORGO_EXECUTOR_COMMISSION=false
export ANTEGEN_ENABLE_REPLAY=true
export ANTEGEN_NATS_URL=nats://nats-cluster:4222
```

## Setting Up NATS for Replay (Optional)

### Install NATS Server

```bash
# Download NATS server
curl -L https://github.com/nats-io/nats-server/releases/download/v2.10.0/nats-server-v2.10.0-linux-amd64.zip -o nats-server.zip
unzip nats-server.zip

# Move to system location
sudo mv nats-server-v2.10.0-linux-amd64/nats-server /usr/local/bin/
```

### Configure NATS

Create `/etc/nats/nats-server.conf`:

```conf
# NATS Server Configuration for Antegen

port: 4222
http: 8222

# JetStream configuration
jetstream {
  store_dir: /var/lib/nats/jetstream
  max_mem: 1GB
  max_file: 10GB
}

# Clustering (optional)
cluster {
  name: antegen_cluster
  port: 6222
  routes: [
    nats://node1:6222
    nats://node2:6222
  ]
}

# Security
authorization {
  user: antegen
  password: $2a$11$ENCRYPTED_PASSWORD_HERE
}

# Monitoring
monitoring {
  port: 8222
  trace: true
}
```

### Start NATS Service

```bash
# Create systemd service
sudo cat > /etc/systemd/system/nats.service << EOF
[Unit]
Description=NATS Server
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/nats-server -c /etc/nats/nats-server.conf
Restart=always
RestartSec=5
User=nats
Group=nats

[Install]
WantedBy=multi-user.target
EOF

# Start NATS
sudo systemctl enable nats
sudo systemctl start nats
```

## Monitoring

### Plugin Logs

The plugin logs to the validator's log output. Monitor with:

```bash
# Follow validator logs
tail -f /path/to/validator.log | grep ANTEGEN

# Common log patterns:
# ANTEGEN: Plugin initialized
# OBSERVER: Thread event for [pubkey]
# EXECUTOR: Processing thread [id]
# SUBMITTER: Transaction submitted [signature]
```

### Health Checks

Create a health check script:

```bash
#!/bin/bash
# /usr/local/bin/antegen-health

# Check plugin is loaded
if grep -q "antegen-plugin" /proc/$(pgrep solana-validator)/maps; then
    echo "✓ Plugin loaded"
else
    echo "✗ Plugin not loaded"
    exit 1
fi

# Check executor keypair balance
BALANCE=$(solana balance /path/to/executor-keypair.json | awk '{print $1}')
if (( $(echo "$BALANCE > 0.1" | bc -l) )); then
    echo "✓ Executor funded: $BALANCE SOL"
else
    echo "✗ Low executor balance: $BALANCE SOL"
fi

# Check data directory
if [ -d "/var/lib/antegen" ]; then
    SIZE=$(du -sh /var/lib/antegen | awk '{print $1}')
    echo "✓ Data directory: $SIZE"
else
    echo "✗ Data directory missing"
fi
```

### Metrics Collection

The plugin exposes metrics via the validator's metrics port:

```bash
# Query plugin metrics
curl http://localhost:10000/metrics | grep antegen

# Key metrics:
# antegen_events_processed_total
# antegen_threads_executed_total
# antegen_execution_latency_ms
# antegen_submission_success_rate
```

## Performance Tuning

### Thread Pool Sizing

Calculate optimal thread count:

```bash
# Formula: CPU cores * 2 + 1
CORES=$(nproc)
THREAD_COUNT=$((CORES * 2 + 1))
echo "Recommended thread_count: $THREAD_COUNT"
```

### Memory Allocation

Ensure sufficient memory for the plugin:

```bash
# Check available memory
free -h

# Reserve memory for plugin (in validator systemd service)
[Service]
MemoryMax=64G
MemoryHigh=60G
```

### Network Optimization

```bash
# Increase network buffer sizes
sudo sysctl -w net.core.rmem_max=134217728
sudo sysctl -w net.core.wmem_max=134217728
sudo sysctl -w net.ipv4.tcp_rmem="4096 87380 134217728"
sudo sysctl -w net.ipv4.tcp_wmem="4096 65536 134217728"

# Make permanent
echo "net.core.rmem_max=134217728" >> /etc/sysctl.conf
echo "net.core.wmem_max=134217728" >> /etc/sysctl.conf
```

## Troubleshooting

### Plugin Fails to Load

```bash
# Check library dependencies
ldd /path/to/libantegen_geyser_plugin.so

# Verify configuration JSON
jq . /path/to/antegen-plugin-config.json

# Check file permissions
ls -la /path/to/libantegen_geyser_plugin.so
ls -la /path/to/executor-keypair.json
```

### No Threads Executing

```bash
# Verify plugin is receiving events
grep "GEYSER->OBSERVER" /path/to/validator.log

# Check executor balance
solana balance /path/to/executor-keypair.json

# Verify thread program is deployed
solana program show AgThdyi1P5RkVeZD2rQahTvs8HePJoGFFxKtvok5s2J1
```

### High Memory Usage

```bash
# Check queue depths
ls -la /var/lib/antegen/queues/

# Clear old data
find /var/lib/antegen -type f -mtime +7 -delete

# Restart plugin (requires validator restart)
sudo systemctl restart solana-validator
```

### Transaction Failures

```bash
# Check submission logs
grep "SUBMITTER.*failed" /path/to/validator.log

# Verify TPU connection
netstat -an | grep 8003

# Test RPC fallback
curl -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' \
  http://127.0.0.1:8899
```

## Security Considerations

### Keypair Security

```bash
# Secure executor keypair
chmod 600 /path/to/executor-keypair.json
chown validator:validator /path/to/executor-keypair.json

# Use encrypted filesystem
mount -t ecryptfs /secure /path/to/keys

# Rotate keypairs periodically
antegen rotate-executor --current /path/to/old.json --new /path/to/new.json
```

### Network Security

```bash
# Firewall rules for NATS (if using)
sudo ufw allow from 10.0.0.0/8 to any port 4222
sudo ufw allow from 10.0.0.0/8 to any port 6222

# Restrict RPC access
sudo ufw allow from 127.0.0.1 to any port 8899
```

### Monitoring Security

```bash
# Set up alerts for suspicious activity
cat > /etc/antegen/alerts.yaml << EOF
alerts:
  - name: high_failure_rate
    condition: failure_rate > 0.1
    action: email admin@example.com

  - name: low_executor_balance
    condition: balance < 0.1
    action: email admin@example.com

  - name: queue_overflow
    condition: queue_depth > 10000
    action: restart_service
EOF
```

## Production Deployment Checklist

- [ ] Plugin binary built with `--release` flag
- [ ] Configuration file validated with `jq`
- [ ] Executor keypair funded with sufficient SOL
- [ ] File permissions set correctly (600 for keypair)
- [ ] Data directory created with sufficient space
- [ ] NATS server configured (if using replay)
- [ ] Monitoring and alerting configured
- [ ] Health checks scheduled in cron
- [ ] Backup executor keypair stored securely
- [ ] Network buffers tuned for performance
- [ ] Firewall rules configured
- [ ] Log rotation configured
- [ ] Metrics collection enabled
- [ ] Documentation of configuration

## Maintenance

### Regular Tasks

```bash
# Daily: Check executor balance
0 0 * * * /usr/local/bin/check-executor-balance.sh

# Weekly: Clean old queue data
0 2 * * 0 find /var/lib/antegen -type f -mtime +7 -delete

# Monthly: Rotate logs
0 3 1 * * /usr/sbin/logrotate -f /etc/logrotate.d/antegen
```

### Upgrading the Plugin

```bash
# 1. Build new version
cd antegen && git pull
cargo build --release --package antegen-geyser-plugin

# 2. Stop validator
sudo systemctl stop solana-validator

# 3. Backup current plugin
cp /path/to/libantegen_geyser_plugin.so /path/to/libantegen_geyser_plugin.so.bak

# 4. Install new version
cp target/release/libantegen_geyser_plugin.so /path/to/

# 5. Restart validator
sudo systemctl start solana-validator

# 6. Verify new version
grep "antegen-plugin v" /path/to/validator.log
```

## Support

For additional help:
- GitHub Issues: [github.com/wuwei-labs/antegen/issues](https://github.com/wuwei-labs/antegen/issues)
- Discord: [discord.gg/antegen](https://discord.gg/antegen)
- Documentation: [docs.antegen.io](https://docs.antegen.io)