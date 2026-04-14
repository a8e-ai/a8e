#!/usr/bin/env bash
set -eu

##############################################################################
# a8e (Articulate) CLI Install Script
#
# Downloads the latest stable 'a8e' CLI binary from GitHub releases.
#
# Supported OS: macOS (darwin), Linux, Windows (MSYS2/Git Bash/WSL)
# Supported Architectures: x86_64, arm64/aarch64
#
# Usage:
#   curl -fsSL https://github.com/a8e-ai/a8e/releases/download/stable/download_cli.sh | bash
#
# Environment variables:
#   A8E_BIN_DIR  - Install directory (default: $HOME/.local/bin)
#   A8E_VERSION  - Specific version, e.g. "v0.1.0" (overrides CANARY)
#   A8E_PROVIDER - Provider for a8e
#   A8E_MODEL    - Model for a8e
#   CANARY       - If "true", downloads canary instead of stable
#   CONFIGURE    - If "false", skips running a8e configure
##############################################################################

REPO="a8e-ai/a8e"
OUT_FILE="a8e"

# --- 1) Check dependencies ---
if ! command -v curl >/dev/null 2>&1; then
  echo "Error: 'curl' is required. Please install curl and try again."
  exit 1
fi

if ! command -v tar >/dev/null 2>&1 && ! command -v unzip >/dev/null 2>&1; then
  echo "Error: Either 'tar' or 'unzip' is required. Please install one and try again."
  exit 1
fi

if [ "${OS:-}" != "windows" ] && ! command -v tar >/dev/null 2>&1; then
  echo "Error: 'tar' is required to extract packages. Please install tar and try again."
  exit 1
fi

# --- 2) Variables ---
if [[ "${WINDIR:-}" ]] || [[ "${windir:-}" ]] || [[ "$OSTYPE" == "msys" ]] || [[ "$OSTYPE" == "cygwin" ]]; then
    DEFAULT_BIN_DIR="$USERPROFILE/a8e"
else
    DEFAULT_BIN_DIR="$HOME/.local/bin"
fi

A8E_BIN_DIR="${A8E_BIN_DIR:-$DEFAULT_BIN_DIR}"
RELEASE="${CANARY:-false}"
CONFIGURE="${CONFIGURE:-true}"

if [ -n "${A8E_VERSION:-}" ]; then
  if [[ ! "$A8E_VERSION" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+(-.*)?$ ]]; then
    echo "[error]: invalid version '$A8E_VERSION'."
    echo "  expected: semver format vX.Y.Z, vX.Y.Z-suffix, or X.Y.Z"
    exit 1
  fi
  A8E_VERSION=$(echo "$A8E_VERSION" | sed 's/^v\{0,1\}/v/')
  RELEASE_TAG="$A8E_VERSION"
else
  RELEASE_TAG="$([[ "$RELEASE" == "true" ]] && echo "canary" || echo "stable")"
fi

# --- 3) Detect OS/Architecture ---
if [ -n "${INSTALL_OS:-}" ]; then
  case "${INSTALL_OS}" in
    linux|windows|darwin) OS="${INSTALL_OS}" ;;
    *) echo "[error]: unsupported INSTALL_OS='${INSTALL_OS}' (expected: linux|windows|darwin)"; exit 1 ;;
  esac
else
  if [[ "${WINDIR:-}" ]] || [[ "${windir:-}" ]] || [[ "$OSTYPE" == "msys" ]] || [[ "$OSTYPE" == "cygwin" ]]; then
    OS="windows"
  elif [[ -f "/proc/version" ]] && grep -q "Microsoft\|WSL" /proc/version 2>/dev/null; then
    if [[ "$PWD" =~ ^/mnt/[a-zA-Z]/ ]]; then
      OS="windows"
    else
      OS="linux"
    fi
  elif [[ "$OSTYPE" == "darwin"* ]]; then
    OS="darwin"
  else
    OS=$(uname -s | tr '[:upper:]' '[:lower:]')
  fi
fi

ARCH=$(uname -m)

case "$OS" in
  linux|darwin|windows) ;;
  mingw*|msys*|cygwin*) OS="windows" ;;
  *) echo "Error: Unsupported OS '$OS'. a8e supports Linux, macOS, and Windows."; exit 1 ;;
esac

case "$ARCH" in
  x86_64) ARCH="x86_64" ;;
  arm64|aarch64) ARCH="aarch64" ;;
  *) echo "Error: Unsupported architecture '$ARCH'."; exit 1 ;;
