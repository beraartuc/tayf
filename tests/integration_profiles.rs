//! Integration tests for the v0.12.0 disk-profile system.
//!
//! A profile is a named, switchable preset at
//! `~/.config/tayf/profiles/<name>.toml` using the same `[[rules]]` schema as
//! `config.toml`. When active, its `[[rules]]` REPLACE `config.toml`'s; the
//! built-ins remain the substrate and `[general]` always comes from
//! `config.toml`. The embedded profile library is retired — the six domain
//! rules are now built-ins.
//!
//! Tests exercise load + REPLACE + theme precedence + hot-reload end-to-end
//! via tempdir-based disk fixtures and PTY harnesses. Per-test
//! `tempfile::TempDir` rooted at `XDG_CONFIG_HOME`, env isolation that removes
//! `HOME` + `XDG_CONFIG_HOME` from the child before re-setting, and
//! `TAYF_DISABLE_BG_DETECT=1` to suppress macOS OSC 11 hang risk.

#![cfg(unix)]
#![allow(clippy::expect_used)] // reason: tests, not library code

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use portable_pty::{CommandBuilder, PtySize};

fn tayf_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tayf"))
}

/// Write a per-test user config TOML into `<xdg>/tayf/config.toml`.
fn write_user_config(xdg: &Path, body: &str) -> PathBuf {
    let dir = xdg.join("tayf");
    std::fs::create_dir_all(&dir).expect("create config dir");
    let p = dir.join("config.toml");
    std::fs::write(&p, body).expect("write config");
    p
}

/// Write a per-test profile TOML into `<xdg>/tayf/profiles/<name>.toml`.
fn write_profile(xdg: &Path, name: &str, body: &str) -> PathBuf {
    let profiles = xdg.join("tayf").join("profiles");
    std::fs::create_dir_all(&profiles).expect("create profiles dir");
    let p = profiles.join(format!("{name}.toml"));
    std::fs::write(&p, body).expect("write profile");
    p
}

/// Spawn tayf under a real PTY with the given extra args. Feeds
/// `echo <token>\nexit\n` to the child shell and drains the master
/// until the child exits.
///
/// `COLORTERM=truecolor` is forced so 24-bit colors render as `38;2;R;G;B`
/// sequences rather than falling back to 8-color ANSI.
fn run_in_pty(xdg: &Path, token: &str, extra_args: &[&str]) -> Vec<u8> {
    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_tayf"));
    cmd.env_remove("HOME");
    cmd.env_remove("XDG_CONFIG_HOME");
    cmd.env("XDG_CONFIG_HOME", xdg);
    cmd.env("TAYF_DISABLE_BG_DETECT", "1");
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.arg("--shell");
    cmd.arg("/bin/sh");
    cmd.arg("--no-hot-reload");
    for a in extra_args {
        cmd.arg(a);
    }

    let mut child = pair.slave.spawn_command(cmd).expect("spawn tayf");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("clone reader");
    let mut writer = pair.master.take_writer().expect("take writer");

    // Give tayf time to install its signal handler, spawn the shell,
    // and be ready to receive input.
    thread::sleep(Duration::from_millis(200));
    let line = format!("echo {token}\nexit\n");
    writer.write_all(line.as_bytes()).expect("write");
    drop(writer);

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let _reader_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        loop {
            match reader.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&tmp[..n]),
                Err(_) => break,
            }
        }
        let _ = tx.send(buf);
    });

    let _ = child.wait();
    rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default()
}

/// True if `s` carries any cyan SGR (foreground code 36) — used as a probe
/// for a profile/user rule that sets `fg = "cyan"`.
fn has_cyan(s: &str) -> bool {
    s.contains("\u{1b}[36m") || s.contains("\u{1b}[36;") || s.contains(";36m") || s.contains(";36;")
}

