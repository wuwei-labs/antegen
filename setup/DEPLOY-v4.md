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
| `/mnt/gigantic` | 1 GB hugetlbfs pool (RAM-backed) |

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
# Cushion for the in-memory accounts-index startup spike — NOT usable capacity.
# Keep low swappiness (tuning step 4) and leave ~100GB+ free on the OS disk for logs.
sudo fallocate -l 64G /swapfile
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

## 1b. Hugepages (2 MB + 1 GB) & NUMA

Reserve the hugepage pools, then mount them. **1 GB ("gigantic") pages must be
reserved at boot** — the kernel can't find contiguous 1 GB blocks at runtime — so
they go on the kernel command line. 2 MB pages could be set live, but doing both at
boot is simplest and survives reboots.

```bash
# Confirm the CPU supports 1 GB pages (non-zero = supported; 0 = drop the 1G pool).
grep -c pdpe1gb /proc/cpuinfo
```

Reserve at boot — edit `/etc/default/grub`, append to `GRUB_CMDLINE_LINUX_DEFAULT`
**inside the quotes** (each `hugepages=N` binds to the `hugepagesz=` before it):

```
GRUB_CMDLINE_LINUX_DEFAULT="... hugepagesz=2M hugepages=128 hugepagesz=1G hugepages=26 default_hugepagesz=2M nvme_core.default_ps_max_latency_us=0"
```

`nvme_core.default_ps_max_latency_us=0` disables NVMe power-saving for consistent
ledger/accounts latency. On **AMD EPYC** hosts (`grep -m1 vendor_id /proc/cpuinfo` →
`AuthenticAMD`) also add `amd_pstate=passive` to pair with the `performance` governor.
Do **not** add PoH core-isolation params (`isolcpus`/`nohz_full`/
`--experimental-poh-pinned-cpu-core`) on this node — see the PoH note in §4.

```bash
sudo update-grub        # RHEL/rocky: sudo grub2-mkconfig -o /boot/grub2/grub.cfg
sudo reboot
```

After reboot, verify the pools allocated (and, on a dual-socket box, that the 1 GB
pages split across NUMA nodes):

```bash
cat /proc/cmdline                                                   # tokens present
cat /sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages          # 128
cat /sys/kernel/mm/hugepages/hugepages-1048576kB/nr_hugepages       # 26
cat /sys/devices/system/node/node*/hugepages/hugepages-1048576kB/nr_hugepages  # ~13/13
```

Mount the pools and persist in `/etc/fstab`:

```bash
sudo mkdir -p /mnt/hugepages /mnt/gigantic
sudo mount -t hugetlbfs -o pagesize=2M,min_size=228589568    none /mnt/hugepages
sudo mount -t hugetlbfs -o pagesize=1G,min_size=27917287424  none /mnt/gigantic
```

```fstab
# /etc/fstab — append (min_size must equal the bytes the validator reserves)
none  /mnt/hugepages  hugetlbfs  pagesize=2M,min_size=228589568    0  0
none  /mnt/gigantic   hugetlbfs  pagesize=1G,min_size=27917287424  0  0
```

> `min_size` reserves those bytes at mount time, so the matching hugepages **must
> already be allocated** (the GRUB step above) or the mount fails with
> `Cannot allocate memory`. `228589568 / 2M = 109` pages; `27917287424 / 1G = 26`.

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
  congestion control, `vm.max_map_count=2000000`, `fs.nr_open=2000000`, low swappiness.
  Apply: `sudo sysctl -p /etc/sysctl.d/21-agave-validator.conf`
  - On **ext4** accounts/ledger (this host), drop the XFS-only line or `sysctl -p`
    errors `cannot stat /proc/sys/fs/xfs/...`:
    `sudo sed -i '/fs.xfs.xfssyncd_centisecs/d' /etc/sysctl.d/21-agave-validator.conf`
- **File limits** → install the repo drop-in (covers interactive `sol` sessions;
  the systemd unit sets its own limits for the service):
  `sudo cp setup/90-solana-nofiles.conf /etc/security/limits.d/90-solana-nofiles.conf`
  (`nofile` and `memlock` = `2000000`).
- **CPU**: governor `performance`, enable Intel/AMD boost.
- **Memory**: disable Transparent Huge Pages, KSM, and NUMA balancing
  (`kernel.numa_balancing=0`). Hugepages (2 MB + 1 GB) and NUMA interleave are set up
  in **§1b**.
- **NVMe** → `/etc/udev/rules.d/60-nvme-scheduler.rules`: I/O scheduler `none`,
  read-ahead `0`.
- **NIC** → ring buffers to hardware max + interrupt coalescing off (fewer drops /
  lower latency under turbine + QUIC bursts). On a **bonded** setup apply to the bond
  *slaves*, not `bond0` (the virtual device rejects these ops). This is automated by
  `setup/nic-tune.sh`, run as an `ExecStartPre` of the validator unit (§9) so it
  re-applies on every start — no separate service needed:
  ```bash
  sudo cp setup/nic-tune.sh /usr/local/bin/nic-tune.sh && sudo chmod +x /usr/local/bin/nic-tune.sh
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

`setup/rpc.sh` is already v4-ready: `numactl --interleave=all` (NUMA, §1b),
in-memory accounts index (no `--*-disk-index` flags), `unified-scheduler`, geyser
plugin config, log to `/home/sol/log`. If
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

Install the repo unit (don't hand-write it — `setup/antegen-crank.service` carries
all the hardening) plus the NIC tuning script it calls:

```bash
sudo cp setup/nic-tune.sh /usr/local/bin/nic-tune.sh && sudo chmod +x /usr/local/bin/nic-tune.sh
sudo cp setup/antegen-crank.service /etc/systemd/system/antegen-crank.service
sudo systemctl daemon-reload
sudo systemctl enable antegen-crank.service
```

Key settings baked into the unit:
- `RequiresMountsFor=/mnt/accounts /mnt/ledger /mnt/gigantic` — won't start before the
  data drives and the 1 GB hugepage pool (§1b) are mounted.
- `LimitNOFILE=2000000`, `LimitMEMLOCK=2000000000`, `TasksMax=infinity`,
  `LimitNPROC=infinity` — fd / memlock / thread headroom.
- `AmbientCapabilities=CAP_NET_RAW CAP_NET_ADMIN CAP_BPF CAP_PERFMON` — **v4 XDP**.
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
grep -i huge /proc/meminfo                                   # hugepages reserved/in use
numastat -m | head                                           # memory spread across NUMA nodes
cat /proc/$(pgrep -f agave-validator)/limits | grep -E 'open files|locked memory'  # 2000000 / 2000000000
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
