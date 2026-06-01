#!/bin/sh
# tayf installer — detect platform, download the matching signed release binary,
# verify it, and install to ~/.local/bin. POSIX sh; no bashisms.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/beraartuc/tayf/main/install.sh | sh
#
# Environment:
#   TAYF_VERSION       install a specific tag (e.g. v0.11.0); default: latest release
#   TAYF_INSTALL_DIR   install directory; default: $HOME/.local/bin
# shellcheck shell=sh
set -eu

export REPO="beraartuc/tayf"
export BIN_NAME="tayf"

log() { printf 'tayf-install: %s\n' "$1" >&2; }
die() { printf 'tayf-install: error: %s\n' "$1" >&2; exit 1; }

# Map (uname -s, uname -m) to a release target triple. Echoes the triple, or
# returns 1 if the platform is unsupported.
detect_target() {
  os="$1"; arch="$2"
  case "$os" in
    Darwin)
      case "$arch" in
        arm64)  echo "aarch64-apple-darwin" ;;
        x86_64) echo "x86_64-apple-darwin" ;;
        *) return 1 ;;
      esac ;;
    Linux)
      case "$arch" in
        x86_64|amd64)  echo "x86_64-unknown-linux-musl" ;;
        aarch64|arm64) echo "aarch64-unknown-linux-musl" ;;
        *) return 1 ;;
      esac ;;
    *) return 1 ;;
  esac
}

# Extract the "tag_name" value from a GitHub releases/latest JSON body.
# Echoes the tag, or nothing if absent.
parse_tag_from_json() {
  printf '%s' "$1" \
    | grep -m1 '"tag_name":' \
    | sed -e 's/.*"tag_name":[[:space:]]*"//' -e 's/".*//'
}

main() {
  die "install.sh is not fully implemented yet"
}

[ "${TAYF_INSTALL_NO_MAIN:-}" = "1" ] || main "$@"