// ---------------------------------------------------------------------------
// Theme-precedence probes — shared helpers.
//
// The 8-cell theme precedence matrix uses the built-in `ipv4` rule as a
// probe. dark.toml styles ipv4 #33c7ff -> 38;2;51;199;255; light.toml styles
// ipv4 #0c6b94 -> 38;2;12;107;148. The token 192.168.1.1 triggers the rule;
// the truecolor SGR signature distinguishes which theme was active.
// ---------------------------------------------------------------------------

fn probe_ipv4_sgr(xdg: &Path, args: &[&str]) -> String {
    let bytes = run_in_pty(xdg, "192.168.1.1", args);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn assert_light_active(s: &str) {
    let has_light = s.contains("38;2;12;107;148");
    let has_dark = s.contains("38;2;51;199;255");
    assert!(has_light, "expected light theme Neon ipv4 SGR (38;2;12;107;148) on ipv4: {s:?}");
    assert!(!has_dark, "must not see dark theme Neon ipv4 SGR (38;2;51;199;255): {s:?}");
}

fn assert_dark_active(s: &str) {
    let has_dark = s.contains("38;2;51;199;255");
    let has_light = s.contains("38;2;12;107;148");
    assert!(has_dark, "expected dark theme Neon ipv4 SGR (38;2;51;199;255) on ipv4: {s:?}");
    assert!(!has_light, "must not see light theme Neon ipv4 SGR (38;2;12;107;148): {s:?}");
}

// ---------------------------------------------------------------------------
// REPLACE semantics: a named profile's [[rules]] replace config.toml's;
// built-ins remain the substrate.
// ---------------------------------------------------------------------------

#[test]
fn named_profile_replaces_config_rules_builtins_remain_substrate() {
    // config.toml disables fqdn; the named profile is silent on fqdn, so under
    // REPLACE semantics fqdn returns to its default (on). The profile enables
    // the default-off `container_id` built-in.
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_user_config(
        xdg.path(),
        "[general]\nprofile = \"work\"\n\n[[rules]]\nname = \"fqdn\"\nenabled = false\n",
    );
    write_profile(xdg.path(), "work", "[[rules]]\nname = \"container_id\"\nenabled = true\n");

    // container_id is a 12-hex shape; under the `work` profile it is enabled.
    let bytes = run_in_pty(xdg.path(), "abc123def456", &[]);
    let s = String::from_utf8_lossy(&bytes);
    assert!(s.contains("abc123def456"), "container_id token must survive: {s:?}");
    assert!(
        s.contains("\u{1b}["),
        "profile-enabled container_id must colorize a 12-hex token: {s:?}"
    );

    // fqdn returns to default-on (config.toml's disable does NOT carry into the
    // profile under REPLACE semantics).
    let fqdn_bytes = run_in_pty(xdg.path(), "example.com", &[]);
    let fqdn_s = String::from_utf8_lossy(&fqdn_bytes);
    assert!(fqdn_s.contains("example.com"), "fqdn token must survive: {fqdn_s:?}");
    assert!(
        fqdn_s.contains("\u{1b}["),
        "fqdn must be default-on under the profile (config disable does not carry): {fqdn_s:?}"
    );
}

#[test]
fn named_profile_new_pattern_rule_fires() {
    // A profile may add brand-new patterns on top of the built-in substrate.
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_user_config(xdg.path(), "[general]\nprofile = \"work\"\n");
    write_profile(
        xdg.path(),
        "work",
        "[[rules]]\nname = \"ticket\"\npattern = '\\bJIRA-[0-9]+\\b'\nstyle = { fg = \"cyan\" }\n",
    );
    let bytes = run_in_pty(xdg.path(), "JIRA-1234", &[]);
    let s = String::from_utf8_lossy(&bytes);
    assert!(s.contains("JIRA-1234"), "ticket token must survive: {s:?}");
    assert!(has_cyan(&s), "profile-added `ticket` rule should colorize JIRA-1234 cyan: {s:?}");
}

#[test]
fn cli_profile_overrides_config_general_profile() {
    // Two disk profiles 'aws' and 'k8s'. CLI passes 'aws'; config sets 'k8s'.
    // The active rule set must reflect aws (its `ticket` rule), not k8s.
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(
        xdg.path(),
        "aws",
        "[[rules]]\nname = \"ticket\"\npattern = '\\bJIRA-[0-9]+\\b'\nstyle = { fg = \"cyan\" }\n",
    );
    write_profile(
        xdg.path(),
        "k8s",
        "[[rules]]\nname = \"pod_marker\"\npattern = '\\bPOD-[A-Z]{4}\\b'\nstyle = { fg = \"magenta\" }\n",
    );
    write_user_config(xdg.path(), "[general]\nprofile = \"k8s\"\n");

    let bytes = run_in_pty(xdg.path(), "'JIRA-1234 POD-ABCD'", &["--profile", "aws"]);
    let s = String::from_utf8_lossy(&bytes);
    let has_magenta = s.contains("\u{1b}[35m") || s.contains(";35m") || s.contains(";35;");
    assert!(has_cyan(&s), "aws profile active -> ticket should be cyan: {s:?}");
    assert!(!has_magenta, "aws profile active -> pod_marker (k8s) should NOT be magenta: {s:?}");
}

// ---------------------------------------------------------------------------
// Theme precedence matrix: --theme (CLI) > [general] theme (config) >
// profile.theme > bg-detect default. Eight cells.
// ---------------------------------------------------------------------------

// Cell 1 of 8: CLI=light, config=dark, profile.theme=dark -> light wins.
#[test]
fn theme_precedence_cli_wins_over_config_and_profile() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(xdg.path(), "myprofile", "theme = \"dark\"\n");
    write_user_config(xdg.path(), "[general]\ntheme = \"dark\"\nprofile = \"myprofile\"\n");
    let s = probe_ipv4_sgr(xdg.path(), &["--theme", "light"]);
    assert_light_active(&s);
}

