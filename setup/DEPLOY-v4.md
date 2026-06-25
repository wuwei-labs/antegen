# Antegen RPC Worker — agave-validator v4 Deployment (new host)

Bring up the Antegen RPC worker from a **fresh Linux host** on agave-validator
**v4.x**. Built from the working v3.1.9 process (`history.txt`), updated for v4.

Run as user `sol` unless a step says `sudo`. Target layout:

| Path | Purpose |
|------|---------|
| `/home/sol` | keypairs, scripts, `libantegen_plugin.so`, `geyser-config.json` |
| `/home/sol/log` | validator log |
| `/mnt/ledger` | ledger (fast NVMe) |
| `/mnt/accounts` | accounts (fast NVMe) |

> **Prerequisite:** the `feat/geyser-4.0.2` branch must exist with the geyser interface
> bumped to v4 (`agave-geyser-plugin-interface = "=4.0.x"` in the workspace
> `Cargo.toml`). Step 6 clones that branch and builds both the plugin
> (`libantegen_plugin.so`) and the `antegen` CLI locally on this host.

---

## 0. OS prep & user

Ubuntu/Debian assumed. Create the `sol` service user and base packages:

```bash
sudo adduser --disabled-password --gecos "" sol      # skip if user exists
sudo apt update
sudo apt install -y libssl-dev libudev-dev pkg-config zlib1g-dev llvm clang \
  cmake make libprotobuf-dev protobuf-compiler libclang-dev \
  build-essential curl wget git ncdu htop
```

Switch to the service user for everything below: `sudo -iu sol`.

## 1. Provision storage

Disk layout for this host (1× 500GB + 2× 4TB NVMe):

| Disk | Use |
|------|-----|
| 500GB NVMe | OS root (`/`) + 32G swap file |
| 4TB NVMe #1 | `/mnt/ledger` |
| 4TB NVMe #2 | `/mnt/accounts` |

The OS already lives on the 500GB, so the swap file sits on `/` — no separate swap
partition needed. Format and mount **one 4TB disk for ledger and the other for
accounts** (keeping them on separate devices avoids I/O contention). Adjust device
names to what `lsblk` reports.

```bash
# Inspect available disks FIRST — pick the large, unmounted NVMe volumes.
# TYPE=disk rows are devices; empty FSTYPE/MOUNTPOINT = safe to format.
# Never touch the disk mounted at "/" (the OS disk).
lsblk -o NAME,SIZE,TYPE,FSTYPE,MOUNTPOINT,MODEL

# Format + mount the two 4TB disks (REPLACE device names with the ones from lsblk;
# mkfs ERASES the target — confirm each is a 4TB disk, empty and unmounted, NOT the
# 500GB OS disk).
sudo mkfs.ext4 /dev/nvme1n1 && sudo mkdir -p /mnt/ledger      # 4TB #1 -> ledger
sudo mkfs.ext4 /dev/nvme2n1 && sudo mkdir -p /mnt/accounts    # 4TB #2 -> accounts
sudo mount /dev/nvme1n1 /mnt/ledger
sudo mount /dev/nvme2n1 /mnt/accounts
sudo chown -R sol:sol /mnt/ledger /mnt/accounts

# 32G swap file on the 500GB OS disk (/ is already there)
sudo fallocate -l 32G /swapfile
sudo chmod 600 /swapfile
sudo mkswap /swapfile
sudo swapon /swapfile
```

> **`/mnt/accounts` is always required.** In-memory index only moves the accounts
> *index* into RAM — the account *data* (AccountsDb storages) still lives on this disk.
> Provision it on fast NVMe regardless. If in-memory index proves unstable or RAM-tight
> (see step 8 fallback), the disk-backed index reuses this same volume:
> `mkdir -p /mnt/accounts/index && sudo chown sol:sol /mnt/accounts/index`

Persist the mounts in `/etc/fstab`. Get the UUIDs with `blkid` (or
`lsblk -o NAME,UUID`), then replace the placeholders below:

```bash
blkid /dev/nvme1n1 /dev/nvme2n1     # copy each UUID into the lines below
```

