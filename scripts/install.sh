#!/usr/bin/env bash
# Antegen CLI installer
# Usage: curl -sSfL https://raw.githubusercontent.com/wuwei-labs/antegen/main/scripts/install.sh | bash
# With RPC: curl -sSfL .../install.sh | bash -s -- --rpc https://api.mainnet-beta.solana.com

set -e

REPO="wuwei-labs/antegen"
BINARY="antegen"
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

    # Create install directory if needed
    mkdir -p "$INSTALL_DIR"

    # Create temp directory
    TMP_DIR=$(mktemp -d)
    trap "rm -rf $TMP_DIR" EXIT

    # Download binary
    if ! curl -sSfL "$URL" -o "$TMP_DIR/$BINARY"; then
        error "Failed to download binary. Check if release exists for your platform."
    fi

    # Make executable and install
    chmod +x "$TMP_DIR/$BINARY"
    mv "$TMP_DIR/$BINARY" "$INSTALL_DIR/$BINARY"

    info "Installed $BINARY to $INSTALL_DIR/$BINARY"
}

# Check if ~/.local/bin is in PATH
check_path() {
    if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
        warn "$INSTALL_DIR is not in your PATH"
        echo ""
        echo "  Add this to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
        echo "    export PATH=\"\$HOME/.local/bin:\$PATH\""
        echo ""

        # Add to PATH for this session so init works
        export PATH="$INSTALL_DIR:$PATH"
    fi
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

# Initialize antegen (config + service)
initialize() {
    info "Initializing antegen..."

    if [ -n "$RPC_URL" ]; then
        "$INSTALL_DIR/$BINARY" init --rpc "$RPC_URL"
    else
        "$INSTALL_DIR/$BINARY" init
    fi
}

main() {
    parse_args "$@"

    info "Installing Antegen CLI..."

    detect_platform
    get_latest_version
    install_binary
    check_path

    if verify_installation; then
        echo ""
        initialize
    else
        error "Installation verification failed"
    fi

    echo ""
    info "Installation complete!"
    echo ""
    echo "  Useful commands:"
    echo "    antegen --help      Show help"
    echo "    antegen stop        Stop the service"
    echo "    antegen start       Start the service"
    echo "    antegen restart     Restart the service"
    echo "    antegen update      Update to latest version"
    echo "    antegen uninstall   Remove the service"
    echo ""
}

main "$@"
