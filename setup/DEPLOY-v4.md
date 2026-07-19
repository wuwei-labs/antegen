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
| `/mnt/hugepages` | 2 MB hugetlbfs pool (RAM-backed) |

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

Clone the repo now — later steps copy config files (`rpc.sh`, `nic-tune.sh`, the
systemd unit, sysctl/limits drop-ins) straight out of `/home/sol/antegen/setup/`, and
§6 builds the plugin + CLI from it:

```bash
cd /home/sol
git clone --branch feat/geyser-4.0.2 https://github.com/wuwei-labs/antegen.git
```

> All `cp /home/sol/antegen/setup/…` commands below assume this clone. Pull the latest
> before deploying if the branch has moved: `git -C /home/sol/antegen pull`.

## 1. Provision storage

Disk layout for this host (1× 500GB + 2× 4TB NVMe):

| Disk | Use |
|------|-----|
| 500GB NVMe | OS root (`/`) + 64G swap file |
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

# 64G swap file on the 500GB OS disk (/ is already there).
# Cushion for memory spikes — NOT usable capacity (low swappiness, tuning step 4).
# Keep low swappiness (tuning step 4) and leave ~100GB+ free on the OS disk for logs.
sudo fallocate -l 64G /swapfile
sudo chmod 600 /swapfile
sudo mkswap /swapfile
sudo swapon /swapfile
```

> **`/mnt/accounts` holds both the account data and (by default here) the index.**
> The account *data* (AccountsDb storages) always lives on this disk. This setup also
> puts the accounts *index* here (`/mnt/accounts/index`, disk-backed) because the v4
> in-memory index OOM'd ~256 GB RAM on mainnet — see step 8. Create the index dir now:
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

## 1b. Hugepages (2 MB only) & NUMA

Reserve the 2 MB hugepage pool, then mount it. Hugepages set at boot survive reboots.

> **No 1 GB ("gigantic") pool with a disk-backed index.** The earlier setup reserved
> 26 × 1 GB pages (~26 GB) for the accounts index — but with the **disk-backed index**
> (§8) agave never uses them, so that 26 GB is locked away from normal allocation and
> made the validator OOM *sooner* during snapshot load (it died at ~230 GB on a 256 GB
> box). So **don't reserve the 1 GB pool here.** Only reintroduce it if you switch to
> an **in-memory** index on a much-larger-RAM host and explicitly back the index with
> hugepages. Keep the small 2 MB pool — agave uses a few hundred MB of it.

Edit `/etc/default/grub`, append to `GRUB_CMDLINE_LINUX_DEFAULT` **inside the quotes**:

```
GRUB_CMDLINE_LINUX_DEFAULT="hugepagesz=2M hugepages=128 default_hugepagesz=2M nvme_core.default_ps_max_latency_us=0"
```

`nvme_core.default_ps_max_latency_us=0` disables NVMe power-saving (APST) for
consistent ledger/accounts latency. On **AMD** hosts (`grep -m1 vendor_id
/proc/cpuinfo` → `AuthenticAMD`) also add `amd_pstate=passive` to pair with the
`performance` governor. Do **not** add PoH core-isolation params
(`isolcpus`/`nohz_full`/`--experimental-poh-pinned-cpu-core`) on this node — see §4.

```bash
sudo update-grub        # RHEL/rocky: sudo grub2-mkconfig -o /boot/grub2/grub.cfg
sudo reboot
```

After reboot, verify:

```bash
cat /proc/cmdline | tr ' ' '\n' | grep -E 'huge|nvme|pstate'    # 2M present, NO 1G
cat /sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages      # 128
cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_driver         # amd-pstate (AMD)
cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor       # performance
```

Mount the 2 MB pool and persist in `/etc/fstab`:

```bash
sudo mkdir -p /mnt/hugepages
sudo mount -t hugetlbfs -o pagesize=2M,min_size=228589568 none /mnt/hugepages
```

```fstab
# /etc/fstab — append
none  /mnt/hugepages  hugetlbfs  pagesize=2M,min_size=228589568  0  0
```

> `min_size` reserves those bytes at mount time, so the matching hugepages **must
> already be allocated** (the GRUB step above) or the mount fails with
> `Cannot allocate memory`. `228589568 / 2M = 109` pages (of the 128 reserved).
> The systemd unit's `RequiresMountsFor` must list only `/mnt/accounts /mnt/ledger`
> (no `/mnt/gigantic`) — see §9.

**NUMA** (dual-socket hosts): don't `numactl --cpunodebind` the validator to one node
— its accounts working set outgrows a single node's RAM and it wants all cores.
Instead **interleave memory** across nodes so neither fills and bandwidth stays
balanced. This is already wired into `setup/rpc.sh` (`exec numactl --interleave=all
agave-validator …`). Keep `kernel.numa_balancing=0` (auto-NUMA off) from the tuning.

## 2. Rust toolchain

```bash
curl https://sh.rustup.rs -sSf | sh
source $HOME/.cargo/env
rustup component add rustfmt
```

## 3. Install agave v4

> **Version requirement — must be ≥ 4.1.2 (Alpenglow).** A `4.0.0`-era build
> **crash-loops** on current mainnet: `solReplayStage` panics at `replay_stage.rs:1818`
> crossing the Alpenglow consensus migration, so the node restarts → replays a few
> minutes → panics → repeats, and can never stay caught up (looks like steady "drift"
> after every fresh snapshot). Run the version the cluster runs (peers are on `4.1.2`).
> Diagnose with `setup/replay-diag.sh` — its first block flags panics/restarts and the
> `agave_votor` migration. **This branch (historically `feat/geyser-4.0.2`) now targets
> 4.1.2**; the plugin's `agave-geyser-plugin-interface` pin is `=4.1.2` to match — rebuild
> the plugin (§6) against the same version you install here.

```bash
cargo install agave-validator@4.1.2          # match the cluster (>= 4.1.2, Alpenglow)
# or: sh -c "$(curl -sSfL https://release.anza.xyz/v4.1.2/install)"
agave-validator --version                     # expect 4.1.x
```

(Confirm whatever version actually resolves on the host, and that it matches what the
cluster's known validators report in `solana gossip`. `cargo install agave-validator@<ver>`
re-installs a specific binary if rollback is ever needed.)

**Install `perf-libs` next to the binary (required for the PoH gate).** `cargo install`
builds only the bare binary — it does **not** include `libpoh-simd.so` (the SIMD PoH
accelerator). Without it the validator logs `Unable to load "libpoh-simd.so"` and its
single-core PoH rate can fall under the 10M h/s gate, so it refuses to start. The
official release installer ships `perf-libs`; run it once to fetch them, then copy the
dir next to the cargo binary (agave looks for `<dir-of-binary>/perf-libs/`):

```bash
sh -c "$(curl -sSfL https://release.anza.xyz/v4.1.2/install)"   # populates perf-libs
cp -r ~/.local/share/solana/install/active_release/bin/perf-libs ~/.cargo/bin/
ls ~/.cargo/bin/perf-libs/libpoh-simd.so                        # confirm present
```

(If you instead point `rpc.sh` at the installer binary directly —
`~/.local/share/solana/install/active_release/bin/agave-validator` — perf-libs are
already alongside it and no copy is needed.)

## 4. Host tuning (community tuner)

There is **no official "online tuner"** for agave. Apply the community tuning from
[`sonicfromnewyoke/solana-rpc`](https://github.com/sonicfromnewyoke/solana-rpc)
via a **`tuned` profile** — it applies the sysctl and CPU-governor settings as one
named profile that reactivates automatically on every boot (no manual `sysctl -p`):

- **tuned profile** → `setup/tuned-solana.conf` folds the sysctl block **and** the CPU
  governor into a single profile: UDP/TCP buffers
  (`net.core.rmem_max=net.core.wmem_max=134217728`), connection backlogs, **BBR**
  congestion control, `vm.max_map_count=2000000`, `fs.nr_open=2000000`, low swappiness,
  `kernel.numa_balancing=0`, governor `performance`. Install and activate:
  ```bash
  sudo apt-get install -y tuned                                    # if not present
  sudo mkdir -p /etc/tuned/solana
  sudo cp /home/sol/antegen/setup/tuned-solana.conf /etc/tuned/solana/tuned.conf
  sudo tuned-adm profile solana
  tuned-adm active                                                 # -> Current active profile: solana
  ```
  This **replaces** the old standalone `/etc/sysctl.d/21-agave-validator.conf` step —
  `include=throughput-performance` gives its base, and the ported keys above cover the
  rest. No XFS `fs.xfs.xfssyncd_centisecs` line (that host is **ext4**).
- **File limits** → install the repo drop-in (covers interactive `sol` sessions;
  the systemd unit sets its own limits for the service):
  `sudo cp /home/sol/antegen/setup/90-solana-nofiles.conf /etc/security/limits.d/90-solana-nofiles.conf`
  (`nofile` and `memlock` = `2000000`).
- **CPU**: governor `performance` (set by the tuned profile above), enable Intel/AMD
  boost. **Critical for the PoH startup gate** — agave benchmarks single-core SHA-256
  at startup and refuses to run if it's below the 10M h/s cluster target; under
  `schedutil` the core clocks down and fails the gate. The unit *also* pins
  `performance` as an `ExecStartPre` (§9) so it's guaranteed at benchmark time even if
  something reset the governor after tuned applied.
- **Memory**: disable Transparent Huge Pages and KSM. NUMA balancing is disabled by the
  tuned profile (`kernel.numa_balancing=0`). The 2 MB hugepage pool and NUMA interleave
  are set up in **§1b** (no 1 GB pool — see that section).
- **NVMe** → `/etc/udev/rules.d/60-nvme-scheduler.rules`: I/O scheduler `none`,
  read-ahead `0`.
- **NIC** → ring buffers to hardware max + interrupt coalescing off (fewer drops /
  lower latency under turbine + QUIC bursts). On a **bonded** setup apply to the bond
  *slaves*, not `bond0` (the virtual device rejects these ops). This is automated by
  `setup/nic-tune.sh`, run as an `ExecStartPre` of the validator unit (§9) so it
  re-applies on every start — no separate service needed:
  ```bash
  sudo cp /home/sol/antegen/setup/nic-tune.sh /usr/local/bin/nic-tune.sh && sudo chmod +x /usr/local/bin/nic-tune.sh
  sudo systemctl disable --now irqbalance     # so IRQ affinity sticks
  sudo /usr/local/bin/nic-tune.sh             # apply now (or just (re)start the unit)
  ```
  The script auto-detects bond slaves, sets each NIC's ring to its `ethtool -g` max,
  disables coalescing, and skips any op a NIC doesn't support. Verify:
  `ethtool -g <slave> | sed -n '8,12p'` (RX/TX at max) and
  `ethtool -c <slave> | grep -i adaptive` (`off`).

> **PoH core pinning — skip on this node.** Isolating a core for PoH
> (`isolcpus`/`nohz_full` + `--experimental-poh-pinned-cpu-core`) is a **voting /
> block-producing** optimization: it buys a jitter-free clock for *leader slots*.
> This is a `--no-voting` **RPC** worker — it runs PoH only to track the tip and
> produces no blocks, so the benefit is negligible while the flag is experimental
> (known `core_affinity` bug needing a `taskset` workaround) and isolation steals
> cores from the replay/RPC pool. Only revisit if a voting validator runs here.

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

Build the plugin and `antegen` CLI from the repo cloned in §0. **The branch already
carries every fix in this guide** — the plugin memory filter, `rpc.sh`, the systemd
unit, and the tuning drop-ins — so there's nothing to edit here; just build.

> **Context (already in the branch, no action needed).** `plugin/src/plugin.rs`
> `update_account()` drops irrelevant accounts before spawning a task during snapshot
> load (`if is_startup && event.is_err()`). Without that, agave streams every account
> on the chain at load and the plugin spawns a task per account — the anonymous-memory
> spike that OOM-killed the validator at load on a 256 GB host.
> `account_data_snapshot_notifications_enabled()` stays **`true`** so the observer can
> backfill threads that already exist on-chain (a cron-triggered thread that never
> changes its account would otherwise never crank).

```bash
cd /home/sol/antegen                                              # cloned in §0
cargo build --release -p antegen-plugin -p antegen-cli

