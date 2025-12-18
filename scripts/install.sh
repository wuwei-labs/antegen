#!/usr/bin/env bash
# Antegen CLI installer
# Usage: curl -sSfL https://raw.githubusercontent.com/wuwei-labs/antegen/main/scripts/install.sh | bash
# With systemd: curl -sSfL .../install.sh | bash -s -- --systemd

set -e

REPO="wuwei-labs/antegen"
BINARY="antegen"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"
SETUP_SYSTEMD=false

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
            --systemd)
                SETUP_SYSTEMD=true
                shift
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

# Setup systemd service (Linux only)
setup_systemd() {
    if [ "$OS" != "linux" ]; then
        warn "Systemd setup is only available on Linux"
        return
    fi

    info "Setting up systemd service..."

    # Create antegen system user (if doesn't exist)
    if ! id -u antegen &>/dev/null; then
        info "Creating antegen system user..."
        sudo useradd --system --no-create-home --shell /usr/sbin/nologin antegen
    else
        info "User 'antegen' already exists"
    fi

    # Create directories
    info "Creating directories..."
    sudo mkdir -p /etc/antegen
    sudo mkdir -p /var/lib/antegen

    # Set ownership
    sudo chown root:antegen /etc/antegen
    sudo chmod 750 /etc/antegen
    sudo chown antegen:antegen /var/lib/antegen
    sudo chmod 700 /var/lib/antegen

    # Generate default config if it doesn't exist
    if [ ! -f /etc/antegen/antegen.toml ]; then
        info "Generating default config..."
        sudo tee /etc/antegen/antegen.toml > /dev/null << 'EOF'
# Antegen Configuration
# Edit this file before starting the service

[executor]
# Path to executor keypair (will be auto-generated on first run)
keypair_path = "/var/lib/antegen/executor.json"

[rpc]
# RPC endpoints (add your own endpoints here)
[[rpc.endpoints]]
url = "https://api.devnet.solana.com"
weight = 1

[thread_program]
# Thread program ID (devnet default)
program_id = "antSQGK1T2zGTn5bTWnHRxoFAw4dEZEq1YBrHGGpLfV"

[load_balancer]
enabled = true
capacity_threshold = 5
takeover_delay_seconds = 10
EOF
        sudo chown root:antegen /etc/antegen/antegen.toml
        sudo chmod 640 /etc/antegen/antegen.toml
    else
        info "Config already exists at /etc/antegen/antegen.toml"
    fi

    # Create systemd service file
    info "Creating systemd service..."
    sudo tee /etc/systemd/system/antegen.service > /dev/null << 'EOF'
[Unit]
Description=Antegen Thread Executor
Documentation=https://antegen.xyz
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=antegen
Group=antegen
ExecStart=/usr/local/bin/antegen start -c /etc/antegen/antegen.toml
Restart=on-failure
RestartSec=5
StandardOutput=journal
StandardError=journal

# Security hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadWritePaths=/var/lib/antegen

[Install]
WantedBy=multi-user.target
EOF

    # Reload systemd and enable service
    info "Enabling service..."
    sudo systemctl daemon-reload
    sudo systemctl enable antegen

    info "Systemd service installed!"
    echo ""
    echo "  Next steps:"
    echo "    1. Edit config:    sudo nano /etc/antegen/antegen.toml"
    echo "    2. Start service:  sudo systemctl start antegen"
    echo "    3. Check status:   sudo systemctl status antegen"
    echo "    4. View logs:      sudo journalctl -u antegen -f"
    echo ""
}

main() {
    parse_args "$@"

    info "Installing Antegen CLI..."

    detect_platform
    get_latest_version
    install_binary
    verify_installation

    if [ "$SETUP_SYSTEMD" = true ]; then
        setup_systemd
    fi

    echo ""
    info "Installation complete!"
    echo ""
    if [ "$SETUP_SYSTEMD" != true ]; then
        echo "  Get started:"
        echo "    $BINARY --help"
        echo ""
        echo "  To install as a systemd service, run:"
        echo "    curl -sSfL https://raw.githubusercontent.com/wuwei-labs/antegen/main/scripts/install.sh | bash -s -- --systemd"
        echo ""
    fi
}

main "$@"