```fstab
# /etc/fstab — append these lines
UUID=<LEDGER-DISK-UUID>     /mnt/ledger     ext4   defaults,noatime   0  2
UUID=<ACCOUNTS-DISK-UUID>   /mnt/accounts   ext4   defaults,noatime   0  2
/swapfile                   none            swap   sw                 0  0
```

Verify the fstab entries mount cleanly before relying on a reboot:

```bash
sudo systemctl daemon-reload
sudo mount -a        # must return with no errors
swapon --show        # confirms /swapfile active
```

## 2. Rust toolchain

```bash
curl https://sh.rustup.rs -sSf | sh
source $HOME/.cargo/env
rustup component add rustfmt
```

## 3. Install agave v4

```bash
cargo install agave-validator@4.0.2          # use latest published 4.0.x
# or: sh -c "$(curl -sSfL https://release.anza.xyz/v4.0.2/install)"
agave-validator --version                     # expect 4.0.x
```

(Patch notes reference `4.0.0-rc.x`; `4.0.2` is the latest published crate — confirm
whatever version actually resolves on the host. `cargo install agave-validator@3.1.9`
re-installs the old binary if rollback is ever needed.)

## 4. Host tuning (community tuner)

There is **no official "online tuner"** for agave. Apply the community tuning from
[`sonicfromnewyoke/solana-rpc`](https://github.com/sonicfromnewyoke/solana-rpc)
(no install script — copy/run the config blocks from its README):

- **sysctl** → `/etc/sysctl.d/21-agave-validator.conf`: UDP/TCP buffers
  (`net.core.rmem_max=net.core.wmem_max=134217728`), connection backlogs, **BBR**
  congestion control, `vm.max_map_count=1000000`, `fs.nr_open=1000000`, low swappiness.
  Apply: `sudo sysctl -p /etc/sysctl.d/21-agave-validator.conf`
- **File limits** → `/etc/security/limits.d/90-solana-nofiles.conf`:
  `nofile` and `memlock` = `2000000`.
- **CPU**: governor `performance`, enable Intel/AMD boost.
- **Memory**: disable Transparent Huge Pages, KSM, and NUMA balancing; configure 2MB
  and 1GB hugepages.
- **NVMe** → `/etc/udev/rules.d/60-nvme-scheduler.rules`: I/O scheduler `none`,
  read-ahead `0`.
- **NIC** (e.g. `bond0` / `enp1s0f0` / `enp1s0f1`): ring buffers `4096`, disable
  interrupt coalescing (`ethtool -G ... 4096`, `ethtool -C ... rx-usecs 0`).

Reboot if governor / hugepages settings require it; otherwise reload udev and re-apply
ethtool. (For systemd ≥ a global FD cap, also bump `DefaultLimitNOFILE` in
`/etc/systemd/system.conf` and `sudo systemctl daemon-reload`.)

## 5. Keypairs

The RPC validator gets its **own new identity**; only the **Antegen worker keypair**
(CRNK) is reused from the old host.

```bash
# NEW RPC validator identity for this host (vanity prefix optional)
solana-keygen grind --starts-with SLY:1
mv SLY*.json /home/sol/rpc-keypair.json

# REUSE the existing Antegen worker keypair — copy from the old host / secure backup:
scp old-host:/home/sol/CRNK*.json /home/sol/

chmod 600 /home/sol/rpc-keypair.json /home/sol/CRNK*.json
chown sol:sol /home/sol/rpc-keypair.json /home/sol/CRNK*.json

solana config set --keypair /home/sol/rpc-keypair.json
solana address && solana balance             # fund the new identity if needed
```

> ⚠️ Because the worker keypair is reused, the on-chain worker already exists — **skip
> `antegen worker create` in step 7** and just verify it resolves. Don't run the cranker
> on both hosts at once with this worker keypair (double submissions); stop
> `antegen-crank` on the old host before starting it here.

## 6. Build Antegen from source (plugin + CLI)

Clone the v4 branch and build both the geyser plugin and the `antegen` CLI on this
host:

```bash
cd /home/sol
git clone --branch feat/geyser-4.0.2 https://github.com/wuwei-labs/antegen.git
cd antegen
cargo build --release -p antegen-plugin -p antegen-cli

# Install the built artifacts
cp target/release/libantegen_plugin.so /home/sol/libantegen_plugin.so
sudo cp target/release/antegen /usr/local/bin/antegen
antegen --version
```

Place and edit the geyser config:

```bash
cp setup/geyser-config.json /home/sol/geyser-config.json
```

Edit `/home/sol/geyser-config.json` so `keypath` points at the reused CRNK worker
keypair from step 5; `libpath` stays `/home/sol/libantegen_plugin.so`.

## 7. Verify the worker (already registered)

The worker keypair was reused from the old host, so the on-chain worker **already
exists** — do NOT run `antegen worker create`. Just confirm it resolves:

```bash
antegen worker get 0                          # confirm the existing worker resolves
```

## 8. Launch script

Copy the v4 launch script and make it executable:

```bash
cp setup/rpc.sh /home/sol/rpc.sh
chmod +x /home/sol/rpc.sh
mkdir -p /home/sol/log && chown sol:sol /home/sol/log
```

`setup/rpc.sh` is already v4-ready: in-memory accounts index (no `--*-disk-index`
flags), `unified-scheduler`, geyser plugin config, log to `/home/sol/log`. If
`/mnt/accounts` storage rejects O_DIRECT on snapshot load, add
`--no-accounts-db-snapshots-direct-io` (commented in the script).

**Fallback to disk-backed index** (if in-memory causes OOM / instability): add these
two flags to `rpc.sh` and restart — uses the `/mnt/accounts/index` dir from step 1:

```
    --accounts-index-limit minimal \
    --accounts-index-path /mnt/accounts/index \
```

(`--accounts-index-limit minimal` is the v4 replacement for the removed
`--enable-accounts-disk-index`; the account data on `/mnt/accounts` is unaffected
either way.)

## 9. systemd service

Create `/etc/systemd/system/antegen-crank.service`:

```ini
[Unit]
Description=Antegen RPC Worker (agave-validator v4)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=sol
ExecStart=/home/sol/rpc.sh
Restart=always
RestartSec=5
LimitNOFILE=2000000
# v4 XDP requirement:
AmbientCapabilities=CAP_NET_RAW CAP_NET_ADMIN CAP_BPF CAP_PERFMON

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable antegen-crank.service
```

Optional log rotation → `/etc/logrotate.d/agave-validator` for
`/home/sol/log/agave-validator.log`.

## 10. (Optional) Snapshot bootstrap

To skip a slow genesis catch-up, fetch a recent snapshot before first start:

```bash
mkdir -p /mnt/ledger/remote
python3 snapshot-finder.py --snapshot_path /mnt/ledger/remote   # community script
```

Otherwise the validator downloads from the configured entrypoints on first boot.

## 11. Start & verify

```bash
sudo systemctl start antegen-crank.service
sudo systemctl status antegen-crank.service                  # active (running)

agave-validator --version                                    # 4.0.x
agave-validator --ledger /mnt/ledger monitor                 # catching up / synced
curl http://127.0.0.1:8899 -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}'         # "result":"ok"
grep antegen_plugin /home/sol/log/agave-validator.log        # plugin loaded, no ABI error
antegen worker get 0                                         # worker resolves
sysctl net.core.rmem_max                                     # 134217728
cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor    # performance
```

## 12. Operational notes

- **Reset state** (reclaim disk / clean restart):
  `sudo systemctl stop antegen-crank.service && sudo rm -rf /mnt/ledger/* /mnt/accounts/* && sudo systemctl start antegen-crank.service`
- **Pool**: `antegen pool get 0`, `antegen pool rotate <id>` as needed.
- **v4 flag changes from v3.1.9**: in-memory accounts index is default
  (`--enable-accounts-disk-index` deprecated, `--disable-accounts-disk-index`
  removed — use `--accounts-index-limit minimal` for disk-backed). Snapshot Direct-I/O
  on by default. Removed flags we don't use: `--monitor`, `--use-quic`/`--use-udp`,
  `--tpu-disable-quic`/`--tpu-enable-udp`,
  `--block-verification-method blockstore-processor`.
```
