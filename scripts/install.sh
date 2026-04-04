#!/usr/bin/env bash
# Antegen CLI + Node installer
# Usage: curl -sSfL https://raw.githubusercontent.com/wuwei-labs/antegen/main/scripts/install.sh | bash
# With RPC: curl -sSfL .../install.sh | bash -s -- --rpc https://api.mainnet-beta.solana.com

set -e

REPO="wuwei-labs/antegen"
BINARY="antegen"
NODE_BINARY="antegen-node"
INSTALL_DIR="${HOME}/.local/bin"
RPC_URL=""

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

# Parse arguments
parse_args() {
    while [ $# -gt 0 ]; do
        case $1 in
            --rpc)
                RPC_URL="$2"
                shift 2
                ;;
            *)
                shift
                ;;
        esac
    done
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
            TARGET="aarch64-unknown-linux-gnu"
            ;;
        *)
            error "Unsupported platform: $OS-$ARCH"
            ;;
    esac

    info "Detected platform: $TARGET"
}

# Get latest CLI version from GitHub (tags matching v{semver} only, no prefix)
get_latest_cli_version() {
    VERSION=$(curl -sS "https://api.github.com/repos/$REPO/releases" | \
        grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/' | grep -E '^v[0-9]' | head -1)

    if [ -z "$VERSION" ]; then
        error "Failed to get latest CLI version from GitHub"
    fi

    info "Latest CLI version: $VERSION"
}

# Get latest node version from GitHub (tags with node- prefix)
get_latest_node_version() {
    NODE_VERSION=$(curl -sS "https://api.github.com/repos/$REPO/releases" | \
        grep '"tag_name"' | grep 'node-v' | head -1 | sed -E 's/.*"node-([^"]+)".*/\1/')

    if [ -z "$NODE_VERSION" ]; then
        warn "No node-specific releases found yet. Skipping node binary install."
        warn "You can install it later with: antegenctl install <version>"
        return 1
    fi

    info "Latest node version: $NODE_VERSION"
    return 0
}

# Download CLI binary to temp and use it to install itself (handles PATH setup)
install_cli_binary() {
    URL="https://github.com/$REPO/releases/download/$VERSION/$BINARY-$VERSION-$TARGET"

    info "Downloading CLI from: $URL"

    # Create temp directory
    TMP_DIR=$(mktemp -d)
    trap "rm -rf $TMP_DIR" EXIT

    # Download binary to temp
    if ! curl -sSfL "$URL" -o "$TMP_DIR/$BINARY"; then
        error "Failed to download CLI binary. Check if release exists for your platform."
    fi
    chmod +x "$TMP_DIR/$BINARY"

    # Use the binary to install itself (handles symlink, PATH setup)
    "$TMP_DIR/$BINARY" install --version "$VERSION"

    # Add to PATH for this session so verify_installation works
    export PATH="$INSTALL_DIR:$PATH"
}

# Download and install the node binary
install_node_binary() {
    NODE_URL="https://github.com/$REPO/releases/download/node-$NODE_VERSION/$NODE_BINARY-$NODE_VERSION-$TARGET"

    info "Downloading node from: $NODE_URL"

    TMP_DIR=$(mktemp -d)
    trap "rm -rf $TMP_DIR" EXIT

    if ! curl -sSfL "$NODE_URL" -o "$TMP_DIR/$NODE_BINARY"; then
        warn "Failed to download node binary. You can install it later with: antegenctl install $NODE_VERSION"
        return 1
    fi
    chmod +x "$TMP_DIR/$NODE_BINARY"

    # Install versioned binary
    mkdir -p "$INSTALL_DIR"
    VERSIONED_PATH="$INSTALL_DIR/$NODE_BINARY-$NODE_VERSION"
    cp "$TMP_DIR/$NODE_BINARY" "$VERSIONED_PATH"
    chmod +x "$VERSIONED_PATH"

    # Create antegen-node symlink
    SYMLINK_PATH="$INSTALL_DIR/$NODE_BINARY"
    rm -f "$SYMLINK_PATH"
    ln -s "$VERSIONED_PATH" "$SYMLINK_PATH"

    # Write node version tracking file
    mkdir -p "$HOME/.antegen"
    echo "$NODE_VERSION" > "$HOME/.antegen/node-version"

    info "Installed $NODE_BINARY $NODE_VERSION"
    return 0
}

# Verify installation
verify_installation() {
    if command -v $BINARY &> /dev/null; then
        INSTALLED_VERSION=$($BINARY --version 2>/dev/null || echo "unknown")
        info "Successfully installed: $INSTALLED_VERSION"
        return 0
    else
        return 1
    fi
}

# Initialize antegen (config + service) - only if RPC provided
initialize() {
    if [ -n "$RPC_URL" ]; then
        info "Initializing antegen with RPC: $RPC_URL"
        "$INSTALL_DIR/$BINARY" init --rpc "$RPC_URL"
        return 0
    fi
    return 1
}

main() {
    parse_args "$@"

    info "Installing Antegen..."

    detect_platform

    # Install CLI
    get_latest_cli_version
    install_cli_binary

    # Install node binary (non-fatal if not available yet)
    if get_latest_node_version; then
        install_node_binary || true
    fi

    if verify_installation; then
        echo ""
        initialize || true
        info "Installation complete!"
        echo ""
        echo "  Restart your shell or run: source ~/.bashrc"
        echo ""
        echo "  Node management:  antegenctl --help"
        echo "  Developer tools:  antegen --help"
        echo ""
    else
        error "Installation verification failed"
    fi
}

main "$@"
