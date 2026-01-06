#!/usr/bin/env bash
set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

error() {
    echo -e "${RED}error:${NC} $1" >&2
    exit 1
}

warn() {
    echo -e "${YELLOW}warning:${NC} $1"
}

info() {
    echo -e "${GREEN}==>${NC} $1"
}

# Extract version from root Cargo.toml
VERSION=$(grep -m1 '^version' "$ROOT_DIR/Cargo.toml" | sed 's/version = "\(.*\)"/\1/')

if [[ -z "$VERSION" ]]; then
    error "Could not extract version from Cargo.toml"
fi

# Validate semver format (X.Y.Z)
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    error "Invalid version format: $VERSION (expected X.Y.Z)"
fi

TAG="v$VERSION"

info "Preparing release $TAG"

# Check if tag already exists
if git tag -l | grep -q "^$TAG$"; then
    error "Tag $TAG already exists"
fi

# Run cargo check to update Cargo.lock
info "Running cargo check..."
cd "$ROOT_DIR"
cargo check --quiet

# Stage all Cargo files
info "Staging Cargo files..."
git add Cargo.toml Cargo.lock
git add crates/*/Cargo.toml 2>/dev/null || true
git add programs/*/Cargo.toml 2>/dev/null || true
git add plugin/*/Cargo.toml 2>/dev/null || true

# Check if there are changes to commit
if git diff --cached --quiet; then
    error "No changes staged for commit. Did you update the version in Cargo.toml?"
fi

# Show diff
echo ""
info "Changes to be committed:"
echo "----------------------------------------"
git diff --cached --stat
echo "----------------------------------------"
echo ""

# Confirmation prompt
read -p "Release $TAG? [y/N] " -n 1 -r
echo ""

if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    info "Aborting release"
    git reset HEAD --quiet
    exit 0
fi

# Commit
info "Creating commit..."
git commit -m "chore: release $TAG"

# Create tag
info "Creating tag $TAG..."
git tag "$TAG"

# Push
info "Pushing to origin..."
git push origin main --tags

echo ""
info "Release $TAG complete!"
echo "    GitHub Actions will now build and publish the release."
