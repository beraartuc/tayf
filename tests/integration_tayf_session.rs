//! Integration test: `TAYF_SESSION=1` child-env marker.
//!
//! Verifies spec §8: tayf MUST set `TAYF_SESSION=1` in the environment of
//! the child shell it spawns (via `CommandBuilder::env` in `src/pty.rs`).
//! This enables always-on rc-file guards (`exec tayf` loop-prevention) and
//! lets any tool inspect `$TAYF_SESSION` to detect it is running inside tayf.
//!
//! ## Assertion strategy
//!
//! tayf colorizes output, so the PTY byte stream is SGR-fragmented: the
//! literal value `1` in `TS<1>TS` may be wrapped in `\x1b[…m` codes. Per the
//! [[feedback-pty-substring-sgr-fragmentation]] memory rule, the test strips
//! ANSI SGR sequences from the captured output before asserting, then scans
//! for the plain sentinel string. It does NOT assert on SGR presence/absence.
//!
//! ## Sentinels
//!
//! The shell command `printf 'TS<%s>TS\n' "$TAYF_SESSION"` prints either
//! `TS<1>TS` (var set) or `TS<>TS` (var unset / empty). The test:
//!   1. asserts the cleaned output CONTAINS `TS<1>TS`, and
//!   2. asserts it does NOT contain `TS<>TS` (negative regression guard).

#![cfg(unix)]
#![allow(clippy::expect_used)] // reason: integration tests, not library code

use std::time::Duration;

mod common;

/// Strip ANSI SGR escape sequences (`ESC [ <params> m`) from a UTF-8 string.
///
/// SGR sequences follow the pattern `\x1b\[[0-9;]*m`. Other escape sequences
/// (OSC, cursor movement, etc.) are left intact — we only need to clean the
/// coloring codes that tayf injects around its rule matches.
fn strip_sgr(s: &str) -> String {
    // Walk byte-by-byte; cheap enough for the small outputs seen in tests.
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'[') {
            // Consume ESC [
            i += 2;
            // Consume digits and semicolons (parameter bytes 0x30–0x3F).
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b';') {
                i += 1;
            }
            // Consume the final byte ('m' for SGR).
            if i < bytes.len() && bytes[i] == b'm' {
                i += 1;
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// Spawn tayf wrapping `/bin/sh`, print `$TAYF_SESSION` between unique
/// sentinels, and assert the output contains `TS<1>TS` after SGR-stripping.
///
/// The negative guard (`TS<>TS` absent) pins that the variable is actually
/// set to the non-empty value `1`, not merely present-but-empty.
#[test]
fn tayf_session_is_set_in_child_env() {
    // Give tayf 500 ms to start the shell, then 3 s for output.
    let output = common::spawn_with_input(
        "printf 'TS<%s>TS\\n' \"$TAYF_SESSION\"; exit\n",
        Duration::from_secs(4),
    );

    let raw = String::from_utf8_lossy(&output);
    let cleaned = strip_sgr(&raw);

    assert!(
        cleaned.contains("TS<1>TS"),
        "TAYF_SESSION must be set to '1' in the child shell; cleaned output was:\n{cleaned}\n\
         (raw bytes: {raw:?})"
    );
    assert!(
        !cleaned.contains("TS<>TS"),
        "TAYF_SESSION must NOT be empty in the child shell; cleaned output was:\n{cleaned}"
    );
}