// Cell 2 of 8: CLI=light, config=dark, profile.theme=none -> light wins.
#[test]
fn theme_precedence_cli_wins_over_config_no_profile_theme() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(xdg.path(), "myprofile", "");
    write_user_config(xdg.path(), "[general]\ntheme = \"dark\"\nprofile = \"myprofile\"\n");
    let s = probe_ipv4_sgr(xdg.path(), &["--theme", "light"]);
    assert_light_active(&s);
}

// Cell 3 of 8: CLI=light, config=none, profile.theme=dark -> light wins.
#[test]
fn theme_precedence_cli_wins_over_profile_no_config_theme() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(xdg.path(), "myprofile", "theme = \"dark\"\n");
    write_user_config(xdg.path(), "[general]\nprofile = \"myprofile\"\n");
    let s = probe_ipv4_sgr(xdg.path(), &["--theme", "light"]);
    assert_light_active(&s);
}

// Cell 4 of 8: CLI=light, config=none, profile.theme=none -> light wins.
#[test]
fn theme_precedence_cli_only_no_config_no_profile_theme() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(xdg.path(), "myprofile", "");
    write_user_config(xdg.path(), "[general]\nprofile = \"myprofile\"\n");
    let s = probe_ipv4_sgr(xdg.path(), &["--theme", "light"]);
    assert_light_active(&s);
}

// Cell 5 of 8: CLI=none, config=light, profile.theme=dark -> config wins.
#[test]
fn theme_precedence_config_wins_over_profile() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(xdg.path(), "myprofile", "theme = \"dark\"\n");
    write_user_config(xdg.path(), "[general]\ntheme = \"light\"\nprofile = \"myprofile\"\n");
    let s = probe_ipv4_sgr(xdg.path(), &[]);
    assert_light_active(&s);
}

// Cell 6 of 8: CLI=none, config=light, profile.theme=none -> config wins.
#[test]
fn theme_precedence_config_wins_no_profile_theme() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(xdg.path(), "myprofile", "");
    write_user_config(xdg.path(), "[general]\ntheme = \"light\"\nprofile = \"myprofile\"\n");
    let s = probe_ipv4_sgr(xdg.path(), &[]);
    assert_light_active(&s);
}

