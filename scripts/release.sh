#!/usr/bin/env bash
set -euo pipefail

# Release script - creates chore commit and tags current version
# Version should already be bumped in feat/fix commits

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

error() { echo -e "${RED}error:${NC} $1" >&2; exit 1; }
info() { echo -e "${GREEN}==>${NC} $1"; }

# Check for uncommitted changes
if [[ -n "$(git status --porcelain)" ]]; then
    error "Uncommitted changes detected. Commit or stash changes before releasing."
fi

# Extract version from Cargo.toml
VERSION=$(sed -n '/\[workspace.package\]/,/^\[/p' "$ROOT_DIR/Cargo.toml" | grep '^version' | sed 's/version = "\(.*\)"/\1/')

if [[ -z "$VERSION" ]]; then
    error "Could not extract version from Cargo.toml"
fi

TAG="v$VERSION"

# Check if tag already exists
if git tag -l | grep -q "^$TAG$"; then
    info "Tag $TAG already exists - nothing to release"
    exit 0
fi

info "Releasing $TAG"

read -p "Create release commit and tag $TAG? [y/N] " -n 1 -r
echo ""

if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    info "Aborting release"
    exit 0
fi

# Create empty chore commit as release marker
info "Creating release commit..."
git commit --allow-empty -m "chore: release $TAG"

# Create tag
info "Creating tag $TAG..."
git tag "$TAG"

# Push
info "Pushing to origin..."
git push origin main "$TAG"

echo ""
info "Release $TAG complete!"
