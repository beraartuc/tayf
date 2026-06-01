#!/bin/sh
# Unit tests for install.sh pure functions. Sources install.sh with the main
# entrypoint disabled, then exercises the detection/parse helpers. No network.
set -eu

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../install.sh
# shellcheck disable=SC1091  # reason: sourced at runtime; path not statically resolvable under plain shellcheck (CI runs `shellcheck` without -x)
TAYF_INSTALL_NO_MAIN=1 . "${here}/../install.sh"

fail=0
expect_target() { # os arch want
  got="$(detect_target "$1" "$2" 2>/dev/null || echo ERR)"
  if [ "$got" = "$3" ]; then
    printf 'ok:   detect_target %s %s -> %s\n' "$1" "$2" "$got"
  else
    printf 'FAIL: detect_target %s %s -> %s (want %s)\n' "$1" "$2" "$got" "$3"
    fail=1
  fi
}

expect_target Darwin arm64   aarch64-apple-darwin
expect_target Darwin x86_64  x86_64-apple-darwin
expect_target Linux  x86_64  x86_64-unknown-linux-musl
expect_target Linux  amd64   x86_64-unknown-linux-musl
expect_target Linux  aarch64 aarch64-unknown-linux-musl
expect_target Linux  arm64   aarch64-unknown-linux-musl
expect_target Linux  armv7l  ERR
expect_target FreeBSD x86_64 ERR

want_tag="v0.11.0"
got_tag="$(parse_tag_from_json '{"url":"x","tag_name": "v0.11.0", "name":"tayf v0.11.0"}')"
if [ "$got_tag" = "$want_tag" ]; then
  printf 'ok:   parse_tag_from_json -> %s\n' "$got_tag"
else
  printf 'FAIL: parse_tag_from_json -> %s (want %s)\n' "$got_tag" "$want_tag"
  fail=1
fi

# parse_tag_from_json on an error body (no tag_name) must yield empty — this is
# the precondition for resolve_version's rate-limit empty-tag guard.
got_empty="$(parse_tag_from_json '{"message":"API rate limit exceeded","documentation_url":"https://docs.github.com"}')"
if [ -z "$got_empty" ]; then
  printf 'ok:   parse_tag_from_json empty on no-tag body\n'
else
  printf 'FAIL: parse_tag_from_json returned non-empty on no-tag body: %s\n' "$got_empty"
  fail=1
fi

# verify_checksum round-trip: a matching .sha256 passes, a wrong one aborts.
tmpd="$(mktemp -d)"
printf 'hello tayf\n' > "${tmpd}/blob"
real="$(sha256_of "${tmpd}/blob")"
printf '%s  blob\n' "$real" > "${tmpd}/blob.sha256"
if ( verify_checksum "${tmpd}/blob" "${tmpd}/blob.sha256" ) >/dev/null 2>&1; then
  printf 'ok:   verify_checksum matches\n'
else
  printf 'FAIL: verify_checksum rejected a correct checksum\n'; fail=1
fi
printf '%s  blob\n' "0000000000000000000000000000000000000000000000000000000000000000" > "${tmpd}/blob.sha256"
if ( verify_checksum "${tmpd}/blob" "${tmpd}/blob.sha256" ) >/dev/null 2>&1; then
  printf 'FAIL: verify_checksum accepted a wrong checksum\n'; fail=1
else
  printf 'ok:   verify_checksum rejects a mismatch\n'
fi
rm -rf "$tmpd"

if [ "$fail" = 0 ]; then printf 'ALL PASS\n'; else printf 'FAILURES\n'; exit 1; fi
