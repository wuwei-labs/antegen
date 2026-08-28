# Parked thread register

**Status:** proposed, not started
**Scope:** `antegen-client`, `antegen-cli`
**Prerequisite:** Defect A below — done. This plan was not worth building on
top of a park that refetched every ten seconds.

## What happened

On 2026-08-28, thread `6Xswn1qctzYZ5QFk8VRJv2PB9U7xPsaar6w1L8AW2Dq5` had been
failing continuously since roughly 2026-08-27 19:43. Its fiber 0 runs SRSLY
`ProcessContract` over 14 accounts; two of them —
`BPBQQcoZszgYdnNgkqUaMHgNY7pvyHsCtFZ5HFNzhhHb` and
`87RqWMkWyoow1PZPA1hGNdrhgwUvBkesAQMFesrZaKkq` — no longer exist. Anchor
reports the first as `owner_token_account`, error 3012.

The fiber is not misconfigured. Both accounts have a thousand transactions of
history; they are addresses the protocol creates and closes across a contract
lifecycle. The fiber pins them, the contract turned over, the pin went stale.
`exec_count` is still 0. The thread has never executed once.

Three things about how that was found are the subject of this plan.

**It was found by eye.** Nothing alerted. The thread sat roughly ten hours
overdue while the node logged the same failure 322 times an hour.

**The obvious query lies.** The service's syslog identifier is
`antegen-node-v8.1.0`, not `antegen-node`, so:

```
journalctl -u antegen-node --since "09:00" | grep -c "parking until state changes"
0
```

Zero. The correct query returns 318. A wrong `-u` fails silently as the most
reassuring possible answer, and there is no second place to look.

**Diagnosis was manual archaeology.** Deriving the fiber PDA, decoding
`CompiledInstructionV0`, and checking fourteen accounts for existence took
about thirty minutes — to recover information the node had in memory at the
moment of failure and discarded.

## Two defects, separable

**Defect A — the failure park refreshes on the success cadence.** *Fixed;
recorded here because the shape of it matters for the rest.*

Measured cadence between failures was 10.2 s (322 attempts over 3275 s). That
is neither `parked_watchdog` (120 s) nor `stall_watchdog` (45 s). It is
`REFRESH_AFTER` (10 s).

The load-bearing fact is that **`Phase::Parked` has exactly one exit, and it is
`upsert`.** `take_due` requires `Phase::Due`; `reclaim_stalled` only rescues
`InFlight`; nothing promotes a parked entry. So `parked_watchdog` re-arms
nothing — it only positions `due` in the ordered index, where it is never read
while parked. Its doc comment claimed otherwise and was wrong.

What actually returns a parked thread is the refresh marker: `take_stale_parked`
→ `spawn_refetch` → `Refetched` → `track` → `upsert`. That marker, therefore,
*is* the retry cadence.

`Outcome::Fatal` set it to `REFRESH_AFTER`, which is a backstop for an account
update already on its way — true after a success, false after a failure,
because nothing executed. And when the cause lies outside the thread's own
account, as here, no update is ever coming: closing `owner_token_account`
provokes no write to the thread. So every 10 s the node refetched identical
state and re-dispatched an identical failing transaction.

The fix gives failure parks their own wall-clock constant, `PARKED_REFRESH`
(120 s), leaving `REFRESH_AFTER` for the success path where it is correct.
Deliberately not `Kind::parked_watchdog()`, which is denominated in each kind's
own chain units — 300 *slots*, 1 *epoch* — and would read as 300 seconds and 1
second if used as a duration.

**A wrong fix worth recording.** The first attempt guarded `upsert` so an
unchanged refetch would leave the entry parked. That would have made
`Phase::Parked` genuinely terminal and stranded the thread permanently — it
never retries, because the only exit had just been closed. A test asserting the
watchdog still re-arms the thread is what caught it. Any future change here
should keep that test: `a_parked_entry_is_never_dispatchable_by_itself`.

**Defect B — nothing retains failure state.** Every attempt rebuilds the same
transaction, simulates it, renders up to 25 log lines at WARN, and discards
everything else. There is no record that survives the attempt, so the 322nd
failure costs exactly as much to diagnose as the first. That is this plan.

## The evidence already on the wire

`simulate_transaction` requests post-simulation account state:

```rust
// crates/client/src/rpc/pool.rs
"accounts": {
    "encoding": "base64+zstd",
    "addresses": addresses
}
```

`SafeSimulationValue` carries `err`, `logs`, `units_consumed`, `accounts`, and
`return_data` (`rpc/response.rs:147-154`). On the error path
(`pool.rs:533-560`) `logs` are rendered to WARN and the function returns
`Err(anyhow!(…))`. `accounts`, `units_consumed`, and `return_data` are
dropped.

So the answer to "which of these fourteen accounts is missing, and what do the
live ones contain" is fetched, deserialized, and thrown away on every single
attempt. Retaining it is not new plumbing.

## Design

### A register, not a queue

A queue implies entries are consumed and drained. What is wanted is a
quarantine table **keyed by thread pubkey**, overwritten in place.

This is the property that bounds it. As a queue, one thread failing every 10 s
for ten hours is 3,600 entries of the same thing. As a register it is one row
with `attempts: 3600`.

```
thread            Pubkey
kind              Time | Slot | Epoch
fiber_index       u8
fiber_pubkey      Pubkey
outcome           Fatal | Retryable | EmptyFiber
signature         FailureSignature
first_seen        DateTime
last_seen         DateTime
attempts          u64
original_due      u64
overdue_seconds   u64
capture           Option<Capture>
```

### The failure signature is the load-bearing part

```
program_id     Pubkey        // innermost failing program
error_code     u32           // 3012
account_name   Option<String> // "owner_token_account"
```

