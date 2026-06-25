#!/bin/bash
# NIC tuning for the agave-validator RPC worker.
# Sets ring buffers to the hardware maximum and disables interrupt coalescing
# (fewer drops + lower latency under turbine/QUIC burst traffic).
#
# Applied per *physical* NIC. On a bonded setup that means the bond SLAVES, not
# bond0 (ring/coalescing ops aren't supported on the virtual bond device).
#
# Run as root from the validator unit:  ExecStartPre=-+/usr/local/bin/nic-tune.sh
# Idempotent: skips the ring-set when already at max, so a service restart does
# NOT needlessly reset the ring (which briefly flaps the link). Safe to re-run.
set -u

log() { echo "nic-tune: $*"; }

# Interfaces to tune: slaves of every bond; else the default-route interface.
ifaces=()
shopt -s nullglob
for sl in /sys/class/net/*/bonding/slaves; do
    read -r -a s < "$sl" && ifaces+=("${s[@]}")
done
if [ ${#ifaces[@]} -eq 0 ]; then
    primary=$(ip route get 1.1.1.1 2>/dev/null | awk '{print $5; exit}')
    [ -n "${primary:-}" ] && ifaces+=("$primary")
fi
if [ ${#ifaces[@]} -eq 0 ]; then
    log "no interfaces found; nothing to do"; exit 0
fi

for nic in "${ifaces[@]}"; do
    [ -e "/sys/class/net/$nic" ] || { log "$nic: missing, skip"; continue; }

    # Parse ring max + current (so we only reset the ring when needed).
    read -r rxmax txmax rxcur txcur < <(ethtool -g "$nic" 2>/dev/null | awk '
        /Pre-set maximums:/          {sect="max"; next}
        /Current hardware settings:/ {sect="cur"; next}
        sect=="max" && $1=="RX:"     {rxm=$2}
        sect=="max" && $1=="TX:"     {txm=$2}
        sect=="cur" && $1=="RX:"     {rxc=$2}
        sect=="cur" && $1=="TX:"     {txc=$2}
        END                          {print rxm+0, txm+0, rxc+0, txc+0}')
    if [ "${rxmax:-0}" -gt 0 ] 2>/dev/null; then
        if [ "$rxcur" = "$rxmax" ] && [ "$txcur" = "$txmax" ]; then
            log "$nic: ring already rx=$rxmax tx=$txmax, skip"
        elif ethtool -G "$nic" rx "$rxmax" tx "$txmax" 2>/dev/null; then
            log "$nic: ring rx=$rxmax tx=$txmax"
        else
            log "$nic: ring set unsupported, skip"
        fi
    else
        log "$nic: no ring max reported, skip ring"
    fi

    # Disable interrupt coalescing -> minimum latency (no link flap; apply each time).
    if ethtool -C "$nic" adaptive-rx off adaptive-tx off rx-usecs 0 tx-usecs 0 2>/dev/null; then
        log "$nic: coalescing off"
    else
        log "$nic: coalescing tweak unsupported, skip"
    fi
done
exit 0
