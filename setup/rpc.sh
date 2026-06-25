#!/bin/bash
# agave-validator v4.x launch script (Antegen RPC worker).
# v4 notes:
#   - In-memory accounts index is the default; --enable-accounts-disk-index and
#     --accounts-index-path were dropped (disk index deprecated in v4).
#   - Snapshot Direct-I/O is on by default in v4. If /mnt/accounts storage rejects
#     O_DIRECT on snapshot load, add: --no-accounts-db-snapshots-direct-io
#
# NUMA: dual-socket host (2 nodes, ~128 GB each). Don't cpunodebind the
# validator to one node -- the accounts working set outgrows 128 GB and agave
# wants all cores. Interleave memory across both nodes instead, so neither node
# fills and memory bandwidth stays balanced.
exec numactl --interleave=all /home/sol/.cargo/bin/agave-validator \
    --identity /home/sol/rpc-keypair.json \
    --no-voting \
    --ledger /mnt/ledger \
    --accounts /mnt/accounts \
    --rpc-port 8899 \
    --full-rpc-api \
    --only-known-rpc \
    --private-rpc \
    --gossip-port 8001 \
    --dynamic-port-range 8000-8025 \
    --known-validator 7Np41oeYqPefeNQEHSv1UDhYrehxin3NStELsSKCT4K2 \
    --known-validator GdnSyH3YtwcxFvQrVVJMm1JhTS4QVX7MFsX56uJLUfiZ \
    --known-validator DE1bawNcRJB9rVm3buyMVfr8mBEoyyu73NBovf2oXJsJ \
    --known-validator CakcnaRDHka2gXyfbEd2d3xsvkJkqsLw2akB3zsN1D2S \
    --entrypoint entrypoint.mainnet-beta.solana.com:8001 \
    --entrypoint entrypoint2.mainnet-beta.solana.com:8001 \
    --entrypoint entrypoint3.mainnet-beta.solana.com:8001 \
    --entrypoint entrypoint4.mainnet-beta.solana.com:8001 \
    --entrypoint entrypoint5.mainnet-beta.solana.com:8001 \
    --expected-genesis-hash 5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d \
    --wal-recovery-mode skip_any_corrupted_record \
    --limit-ledger-size 50000000 \
    --use-snapshot-archives-at-startup when-newest \
    --block-verification-method unified-scheduler \
    --unified-scheduler-handler-threads 12 \
    --accounts-db-cache-limit-mb 8192 \
    --skip-startup-ledger-verification \
    --geyser-plugin-config /home/sol/geyser-config.json \
    --log /home/sol/log/agave-validator.log
