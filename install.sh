#!/bin/sh
# SoliDB installer — works with any POSIX shell
# Usage: curl -sSL https://raw.githubusercontent.com/solisoft/solidb/main/install.sh | sh
#   or:  sh install.sh [--system]

set -e

REPO="solisoft/solidb"
INSTALL_DIR="$HOME/.local/bin"
SYSTEM_INSTALL=0

for arg in "$@"; do
  case "$arg" in
    --system) SYSTEM_INSTALL=1; INSTALL_DIR="/usr/local/bin" ;;
    --help|-h)
      echo "Usage: install.sh [--system]"
      echo "  --system  Install to /usr/local/bin (requires sudo)"
      echo "  Default:  Install to ~/.local/bin"
      exit 0
      ;;
    *) echo "Unknown option: $arg"; exit 1 ;;
  esac
done

# --- Detect OS ---
OS="$(uname -s)"
case "$OS" in
  Linux*)  OS="linux" ;;
  Darwin*) OS="darwin" ;;
  MINGW*|MSYS*|CYGWIN*)
    echo "Error: this installer does not support Windows."
    echo "Download solidb-windows-amd64.zip from:"
    echo "  https://github.com/solisoft/solidb/releases/latest"
    echo "then extract it and add the folder to your PATH."
    exit 1
    ;;
  *) echo "Error: unsupported operating system: $OS"; exit 1 ;;
esac

# --- Detect architecture ---
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|amd64)   ARCH="amd64" ;;
  aarch64|arm64)   ARCH="arm64" ;;
  *) echo "Error: unsupported architecture: $ARCH"; exit 1 ;;
esac

echo "Detected platform: ${OS}-${ARCH}"

# --- Pick a download tool ---
if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL "$1"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -qO- "$1"; }
else
  echo "Error: curl or wget is required"; exit 1
fi

# --- Get latest version tag ---
API_URL="https://api.github.com/repos/${REPO}/releases/latest"
TAG=""
if TAG=$(fetch "$API_URL" 2>/dev/null | grep '"tag_name"' | head -1 | sed 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/'); then
  if [ -z "$TAG" ]; then
    TAG=""
  fi
fi

if [ -z "$TAG" ]; then
  echo "Warning: could not fetch latest release, falling back to v0.1.53"
  TAG="v0.1.53"
fi

echo "Installing SoliDB ${TAG} ..."

# --- Download and extract ---
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

# Try the gnu binary first (better performance on compatible systems)
TARBALL="solidb-${OS}-${ARCH}.tar.gz"
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${TAG}/${TARBALL}"

echo "Downloading ${DOWNLOAD_URL} ..."
fetch "$DOWNLOAD_URL" > "${TMP_DIR}/${TARBALL}"
tar xzf "${TMP_DIR}/${TARBALL}" -C "$TMP_DIR"

# Check if binary is compatible with system
if ! "${TMP_DIR}/solidb" --version >/dev/null 2>&1; then
  # If it fails and we're on Linux, try the musl static binary instead
  if [ "$OS" = "linux" ]; then
    echo "GNU binary not compatible (glibc version mismatch). Trying static musl build..."
    rm -rf "${TMP_DIR}"/*
    TARBALL="solidb-linux-${ARCH}-musl.tar.gz"
    DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${TAG}/${TARBALL}"
    echo "Downloading ${DOWNLOAD_URL} ..."
    if fetch "$DOWNLOAD_URL" > "${TMP_DIR}/${TARBALL}" 2>/dev/null; then
      tar xzf "${TMP_DIR}/${TARBALL}" -C "$TMP_DIR"
      echo "Using static musl build instead."
    else
      echo "Error: static musl binary not available for this release."
      exit 1
    fi
  else
    echo "Error: binary not compatible with system."
    exit 1
  fi
fi

# --- Install binary ---
if [ "$SYSTEM_INSTALL" = "1" ]; then
  echo "Installing to ${INSTALL_DIR} (may require sudo) ..."
  sudo install -m 755 "${TMP_DIR}/solidb" "${INSTALL_DIR}/solidb"
else
  mkdir -p "$INSTALL_DIR"
  install -m 755 "${TMP_DIR}/solidb" "${INSTALL_DIR}/solidb"
fi

# --- Check PATH ---
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) ;;
  *)
    echo ""
    echo "WARNING: ${INSTALL_DIR} is not in your PATH."
    echo "Add this to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
    echo ""
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    echo ""
    ;;
esac

# --- Verify ---
if command -v solidb >/dev/null 2>&1; then
  echo ""
  echo "SoliDB installed successfully!"
  solidb --version
else
  echo ""
  echo "SoliDB installed to ${INSTALL_DIR}/solidb"
  echo "Run 'solidb --version' to verify (you may need to reload your shell)."
fi
