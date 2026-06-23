#!/usr/bin/env sh
set -e

REPO="gndps/bfcli"
INSTALL_DIR="/usr/local/bin"
BINARY="bfcli"

# Detect OS and architecture
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Darwin)
        OS_NAME="apple-darwin"
        ;;
    Linux)
        OS_NAME="unknown-linux-gnu"
        ;;
    *)
        echo "Unsupported OS: $OS" >&2
        exit 1
        ;;
esac

case "$ARCH" in
    arm64|aarch64)
        ARCH_NAME="aarch64"
        ;;
    x86_64|amd64)
        ARCH_NAME="x86_64"
        ;;
    *)
        echo "Unsupported architecture: $ARCH" >&2
        exit 1
        ;;
esac

TARGET="${ARCH_NAME}-${OS_NAME}"
TARBALL="${BINARY}-${TARGET}.tar.gz"
URL="https://github.com/${REPO}/releases/latest/download/${TARBALL}"

echo "Downloading bfcli for ${TARGET}..."
echo "  URL: ${URL}"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$URL" -o "$TMP_DIR/$TARBALL"
elif command -v wget >/dev/null 2>&1; then
    wget -q "$URL" -O "$TMP_DIR/$TARBALL"
else
    echo "Error: curl or wget is required to download bfcli" >&2
    exit 1
fi

echo "Extracting..."
tar -xzf "$TMP_DIR/$TARBALL" -C "$TMP_DIR"

echo "Installing to ${INSTALL_DIR}/${BINARY}..."
if [ -w "$INSTALL_DIR" ]; then
    mv "$TMP_DIR/$BINARY" "$INSTALL_DIR/$BINARY"
    chmod +x "$INSTALL_DIR/$BINARY"
else
    sudo mv "$TMP_DIR/$BINARY" "$INSTALL_DIR/$BINARY"
    sudo chmod +x "$INSTALL_DIR/$BINARY"
fi

echo ""
echo "bfcli installed successfully!"
echo ""
echo "Next steps:"
echo "  1. Run:  bfcli init"
echo "  2. Add to your ~/.bash_profile:"
echo "     [ -f ~/.bfcli/.bflist ] && source ~/.bfcli/.bflist"
echo "  3. Place shell files in ~/.bfcli/src_files/"
echo "  4. Run:  bfcli update"