// Cell 7 of 8: CLI=none, config=none, profile.theme=light -> profile wins.
#[test]
fn theme_precedence_profile_theme_wins_when_others_unset() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(xdg.path(), "myprofile", "theme = \"light\"\n");
    write_user_config(xdg.path(), "[general]\nprofile = \"myprofile\"\n");
    let s = probe_ipv4_sgr(xdg.path(), &[]);
    assert_light_active(&s);
}

// Cell 8 of 8: CLI=none, config=none, profile.theme=none -> bg-detect default
// = dark (TAYF_DISABLE_BG_DETECT=1 -> deterministic Dark per convention).
#[test]
fn theme_precedence_bg_detect_default_when_all_unset() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(xdg.path(), "myprofile", "");
    write_user_config(xdg.path(), "[general]\nprofile = \"myprofile\"\n");
    let s = probe_ipv4_sgr(xdg.path(), &[]);
    assert_dark_active(&s);
}

// ---------------------------------------------------------------------------
// Clean removal: a retired embedded name (`aws`/`docker`/...) with no matching
// disk file yields the standard NotFound (no soft-alias / migration shim).
// ---------------------------------------------------------------------------

#[test]
fn retired_profile_name_is_not_found() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    // No profiles/aws.toml on disk -> NotFound (the rules are now built-in).
    std::fs::create_dir_all(xdg.path().join("tayf/profiles")).expect("mkdir");

    let out = Command::new(tayf_bin())
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env("XDG_CONFIG_HOME", xdg.path())
        .env("TAYF_DISABLE_BG_DETECT", "1")
        .arg("--profile")
        .arg("aws")
        .arg("--no-hot-reload")
        .arg("--shell")
        .arg("/bin/sh")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn tayf");
    let code = out.status.code();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(code, Some(64), "expected EX_USAGE (64); got {code:?}; stderr: {stderr}");
    assert!(stderr.contains("profile 'aws' not found"), "byte-pinned NotFound wording: {stderr}");
    // No embedded fallback: searched must not mention a synthetic namespace.
    assert!(!stderr.contains("<embedded:"), "must not mention embedded namespace; got: {stderr}");
}

#[test]
fn profile_not_found_byte_pinned_diagnostic() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    std::fs::create_dir_all(xdg.path().join("tayf/profiles")).expect("mkdir");

    let out = Command::new(tayf_bin())
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env("XDG_CONFIG_HOME", xdg.path())
        .env("TAYF_DISABLE_BG_DETECT", "1")
        .arg("--profile")
        .arg("bogus")
        .arg("--no-hot-reload")
        .arg("--shell")
        .arg("/bin/sh")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn tayf");
    let code = out.status.code();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(code, Some(64), "expected EX_USAGE (64); got {code:?}; stderr: {stderr}");
    assert!(stderr.contains("profile 'bogus' not found"), "byte-pinned NotFound wording: {stderr}");
    assert!(stderr.contains("(searched:"), "must list searched paths; got: {stderr}");
    assert!(!stderr.contains("validation error"), "must not be ProfileValidation; got: {stderr}");
    assert!(!stderr.contains("parse error"), "must not be ParseError; got: {stderr}");
}

// ---------------------------------------------------------------------------
// Disk-profile styles-key validation routes through the profile path
// (RuleSource::DiskProfile -> Error::ProfileValidation with the profile name).
// ---------------------------------------------------------------------------

