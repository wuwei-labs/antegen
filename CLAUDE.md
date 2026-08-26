# Antegen

## Commit conventions (read before committing)

These rules are not enforced by CI — nothing rejects a commit that breaks
them. They matter anyway: `release-please` reads commit subjects to decide
version bumps and write changelogs, so a malformed subject silently produces a
wrong release. AI assistants helping with commits — Claude Code or otherwise —
must follow them too.

1. **Conventional Commits format** for the commit subject:
   ```
   <type>(<scope>): <description>
   ```
   - Allowed types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`,
     `test`, `build`, `ci`, `chore`, `revert`.
   - Allowed scopes (optional): `cron`, `client`, `ws`, `thread`,
     `fiber`, `cli`, `geyser`. Scope tells
     `release-please` which package's changelog the commit belongs to.
   - Append `!` after type/scope (or include a `BREAKING CHANGE:`
     footer) for breaking changes — these become a major version bump.
   - Subject starts with a letter and reads as a present-tense imperative.
2. **No `Co-Authored-By:` footer.** Do not list AI assistants (Claude
   Code, Copilot, etc.) as co-authors.
3. **Never bump versions or edit `CHANGELOG.md` manually** (except
   inside a Release PR opened by release-please). See "Versioning &
   Changelog Protocol" below.

Examples:

```
feat(thread): add cleanup_stale_signal instruction

Allows recovery of signal accounts whose thread is gone. Refunds
rent to the original payer.
```

```
fix(cron): clamp DOM/DOW interaction to POSIX semantics
```

## Versioning & Changelog Protocol

Versioning and `CHANGELOG.md` updates are **automated by `release-please`**.
Contributors do not bump versions or write changelog entries by hand.

### How releases work

1. Land code on `main` via PR.
   - Prefer a **merge commit** when the branch spans several packages, so
     each commit keeps its own scope and release-please attributes it to the
     right changelog. Squash-merging collapses everything into the PR title,
     which then becomes the only input release-please sees — and since nothing
     validates that title any more, a malformed one bumps the wrong versions
     with no warning.
2. `release-please.yml` runs on every push to `main`. For each package
   with new conventional commits since its last release tag, it opens
   or updates a single Release PR that:
   - Bumps the version in that package's `Cargo.toml`.
   - Prepends a new entry to that package's `CHANGELOG.md`.
3. Maintainer reviews the Release PR. Edit any entries inline to add
   "why" context, polish wording, group related items, etc.
4. Merge the Release PR. release-please then:
   - Creates per-package tags (format: `<component>-v<X.Y.Z>`).
   - Creates a GitHub Release for each tagged package.
   - Triggers `verifiable-build` (programs), `publish-crates`, and
     binary attach jobs in the same workflow run.

### Conventional Commits → semver mapping

| Commit type | Version bump |
|-------------|--------------|
| `fix:` | patch |
| `feat:` | minor |
| `feat!:` or any commit with `BREAKING CHANGE:` footer | major |
| `chore:`, `docs:`, `style:`, `refactor:`, `perf:`, `test:`, `build:`, `ci:` | no version bump (still appears in changelog under appropriate section) |

Use a scope to direct the change at a specific package:
```
feat(thread): add resume_paused instruction
fix(cron): handle 29-Feb edge in non-leap years
```

### Component / tag mapping

`release-please` produces per-package tags using the `component` field
in `.github/release-please-config.json`. The current mapping:

| Path | Component | Tag format | Publish target |
|------|-----------|-----------|----------------|
| `crates/cron` | `antegen-cron` | `antegen-cron-v<X.Y.Z>` | crates.io |
| `crates/client` | `antegen-client` | `antegen-client-v<X.Y.Z>` | crates.io |
| `crates/ws` | `antegen-ws` | `antegen-ws-v<X.Y.Z>` | crates.io |
| `programs/thread` | `antegen-thread-program` | `antegen-thread-program-v<X.Y.Z>` | crates.io + verifiable `.so` |
| `programs/fiber` | `antegen-fiber-program` | `antegen-fiber-program-v<X.Y.Z>` | crates.io + verifiable `.so` |
| `crates/cli` | `antegen-cli` | `antegen-cli-v<X.Y.Z>` | `antegen` binary (CLI **and** node daemon) |
| `plugin/geyser` | `antegen-geyser-plugin` | `antegen-geyser-plugin-v<X.Y.Z>` | binary only (`publish = false`) |

`programs/reentrance-test` is a test-only program (`publish = false`)
and is **not** tracked by release-please.

### Downstream consumption (e.g. `wuwei-labs/srsly`)

Per-program tags + sha256-attested `.so` artifacts let downstream
consumers pin antegen without sibling-cloning this repo:

```toml
# In a downstream Cargo.toml
antegen-thread-program = { git = "https://github.com/wuwei-labs/antegen", rev = "<sha>" }
# or, once published:
antegen-thread-program = "5.0.12"
```

Verifiable program binaries are downloadable from each release, e.g.
`https://github.com/wuwei-labs/antegen/releases/download/antegen-thread-program-v5.0.12/antegen_thread_program.so`

