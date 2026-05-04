# Antegen

## Commit conventions (read before committing)

Every commit in this repo MUST satisfy these rules. CI will reject PRs
that violate them. AI assistants helping with commits — Claude Code or
otherwise — must follow them too.

1. **Conventional Commits format** for the commit subject:
   ```
   <type>(<scope>): <description>
   ```
   - Allowed types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`,
     `test`, `build`, `ci`, `chore`, `revert`.
   - Allowed scopes (optional): `cron`, `client`, `thread`, `fiber`,
     `cli-core`, `cli`, `ctl`, `geyser`. Scope tells `release-please`
     which package's changelog the commit belongs to.
   - Append `!` after type/scope (or include a `BREAKING CHANGE:`
     footer) for breaking changes — these become a major version bump.
   - Subject starts with a letter and reads as a present-tense imperative.
2. **DCO sign-off** on every commit. Use `git commit -s` so the
   `Signed-off-by:` trailer is added.
3. **No `Co-Authored-By:` footer.** Do not list AI assistants (Claude
   Code, Copilot, etc.) as co-authors. The DCO sign-off is the only
   authorship trailer this repo uses.
4. **Never bump versions or edit `CHANGELOG.md` manually** (except
   inside a Release PR opened by release-please). See "Versioning &
   Changelog Protocol" below.

Examples:

```
feat(thread): add cleanup_stale_signal instruction

Allows recovery of signal accounts whose thread is gone. Refunds
rent to the original payer.

Signed-off-by: Anthony Anderson <anthony@wuwei.dev>
```

```
fix(cron): clamp DOM/DOW interaction to POSIX semantics

Signed-off-by: Anthony Anderson <anthony@wuwei.dev>
```

## Versioning & Changelog Protocol

Versioning and `CHANGELOG.md` updates are **automated by `release-please`**.
Contributors do not bump versions or write changelog entries by hand.

### How releases work

1. Land code on `main` via PR. The PR title MUST follow Conventional
   Commits format (validated by `pr-title-check.yml`).
   - Use squash-merge so the merge commit on `main` carries the PR title.
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
| `crates/client` | `antegen-client` | `antegen-client-v<X.Y.Z>` | crates.io + `antegen-node` binary |
| `programs/thread` | `antegen-thread-program` | `antegen-thread-program-v<X.Y.Z>` | crates.io + verifiable `.so` |
| `programs/fiber` | `antegen-fiber-program` | `antegen-fiber-program-v<X.Y.Z>` | crates.io + verifiable `.so` |
| `cli/core` | `antegen-cli-core` | `antegen-cli-core-v<X.Y.Z>` | crates.io |
| `cli/antegen` | `antegen-cli` | `antegen-cli-v<X.Y.Z>` | crates.io + `antegen` binary |
| `cli/antegenctl` | `antegenctl` | `antegenctl-v<X.Y.Z>` | crates.io + `antegenctl` binary |
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
- `antegen-client` API change → bump `antegen-cli-core` and `antegen-cli`

### New packages

When adding a new package:
1. Start at version `0.1.0` in its `Cargo.toml`.
2. Create a `CHANGELOG.md` with a single header (release-please will
   prepend entries above it on the first release).
3. Add the path → component mapping in `.github/release-please-config.json`.
4. Add the path with its starting version in `.github/.release-please-manifest.json`.

### Commits

- Conventional Commits format is required (enforced by `pr-title-check.yml`).
- Sign-off (`-s`) is required per DCO (enforced by `dco.yml`).
- Do **not** include `Co-Authored-By` footers in commit messages.
