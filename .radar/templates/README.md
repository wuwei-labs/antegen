# Custom radar templates

Rules that radar does not ship, kept here and merged with the builtin set by
`.githooks/pre-commit`. `radar -t` *replaces* the builtin templates rather than
adding to them, so anything that runs these has to merge the two directories
first — running with `-t .radar/templates` alone silently drops the other 38
rules and looks clean for the wrong reason.

## `unvalidated_manual_account_access.yaml`

Flags an instruction handler that reads or writes an `UncheckedAccount`'s raw
data without proving which account it was handed — no address derivation, no
key comparison, nothing Anchor checked on its behalf.

Written after the [2026-08-26 fiber rent
sweep](../../post-mortems/2026-08-26-fiber-rent-sweep.md), because **none of
radar's 38 builtin rules could see that bug.** Scanning the vulnerable commit
(`e63a302`) and the fix (`163d906`) produces identical findings — same rules,
same files, only line numbers shifted by the 15 lines the patch added.

The reason is worth understanding before writing rules of your own: every
builtin that could plausibly have caught it evaluates its safety condition at
**file** scope. `fiber::create` derived and checked the fiber PDA in
`initialize_fiber` and skipped it entirely on the update-in-place branch, which
is what made the exploit possible. A file-scoped rule sees a
`find_program_address` *somewhere* in `create.rs` and falls silent. No
file-scoped rule can distinguish "validated on every path" from "validated on
one path" — which is exactly the shape this class of bug takes.

So this rule iterates functions, and restricts itself to instruction handlers
(those taking a `Context`) so that byte-level helpers, which legitimately
receive an already-validated `AccountInfo`, do not produce noise.

### Validation

| Tree | Result |
|---|---|
| `e63a302` (vulnerable) | fires at `create.rs:57` — the exploited branch |
| `163d906` (patched) | `create.rs` clears |
| current `main` | clean, no false positives |

At the vulnerable commit it also flags `close.rs`, `swap.rs` and `update.rs`,
which were genuinely unproven at that point — `close` was the second half of
the exploit chain, trusting a `state.thread` that `create` had just rewritten.

## Known limitation in the builtin rules

Thirteen of radar's 38 Anchor templates suppress at file scope while iterating
per-item, so satisfying the rule *once* in a file hides every other instance in
that file. Three are High severity. Minimal reproduction:

```rust
#[derive(Accounts)]
pub struct Touch<'info> {
    pub signer: Signer<'info>,

    /// CHECK: constrained
    #[account(address = crate::ID)]
    pub checked: UncheckedAccount<'info>,

    /// CHECK: NOT constrained — a caller can substitute any account
    #[account(mut)]
    pub wide_open: UncheckedAccount<'info>,
}
```

`Unconstrained UncheckedAccount` (High) reports this file when *both* accounts
are bare, and goes silent as soon as `checked` gains its `address` constraint —
while `wide_open` stays exactly as exposed as it was.

The practical consequence: **a clean radar run means each file contains at
least one instance of the expected pattern, not that every account is
checked.** Do not read a green scan as proof that newly added accounts are
constrained.

Affected builtins: `Closing Accounts Insecurely`, `Duplicate Mutable Accounts`,
`Init If Needed Reinitialization`, `Missing Bump Seed Canonicalization`,
`Missing has_one Constraint`, `Missing Owner Check`, `Missing Signer Check`,
`Missing Token Authority Constraint`, `Missing Token Mint Constraint`, `PDA
Sharing`, `Unchecked Close Target`, `Unconstrained UncheckedAccount`,
`Unvalidated Sysvar Account`.

`Missing Signer Check` is the sharpest edge of the thirteen: its suppression is
`nodes.find_by_names("Signer").exit_on_value()`, so a single `Signer<'info>`
anywhere in a file disables the check for every other instruction defined in
it.

This part is fixable in the templates alone — the AST does nest each
`#[account(...)]` under its own field, and a rule can scope to it by slicing
`node.access_path` at `.ty` to get the field prefix
(`...struct.fields.named[N]`) and only accepting attributes underneath it. A
scoped rewrite of `Unconstrained UncheckedAccount` correctly reports
`wide_open` in the example above, where the shipped rule reports nothing.

It is only half a fix, though, because of the line-number bug below: the
scoped rule identifies the right *account* but still prints the first
`UncheckedAccount`'s line. Both need fixing for the result to be actionable,
and they live in different places — one in the YAML, one in the core.

## Other radar behaviour worth knowing

- **Its exit status is not a verdict.** radar exits `0` when it has findings
  and `1` when the scan is clean. Gate on the results file, not `$?`.
- **It can serve stale results.** Cached ASTs are cleared by tearing the
  containers down, with a 30-second timeout; on timeout it continues and
  reports the previous run's findings against the current tree, line numbers
  and all. `.githooks/pre-commit` fails rather than trusting such a run.
- **Reported line numbers are unreliable for any repeated identifier.** Spans
  are not taken from the parser. `api/utils/ast.py::enrich_ast_with_source_lines`
  regex-searches the file for each identifier by name and assigns
  `positions[0]` — its first textual occurrence. The branch meant to hand later
  nodes their own position tests `pos not in node.get("src", [])`, comparing
  against the node's own (absent) `src` rather than against the positions
  already consumed, so it always re-assigns the first one.

  Every node sharing an identifier therefore reports the same location. In a
  struct with three `UncheckedAccount` fields, all three report the first
  one's line; a second `Signer` field reports the first `Signer`'s line. It is
  also why `Invoke Signed Unvalidated Seeds` points at
  `use ...::invoke_signed;` — the import is where that identifier first
  appears in the file.

  Treat a finding as naming a *file and a rule*, not a line. Read the rule and
  find the real site yourself.
