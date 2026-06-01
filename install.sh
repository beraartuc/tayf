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

# Download $1 to file $2 ("-" = stdout). Prefers curl, falls back to busybox/GNU
# wget (Alpine ships wget, not curl). Returns non-zero on HTTP/transport error.
http_get() {
  url="$1"; out="$2"
  if command -v curl >/dev/null 2>&1; then
    if [ "$out" = "-" ]; then curl -fsSL "$url"; else curl -fsSL -o "$out" "$url"; fi
  elif command -v wget >/dev/null 2>&1; then
    if [ "$out" = "-" ]; then wget -qO- "$url"; else wget -qO "$out" "$url"; fi
  else
    die "need curl or wget to download"
  fi
}

# Resolve the tag to install: $TAYF_VERSION if set, else the latest release via
# the GitHub API. Aborts with a clear message if the tag cannot be resolved
# (e.g. the unauthenticated API rate-limited the request).
resolve_version() {
  if [ -n "${TAYF_VERSION:-}" ]; then
    printf '%s' "$TAYF_VERSION"
    return 0
  fi
  body="$(http_get "https://api.github.com/repos/${REPO}/releases/latest" - || true)"
  tag="$(parse_tag_from_json "$body")"
  [ -n "$tag" ] || die "could not resolve the latest release tag (GitHub API rate limit?) — set TAYF_VERSION=vX.Y.Z"
  printf '%s' "$tag"
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    die "need sha256sum or shasum to verify the download"
  fi
}

# Mandatory integrity check. $1 = binary, $2 = .sha256 file ("<hash>  <name>").
# Aborts (no install) on mismatch.
verify_checksum() {
  want="$(awk '{print $1}' "$2")"
  got="$(sha256_of "$1")"
  [ -n "$want" ] || die "checksum file was empty"
  if [ "$want" != "$got" ]; then
    die "checksum mismatch: expected ${want}, got ${got} — refusing to install"
  fi
  log "checksum OK (${got})"
}

# Best-effort provenance check. $1 = binary, $2 = .sigstore.json bundle.
#  - gh missing or not authenticated -> skip (non-fatal) + print the manual cmd.
#  - verifies -> proceed.
#  - gh authenticated but verification FAILS -> abort (genuine red flag).
# (gh attestation verify requires an authenticated gh even for public repos.)
verify_attestation() {
  bin="$1"; bundle="$2"
  if ! command -v gh >/dev/null 2>&1; then
    log "provenance check skipped (gh CLI not found)"
    log "  verify manually: gh attestation verify ${bin} --repo ${REPO}"
    return 0
  fi
  if ! gh auth status >/dev/null 2>&1; then
    log "provenance check skipped (gh not authenticated)"
    log "  verify manually: gh attestation verify ${bin} --repo ${REPO}"
    return 0
  fi
  if [ ! -f "$bundle" ]; then
    log "provenance check skipped (attestation bundle not available)"
    return 0
  fi
  if gh attestation verify "$bin" --repo "$REPO" --bundle "$bundle" >/dev/null 2>&1; then
    log "provenance verified (Sigstore build attestation)"
    return 0
  fi
  die "provenance verification FAILED for ${bin} — refusing to install. This may be a transient Sigstore/TUF fetch issue or a real mismatch; re-run, or pin TAYF_VERSION and verify manually: gh attestation verify ${bin} --repo ${REPO}"
}

main() {
  die "install.sh is not fully implemented yet"
}

[ "${TAYF_INSTALL_NO_MAIN:-}" = "1" ] || main "$@"
