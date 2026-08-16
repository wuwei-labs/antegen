#!/usr/bin/env bash
# Keep `[workspace.dependencies]` in step with each member's own version.
#
# Internal crates are pinned by version as well as path so they can be
# published — cargo strips the path and keeps the version requirement. But
# release-please bumps a member's own Cargo.toml and leaves the pin alone, so
# the two drift apart on every release and the workspace stops resolving:
#
#   error: failed to select a version for the requirement `antegen-ws = "^0.1.0"`
#   candidate versions found which didn't match: 0.2.0
#
# That has broken main and failed the publish job on three consecutive releases.
# Fixing it by hand each time is the step that keeps getting missed, so this
# derives the pins from the members instead.
#
# Usage:
#   scripts/sync-workspace-pins.sh           rewrite Cargo.toml in place
#   scripts/sync-workspace-pins.sh --check   report drift, exit 1, change nothing

set -euo pipefail

cd "$(dirname "$0")/.."

CHECK=0
[ "${1:-}" = "--check" ] && CHECK=1

# crate name -> directory holding its Cargo.toml
MEMBERS="
antegen-cli-core cli/core
antegen-cron crates/cron
antegen-client crates/client
antegen-geyser-plugin plugin/geyser
antegen-ws crates/ws
antegen-fiber-program programs/fiber
antegen-thread-program programs/thread
"

status=0

while read -r crate path; do
    [ -z "$crate" ] && continue

    actual="$(sed -n 's/^version = "\(.*\)"/\1/p' "${path}/Cargo.toml" | head -1)"
    pinned="$(sed -n "s/^${crate} = { version = \"\([^\"]*\)\".*/\1/p" Cargo.toml | head -1)"

    if [ -z "$actual" ]; then
        echo "error: could not read a version from ${path}/Cargo.toml" >&2
        exit 1
    fi
    if [ -z "$pinned" ]; then
        echo "error: ${crate} is not pinned in [workspace.dependencies]" >&2
        exit 1
    fi
    [ "$pinned" = "$actual" ] && continue

    if [ "$CHECK" = "1" ]; then
        echo "::error::${crate}: workspace pin is ${pinned} but ${path} is ${actual}"
        status=1
    else
        echo "${crate}: ${pinned} -> ${actual}"
        # Only the version field on that crate's own line.
        sed -i.bak "s|^${crate} = { version = \"${pinned}\"|${crate} = { version = \"${actual}\"|" Cargo.toml
        rm -f Cargo.toml.bak
    fi
done <<EOF
$MEMBERS
EOF

exit $status
