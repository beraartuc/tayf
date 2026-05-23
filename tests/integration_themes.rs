//! End-to-end integration tests for `--theme`.
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
    // Use an explicit empty config so the host's user config (which may be
    // present and even malformed on developer machines) cannot mask the
    // theme error with a "config error" stderr.
    let dir = tempfile::tempdir().expect("tmpdir");
    let cfg_path = dir.path().join("config.toml");
    std::fs::write(&cfg_path, "").expect("write empty config");

    let out = Command::new(tayf_bin())
        .arg("--config")
        .arg(&cfg_path)
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

#[test]
fn theme_help_text_mentions_disk_themes() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_tayf"))
        .arg("--help")
        .output()
        .expect("run tayf --help");
    assert!(out.status.success(), "tayf --help should succeed");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Disk themes"),
        "help text should mention disk themes (Rev2 I-11): got: {stdout}"
    );
    assert!(
        stdout.contains("themes/"),
        "help text should point at <config_base>/themes/: got: {stdout}"
    );
}

#[test]
fn config_general_theme_drives_resolution_when_cli_omits() {
    // Pins the `[general] theme = "..."` config-driven path end-to-end.
    // When --theme is NOT passed on the CLI, the binary must resolve
    // the theme from the config file via `effective_theme` reconciliation
    // in `Tayf::run`. A bogus name surfaces as Error::Theme → EX_USAGE 64.
    let dir = tempfile::tempdir().expect("tmpdir");
    let cfg_path = dir.path().join("config.toml");
    std::fs::write(&cfg_path, "[general]\ntheme = \"totally-not-a-real-theme-from-config\"\n")
        .expect("write config");

    let out = Command::new(tayf_bin())
        .arg("--config")
        .arg(&cfg_path)
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
        stderr.contains("totally-not-a-real-theme-from-config"),
        "stderr must echo the bogus config-driven name; got: {stderr}"
    );
    assert!(stderr.contains("dark"), "stderr must list 'dark'; got: {stderr}");
    assert!(stderr.contains("light"), "stderr must list 'light'; got: {stderr}");
}
