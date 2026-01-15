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

# Extract version from [workspace.package] section in root Cargo.toml
VERSION=$(sed -n '/\[workspace.package\]/,/^\[/p' "$ROOT_DIR/Cargo.toml" | grep '^version' | sed 's/version = "\(.*\)"/\1/')

if [[ -z "$VERSION" ]]; then
    error "Could not extract version from Cargo.toml"
fi

# Validate semver format (X.Y.Z)
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    error "Invalid version format: $VERSION (expected X.Y.Z)"
fi

TAG="v$VERSION"

# Check if tag already exists
if git tag -l | grep -q "^$TAG$"; then
    info "Tag $TAG already exists - nothing to release"
    exit 0
fi

# Check for uncommitted changes
if [[ -n "$(git status --porcelain)" ]]; then
    error "Uncommitted changes detected. Commit or stash changes before releasing."
fi

info "Releasing $TAG"

# Confirmation prompt
read -p "Create and push tag $TAG? [y/N] " -n 1 -r
echo ""

if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    info "Aborting release"
    exit 0
fi

# Create tag
info "Creating tag $TAG..."
git tag "$TAG"

# Push commits and tag
info "Pushing to origin..."
git push origin main "$TAG"

echo ""
info "Release $TAG complete!"
echo "    GitHub Actions will now build and publish the release."