# Install the built artifacts
cp target/release/libantegen_plugin.so /home/sol/libantegen_plugin.so
sudo cp target/release/antegen /usr/local/bin/antegen
antegen --version
```

Place and edit the geyser config:

```bash
cp /home/sol/antegen/setup/geyser-config.json /home/sol/geyser-config.json
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
cp /home/sol/antegen/setup/rpc.sh /home/sol/rpc.sh
chmod +x /home/sol/rpc.sh
mkdir -p /home/sol/log && chown sol:sol /home/sol/log
```

`setup/rpc.sh` is already v4-ready and tuned for **low memory on 256 GB** (see the
memory note below): `numactl --interleave=all` (NUMA, §1b), **mmap storage access**
(`--accounts-db-access-storages-method mmap`), **disk-backed accounts index**
(`--accounts-index-limit minimal` + `--accounts-index-path /mnt/accounts/index`),
**no `--full-rpc-api`** (crank-only RPC surface), `unified-scheduler`, geyser plugin
config, log to `/home/sol/log`. If `/mnt/accounts` storage rejects O_DIRECT on
snapshot load, add `--no-accounts-db-snapshots-direct-io` (commented in the script).

> **v4 memory on 256 GB — four things, in order of impact.** v4 OOM-killed this host
> at snapshot load (212 GB anon, `file-rss:0`). The fixes:
> 1. **Plugin** `account_data_snapshot_notifications_enabled() → false` (§6) — stops
>    agave streaming every account to the plugin at load. The keystone.
> 2. **`--accounts-db-access-storages-method mmap`** — v4's default (`file`) reads
>    account storages into *anonymous* RAM (non-reclaimable); mmap makes them
>    file-backed/reclaimable. This was the bulk of the 212 GB anon.
> 3. **No 1 GB hugepages** (§1b) — reclaims ~26 GB the disk-index build never uses.
> 4. **`--accounts-index-limit minimal`** — keeps the index on the `/mnt/accounts`
>    NVMe (mmap'd bucket map) instead of fully in RAM. Create the dir first:
>    `mkdir -p /mnt/accounts/index && sudo chown sol:sol /mnt/accounts/index`.
>
> Note: `minimal` is [deprecated on agave master](https://github.com/anza-xyz/agave/blob/master/CHANGELOG.md)
> and 256 GB is the [documented floor](https://docs.anza.xyz/operations/requirements)
> for a mainnet full-RPC node — this config buys runway, not forever. Long term:
> 512 GB RAM, or keep this node crank-only (no heavy RPC).

**Optional — switch back to in-memory index** (faster, but needs *much* more RAM than
256 GB on mainnet): remove the two `--accounts-index-*` lines from `rpc.sh` and restart.
Only do this on a high-RAM host and watch `free -h` available + swap during the startup
index build. (`--accounts-index-limit minimal` is the v4 replacement for the removed
`--enable-accounts-disk-index`; account data on `/mnt/accounts` is unaffected either way.)

## 9. systemd service

Install the repo unit (don't hand-write it — `setup/antegen-crank.service` carries
all the hardening) plus the NIC tuning script it calls:

```bash
sudo cp /home/sol/antegen/setup/nic-tune.sh /usr/local/bin/nic-tune.sh && sudo chmod +x /usr/local/bin/nic-tune.sh
sudo cp /home/sol/antegen/setup/antegen-crank.service /etc/systemd/system/antegen-crank.service
sudo systemctl daemon-reload
sudo systemctl enable antegen-crank.service
```

Key settings baked into the unit:
- `RequiresMountsFor=/mnt/accounts /mnt/ledger` — won't start before the data drives
  are mounted. (No `/mnt/gigantic` — the 1 GB pool was dropped in §1b.)
- `LimitNOFILE=2000000`, `LimitMEMLOCK=2000000000`, `TasksMax=infinity`,
  `LimitNPROC=infinity` — fd / memlock / thread headroom.
- `AmbientCapabilities=CAP_NET_RAW CAP_NET_ADMIN CAP_BPF CAP_PERFMON` — **v4 XDP**.
- `ExecStartPre=…echo performance…scaling_governor` — pins all cores to the
  performance governor before start so the PoH single-core gate (§4) isn't measured
  under a clocked-down `schedutil`.
- `ExecStartPre=-+/usr/local/bin/nic-tune.sh` — applies NIC tuning (§4) as root on
  every start; `-` so a NIC hiccup never blocks the validator.
- `Restart=always` + `StartLimitIntervalSec=0` — never give up restarting.
- `KillSignal=SIGTERM` + `TimeoutStopSec=120` + `SendSIGKILL=yes` — clean state flush
  on stop; `OOMScoreAdjust=-900` keeps the OOM killer off it.

> If `RequiresMountsFor` lists a mount the validator doesn't actually consume, the
> unit still waits on it — keep the list to the paths agave reads (`/mnt/accounts`,
> `/mnt/ledger`, and whichever hugepage mount it's pointed at).

Verify the running process got the limits after first start (§11):

```bash
cat /proc/$(pgrep -f agave-validator)/limits | grep -E 'open files|locked memory'
```

### Log rotation

The validator logs to `/home/sol/log/agave-validator.log` on the root partition,
which is **separate** from `--limit-ledger-size` — nothing caps it by default, so on
a small `/` it will fill the disk. Install the shipped config:

```bash
sudo cp setup/logrotate-agave-validator /etc/logrotate.d/agave-validator
sudo logrotate -d /etc/logrotate.d/agave-validator   # dry-run: expect no errors
sudo logrotate -f /etc/logrotate.d/agave-validator   # force one rotation
ls -lh /home/sol/log/                                # see .1 / .gz appear, live log small
```

Tuned for a small `/` (`rotate 2` + `maxsize 2G` ≈ 2 days, ~4.5G worst case). It uses
`copytruncate` because agave only reopens its log on `SIGUSR1`; a plain rename would
leave the process writing to a deleted inode (disk fills via a handle `ls` can't see —
check with `sudo lsof -p $(pgrep -f agave-validator) | grep 'log.*(deleted)'`).

`maxsize` only triggers when logrotate runs (`logrotate.timer`, daily by default). If
the log gains ≫2G/day, add an hourly logrotate timer so the cap is enforced between
daily runs. Confirm the timer is live: `systemctl list-timers 'logrotate*'`.

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
grep -i huge /proc/meminfo                                   # hugepages reserved/in use
numastat -m | head                                           # memory spread across NUMA nodes
cat /proc/$(pgrep -f agave-validator)/limits | grep -E 'open files|locked memory'  # 2000000 / 2000000000
```

## 12. Operational notes

- **Reset state** (reclaim disk / clean restart):
  `sudo systemctl stop antegen-crank.service && sudo rm -rf /mnt/ledger/* /mnt/accounts/* && sudo systemctl start antegen-crank.service`
- **Pool**: `antegen pool get 0`, `antegen pool rotate <id>` as needed.
- **v4 flag changes from v3.1.9**: v4 makes the **in-memory** accounts index the
  default (`--enable-accounts-disk-index` deprecated, `--disable-accounts-disk-index`
  removed). We override back to **disk-backed** via `--accounts-index-limit minimal`
  + `--accounts-index-path` because in-memory OOM'd 256 GB on mainnet. Snapshot
  Direct-I/O on by default. Removed flags we don't use: `--monitor`,
  `--use-quic`/`--use-udp`, `--tpu-disable-quic`/`--tpu-enable-udp`,
  `--block-verification-method blockstore-processor`.
```