#[test]
fn profile_rule_styles_capture_group_key_unknown_byte_pinned_diagnostic() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(
        xdg.path(),
        "named-key-bogus",
        "[[rules]]\nname = \"ts_iso\"\npattern = '(?P<date>\\d{4}-\\d{2}-\\d{2})T(?P<time>\\d{2}:\\d{2}:\\d{2})'\nstyle = { fg = \"green\" }\nstyles = { bogus = { fg = \"red\" } }\n",
    );
    let out = Command::new(tayf_bin())
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env("XDG_CONFIG_HOME", xdg.path())
        .env("TAYF_DISABLE_BG_DETECT", "1")
        .arg("--profile")
        .arg("named-key-bogus")
        .arg("--no-hot-reload")
        .arg("--shell")
        .arg("/bin/sh")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn tayf");
    let code = out.status.code();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(code, Some(64), "expected EX_USAGE (64); got {code:?}; stderr: {stderr}");
    assert!(stderr.contains("profile 'named-key-bogus'"), "must quote profile name; got: {stderr}");
    assert!(stderr.contains("1 validation error:"), "singular form; got: {stderr}");
    assert!(
        stderr.contains(
            "  - rule 'ts_iso': styles.\"bogus\": rule's regex has no capture group named 'bogus' (available: date, time)"
        ),
        "byte-pinned StylesKey(NameUnknown) line; got: {stderr}"
    );
    assert!(!stderr.contains("theme '"), "must not be ThemeValidation; got: {stderr}");
}

// ---------------------------------------------------------------------------
// Hot-reload: a named profile stays active across a reload; editing
// config.toml's [general] (a reloaded input) is re-resolved. Under REPLACE
// semantics, editing config.toml's [[rules]] while a profile is active is a
// no-op — so we verify the profile rule survives a config rewrite that only
// touches [general].
// ---------------------------------------------------------------------------

#[test]
fn profile_rule_survives_hot_reload_of_config_general() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(
        xdg.path(),
        "myaws",
        "[[rules]]\nname = \"ticket\"\npattern = '\\bJIRA-[0-9]+\\b'\nstyle = { fg = \"cyan\" }\n",
    );
    let cfg_path = write_user_config(xdg.path(), "[general]\nprofile = \"myaws\"\n");

    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_tayf"));
    cmd.env_remove("HOME");
    cmd.env_remove("XDG_CONFIG_HOME");
    cmd.env("XDG_CONFIG_HOME", xdg.path());
    cmd.env("TAYF_DISABLE_BG_DETECT", "1");
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.arg("--shell");
    cmd.arg("/bin/sh");
    cmd.arg("--profile");
    cmd.arg("myaws");
    let mut child = pair.slave.spawn_command(cmd).expect("spawn tayf");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("clone reader");
    let mut writer = pair.master.take_writer().expect("take writer");

    thread::sleep(Duration::from_millis(400));

    // MARK_PRE: ticket should be cyan (profile active).
    writer.write_all(b"echo 'MARK_PRE JIRA-1234'\n").expect("write pre-mark");
    thread::sleep(Duration::from_millis(300));

    // Edit config.toml [general] (toggle the banner). Under REPLACE the
    // profile's rules remain the active set; the ticket rule must survive.
    std::fs::write(&cfg_path, "[general]\nprofile = \"myaws\"\nshow_reload_banner = true\n")
        .expect("rewrite config");
    thread::sleep(Duration::from_millis(1500));

    // MARK_POST: ticket still cyan (profile active throughout).
    writer.write_all(b"echo 'MARK_POST JIRA-1234'\nexit\n").expect("write post-mark + exit");
    drop(writer);

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let _reader_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        loop {
            match reader.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&tmp[..n]),
                Err(_) => break,
            }
        }
        let _ = tx.send(buf);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while std::time::Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let bytes = rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
    let s = String::from_utf8_lossy(&bytes);

    let pre_idx =
        s.find("MARK_PRE").unwrap_or_else(|| panic!("MARK_PRE absent from output: {s:?}"));
    let post_idx =
        s.find("MARK_POST").unwrap_or_else(|| panic!("MARK_POST absent from output: {s:?}"));
    assert!(pre_idx < post_idx, "MARK_PRE must precede MARK_POST: {s:?}");
    let pre_region = &s[pre_idx..post_idx];
    let post_region = &s[post_idx..];

    assert!(has_cyan(pre_region), "pre-edit: ticket should be cyan: {pre_region:?}");
    assert!(
        has_cyan(post_region),
        "post-edit: ticket should still be cyan (profile active across reload): {post_region:?}"
    );
}
