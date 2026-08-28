# Security Policy

Antegen runs automation on Solana mainnet. The on-chain programs custody
lamports on behalf of thread owners, so a bug in them can lose user funds.
Please report anything that looks like it can.

## Reporting a vulnerability

Email **anthony@wuwei.dev**. Do not open a public GitHub issue, and do not
disclose publicly until a fix is deployed.

Include whatever you have:

- Which program (`antegen-thread-program`, `antegen-fiber-program`) and which
  instruction.
- The program ID and cluster you observed it on.
- Steps to reproduce, ideally a failing `litesvm` test or a transaction
  signature.
- What an attacker gains — stolen lamports, an unauthorized close, a stuck
  thread, a denial of service.

We will acknowledge receipt within 72 hours and give you an assessment and a
rough timeline within 7 days.

## Scope

In scope:

| Program | ID |
|---|---|
| `antegen-thread-program` | `AgTv5w1UvUb6zeqkThwMrztGu9hpepBu8YLghuR4dpSx` |
| `antegen-fiber-program` | `AgFv5afjW9DmSPkiEvJ1er5bAAmRUqaBeTB6Cr8e1hKx` |

Also in scope: the `antegen-client` and `antegen-cron` crates, the geyser
plugin, and the CLI/node daemon, where a flaw there causes incorrect on-chain
behaviour (mis-scheduled executions, a crank that can be induced to sign
something it should not).

Out of scope:

- `programs/reentrance-test` — a test-only program, never deployed.
- Findings that require a compromised validator, a compromised keypair, or
  physical access.
- Denial of service by paying for it (spamming transactions, exhausting your
  own thread balance).
- Anything already described in [`post-mortems/`](./post-mortems).

## Disclosure

We aim to ship a fix, deploy it, and publish a post-mortem under
[`post-mortems/`](./post-mortems). You are credited there unless you ask not to
be. There is no paid bounty program today; we will say so up front rather than
leave it ambiguous.

## Audits

The programs are **unaudited**. The `auditors` field in each program's on-chain
`security.txt` reads `None` and will be updated when that changes.

## On-chain `security.txt`

Both deployed programs embed this contact information via
[`solana-security-txt`](https://github.com/neodyme-labs/solana-security-txt),
so explorers can surface it directly from the deployed binary. The values live
in each program's `src/lib.rs`; if you change the contact address here, change
it there too or the two disagree for anyone reading the chain.
