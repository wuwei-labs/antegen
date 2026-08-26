# Fiber rent sweep — 2026-08-26

**Status:** patched on mainnet 2026-08-26 10:23:02 UTC; recovery funded and in progress
**Severity:** high impact, narrow surface
**Scope:** `antegen-fiber-program`, `antegen-thread-program`,
`antegen-client`, `antegen-cli`. Downstream impact and its recovery are
written up by the affected integrator.
**Funds at risk:** rent-exempt lamports only. No thread balances, no token
accounts, no authority over anything a fiber's instruction touches.
**Reporter:** [`8HKKBaA8UEgqHsbE9cPzqMk9QLeW1h9P1JspqCw4EcFz`](https://solscan.io/account/8HKKBaA8UEgqHsbE9cPzqMk9QLeW1h9P1JspqCw4EcFz)
(unsolicited, via live exploitation)

## Summary

A missing address check in `antegen-fiber-program`'s `create` instruction let
any wallet claim ownership of an existing fiber account and then close it,
sweeping its rent-exempt balance. In a 17-second window, 623 fiber accounts
were emptied for a total of **5.383511280 SOL**.

The threads that owned those fibers were not touched. They survived intact,
still listing fiber indices whose accounts no longer existed — which is what
stopped them executing, and why the failure surfaced as an executor problem
rather than an on-chain one.

We are treating the swept SOL as a bug bounty. It will not be pursued, and
every destroyed fiber account is being restored at our cost.

## Impact

| | |
|---|---|
| Fiber accounts destroyed | 623 |
| Lamports swept | 5,383,511,280 (**5.383511280 SOL**) |
| Average per account | ~0.00864 SOL (one rent-exempt balance) |
| Threads left unable to execute | 358 (all one integrator) |
| Exploit window | 2026-08-26 08:30:53 → 08:31:10 UTC (17 seconds) |
| Exploit transactions | 623, all successful, all top-level |
| User funds lost | none |

Only rent-exempt lamports were reachable. A fiber account holds a compiled
instruction and its rent — no balances, and no authority over the accounts
that instruction names. The blast radius was every fiber in the program; the
value inside each one was the minimum needed to keep a ~1.2 KB account alive.

Every affected thread happened to belong to a single integrator, which is a
fact about who had adopted fibers by 2026-08-26 rather than anything the
attack selected for. Any fiber in the program was reachable the same way.

The symptom was automation stopping. Executors logged `Fiber <pubkey> not
found` and retried against an address that would never resolve again.

## Root cause

`fiber::create` derived and verified the fiber PDA **only when initializing a
new account**. The branch that handled an account which already had data
performed no derivation check at all — and that branch rewrites the field
recording who owns the fiber:

```rust
// programs/fiber/src/instructions/create.rs — before
if fiber_info.data_len() == 0 {
    initialize_fiber(...)   // ← derives and checks the PDA
} else {
    // ← no PDA check, and this rewrites state.thread to the signer
    state.thread = thread_key;
    state.compiled_instruction = compiled_bytes;
    ...
}
```

`close` guarded itself with `require!(read.thread() == thread_info.key())`.
That check was sound in isolation and useless in practice: `create` could set
`state.thread` to whatever it liked first.

The two instructions composed into a complete bypass:

1. `fiber::create` — signer is the attacker's own wallet, `fiber` is a victim's
   fiber PDA. The account has data, so the unchecked branch runs and sets
   `state.thread = attacker`.
2. `fiber::close` — `read.thread() == signer` now passes. Rent is swept to the
   attacker.

Two instructions in one transaction, 623 times.
[Example transaction.](https://solscan.io/tx/59cNJMT1CSXorTeEBd2Hvf3t61Efb6Msy38rAstDnb2dVMc68Vp1U5YBJipG8y3axS754CdXXmarBmbaWrUBxhRc)

### Why this was easy to miss

The check was not absent from the codebase — it was present, correct, and
three lines away, in the sibling branch of the same `if`. Every neighbouring
instruction was already bound properly: `update` carried an Anchor `seeds`
constraint, `thread_exec` verified both the derived address *and* the stored
owner. `create`'s initialize path did the same. One branch of one instruction
did not, because it was written as "the account already exists, so just
overwrite it" — reasonable, until you notice it also overwrites the field
every downstream ownership check reads.

That is the shape worth naming: **a stored-owner comparison is only as strong
as the weakest instruction that can write it.** `close` looked defended. It
was defended against everything except its own program.

## Timeline

All times UTC, 2026-08-26.

| Time | Event |
|---|---|
| 08:30:53 | First exploit transaction |
| 08:31:10 | Last exploit transaction — 623 accounts, 5.3835 SOL |
| 09:11 | Executor logs surfaced: `Fiber ... not found`, threads retrying |
| ~09:15 | Root cause identified on chain; confirmed not a node bug |
| ~09:30 | Fix written, regression test confirmed failing without it |
| ~10:00 | Full audit of every rent-bearing path completed |
| ~10:00 | [antegen#67](https://github.com/wuwei-labs/antegen/pull/67) merged and released |
| 10:22:28 | Thread program upgraded on mainnet ([tx](https://solscan.io/tx/4FwAQUmiYtKszFjYZJESrqb6wVQuvBg5jX6UQuDqHcMgWEHvbFEAj1ULm7MjopKapaJUhBfBVPDQXP932H4rELgK)) |
| 10:23:02 | Fiber program upgraded ([tx](https://solscan.io/tx/2KPWpvghDUoLbzeLn8nMVR829txh1QfXkXRdag3XAykt7R9oy1LSQqGhhxonNnqW4ZLbMj8b7U8mjuVNvip8QT6L)) — vulnerability closed |

The gap between exploitation and detection was ~40 minutes, and it was closed
by an executor's error logs rather than by any alerting we had in place. Going
from those logs to "358 threads are wedged and here is what each is missing"
then took hand-written queries against chain, because no tool could answer it.
`antegen thread doctor` exists because of that; see below.

## Fix

[antegen#67](https://github.com/wuwei-labs/antegen/pull/67).

**The vulnerability.** `create` now derives and checks the PDA before either
branch, so the signer must be the thread the account actually belongs to:

```rust
require!(
    Pubkey::find_program_address(
        &[SEED_THREAD_FIBER, thread_key.as_ref(), &[fiber_index]],
        &crate::ID,
    ).0.eq(&fiber_info.key()),
    AntegenFiberError::InvalidFiberPDA
);
```

Every writer of `state.thread` is now address-bound: `create` (both branches),
`update` (Anchor `seeds`). Forging the field now requires a SHA-256 preimage.

**Hardening found during the audit.** `close` gained a required `fiber_index`,
bound by seeds — a breaking change, taken deliberately. Its stored-owner check
inherited its guarantee from the write paths rather than holding one, and it
could not tell which index an account represented.

**A second, unrelated bug the audit surfaced.** `fiber_close` and `fiber_swap`
in the thread program removed one index from `fiber_ids` while closing whichever
account was passed, with nothing cross-checking the two. A mismatched pair left
a thread naming an account that was gone and stranded the other's rent
permanently. Authority-only, never exploited, now rejected by seeds constraints.

**The executor's response, in `antegen-client`.** A build failure on a missing
account was classified retryable, so each affected thread was re-dispatched on
a backoff schedule forever, spending one `getAccount` per attempt on an address
that would never resolve. Every rebuild derives the same PDA and gets the same
null back, so this is fatal rather than transient. It now parks: the watchdog
still re-examines the thread, and an account update re-arms it the moment the
fiber returns.

This did not cause the incident, but it is why 358 dead threads produced a
sustained load against RPC instead of quietly going idle, and it is the reason
the logs were loud enough to notice at all.

**Tests.** Six adversarial tests now pin every path by which rent can leave a
fiber. The exploit test was confirmed to fail without the fix, not merely to
pass with it.

## Recovery

Fiber contents are not recoverable from chain state — `close` zeroes the
account. The only surviving record is the transaction history that wrote them.

`antegen thread doctor` reconstructs them. It walks each thread's history,
decodes every instruction that ever wrote a fiber — including the CPI'd ones,
which are the majority — and folds them in slot order. Folding rather than
reading the creation transaction is the point; many fibers were rewritten by a
later `update_fiber`, so creation data alone restores stale content.

```
antegen thread doctor                       # which threads cannot execute
antegen thread doctor --verify              # prove the reconstruction
antegen thread doctor --reconstruct \
    --output one.json --limit 1             # rebuild a single fiber
antegen thread doctor --confirm one.json    # check what landed
```

Diagnosis is cheap — an existence check per tracked fiber. `--reconstruct` is
the expensive part, a full history walk per affected thread, which is why it is
its own flag rather than implied by naming an output file.

The command is read-only and what it produces is a manifest. It deliberately
cannot replay: writing a fiber requires the owning *thread's* authority to sign, which
is a property of whoever created the thread, not of antegen. Where that
authority is a program PDA, only that program can produce the transaction.
Recovery therefore belongs to each integrator, and this repository's
contribution is the reconstruction plus two independent proofs of it:

- `--verify` replays history for fibers the attacker did not reach and diffs
  the result against their live on-chain state. Every survivor reproduces
  byte-identically, including one folded from 15 successive writes. This is the
  evidence that a reconstruction is trustworthy *before* anything is replayed.
- `--confirm` diffs what landed on chain against the manifest that was
  replayed. It deliberately does not re-derive from history, because by then
  the replay transaction is itself part of that fiber's history and a
  history-based check would fold in the write it is meant to be checking.

`--limit 1` exists so an operator can rebuild one fiber, replay it, confirm
it, and only then continue. Both proofs exit non-zero on mismatch so a batch stops rather
than grinding through a bad reconstruction.

## What we are changing beyond the patch

Four of these shipped. One has not started, and is marked so rather than left
to blur into the others — an action list where done and not-done read alike is
how the not-done part gets forgotten.

1. **Address-bind every account, always.** *(shipped, [#67](https://github.com/wuwei-labs/antegen/pull/67))*
   A stored field naming an owner is a convenience, not a boundary. Where an
   account's address encodes its relationship, derive and check it — even when
   another instruction "already validated" it. Every fiber read and write path
   is now bound: `create` and `close` derive explicitly, `update`,
   `fiber_close` and `fiber_swap` carry `seeds` constraints, and `thread_exec`
   checks the derived address *and* the stored owner.

2. **Audit by capability, not by instruction.** *(done during the response)*
   Enumerating every path that can decrease an account's lamports and asking
   what authorizes each found the `fiber_close`/`fiber_swap` desync, which
   reviewing instructions one at a time had not. Stated here as the method to
   reach for next time, not as pending work.

3. **Make the failure diagnosable.** *(shipped — `antegen thread doctor`)*
   Working out that 358 threads were wedged, and why, meant reading executor
   logs and then querying chain by hand. Nothing in the tooling could answer
   "which threads cannot execute, and what is missing". `thread doctor` is that
   question as a command, and it outlives this incident: `fiber_ids` and the
   accounts it names are not kept in agreement by anything on chain, so a
   thread can be intact, still scheduled, and permanently unable to build for
   reasons that have nothing to do with an attacker. It exits non-zero when any
   thread is unhealthy, which is what lets it run as a check rather than only
   as a forensic tool after someone has already noticed.

4. **Alert on the symptom.** *(outstanding — nothing built)* 623 accounts
   closed by one wallet in 17 seconds should page someone. It did not.
   Hundreds of threads failing to build against the same error should page
   someone too. It did not. There is no alerting in this repository today:
   `observability.rs` carries actor lifecycle and nothing that watches for a
   condition. Detection depended on a person reading executor logs 40 minutes
   later, and nothing has changed about that. This is the real outstanding
   item from the incident and it is larger than the bug was.

5. **Test the negative.** *(shipped for this incident; standing practice
   thereafter)* A regression test that has never been observed to fail is not
   evidence of anything. The exploit test was confirmed failing against a build
   without the fix before being trusted, and the recovery tooling's `--verify`
   and `--confirm` were each proven to reject a deliberately corrupted input.

## Thanks

To [`8HKKBaA8UEgqHsbE9cPzqMk9QLeW1h9P1JspqCw4EcFz`](https://solscan.io/account/8HKKBaA8UEgqHsbE9cPzqMk9QLeW1h9P1JspqCw4EcFz):
thank you. Finding this required composing two instructions that each look
correct on their own and spotting that one populates the other's only guard.
That is genuinely good work, and it was found in a program where every
neighbouring code path was already bound correctly.

The 5.383511280 SOL is yours. We consider it a bug bounty, we are not pursuing
it, and we would rather have paid it now than discovered this later with more
at stake — the same composition against a program holding user balances would
have been a materially worse day for everyone.

If you would like to look at the rest of the program surface, we would welcome
it, and we would rather hear from you first next time. Open an issue, or reach
us at the contact in the repository, and we will treat it as a disclosure with
a bounty attached rather than a race.

## Deployment

Both programs upgraded on mainnet-beta on 2026-08-26, signed by the config
admin.

| Program | Address | Upgrade transaction | Slot | Time (UTC) |
|---|---|---|---|---|
| `antegen-thread-program` | [`AgTv5w…4dpSx`](https://solscan.io/account/AgTv5w1UvUb6zeqkThwMrztGu9hpepBu8YLghuR4dpSx) | [`4FwAQUmi…rELgK`](https://solscan.io/tx/4FwAQUmiYtKszFjYZJESrqb6wVQuvBg5jX6UQuDqHcMgWEHvbFEAj1ULm7MjopKapaJUhBfBVPDQXP932H4rELgK) | 441846236 | 10:22:28 |
| `antegen-fiber-program` | [`AgFv5a…e1hKx`](https://solscan.io/account/AgFv5afjW9DmSPkiEvJ1er5bAAmRUqaBeTB6Cr8e1hKx) | [`2KPWpvgh…8QT6L`](https://solscan.io/tx/2KPWpvghDUoLbzeLn8nMVR829txh1QfXkXRdag3XAykt7R9oy1LSQqGhhxonNnqW4ZLbMj8b7U8mjuVNvip8QT6L) | 441846329 | 10:23:02 |

The thread program went first, 34 seconds ahead of the fiber program. That
order was deliberate. Making `fiber_index` mandatory on `fiber::close` is a
wire break, and the two programs disagree about it in only one direction: the
old fiber program ignores the trailing index byte the new thread program sends,
because Anchor dispatches instruction data with `deserialize` and tolerates
trailing bytes. Deploying fiber first would instead have left the old thread
program sending no index at all to an instruction that now requires one,
breaking `fiber_close` and `thread_close` until the second upgrade landed.
Thread-first has no such window.

The vulnerability closed with the fiber upgrade at 10:23:02 — **1h 51m 52s**
after the last exploit transaction. The reporter has sent no transaction since
08:39:14, well before either upgrade, and no fiber account has been swept
since. Fiber accounts are being created again and are staying alive.
