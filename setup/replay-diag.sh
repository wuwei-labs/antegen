#!/usr/bin/env bash
# replay-diag.sh -- classify RPC-node drift as replay-compute-bound vs ingest-starved.
#
# Reads agave-validator metrics datapoints from the log and summarizes:
#   replay-slot-stats        -> per-slot replay cost (is replay > 400ms budget?)
#   replay-loop-timing-stats -> where the replay loop spends time (waiting vs working)
#
# Datapoint lines look like:
#   ... datapoint: replay-slot-stats slot=123i replay_total_elapsed=12345i ...
# values carry an influx type suffix (i/u); we strip it.
#
# Usage:
#   ./replay-diag.sh                 # analyze last N slots already in the log
#   ./replay-diag.sh -f              # follow live, print each slot as it lands (Ctrl-C to stop)
#   ./replay-diag.sh -n 500          # window size for static analysis (default 200)
#   LOG=/path/to.log ./replay-diag.sh
set -euo pipefail

LOG="${LOG:-/home/sol/log/agave-validator.log}"
N=200
FOLLOW=0
PROBE=0
RAW=0
while getopts "fprn:l:" opt; do
  case "$opt" in
    f) FOLLOW=1 ;;
    p) PROBE=1 ;;
    r) RAW=1 ;;
    n) N="$OPTARG" ;;
    l) LOG="$OPTARG" ;;
    *) echo "usage: $0 [-f] [-p] [-r] [-n slots] [-l logfile]" >&2; exit 2 ;;
  esac
done

[ -r "$LOG" ] || { echo "cannot read $LOG (set LOG= or -l)" >&2; exit 1; }

BUDGET_US=400000   # 400ms cluster slot budget

# pull one key=val (influx line protocol) from a datapoint line; prints integer or nothing
kv() { sed -n "s/.*[ =]$2=\([0-9][0-9]*\)[iu]\{0,1\}.*/\1/p" <<<"$1" | head -1; }

# ---- raw mode: dump one full line of each replay datapoint (reveals exact field names) ----
if [ "$RAW" = 1 ]; then
  echo "=== last replay-slot-stats line ==="
  grep -e 'replay-slot-stats' "$LOG" | tail -1
  echo; echo "=== last replay-loop-timing-stats line ==="
  grep -e 'replay-loop-timing-stats' "$LOG" | tail -1
  exit 0
fi

# ---- probe mode: discover what datapoints exist (run this if analysis finds nothing) ----
if [ "$PROBE" = 1 ]; then
  echo "=== datapoint names present in $LOG (name<TAB>count) ==="
  grep -o 'datapoint: [a-zA-Z0-9_-]*' "$LOG" | sort | uniq -c | sort -rn | head -40
  echo
  echo "total 'datapoint:' lines: $(grep -c 'datapoint:' "$LOG")"
  echo "(0 => metrics not logged: add RUST_LOG=info,solana_metrics::metrics=info and restart)"
  echo
  echo "=== replay-* tokens seen ==="
  grep -o 'replay[a-zA-Z0-9_-]*' "$LOG" | sort | uniq -c | sort -rn | head
  exit 0
fi

# ---- follow mode: stream each replay-slot-stats line as it lands ----
if [ "$FOLLOW" = 1 ]; then
  echo "following $LOG -- slot | replay_total_ms | execute_ms | txs | verdict   (Ctrl-C to stop)"
  stdbuf -oL tail -F "$LOG" 2>/dev/null | stdbuf -oL grep --line-buffered -e 'replay-slot-stats' | \
  while IFS= read -r line; do
    slot=$(kv "$line" slot); rt=$(kv "$line" replay_total_elapsed)
    ex=$(kv "$line" execute_batches_us); [ -z "$ex" ] && ex=$(kv "$line" total_execute_us)
    tx=$(kv "$line" total_transactions)
    [ -z "$rt" ] && continue
    v="ok"; [ "$rt" -gt "$BUDGET_US" ] && v="OVER-BUDGET"
    printf "%-10s %8.1f %10s %6s  %s\n" "${slot:-?}" \
      "$(awk "BEGIN{print $rt/1000}")" "${ex:-?}" "${tx:-?}" "$v"
  done
  exit 0
fi

# ---- static analysis over the last N of each datapoint ----
echo "=== log: $LOG   window: last $N datapoints each ==="
echo

echo "--- crash / restart / migration check (look here FIRST) ---"
panics=$(grep -c "thread '.*' panicked\|datapoint: panic " "$LOG" 2>/dev/null || true)
restarts=$(grep -c 'Pre startup initializing\|Starting validator with' "$LOG" 2>/dev/null || true)
echo "  panics logged: ${panics:-0}   restarts logged: ${restarts:-0}"
if [ "${panics:-0}" -gt 0 ]; then
  echo "  >>> CRASH-LOOP LIKELY: replay/other thread is panicking. Last panic:"
  grep "panicked at" "$LOG" | tail -1 | sed 's/^/      /'
  echo "      (a crash-looping validator can never stay caught up -- fix the crash before any tuning)"
fi
if grep -q 'Alpenglow migration\|agave_votor' "$LOG" 2>/dev/null; then
  echo "  note: Alpenglow/votor migration active in this log -- match the cluster's agave version."
fi
echo

