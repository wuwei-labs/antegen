#!/bin/bash
# Antegen CLI installer
# Usage: curl -sSfL https://raw.githubusercontent.com/wuwei-labs/antegen/main/scripts/install.sh | sh

set -e

REPO="wuwei-labs/antegen"
BINARY="antegen"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

error() {
    echo -e "${RED}[ERROR]${NC} $1"
    exit 1
}

# Detect OS and architecture
detect_platform() {
    OS=$(uname -s | tr '[:upper:]' '[:lower:]')
    ARCH=$(uname -m)

    case "$OS-$ARCH" in
        darwin-x86_64)
            TARGET="x86_64-apple-darwin"
            ;;
        darwin-arm64)
            TARGET="aarch64-apple-darwin"
            ;;
        linux-x86_64)
            TARGET="x86_64-unknown-linux-gnu"
            ;;
        linux-aarch64|linux-arm64)
            error "Linux ARM64 binaries not yet available. Use 'cargo install antegen-cli' instead."
            ;;
        *)
            error "Unsupported platform: $OS-$ARCH"
            ;;
    esac

    info "Detected platform: $TARGET"
}

# Get latest version from GitHub
get_latest_version() {
    VERSION=$(curl -sS "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/')

    if [ -z "$VERSION" ]; then
        error "Failed to get latest version from GitHub"
    fi

    info "Latest version: $VERSION"
}

# Download and install binary
install_binary() {
    URL="https://github.com/$REPO/releases/download/$VERSION/$BINARY-$VERSION-$TARGET"

    info "Downloading from: $URL"

    # Create temp directory
    TMP_DIR=$(mktemp -d)
    trap "rm -rf $TMP_DIR" EXIT

    # Download binary
    if ! curl -sSfL "$URL" -o "$TMP_DIR/$BINARY"; then
        error "Failed to download binary"
    fi

    # Make executable
    chmod +x "$TMP_DIR/$BINARY"

    # Install to destination
    if [ -w "$INSTALL_DIR" ]; then
        mv "$TMP_DIR/$BINARY" "$INSTALL_DIR/$BINARY"
    else
        info "Requesting sudo to install to $INSTALL_DIR"
        sudo mv "$TMP_DIR/$BINARY" "$INSTALL_DIR/$BINARY"
    fi

    info "Installed $BINARY to $INSTALL_DIR/$BINARY"
}

# Verify installation
verify_installation() {
    if command -v $BINARY &> /dev/null; then
        INSTALLED_VERSION=$($BINARY --version 2>/dev/null || echo "unknown")
        info "Successfully installed: $INSTALLED_VERSION"
    else
        warn "$BINARY installed but not found in PATH"
        warn "Add $INSTALL_DIR to your PATH"
    fi
}

main() {
    info "Installing Antegen CLI..."

    detect_platform
    get_latest_version
    install_binary
    verify_installation

    echo ""
    info "Installation complete!"
    echo ""
    echo "  Get started:"
    echo "    $BINARY --help"
    echo ""
}

main "$@"
