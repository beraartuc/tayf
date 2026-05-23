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

/// Build a tayf command with XDG_CONFIG_HOME isolated to a tempdir.
/// Rev2 I-2: clear HOME and XDG_CONFIG_HOME explicitly so the
/// developer's real `~/.config/tayf/themes/` cannot leak into a test.
fn tayf_cmd_with_xdg(xdg: &std::path::Path) -> std::process::Command {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_tayf"));
    cmd.env_remove("HOME");
    cmd.env_remove("XDG_CONFIG_HOME");
    cmd.env("XDG_CONFIG_HOME", xdg);
    cmd
}

fn write_disk_theme(xdg: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let themes = xdg.join("tayf").join("themes");
    std::fs::create_dir_all(&themes).expect("create themes dir");
    let p = themes.join(format!("{name}.toml"));
    std::fs::write(&p, body).expect("write theme");
    p
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

#[test]
fn disk_theme_with_builtin_name_errors_at_startup() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_disk_theme(
        xdg.path(),
        "dark",
        "[[rules]]\nname = \"log_level\"\nstyle = { fg = \"red\" }\n",
    );

    let out = tayf_cmd_with_xdg(xdg.path())
        .arg("--theme")
        .arg("dark")
        .arg("--no-color")
        .arg("--no-hot-reload")
        .env("TAYF_DISABLE_BG_DETECT", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("run tayf");

    assert_eq!(
        out.status.code(),
        Some(64),
        "collision must exit EX_USAGE; got status: {:?}, stderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("shadows the built-in"),
        "collision diagnostic missing; stderr: {stderr}"
    );
}

#[test]
fn disk_theme_collision_case_insensitive_errors() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_disk_theme(xdg.path(), "dark", "");

    let out = tayf_cmd_with_xdg(xdg.path())
        .arg("--theme")
        .arg("DARK")
        .arg("--no-color")
        .arg("--no-hot-reload")
        .env("TAYF_DISABLE_BG_DETECT", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("run tayf");

    assert_eq!(out.status.code(), Some(64));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("shadows the built-in"), "got: {stderr}");
}

#[test]
fn disk_theme_with_validation_errors_lists_all() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    let body = r#"
[[rules]]
name = "nope_typo"

[[rules]]
name = "ipv4"
pattern = "\\d+"
style = { fg = "red" }

[[rules]]
name = "log_level"
enabled = false
"#;
    write_disk_theme(xdg.path(), "bad", body);

    let out = tayf_cmd_with_xdg(xdg.path())
        .arg("--theme")
        .arg("bad")
        .arg("--no-color")
        .arg("--no-hot-reload")
        .env("TAYF_DISABLE_BG_DETECT", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("run tayf");

    assert_eq!(out.status.code(), Some(64));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("3 validation errors"), "header: {stderr}");
    assert!(stderr.contains("rule 'nope_typo'"), "single-quote name: {stderr}");
    assert!(stderr.contains("rule 'ipv4'"), "single-quote name: {stderr}");
    assert!(stderr.contains("rule 'log_level'"), "single-quote name: {stderr}");
}

#[test]
fn disk_theme_general_section_rejected() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    let body = r#"
[general]
respect_existing_colors = false

[[rules]]
name = "ipv4"
style = { fg = "red" }
"#;
    write_disk_theme(xdg.path(), "mine", body);

    let out = tayf_cmd_with_xdg(xdg.path())
        .arg("--theme")
        .arg("mine")
        .arg("--no-color")
        .arg("--no-hot-reload")
        .env("TAYF_DISABLE_BG_DETECT", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("run tayf");

    assert_eq!(out.status.code(), Some(64));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("rule '<general>'"), "general sentinel name: {stderr}");
    assert!(stderr.contains("themes must not set [general]"), "kind message: {stderr}");
}

#[test]
fn unknown_theme_name_lists_disk_themes_in_error() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_disk_theme(xdg.path(), "foo", "");
    write_disk_theme(xdg.path(), "bar", "");

    let out = tayf_cmd_with_xdg(xdg.path())
        .arg("--theme")
        .arg("baz")
        .arg("--no-color")
        .arg("--no-hot-reload")
        .env("TAYF_DISABLE_BG_DETECT", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("run tayf");

    assert_eq!(out.status.code(), Some(64));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("available:"), "available list: {stderr}");
    assert!(stderr.contains("bar"), "should list bar: {stderr}");
    assert!(stderr.contains("dark"), "should list dark: {stderr}");
    assert!(stderr.contains("foo"), "should list foo: {stderr}");
    assert!(stderr.contains("light"), "should list light: {stderr}");
}
