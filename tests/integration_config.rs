//! End-to-end smoke for the v0.2.0 TOML config loader.
//!
//! These tests spawn the compiled `tayf` binary with `--config <path>` and
//! inspect exit code + stderr. They do NOT exercise the full PTY loop; the
//! goal is to verify config-loading error surfaces (path resolution, parse
//! errors, validation) all the way out to the process boundary.

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

fn tayf_bin() -> PathBuf {
    // CARGO_BIN_EXE_tayf is set by cargo for integration tests.
    PathBuf::from(env!("CARGO_BIN_EXE_tayf"))
}

#[test]
fn missing_config_path_exits_64() {
    let out = Command::new(tayf_bin())
        .args(["--config", "/this/path/does/not/exist.toml"])
        .output()
        .expect("spawn tayf");
    assert_eq!(out.status.code(), Some(64), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("config error"));
    assert!(stderr.contains("/this/path/does/not/exist.toml"));
}

#[test]
fn broken_toml_exits_64_with_friendly_message_and_path() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let path = dir.path().join("broken.toml");
    // `[general]\nrespect_existing_colors = notabool\n` reliably fails to
    // parse in toml 0.9 (unquoted identifier where a bool is expected).
    std::fs::write(&path, "[general]\nrespect_existing_colors = notabool\n").expect("write");
    let out = Command::new(tayf_bin()).args(["--config"]).arg(&path).output().expect("spawn tayf");
    assert_eq!(out.status.code(), Some(64), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("config error"), "stderr: {stderr}");
    assert!(
        stderr.contains(path.to_str().unwrap()),
        "file path must appear in stderr (path-threading regression check): {stderr}"
    );
}

#[test]
fn new_rule_missing_pattern_exits_64_with_path_and_rule_name() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let path = dir.path().join("cfg.toml");
    let mut f = std::fs::File::create(&path).expect("create");
    f.write_all(b"[[rules]]\nname = \"uuid\"\nstyle = { fg = \"red\" }\n").unwrap();
    let out = Command::new(tayf_bin()).args(["--config"]).arg(&path).output().expect("spawn tayf");
    assert_eq!(out.status.code(), Some(64), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("uuid"));
    assert!(stderr.to_lowercase().contains("pattern"));
    // Path-threading regression check: validation errors from inside
    // `apply_user_rules` must still carry the actual config file path,
    // not the empty-string sentinel.
    assert!(
        stderr.contains(path.to_str().unwrap()),
        "config file path must appear in validation errors: {stderr}"
    );
}

#[test]
fn help_short_circuits_before_config_load() {
    // `--help` is handled by clap inside `Args::try_parse_from_env` BEFORE
    // `Tayf::run` is even entered, so `--config <path>` is never consulted
    // and no PTY is allocated. This test guards against a future refactor
    // that accidentally reorders config loading before help dispatch.
    let dir = tempfile::tempdir().expect("tmpdir");
    let path = dir.path().join("empty.toml");
    std::fs::write(&path, "").expect("write");
    let out = Command::new(tayf_bin())
        .args(["--config"])
        .arg(&path)
        .arg("--help")
        .output()
        .expect("spawn tayf");
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));
}
