#!/usr/bin/env bash
# Antegen installer
#
# Installs the `antegen` binary, which is both the CLI and the executor daemon
# (`antegen node run`). The CLI no longer manages its own versions — re-running
# this script is how you update, and `--version` is how you pin or roll back.
#
# Usage:  curl -sSfL https://raw.githubusercontent.com/wuwei-labs/antegen/main/scripts/install.sh | bash
# Pin:    ... | bash -s -- --version v6.0.0
# Config: ... | bash -s -- --rpc https://api.mainnet-beta.solana.com

set -euo pipefail

REPO="wuwei-labs/antegen"
BINARY="antegen"
INSTALL_DIR="${HOME}/.local/bin"
RELEASE_TAG_PREFIX="antegen-cli-"
RPC_URL=""
VERSION=""

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info() { echo -e "${GREEN}[INFO]${NC} $1"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
error() {
    echo -e "${RED}[ERROR]${NC} $1"
    exit 1
}

parse_args() {
    while [ $# -gt 0 ]; do
        case $1 in
            --rpc)
                RPC_URL="$2"
                shift 2
                ;;
            --version)
                VERSION="$2"
                shift 2
                ;;
            *)
                shift
                ;;
        esac
    done

    # Accept both `6.0.0` and `v6.0.0`
    if [ -n "$VERSION" ] && [ "${VERSION#v}" = "$VERSION" ]; then
        VERSION="v${VERSION}"
    fi
}

detect_platform() {
    OS=$(uname -s | tr '[:upper:]' '[:lower:]')
    ARCH=$(uname -m)

    case "$OS-$ARCH" in
        darwin-x86_64) TARGET="x86_64-apple-darwin" ;;
        darwin-arm64) TARGET="aarch64-apple-darwin" ;;
        linux-x86_64) TARGET="x86_64-unknown-linux-gnu" ;;
        linux-aarch64 | linux-arm64) TARGET="aarch64-unknown-linux-gnu" ;;
        *) error "Unsupported platform: $OS-$ARCH" ;;
    esac

    info "Detected platform: $TARGET"
}

# Latest release carrying the antegen binary. Releases are per-package, so
# filter to the CLI component rather than taking whatever is newest.
get_latest_version() {
    VERSION=$(curl -sSfL "https://api.github.com/repos/$REPO/releases?per_page=100" |
        grep '"tag_name"' |
        sed -E 's/.*"tag_name": *"([^"]+)".*/\1/' |
        grep "^${RELEASE_TAG_PREFIX}v" |
        head -1 |
        sed -E "s/^${RELEASE_TAG_PREFIX}//")

    [ -n "$VERSION" ] || error "Could not determine the latest ${BINARY} release from GitHub"

    info "Latest version: $VERSION"
}

# Download to a temp file and rename() into place.
#
# Writing directly over the destination fails with ETXTBSY on Linux when the
# daemon is running from that path. rename() is atomic and leaves the running
# process on its old inode until it restarts, which is what we want.
install_binary() {
    URL="https://github.com/$REPO/releases/download/${RELEASE_TAG_PREFIX}${VERSION}/${BINARY}-${VERSION}-${TARGET}"

    info "Downloading $URL"

    TMP_DIR=$(mktemp -d)
    trap 'rm -rf "$TMP_DIR"' EXIT

    curl -sSfL "$URL" -o "$TMP_DIR/$BINARY" ||
        error "Failed to download the binary. Check that $VERSION has a release for $TARGET."

    chmod +x "$TMP_DIR/$BINARY"

    mkdir -p "$INSTALL_DIR"
    mv "$TMP_DIR/$BINARY" "$INSTALL_DIR/$BINARY"

    info "Installed $BINARY $VERSION to $INSTALL_DIR/$BINARY"
}

# `antegenctl` was a symlink to this same binary, not a separate program. Left
# behind it keeps resolving to whatever it last pointed at.
remove_legacy_symlinks() {
    if [ -L "$INSTALL_DIR/antegenctl" ]; then
        rm -f "$INSTALL_DIR/antegenctl"
        info "Removed the deprecated antegenctl symlink; use \`antegen node\` instead."
    fi
}

# rustup-style: a sourceable env file, referenced from the shell rc.
ENV_FILE="${HOME}/.antegen/env"

configure_path() {
    mkdir -p "${HOME}/.antegen"
    cat >"$ENV_FILE" <<'EOF'
# Antegen PATH setup - sourced by shell rc files
case ":${PATH}:" in
    *:"$HOME/.local/bin":*)
        ;;
    *)
        export PATH="$HOME/.local/bin:$PATH"
        ;;
esac
EOF

    case ":${PATH}:" in
        *:"$INSTALL_DIR":*) return 0 ;;
    esac

    for rc in .zshenv .zshrc .bashrc .bash_profile .profile; do
        rc_path="${HOME}/${rc}"
        [ -f "$rc_path" ] || continue

        if grep -q '.antegen/env' "$rc_path" 2>/dev/null; then
            return 0
        fi

        # shellcheck disable=SC2016  # $HOME must expand when the rc is sourced, not now
        printf '\n# Added by antegen\n. "$HOME/.antegen/env"\n' >>"$rc_path"
        info "Added antegen to PATH in ~/${rc}"
        info "Run 'source ~/${rc}' or restart your shell to apply."
        return 0
    done
}

verify_installation() {
    "$INSTALL_DIR/$BINARY" --version >/dev/null 2>&1
}

main() {
    parse_args "$@"

    info "Installing Antegen..."

    detect_platform
    [ -n "$VERSION" ] || get_latest_version
    install_binary
    remove_legacy_symlinks
    configure_path

    verify_installation || error "Installation verification failed"

    info "Successfully installed: $("$INSTALL_DIR/$BINARY" --version)"
    echo ""

    if [ -n "$RPC_URL" ]; then
        info "Initializing antegen with RPC: $RPC_URL"
        "$INSTALL_DIR/$BINARY" init --rpc "$RPC_URL"
        echo ""
    fi

    echo "  Start the node:   antegen node start"
    echo "  Node management:  antegen node --help"
    echo "  Developer tools:  antegen --help"
    echo ""
    echo "  To update, re-run this script. It replaces the binary in place;"
    echo "  run \`antegen node update\` to move a running node onto it."
    echo ""
}

main "$@"