Extracted from the `AnchorError caused by account: X. Error Number: N.` line,
falling back to the raw `InstructionError` shape when a program does not emit
one.

One concept, three consumers:

- **capture policy** — capture once per `(thread, signature)`. A signature that
  has not changed does not re-capture. This is what bounds storage.
- **log suppression** — full WARN dump on first sight of a signature, `debug`
  after. A signature change re-emits.
- **backoff escalation** — N consecutive identical signatures drive
  120 → 240 → 480 s, capped. A signature change resets to 120 s.

Building the signature once and deriving all three from it is the reason to do
this as one feature rather than three.

### Capture

Written once per signature, from the simulation the node already performs:

```
slot              u64
instruction       SerializableInstruction   // decompiled, post-PAYER substitution
accounts          Vec<(Pubkey, Option<AccountSnapshot>)>
logs              Vec<String>               // full, not the 25-line WARN cap
units_consumed    Option<u64>
return_data       Option<Value>
```

`AccountSnapshot` always keeps `owner`, `lamports`, `len`, `executable`.
`data` is kept up to a per-entry cap and truncated beyond it, because the
diagnostic value is concentrated in "does it exist and who owns it" — which is
what actually resolved this incident.

`None` for an account is the finding, not a gap: it is the missing account.

### Where it lives

The scheduler knows phase but has no business holding log strings. The worker
produces the capture but does not outlive the attempt. So: the worker attaches
an optional capture to `ExecutionResult`, the processor forwards it on the
outcome message it already sends, and the register sits beside the scheduler in
staging state, updated on the same `record_outcome` path that reschedules.

### Persistence

A JSON file, written on change, debounced, serialized off the actor loop and
swapped in by atomic rename. Path from `ClientConfig`, defaulting beside the
config file.

A file rather than an admin socket because it needs no new listening surface,
no auth decision, and — the part that matters — it still answers questions
when the node is down, which is when they are most often asked.

### CLI

```
antegen node parked                 # table: thread, signature, attempts, overdue, last_seen
antegen node parked <pubkey>        # full entry, decoded instruction, per-account state
antegen node parked --json          # for scripts
```

Reads the file. No daemon interaction. This matches `doctor`'s existing
posture: diagnosis is a read.

### Replay

Worth being precise, because the word covers two different things and only one
of them is useful.

**Retry-replay is worthless here.** Re-submitting the captured transaction
against current state is exactly what the node already does on its watchdog,
and the reason it fails is that state moved. Replaying it adds nothing the
next tick would not do.

**Forensic replay is the value.** `antegen node parked <pubkey>` printing the
decoded instruction and each account's state at failure is the thirty minutes
of archaeology, pre-done.

**One live variant is worth having:** `antegen node parked <pubkey> --recheck`
re-simulates the *captured instruction* against *current* state and reports
whether it would still fail. That answers "is it fixed yet" without waiting
for a watchdog, and it is nearly free once the capture exists.

### Observability

The register is already the shape metrics want:

```
antegen_parked_threads{outcome="fatal"}
antegen_parked_oldest_seconds
antegen_parked_by_signature{program,error_code}
```

`antegen_parked_oldest_seconds` is the one that matters. It would have
surfaced this at twenty minutes instead of ten hours.

## Alternatives considered

**Structured logging only** — emit the capture as one JSON WARN line per
signature and let a log pipeline own it. Cheaper, and searchable if a pipeline
exists. Rejected as the primary answer because it still requires knowing the
right `journalctl` invocation, which is the failure that started this, and
because it gives the node no in-process state to serve metrics from.

**Admin socket / localhost HTTP** — real-time and queryable, but adds a
listening surface and an auth decision, and is unavailable exactly when the
node has fallen over.

**Push to `api.loa.sh` only** — the pipe exists (`actors/observability.rs`),
but it requires network egress and does not help a local debug session.

The file does not preclude either. Both become straightforward once the
register exists in process.

## Cost

Contained to `antegen-client` and `antegen-cli` — no program change, no IDL
change, no coordinated deploy. That is the main argument for doing it properly
rather than adding another log line.

Watch items:

- **Capture size.** Fourteen accounts of up to 10 KB each is 140 KB per entry
  if uncapped. Cap per entry and truncate `data`.
- **Hot path.** Serialization must not run on the actor loop; the eviction and
  reconcile paths already establish the off-loop pattern.
- **Register growth.** Bounded by fleet size by construction, but eviction
  policy is still needed (see open questions).

No secret material is involved — everything captured is public chain state.

## Testing

Per the post-mortem's standard, each of these should be observed failing
against a build without the change:

- 100 identical failures produce one capture and `attempts: 100`.
- A changed signature replaces the capture and resets the backoff.
- Log output for the second and subsequent identical failures is `debug`, not
  `warn`.
- The register survives a node restart and the CLI reads it with the node
  stopped.
- A capture with oversized account data is truncated, and `owner`/`lamports`/
  `len` survive truncation.

## Open questions

- **Eviction.** An entry clearly leaves on success and on thread deletion.
  Does it also expire on age? A thread parked for a week is still the most
  interesting row in the table, which argues against a TTL.
- **Restart authority.** On startup, is the loaded file authoritative, or
  advisory until each thread is re-observed failing? Loading it as truth risks
  reporting a stale park for something that has since recovered.
- **Key granularity.** Keyed by thread, or by `(thread, fiber_index)`? A
  multi-fiber thread can fail differently per fiber, and the signature would
  thrash between them under a thread-only key.
- **Does the register belong in `antegen-client` at all,** or should the
  capture be emitted and a separate tool own retention? Keeping it in-process
  is what makes the gauges possible, which is the argument for here.