esac

echo "Detected: $OS ($ARCH)"

# --- 4) Build download URL ---
if [ "$OS" = "darwin" ]; then
  FILE="a8e-$ARCH-apple-darwin.tar.bz2"
  EXTRACT_CMD="tar"
elif [ "$OS" = "windows" ]; then
  if [ "$ARCH" != "x86_64" ]; then
    echo "Error: Windows currently only supports x86_64."
    exit 1
  fi
  FILE="a8e-$ARCH-pc-windows-msvc.zip"
  EXTRACT_CMD="unzip"
  OUT_FILE="a8e.exe"
else
  FILE="a8e-$ARCH-unknown-linux-gnu.tar.bz2"
  EXTRACT_CMD="tar"
fi

DOWNLOAD_URL="https://github.com/$REPO/releases/download/$RELEASE_TAG/$FILE"

# --- 4b) Skip if already up-to-date ---
# Resolve the actual version that RELEASE_TAG points to, then compare to the
# locally installed binary. If they match, skip the download. Set FORCE=true
# to bypass this check and reinstall regardless.
resolve_target_version() {
  local tag="$1" resp ver
  if [ "$tag" = "stable" ]; then
    resp=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null) || return 1
  else
    resp=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/tags/$tag" 2>/dev/null) || return 1
  fi
  ver=$(printf '%s' "$resp" | grep -m1 '"tag_name"' | sed -E 's/.*"tag_name"[[:space:]]*:[[:space:]]*"v?([^"]+)".*/\1/')
  if [ -z "$ver" ] || [ "$ver" = "stable" ] || [ "$ver" = "canary" ]; then
    ver=$(printf '%s' "$resp" | grep -m1 '"name"' \
      | sed -E 's/.*"name"[[:space:]]*:[[:space:]]*"[^"0-9]*v?([0-9][0-9.]*[A-Za-z0-9.+-]*)".*/\1/')
  fi
  printf '%s' "$ver"
}

EXISTING_BIN="$A8E_BIN_DIR/$OUT_FILE"
if [ "${FORCE:-false}" != "true" ] && [ -x "$EXISTING_BIN" ]; then
  TARGET_VERSION=$(resolve_target_version "$RELEASE_TAG" || true)
  if [ -n "${TARGET_VERSION:-}" ]; then
    CURRENT_VERSION=$("$EXISTING_BIN" --version 2>/dev/null | head -1 | awk '{print $NF}' || true)
    if [ -n "$CURRENT_VERSION" ] && [ "$CURRENT_VERSION" = "$TARGET_VERSION" ]; then
      echo "a8e is already up to date (v$CURRENT_VERSION). Set FORCE=true to reinstall."
      exit 0
    fi
    if [ -n "$CURRENT_VERSION" ]; then
      echo "Updating a8e: v$CURRENT_VERSION -> v$TARGET_VERSION"
    fi
  fi
fi

