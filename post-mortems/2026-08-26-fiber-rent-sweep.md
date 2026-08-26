# Fiber rent sweep — 2026-08-26

**Status:** resolved, patched, recovery funded
**Severity:** high impact, narrow surface
**Funds at risk:** rent-exempt lamports only. No user deposits, no ATLAS, no
escrow, no thread balances.
**Reporter:** [`8HKKBaA8UEgqHsbE9cPzqMk9QLeW1h9P1JspqCw4EcFz`](https://solscan.io/account/8HKKBaA8UEgqHsbE9cPzqMk9QLeW1h9P1JspqCw4EcFz)
(unsolicited, via live exploitation)

## Summary

A missing address check in `antegen-fiber-program`'s `create` instruction let
any wallet claim ownership of an existing fiber account and then close it,
sweeping its rent-exempt balance. In a 17-second window, 623 fiber accounts
were emptied for a total of **5.383511280 SOL**.

The 358 threads that owned those fibers were not touched. They survived
intact, still listing fiber indices whose accounts no longer existed, which
stopped them executing.

We are treating the swept SOL as a bug bounty. It will not be pursued. Every
destroyed fiber account is being restored at our own cost.

## Impact

| | |
|---|---|
| Fiber accounts destroyed | 623 |
| Lamports swept | 5,383,511,280 (**5.383511280 SOL**) |
| Average per account | ~0.00864 SOL (one rent-exempt balance) |
| Threads left unable to execute | 358 |
| Exploit window | 2026-08-26 08:30:53 → 08:31:10 UTC (17 seconds) |
| Exploit transactions | 623, all successful, all top-level |
| User funds lost | none |

Only rent-exempt lamports were reachable. A fiber account holds a compiled
instruction and its rent — no balances, no authority over tokens. The blast
radius was every fiber in the program; the value inside each one was the
minimum needed to keep a ~1.2 KB account alive.

The user-visible symptom was automation stopping. Executors logged
`Fiber <pubkey> not found` and retried against an address that would never
resolve again.

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
| same day | [antegen#67](https://github.com/wuwei-labs/antegen/pull/67) merged and released |

The gap between exploitation and detection was ~40 minutes, and it was closed
by an executor's error logs rather than by any alerting we had in place. That
is its own finding.

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

**Tests.** Six adversarial tests now pin every path by which rent can leave a
fiber. The exploit test was confirmed to fail without the fix, not merely to
pass with it.

## Recovery

Fiber contents are not recoverable from chain state — the accounts were zeroed.
They are reconstructed by replaying each fiber's creation and every subsequent
update from transaction history, in slot order. Replaying only the creation
would restore stale content; many fibers were rewritten after creation.

The reconstruction is verified against the fibers the attacker did not reach:
every survivor reproduces byte-identically from history, including one folded
from 15 successive writes.

Restoring a fiber requires the owning thread's authority to sign. For the
affected threads that authority is a program PDA with no private key, so
recovery runs through the owning program under an admin-gated instruction that
can only stage the two instruction shapes those fibers are allowed to hold.

Every recreated account is funded by us. Nobody who had a thread stop is being
asked to pay to restart it.

## What we are changing beyond the patch

1. **Address-bind every account, always.** A stored field naming an owner is a
   convenience, not a boundary. Where an account's address encodes its
   relationship, derive and check it — even when another instruction "already
   validated" it.
2. **Audit by capability, not by instruction.** The fix came from enumerating
   every path that can decrease an account's lamports and asking what
   authorizes each, rather than reviewing instructions one at a time. The
   `fiber_close`/`fiber_swap` desync was found this way and would not have been
   found otherwise.
3. **Alert on the symptom.** 623 accounts closed by one wallet in 17 seconds
   should page someone. It did not. Repeated build failures across many threads
   should page someone too. They did not.
4. **Test the negative.** A regression test that has never been observed to
   fail is not evidence of anything.

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