echo "--- replay-slot-stats: per-slot replay cost (budget ${BUDGET_US}us = 400ms) ---"
{ grep -e 'replay-slot-stats' "$LOG" 2>/dev/null || true; } | tail -n "$N" | \
awk -v B="$BUDGET_US" '
  function num(s,k,  m){ if (match(s, k"=[0-9]+")) { m=substr(s,RSTART+length(k)+1,RLENGTH-length(k)-1); return m+0 } return -1 }
  {
    rt=num($0,"replay_total_elapsed"); if (rt<0) next
    n++; sum+=rt; if(rt>max){max=rt; maxslot=num($0,"slot")}
    if(rt>B) over++
  }
  END{
    if(n==0){ print "  no replay_total_elapsed samples found"; exit }
    printf "  samples=%d  mean=%.1fms  max=%.1fms (slot %d)\n", n, sum/n/1000, max/1000, maxslot
    printf "  over-budget slots (>400ms): %d/%d (%.0f%%)\n", over+0, n, (over+0)*100/n
    if(over*2>n) print "  >>> REPLAY-COMPUTE-BOUND: replay itself misses the 400ms budget."
    else        print "  >>> replay fits budget on most slots -> drift is NOT replay compute; check ingestion below."
  }'
echo

echo "--- replay-loop-timing-stats: where the replay loop spends time (auto-ranked) ---"
# Build-agnostic: sum every *_elapsed / *_us field and rank. No hardcoded names.
{ grep -e 'replay-loop-timing-stats' "$LOG" 2>/dev/null || true; } | tail -n "$N" | \
awk '
  {
    n++
    # walk every key=int token on the line
    s=$0
    while (match(s, /[a-zA-Z_][a-zA-Z0-9_]*=[0-9]+/)) {
      tok=substr(s,RSTART,RLENGTH); s=substr(s,RSTART+RLENGTH)
      eq=index(tok,"="); k=substr(tok,1,eq-1); v=substr(tok,eq+1)+0
      # skip the outer-loop aggregate (total_elapsed*) -- it double-counts components
      if (k ~ /(_elapsed|_us|_time)$/ && k !~ /^total/) { sum[k]+=v; tot+=v }
    }
  }
  END{
    if(n==0){ print "  no replay-loop-timing-stats found"; exit }
    if(tot==0){ print "  matched "n" lines but no *_elapsed/_us fields -- run with -r to dump a raw line"; exit }
    # find + print top fields by share
    printf "  samples=%d  top time sinks:\n", n
    for (k in sum) { keys[++m]=k }
    # simple selection sort, top 6
    for (i=1;i<=6 && i<=m;i++){ bi=i; for(j=i+1;j<=m;j++) if(sum[keys[j]]>sum[keys[bi]]) bi=j
      t=keys[i]; keys[i]=keys[bi]; keys[bi]=t
      printf "    %-34s %5.1f%%\n", keys[i], sum[keys[i]]*100/tot }
    top=keys[1]
    if (top ~ /wait|receive|idle|fetch/) print "  >>> INGEST/WAIT-BOUND: dominant cost is waiting for data, not replaying."
    else print "  >>> dominant cost is "top" -- paste this block, I will interpret."
  }'
echo

echo "--- ingestion health: repair vs turbine (last $N) ---"
echo "  repair-related datapoints present: $(grep -e 'repair_stats' -e 'serve_repair' "$LOG" | tail -n "$N" | wc -l | tr -d ' ')"
echo "  window-insert / shred_fetch present: $(grep -e 'window-insert' -e 'shred_fetch' "$LOG" | tail -n "$N" | wc -l | tr -d ' ')"
echo "  (high repair volume + low turbine insert = turbine not delivering; widen --dynamic-port-range, check UDP)"
echo

echo "--- slot gap vs cluster (are we actually behind? sampled twice, 20s apart) ---"
REF_URL="${REF_URL:-https://api.mainnet-beta.solana.com}"
node_slot() { grep -o 'replay-slot-stats slot=[0-9]*' "$LOG" | tail -1 | grep -o '[0-9]*$'; }
cluster_slot() {
  curl -s "$REF_URL" -X POST -H 'content-type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"getSlot","params":[{"commitment":"processed"}]}' \
    2>/dev/null | grep -o '"result":[0-9]*' | grep -o '[0-9]*'
}
n1=$(node_slot); c1=$(cluster_slot)
if [ -z "${c1:-}" ]; then
  echo "  could not reach reference RPC ($REF_URL); set REF_URL= to another mainnet RPC"
else
  echo "  t=0s : node=$n1 cluster=$c1 behind=$((c1-n1))"
  sleep 20
  n2=$(node_slot); c2=$(cluster_slot)
  echo "  t=20s: node=$n2 cluster=$c2 behind=$((c2-n2))"
  echo "  node advanced $((n2-n1)) slots in 20s (cluster ~50); behind delta=$(( (c2-n2)-(c1-n1) ))"
  if [ "$(( (c2-n2)-(c1-n1) ))" -gt 5 ]; then
    echo "  >>> LOSING GROUND: gap growing -> real drift, keep digging (slot completion / repair)."
  elif [ "$((c2-n2))" -lt 10 ]; then
    echo "  >>> CAUGHT UP: gap small and stable -> node is healthy; the 'drift' may be intermittent/misread."
  else
    echo "  >>> BEHIND BUT STABLE: gap not growing -> node holds pace; may just need to finish catching up."
  fi
fi
