//! End-to-end smoke tests for the tayf binary.
//!
//! These exercise the spawn -> output -> exit path with real PTY allocation.
//! Run with `cargo test --test integration_smoke`.

mod common;

use std::time::Duration;

use common::spawn_capture;

fn tayf_bin() -> &'static str {
    env!("CARGO_BIN_EXE_tayf")
}

#[test]
fn version_flag_prints_banner() {
    let out = spawn_capture(tayf_bin(), &["--version"], Duration::from_secs(5));
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("tayf"), "missing tayf in: {s}");
    assert!(s.contains("rustc"), "missing rustc in: {s}");
}

#[test]
fn help_flag_prints_usage() {
    let out = spawn_capture(tayf_bin(), &["--help"], Duration::from_secs(5));
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("Usage:") || s.contains("USAGE:"), "missing usage: {s}");
    assert!(s.contains("--shell"));
    assert!(s.contains("--login"));
    assert!(s.contains("--no-color"));
}
