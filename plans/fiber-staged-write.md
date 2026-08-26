# Staged fiber writes

**Status:** proposed, not started
**Scope:** `antegen-fiber-program`, `antegen-thread-program`, `antegen-cli`
**Follows from:** [`post-mortems/2026-08-26-fiber-rent-sweep.md`](../post-mortems/2026-08-26-fiber-rent-sweep.md)

## The problem

A fiber's instruction is delivered in a single transaction, as an argument to
`update`. That caps what a fiber can ever hold at whatever fits in 1232 bytes
alongside the accounts required to authorize the write — and the cap binds well
before the fiber account itself is full.

This surfaced during the recovery, repairing SRSLY contract threads. Their two
fiber entrypoints:

| fiber | instruction | accounts | repair transaction |
|---|---|---|---|
| 0 | `ProcessContract` | 13 | 899 B — fits |
| 1 | `CloseRental` | 26 | **1387 B — 155 B over** |

The account metas are the bulk of it: 26 × 34 = 884 bytes, carried as
instruction *data*, because the repair instruction takes the whole
`SerializableInstruction` as an argument. They are not accounts of the repair
transaction, so nothing about the transaction's own account list can compress
them.

The integrator worked around it with an address lookup table over the repair's
own nine accounts, which bought back 214 bytes and landed the transaction at
1173. That works, and it is the right thing to have done under incident
pressure, but it is a client-side workaround with two problems:

- **It is nearly exhausted on arrival.** The table buys back 214 bytes, and two
  compute-budget instructions spend most of it. Both are necessary, and each was
  found by a transaction failing in a way that named something else:

  | | bytes |
  |---|---|
  | bare repair | 1387 |
  | `SetComputeUnitPrice` — without a fee, a 197k-CU transaction is passed over by leaders and expires unconfirmed | 1431 |
  | `SetComputeUnitLimit` — `CloseRental` that also promotes a queued rental exceeds the 200k default | 1439 |
  | **compressed via the table** | **1225 of 1232** |

  Seven bytes. One more account in the payload, or any third budget
  instruction, and no client-side arrangement fits.
- **It does not generalize.** The saving is bounded by the repair's own account
  count. A fiber payload four accounts larger than `CloseRental` cannot be
  written by any client, however clever.

## Why this belongs in antegen

The fiber account already *is* the state account that holds a serialized
instruction, and it is allocated at a fixed size regardless of payload:

```rust
// programs/fiber/src/state/fiber.rs
#[max_len(1024)]
pub compiled_instruction: Vec<u8>,

// programs/fiber/src/instructions/create.rs
let space = 8 + FiberVersionedState::INIT_SPACE;   // fixed, not payload-sized
```

Storage for a large payload is already reserved and already paid for. The
bottleneck is purely transport: one transaction must carry the whole payload at
once. That is the same shape as a program deployment, where the BPF loader
writes code into a buffer account across many `Write { offset, bytes }`
transactions and the final `Upgrade` merely names the buffer — and the answer
should be the same shape too, except that here no buffer account is needed
because the destination is already allocated. The fiber is its own buffer.

Solving it in each consumer program means every consumer re-solves it, and each
one is capped by its own account layout. Solving it here removes the ceiling
once.

## Design

Two instructions on the fiber program, mirrored by the thread program's
authority-checked wrappers:

```
fiber_stage(fiber_index, offset, chunk)   // write bytes at offset
fiber_commit(fiber_index, priority_fee)   // seal; fiber becomes executable
```

### A torn fiber must not execute

A half-staged fiber is still an allocated, thread-owned account. If
`thread_exec` will run whatever it finds, a partial write is worse than a
missing fiber: a missing one fails to build and stops, a torn one may build
into something that was never intended.

`Fiber` is already an enum over `Legacy` and `V1` with an explicit `version`
byte, and `thread_exec` already matches on it. A `Staged` variant fits that
existing shape, and makes "not yet sealed" unrepresentable-as-executable rather
than a flag someone must remember to check.

