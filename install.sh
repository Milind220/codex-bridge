#!/usr/bin/env bash
set -euo pipefail

REPO="Milind220/codex-bridge"
BIN_NAME="codex-bridge"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64) ARCH="x86_64" ;;
  arm64|aarch64) ARCH="aarch64" ;;
  *) echo "unsupported arch: $ARCH"; exit 1 ;;
esac

case "$OS" in
  darwin) TARGET="${ARCH}-apple-darwin" ;;
  linux) TARGET="${ARCH}-unknown-linux-musl" ;;
  *) echo "unsupported os: $OS"; exit 1 ;;
esac

TAG="${TAG:-latest}"
if [[ "$TAG" == "latest" ]]; then
  URL="https://github.com/${REPO}/releases/latest/download/${BIN_NAME}-${TARGET}.tar.gz"
else
  URL="https://github.com/${REPO}/releases/download/${TAG}/${BIN_NAME}-${TARGET}.tar.gz"
fi

mkdir -p "$INSTALL_DIR"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

curl -fsSL "$URL" -o "$TMP/${BIN_NAME}.tar.gz"
tar -xzf "$TMP/${BIN_NAME}.tar.gz" -C "$TMP"
install -m 0755 "$TMP/${BIN_NAME}" "$INSTALL_DIR/${BIN_NAME}"

echo "installed: $INSTALL_DIR/$BIN_NAME"
echo "ensure $INSTALL_DIR is in PATH"