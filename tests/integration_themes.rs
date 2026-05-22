//! End-to-end integration tests for `--theme` and `[general] theme`.
//!
//! These spawn the compiled `tayf` binary as a subprocess. The unknown-theme
//! case exits with BSD `EX_USAGE` (64) per spec §4.6, and `--help` advertises
//! the new flag.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tayf_bin() -> PathBuf {
    // CARGO_BIN_EXE_tayf is set by cargo for integration tests of binaries
    // in the same crate. Tayf is the only binary, so this resolves.
    PathBuf::from(env!("CARGO_BIN_EXE_tayf"))
}

#[test]
fn unknown_theme_exits_ex_usage() {
    let out = Command::new(tayf_bin())
        .arg("--theme")
        .arg("totally-not-a-real-theme")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn tayf");
    assert_eq!(
        out.status.code(),
        Some(64),
        "expected EX_USAGE; got {:?}; stderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("totally-not-a-real-theme"),
        "stderr must echo the bad name; got: {stderr}"
    );
    assert!(stderr.contains("dark"), "stderr must list 'dark'; got: {stderr}");
    assert!(stderr.contains("light"), "stderr must list 'light'; got: {stderr}");
}

#[test]
fn theme_flag_appears_in_help_text() {
    let out = Command::new(tayf_bin())
        .arg("--help")
        .stdin(Stdio::null())
        .output()
        .expect("spawn tayf --help");
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--theme"), "help must mention --theme; got: {stdout}");
}
