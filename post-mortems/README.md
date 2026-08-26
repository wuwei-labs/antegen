# Post-mortems

Incident write-ups for antegen: the programs in this repository, the client
that executes against them, and the tooling around both.

Scope is antegen's own surface. When an incident here reaches a downstream
integrator, that integrator writes up their own impact and recovery in their
own repository; this one covers the defect, the fix, and what antegen changes
because of it. Cross-link rather than duplicate — two write-ups drifting apart
is worse than one.

One file per incident, named `YYYY-MM-DD-short-slug.md`, dated by when the
incident *started* rather than when it was found or fixed.

## Index

| Date | Incident | Impact |
|------|----------|--------|
| 2026-08-26 | [Fiber rent sweep](./2026-08-26-fiber-rent-sweep.md) | 623 fiber accounts emptied, 5.3835 SOL, 358 threads halted. No user funds. |

## What belongs here

Anything in antegen that reached mainnet and cost money, downtime, or trust:
exploited vulnerabilities, wrong state written on chain, releases that broke
deployed programs, outages in the executor network.

Near-misses caught before deploy do not need a write-up. A vulnerability found
by an outside party does, whether or not it was exploited.

## Writing one

Be exact and be brief. Numbers come from chain, not from memory — every figure
in a post-mortem should be reproducible by someone re-running the query.

Cover, in roughly this order:

- **Summary** — what happened, in a paragraph.
- **Impact** — a table of measured quantities. Say explicitly what was *not*
  affected; readers assume the worst otherwise.
- **Root cause** — the actual code, before and after. If it is subtle, say why
  it was subtle. Post-mortems that make a bug sound obvious teach nobody
  anything and are usually dishonest.
- **Timeline** — UTC. Include detection, and be honest about the gap between
  the incident and noticing it.
- **Fix** — what shipped, and what the audit turned up beyond the original bug.
- **What we are changing** — the class of mistake, not the instance.

Name external reporters and thank them, unless they ask otherwise. If a bounty
was paid, state the amount.

Write about decisions and code, not about people. The goal is that someone
reading in a year understands the failure well enough to avoid its shape
elsewhere.