Staging must therefore be a distinct state, not an in-place mutation of a live
`V1` fiber. Writing chunks directly over a functioning fiber's
`compiled_instruction` would break it for as long as the write is in flight,
which for a multi-transaction write is unbounded.

### Chunk count is small

Each chunk still needs the thread's authority to sign, and for a PDA authority
that means routing through the consumer program. For SRSLY that is
srsly → thread → fiber with nine accounts of overhead, leaving roughly 900
usable bytes per transaction. `CloseRental`'s ~932-byte payload is **two chunks
plus a commit** — a short write, not a long loop.

### Funding

`fiber_update` currently pre-funds a new fiber out of the thread's lamports:

```rust
// programs/thread/src/instructions/fiber_update.rs
**thread.to_account_info().try_borrow_mut_lamports()? -= rent_lamports;
**fiber_info.try_borrow_mut_lamports()? += rent_lamports;
```

This is a raw lamport move with no rent check on the thread; the runtime checks
at end of transaction and rejects with `InsufficientFundsForRent` against the
*thread*, after both programs have returned success. During the recovery that
read as an executor failure rather than a funding one, and cost time to
attribute.

Staging should decide funding deliberately rather than inherit this:

- The rent moves once, at allocation, not per chunk.
- A staging write that cannot complete should not be able to leave the thread
  below its own rent-exempt floor. Preferably the thread's solvency is checked
  where the deduction happens, so the error names the real cause.
- Worth considering whether the *payer* should fund the fiber directly rather
  than the thread, which is what the recovery tooling ended up doing by hand.

## The other ceiling

`max_len(1024)` against `CloseRental`'s ~932 serialized bytes leaves roughly
**two more accounts** of room. Staging removes the transport limit and would
then run straight into the storage limit almost immediately.

Raising `max_len` is a size change to an allocated account and therefore a
migration question for existing fibers, not a one-line edit. It should be
decided in this change rather than discovered after it, because shipping
staging alone buys very little headroom.

## Alternatives considered

**Lookup table only** — shipped downstream, measured, works today for
`CloseRental` at 1217/1232. Rejected as the durable answer: bounded by the
caller's account count, and already almost full.

**A separate buffer account, loader-style** — closer to how deploys work, but
the fiber is already allocated at full size, so a second account adds rent,
another PDA, and a copy step to reach the same place.

**Raise `max_len` alone** — does nothing. Storage is not the current binding
constraint; transport is. It becomes necessary *after* staging, not instead
of it.

## Cost

A fiber-program change means a new `antegen-fiber-program` version, which
consumers pin — SRSLY pins `antegen-fiber-program = "5.2.0"` — and antegen
deploys before its consumers. Two coordinated deploys, in order. That is the
main reason this was not attempted during the incident and should not be
rushed now: the lookup-table workaround has the affected contracts moving.

## Testing

Per the post-mortem's fifth item, each of these must be observed failing
against a build without the change before it is trusted:

- A staged-but-uncommitted fiber is refused by `thread_exec`.
- A commit over a payload assembled from chunks produces bytes identical to the
  same payload written in one shot by `update`.
- A chunk written at a bad offset, or a commit with a gap in the staged range,
  is rejected rather than sealed.
- Staging cannot leave the thread below its rent-exempt floor.
- A payload at the raised `max_len` boundary round-trips; one byte past it is
  rejected.

## Open questions

- Should `fiber_stage` be reachable only through the thread program, or
  directly on the fiber program with the same authority check? The former keeps
  one authorization path; the latter costs fewer accounts per chunk.
- Does a staged fiber expire? An abandoned staging state holds rent and
  occupies an index. A commit deadline, or a `fiber_stage_cancel`, may be
  needed.
- Should `update` remain as the single-transaction fast path for payloads that
  fit, or become a thin wrapper over stage+commit? Keeping both means two write
  paths to keep in agreement — the exact shape that produced the
  `fiber_close`/`fiber_swap` desync behind this post-mortem.
