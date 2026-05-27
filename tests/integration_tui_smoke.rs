//! v0.5.4 — `tayf config dump` + `tayf config status` smoke tests.
//!
//! Spawns the binary subprocess (no PTY needed — dump/status are
//! pure stdout writers) and asserts byte-pinned output shapes.

#![allow(clippy::expect_used)]
// reason: tests, not library code

use std::process::Command;

fn tayf_bin() -> &'static str {
    env!("CARGO_BIN_EXE_tayf")
}

#[test]
fn dump_default_parses_as_valid_toml() {
    let out =
        Command::new(tayf_bin()).args(["config", "dump"]).output().expect("spawn tayf config dump");
    assert!(out.status.success(), "exit non-zero: {:?}", out.status);
    let body = String::from_utf8(out.stdout).expect("utf8");
    let parsed: toml::Value = toml::de::from_str(&body).expect("dump output must be valid TOML");
    assert!(parsed.get("patterns").is_some(), "must contain [[patterns]]");
    assert!(parsed.get("themes").is_some(), "must contain [themes.*]");
    assert!(parsed.get("profiles").is_some(), "must contain [profiles.*]");
}

#[test]
fn dump_kind_patterns_only_emits_patterns_section() {
    let out = Command::new(tayf_bin())
        .args(["config", "dump", "--kind", "patterns"])
        .output()
        .expect("spawn");
    assert!(out.status.success());
    let body = String::from_utf8(out.stdout).expect("utf8");
    assert!(body.contains("[[patterns]]"), "must contain [[patterns]]");
    assert!(!body.contains("[themes."), "must NOT contain [themes.*]; got:\n{body}");
    assert!(!body.contains("[profiles."), "must NOT contain [profiles.*]; got:\n{body}");
}

#[test]
fn status_no_config_renders_byte_pinned_lines() {
    let out = Command::new(tayf_bin())
        .args(["config", "status"])
        .env("HOME", "/nonexistent-tayf-test")
        .env_remove("XDG_CONFIG_HOME")
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "exit non-zero: {:?}; stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let body = String::from_utf8(out.stdout).expect("utf8");
    // Byte-pinned per memory feedback_test_assertion_specificity.
    assert!(body.contains("config: (no config file)\n"), "got:\n{body}");
    assert!(body.contains("theme: (unresolved: no config + no --theme)\n"), "got:\n{body}");
    assert!(body.contains("profile: (unresolved: no config + no --profile)\n"), "got:\n{body}");
    assert!(body.contains("bg detect: (probed at runtime)\n"), "got:\n{body}");
    assert!(body.contains("hot reload: no config dir resolved\n"), "got:\n{body}");
}

#[test]
fn status_with_broken_config_exits_64_and_prints_partial() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let cfg = tmp.path().join("config.toml");
    std::fs::write(&cfg, b"[broken").expect("write");
    let out = Command::new(tayf_bin())
        .arg("--config")
        .arg(&cfg)
        .args(["config", "status"])
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(64), "must exit EX_USAGE (64); got {:?}", out.status);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("config parse error"),
        "stderr must mention parse error; got: {stderr}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("theme:"), "stdout still prints partial state; got: {stdout}");
}