### Cross-package coupling

When a change spans multiple packages, write **separate commits** with
the appropriate scope so each package's changelog reflects its own
changes. Example: a thread-program change that requires a client bump:

```
feat(thread): add resume_paused instruction
chore(client): wire resume_paused into ThreadClient
```

Common couplings to watch:
- `antegen-thread-program` IDL change → bump `antegen-client`
- `antegen-cron` API change → bump `antegen-thread-program`
- `antegen-client` change → bump `antegen-cli`, or the fix never reaches a
  binary (release-please attributes by path and cannot see the dependency)

### `last-release-sha`, and why it is there

`.github/release-please-config.json` pins `last-release-sha`. It tells
release-please not to look at commits at or before that SHA, which is the
release commit where `antegen-fiber-program` reached 6.0.0 and
`antegen-thread-program` 5.2.3.

It is load-bearing. Commit `07652fa` carries a `Release-As: 5.2.0` trailer,
added in May to force those two packages back down after release-please cut a
phantom v6.0.0 off `!` markers. That trailer is normally inert — once 5.2.0
shipped it sits behind the last-release boundary and is never read again. But
if release-please ever loses that boundary and re-scans from the start of
history, it finds the trailer, honours it, and proposes rolling both packages
back to 5.2.0. That happened on 2026-08-26 against a fiber 6.0.0 that was
already tagged, released, and deployed to mainnet.

You cannot remove the trailer without rewriting `main`, so the anchor is the
fix: it keeps the scan from ever reaching back that far.

Three consequences worth knowing:

- **Never merge a release PR that lowers a version.** Check the manifest diff.
  A release-please PR that proposes a version below what is already tagged
  means it has lost the boundary, not that the version is wrong.
- **Move the anchor forward only when it is safe.** It never needs routine
  updating. If you do move it, it must land on or after a release commit, or
  release-please will re-release everything in between.
- **Do not walk a program major back.** The May trailer exists because a major
  on `thread`/`fiber` was treated as wrong on the reasoning that the program ID
  had not changed. That is no longer the position: with the legacy programs
  sunset, a major against a stable program ID is fine, and `feat(fiber)!` →
  6.0.0 is a correct release, not a phantom one. A `!` on a program scope
  should be left to cut its major. Reaching for another `Release-As:` trailer
  to force it down is what created this problem in the first place, and the
  trailer outlives the release it was written for.

### New packages

When adding a new package:
1. Start at version `0.1.0` in its `Cargo.toml`.
2. Create a `CHANGELOG.md` with a single header (release-please will
   prepend entries above it on the first release).
3. Add the path → component mapping in `.github/release-please-config.json`.
4. Add the path with its starting version in `.github/.release-please-manifest.json`.

### Commits

- Conventional Commits format, because release-please depends on it.
- Do **not** include `Co-Authored-By` footers in commit messages.
