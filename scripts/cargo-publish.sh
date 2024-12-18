#!/bin/bash

# Publish shared libs
cargo publish -p solana-cron
sleep 25
cargo publish -p antegen-macros
sleep 25
cargo publish -p antegen-utils
sleep 25

# Publish programs
cargo publish -p antegen-network-program
sleep 25
cargo publish -p antegen-thread-program
sleep 25

# Publish SDK
cargo publish -p antegen-sdk
sleep 25

# Publish downstream bins and libs
# These are most likely to fail due to Anchor dependency issues.
cargo publish -p antegen-client
sleep 25
cargo publish -p antegen-cli
