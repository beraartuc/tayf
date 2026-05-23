//! End-to-end integration tests for the v0.3.0 ANSI state machine.
//!
//! Spawns tayf in a captive `/bin/sh`, sends `printf` commands that emit
//! specific ANSI byte sequences, and asserts on the captured PTY output.
//! Verifies OSC / DCS / SGR / multi-byte ESC handling through the full
//! Pipeline three-path architecture (spec §5).

#![allow(clippy::expect_used)] // reason: tests, not library code

mod common;

use std::io::Write;
use std::time::Duration;

/// Default test timeout. PTY drain is bounded by the child exiting on
/// `exit\n`; the timeout is a safety net for a hung run.
const TIMEOUT: Duration = Duration::from_secs(5);

#[test]
fn osc_8_hyperlink_intact_through_real_shell() {
    // Hyperlink OSC: ESC ] 8 ; ; <uri> BEL <text> ESC ] 8 ; ; BEL.
    // The state machine must collect every byte of both OSC segments
    // and pass them through untouched (rules never fire inside OSC).
    let script = "printf '\\033]8;;https://example.com\\007click\\033]8;;\\007'\nexit\n";
    let out = common::spawn_with_input(script, TIMEOUT);
    let needle = b"\x1b]8;;https://example.com\x07click\x1b]8;;\x07";
    assert!(
        out.windows(needle.len()).any(|w| w == needle),
        "hyperlink missing from tayf output: {:?}",
        String::from_utf8_lossy(&out)
    );
}

#[test]
fn osc_2_title_intact_through_real_shell() {
    // Window-title OSC: ESC ] 2 ; <title> BEL.
    let script = "printf '\\033]2;test-title\\007'\nexit\n";
    let out = common::spawn_with_input(script, TIMEOUT);
    let needle = b"\x1b]2;test-title\x07";
    assert!(
        out.windows(needle.len()).any(|w| w == needle),
        "title sequence missing from tayf output: {:?}",
        String::from_utf8_lossy(&out)
    );
}

#[test]
fn sgr_respect_true_via_config_skips_rules() {
    // With `respect_existing_colors = true`, a line that already carries
    // any SGR must pass through byte-for-byte; tayf must NOT add its own
    // SGR around the embedded IPv4.
    let dir = tempfile::tempdir().expect("tmpdir");
    let cfg_path = dir.path().join("config.toml");
    let mut f = std::fs::File::create(&cfg_path).expect("create cfg");
    writeln!(f, "[general]").expect("write");
    writeln!(f, "respect_existing_colors = true").expect("write");
    drop(f);

    let script = "printf '\\033[31mERROR 192.168.1.1\\033[0m\\n'\nexit\n";
    let out = common::spawn_with_input_and_args(
        script,
        &["--config", cfg_path.to_str().expect("utf-8 path")],
        TIMEOUT,
    );
    let needle = b"\x1b[31mERROR 192.168.1.1\x1b[0m";
    assert!(
        out.windows(needle.len()).any(|w| w == needle),
        "SGR-wrapped ERROR line not byte-for-byte intact under respect=true: {:?}",
        String::from_utf8_lossy(&out)
    );
}

#[test]
fn sgr_respect_false_default_applies_rules() {
    // With `respect_existing_colors = false`, tayf's rules run even on
    // SGR-bearing lines. We feed a plain (no-SGR) line so we don't have
    // to assert on the precise interleaving of pre-existing SGR with
    // tayf-injected SGR; the goal is simply to confirm tayf injected
    // some SGR introducer of its own around the matched payload.
    let dir = tempfile::tempdir().expect("tmpdir");
    let cfg_path = dir.path().join("config.toml");
    let mut f = std::fs::File::create(&cfg_path).expect("create cfg");
    writeln!(f, "[general]").expect("write");
    writeln!(f, "respect_existing_colors = false").expect("write");
    drop(f);

    let script = "printf 'ERROR 192.168.1.1\\n'\nexit\n";
    let out = common::spawn_with_input_and_args(
        script,
        &["--config", cfg_path.to_str().expect("utf-8 path")],
        TIMEOUT,
    );
    let s = String::from_utf8_lossy(&out);
    assert!(
        out.windows(2).any(|w| w == b"\x1b["),
        "expected tayf to inject SGR sequences on plain ERROR line: {s:?}"
    );
}

#[test]
fn multi_byte_esc_g0_designate_handled() {
    // G0 designate: ESC ( B — selects ASCII as the G0 character set.
    // The state machine must treat the entire 3-byte sequence as one
    // escape and pass it through (rules never fire inside ESC sequences).
    let script = "printf '\\033(B test\\n'\nexit\n";
    let out = common::spawn_with_input(script, TIMEOUT);
    let needle = b"\x1b(B";
    assert!(
        out.windows(needle.len()).any(|w| w == needle),
        "G0 designate sequence missing: {:?}",
        String::from_utf8_lossy(&out)
    );
}

#[test]
fn dcs_query_passthrough() {
    // DCS device-attribute-style SGR query: ESC P $ q m ESC \\
    // The state machine must collect the DCS payload and the
    // string-terminator ESC \\ as one unit and pass them through.
    let script = "printf '\\033P$qm\\033\\\\'\nexit\n";
    let out = common::spawn_with_input(script, TIMEOUT);
    let needle = b"\x1bP$qm\x1b\\";
    assert!(
        out.windows(needle.len()).any(|w| w == needle),
        "DCS sequence missing: {:?}",
        String::from_utf8_lossy(&out)
    );
}