# --- 5) Download & extract ---
echo "Downloading a8e ($RELEASE_TAG): $FILE..."
if ! curl -sLf "$DOWNLOAD_URL" --output "$FILE"; then
  if [ -z "${A8E_VERSION:-}" ] && [ "${CANARY:-false}" != "true" ]; then
    LATEST_TAG=$(curl -s "https://api.github.com/repos/$REPO/releases/latest" | \
      grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')
    if [ -z "$LATEST_TAG" ]; then
      echo "Error: Failed to download $DOWNLOAD_URL and latest tag unavailable"
      exit 1
    fi
    DOWNLOAD_URL="https://github.com/$REPO/releases/download/$LATEST_TAG/$FILE"
    if ! curl -sLf "$DOWNLOAD_URL" --output "$FILE"; then
      echo "Error: Failed to download from $DOWNLOAD_URL"
      exit 1
    fi
  else
    echo "Error: Failed to download $DOWNLOAD_URL"
    exit 1
  fi
fi

TMP_DIR="/tmp/a8e_install_$RANDOM"
mkdir -p "$TMP_DIR"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "Extracting..."
set +e
if [ "$EXTRACT_CMD" = "tar" ]; then
  tar -xjf "$FILE" -C "$TMP_DIR" 2>/tmp/a8e_extract_err.log
  extract_exit_code=$?
else
  unzip -q "$FILE" -d "$TMP_DIR" 2>/tmp/a8e_extract_err.log
  extract_exit_code=$?
fi
set -e

if [ $extract_exit_code -ne 0 ]; then
  echo "Error: Failed to extract $FILE:"
  cat /tmp/a8e_extract_err.log
  rm -f /tmp/a8e_extract_err.log
  exit 1
fi
rm -f "$FILE" /tmp/a8e_extract_err.log

EXTRACT_DIR="$TMP_DIR"
if [ "$OS" = "windows" ] && [ -d "$TMP_DIR/a8e-package" ]; then
  EXTRACT_DIR="$TMP_DIR/a8e-package"
fi

if [ "$OS" = "windows" ]; then
  chmod +x "$EXTRACT_DIR/a8e.exe"
else
  chmod +x "$EXTRACT_DIR/a8e"
fi

# --- 6) Install ---
if [ ! -d "$A8E_BIN_DIR" ]; then
  echo "Creating directory: $A8E_BIN_DIR"
  mkdir -p "$A8E_BIN_DIR"
fi

echo "Installing a8e to $A8E_BIN_DIR/$OUT_FILE"
if [ "$OS" = "windows" ]; then
  mv "$EXTRACT_DIR/a8e.exe" "$A8E_BIN_DIR/$OUT_FILE"
  for dll in "$EXTRACT_DIR"/*.dll; do
    [ -f "$dll" ] && mv "$dll" "$A8E_BIN_DIR/"
  done
  ALIAS_FILE="arti.exe"
  cp -f "$A8E_BIN_DIR/$OUT_FILE" "$A8E_BIN_DIR/$ALIAS_FILE"
else
  mv "$EXTRACT_DIR/a8e" "$A8E_BIN_DIR/$OUT_FILE"
  ALIAS_FILE="arti"
  ln -sf "$OUT_FILE" "$A8E_BIN_DIR/$ALIAS_FILE"
fi
echo "Created alias: $A8E_BIN_DIR/$ALIAS_FILE -> $OUT_FILE"

# --- 7) Optional configure ---
if [ "$CONFIGURE" = true ]; then
  echo ""
  echo "Running a8e configure..."
  echo ""
  "$A8E_BIN_DIR/$OUT_FILE" configure
else
  echo "Skipping 'a8e configure' — run it manually later if needed."
fi

# --- 8) PATH check ---
if [[ ":$PATH:" != *":$A8E_BIN_DIR:"* ]]; then
  echo ""
  echo "Warning: a8e installed, but $A8E_BIN_DIR is not in your PATH."

  if [ "$OS" = "windows" ]; then
    echo "Add to your PowerShell profile:"
    echo '  $env:PATH = "$env:USERPROFILE\a8e;$env:PATH"'
  else
    SHELL_NAME=$(basename "$SHELL")
    echo ""
    if [ "$CONFIGURE" = true ]; then
      echo "What would you like to do?"
      echo "1) Add it for me"
      echo "2) Show instructions"
      if [ -t 0 ]; then
        read -p "Enter choice [1/2]: " choice
      elif [ -r /dev/tty ]; then
        read -p "Enter choice [1/2]: " choice < /dev/tty
      else
        choice=2
      fi

      case "$choice" in
      1)
        RC_FILE="$HOME/.${SHELL_NAME}rc"
        echo "export PATH=\"$A8E_BIN_DIR:\$PATH\"" >> "$RC_FILE"
        echo "Done! Run 'source $RC_FILE' to apply."
        ;;
      2|*)
        echo "Add to your ~/.${SHELL_NAME}rc:"
        echo "    export PATH=\"$A8E_BIN_DIR:\$PATH\""
        ;;
      esac
    else
      echo "Add to your ~/.$(basename "$SHELL")rc:"
      echo "    export PATH=\"$A8E_BIN_DIR:\$PATH\""
    fi
  fi
  echo ""
fi

echo ""
echo '    _         _   _            _       _       '
echo '   / \   _ __| |_(_) ___ _   _| | __ _| |_ ___ '
echo '  / _ \ | '"'"'__| __| |/ __| | | | |/ _` | __/ _ \'
echo ' / ___ \| |  | |_| | (__| |_| | | (_| | ||  __/'
echo '/_/   \_\_|   \__|_|\___|\__,_|_|\__,_|\__\___|'
echo ""
echo "   a8e CLI · Speak Freely. Code Locally."
echo ""
echo "a8e installed successfully! Run 'a8e --help' (or the shorter alias 'arti --help') to get started."
