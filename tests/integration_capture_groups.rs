//! Integration tests for v0.3.5 capture-group styling.
//!
//! These tests exercise two end-to-end shapes:
//!
//! 1. PTY-based output assertions (tests 1–5, 8) — spawn `tayf` through a
//!    real `portable-pty` allocation, feed `echo <token>` into the child
//!    shell, then assert on the SGR sequences tayf injects around the
//!    capture groups. The defensive `--shell /bin/sh` + the
//!    `TAYF_DISABLE_BG_DETECT=1` env are the v0.3.4 known gotcha pattern
//!    that avoids the macOS portable-pty OSC 11 hang risk.
//!
//! 2. Subprocess error assertions (tests 6, 7, 9) — config / theme
//!    validation failures surface BEFORE the PTY loop is entered, so a
//!    plain `Command::new` + `output()` is enough. These pin the exit
//!    code (64 = `EX_USAGE`) and the user-facing diagnostic strings.
//!
//! Test isolation (v0.3.4 I-2 standard): every spawn explicitly removes
//! `HOME` and `XDG_CONFIG_HOME` from the child env BEFORE re-setting
//! `XDG_CONFIG_HOME` to a per-test `tempfile::TempDir`. Without the
//! removal the developer's real `~/.config/tayf/` could leak into a
//! test and mask or trigger validation errors.

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

/// Write a per-test disk theme TOML into `<xdg>/tayf/themes/<name>.toml`.
fn write_disk_theme(xdg: &Path, name: &str, body: &str) -> PathBuf {
    let themes = xdg.join("tayf").join("themes");
    std::fs::create_dir_all(&themes).expect("create themes dir");
    let p = themes.join(format!("{name}.toml"));
    std::fs::write(&p, body).expect("write theme");
    p
}

/// Spawn tayf under a real PTY with the given extra args. Feeds
/// `echo <token>\nexit\n` to the child shell and drains the master until
/// the child exits or 5 s elapses. The child shell is forced to
/// `/bin/sh`; `TAYF_DISABLE_BG_DETECT=1` suppresses macOS OSC 11 query
/// hangs. The Rev2 I-2 env isolation (remove `HOME` + `XDG_CONFIG_HOME`,
/// then re-set `XDG_CONFIG_HOME` to `xdg`) is applied here.
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

    // Give tayf time to install its signal handler, spawn the shell, and
    // be ready to receive input.
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

/// Count the non-reset SGR introducers (`\x1b[` followed by a non-`0`
/// digit, allowing a leading `0;`/`0m`-style reset to be filtered out).
/// The simpler invariant we want is: how many *colorizing* SGRs landed on
/// this line? Resets and bare `\x1b[0m` are excluded from the count.
fn count_non_reset_sgrs(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut count = 0usize;
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == 0x1b && bytes[i + 1] == b'[' {
            let mut j = i + 2;
            // Skip the parameter bytes (digits + ';') until the final `m`
            // or any non-SGR final byte (which we just count once and
            // move on without inspecting further).
            while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b';') {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'm' {
                // Look at the SGR parameters to decide if this is a pure
                // reset (`\x1b[m` or `\x1b[0m`). Anything else counts.
                let params = &bytes[i + 2..j];
                let is_reset = params.is_empty() || params == b"0";
                if !is_reset {
                    count += 1;
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    count
}

// ---------------------------------------------------------------------------
// 1. ISO timestamp → ≥ 5 non-reset SGRs (date / T / time / .ms / tz).
// ---------------------------------------------------------------------------
#[test]
fn iso_timestamp_match_renders_five_distinct_sgrs() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    let bytes = run_in_pty(xdg.path(), "2026-05-24T10:30:45.123Z", &[]);
    let s = String::from_utf8_lossy(&bytes);
    assert!(s.contains("2026-05-24"), "echoed token must survive: {s:?}");
    let n = count_non_reset_sgrs(&s);
    assert!(n >= 5, "expected at least 5 non-reset SGRs for ISO timestamp, got {n} in: {s:?}");
}

// ---------------------------------------------------------------------------
// 2. Syslog timestamp → token survives in output. The syslog branch of
//    the timestamp alternation has no capture groups; whatever default
//    style is applied still must not destroy the substring.
// ---------------------------------------------------------------------------
#[test]
fn syslog_timestamp_substring_survives_colorization() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    let bytes = run_in_pty(xdg.path(), "May 24 10:30:45 host msg", &[]);
    let s = String::from_utf8_lossy(&bytes);
    assert!(s.contains("May 24 10:30:45"), "syslog substring must survive colorization: {s:?}");
}

// ---------------------------------------------------------------------------
// 3. HTTP URL → ≥ 3 non-reset SGRs (scheme / "://" / host+path).
// ---------------------------------------------------------------------------
#[test]
fn http_url_match_renders_three_sgrs() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    let bytes = run_in_pty(xdg.path(), "see https://example.com/p", &[]);
    let s = String::from_utf8_lossy(&bytes);
    // The URL is split across capture-group SGR runs (scheme / "://" /
    // host+path), so the full literal string is not contiguous. Assert
    // each capture-group substring survives separately instead.
    assert!(s.contains("https"), "URL scheme must survive: {s:?}");
    assert!(s.contains("://"), "URL separator must survive: {s:?}");
    assert!(s.contains("example.com/p"), "URL host+path must survive: {s:?}");
    let n = count_non_reset_sgrs(&s);
    assert!(n >= 3, "expected at least 3 non-reset SGRs for URL, got {n} in: {s:?}");
}

// ---------------------------------------------------------------------------
// 4. Permission → ≥ 4 non-reset SGRs (type / u-rwx / g-rwx / o-rwx).
//    The `echo` argument is wrapped in single quotes via shell so the
//    permission token reaches stdout intact (sh would otherwise treat
//    `drwxr-xr-x` as one bare word; quoting is purely defensive).
// ---------------------------------------------------------------------------
#[test]
fn permission_match_renders_four_sgrs() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    let bytes = run_in_pty(xdg.path(), "'drwxr-xr-x file.txt'", &[]);
    let s = String::from_utf8_lossy(&bytes);
    // Permission is split across 4 capture-group SGR runs (type / u-rwx /
    // g-rwx / o-rwx); the literal `drwxr-xr-x` is NOT contiguous in the
    // colorized output. Assert each component substring separately.
    assert!(s.contains("rwx"), "user-rwx triplet must survive: {s:?}");
    assert!(s.contains("r-x"), "group/other r-x triplet must survive: {s:?}");
    let n = count_non_reset_sgrs(&s);
    assert!(n >= 4, "expected at least 4 non-reset SGRs for permission, got {n} in: {s:?}");
}

// ---------------------------------------------------------------------------
// 5. User config `styles."1" = { fg = "red" }` REPLACES the built-in
//    `group_styles` for `timestamp`. Capture group 1 of the ISO branch
//    is the date, so we expect a red SGR (`31m` or `31;`) somewhere in
//    the colorized output. Other groups fall back to no per-group
//    overlay (None) — but the rule's top-level `style` (BrightBlack)
//    still applies to the whole match.
// ---------------------------------------------------------------------------
#[test]
fn user_config_styles_overrides_builtin_group_styles_replace_semantics() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    let cfg = write_user_config(
        xdg.path(),
        r#"
[[rules]]
name = "timestamp"
styles = { "1" = { fg = "red" } }
"#,
    );

    let bytes = run_in_pty(
        xdg.path(),
        "2026-05-24T10:30:45.123Z",
        &["--config", cfg.to_str().expect("utf-8 path")],
    );
    let s = String::from_utf8_lossy(&bytes);
    assert!(s.contains("2026-05-24"), "token must survive: {s:?}");
    let has_red = s.contains("\u{1b}[31m")
        || s.contains("\u{1b}[31;")
        || s.contains(";31m")
        || s.contains(";31;");
    assert!(has_red, "expected a red SGR (31) somewhere in: {s:?}");
}

// ---------------------------------------------------------------------------
// 6. User-config `styles."99"` on a rule whose regex has < 99 capture
//    groups → tayf exits 64 with a `CaptureGroupIndexOutOfRange`
//    diagnostic. ipv4 has no capture groups (`captures_len = 1`), so
//    the diagnostic specializes to "rule's regex has no capture groups;
//    styles cannot be set" (v0.3.7+: routed through the shared
//    `ThemeRuleErrorKind::Display` impl).
// ---------------------------------------------------------------------------
#[test]
fn out_of_range_styles_in_user_config_exits_64_with_diagnostic() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    let cfg = write_user_config(
        xdg.path(),
        r#"
[[rules]]
name = "ipv4"
styles = { "99" = { fg = "red" } }
"#,
    );

    let out = Command::new(tayf_bin())
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env("XDG_CONFIG_HOME", xdg.path())
        .env("TAYF_DISABLE_BG_DETECT", "1")
        .arg("--config")
        .arg(&cfg)
        .arg("--no-hot-reload")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn tayf");
    assert_eq!(
        out.status.code(),
        Some(64),
        "expected EX_USAGE for out-of-range user styles; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("rule 'ipv4'"), "diagnostic must name the rule; got: {stderr}");
    assert!(stderr.contains("no capture groups"), "got: {stderr}");
    assert!(stderr.contains("styles cannot be set"), "got: {stderr}");
    assert!(!stderr.contains("valid: 1..=0"), "regression guard: {stderr}");
}

// ---------------------------------------------------------------------------
// 7. Rev2 I-8 — theme `styles."5"` becomes out-of-range AFTER a user
//    config pattern override drops `timestamp`'s captures_len from 6 to
//    3. The effective merged captures_len is what validation must see.
//    Expected: exit 64 + `valid: 1..=2` (the override has 2 capture
//    groups, so the valid integer range is 1..=2).
// ---------------------------------------------------------------------------
#[test]
fn theme_styles_5_out_of_range_after_user_pattern_override_2_captures() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_disk_theme(
        xdg.path(),
        "mine",
        r#"
[[rules]]
name = "timestamp"
styles = { "5" = { fg = "blue" } }
"#,
    );
    let cfg = write_user_config(
        xdg.path(),
        r#"
[[rules]]
name = "timestamp"
pattern = '(\d{4})-(\d{2})'
"#,
    );

    let out = Command::new(tayf_bin())
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env("XDG_CONFIG_HOME", xdg.path())
        .env("TAYF_DISABLE_BG_DETECT", "1")
        .arg("--config")
        .arg(&cfg)
        .arg("--theme")
        .arg("mine")
        .arg("--no-hot-reload")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn tayf");
    assert_eq!(
        out.status.code(),
        Some(64),
        "expected EX_USAGE; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("valid: 1..=2"),
        "diagnostic must reflect effective merged captures_len (=3, so range 1..=2); \
         got: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// 8. `styles = {}` is accepted as a no-op. tayf starts normally; output
//    of an ipv4 echo still survives (and is colorized with the built-in
//    default style; we just assert no validation error and the token
//    reaches stdout).
// ---------------------------------------------------------------------------
#[test]
fn empty_styles_map_accepted_as_no_op() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    let cfg = write_user_config(
        xdg.path(),
        r#"
[[rules]]
name = "ipv4"
styles = {}
"#,
    );

    let bytes =
        run_in_pty(xdg.path(), "192.168.1.1", &["--config", cfg.to_str().expect("utf-8 path")]);
    let s = String::from_utf8_lossy(&bytes);
    assert!(s.contains("192.168.1.1"), "ipv4 token must survive empty styles map: {s:?}");
    assert!(
        !s.contains("config error"),
        "empty styles map must not surface as a config error: {s:?}"
    );
    assert!(
        !s.contains("validation errors"),
        "empty styles map must not surface as theme validation: {s:?}"
    );
}

// ---------------------------------------------------------------------------
// 9. Rev2 I-6 — malformed key `"01"` (leading zero) is rejected by the
//    config grammar gate. Exit 64 + the `CaptureGroupKeyMalformed`
//    diagnostic surfaces verbatim.
// ---------------------------------------------------------------------------
#[test]
fn malformed_styles_key_in_user_config_emits_capture_group_key_malformed() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    let cfg = write_user_config(
        xdg.path(),
        r#"
[[rules]]
name = "ipv4"
styles = { "01" = { fg = "red" } }
"#,
    );

    let out = Command::new(tayf_bin())
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env("XDG_CONFIG_HOME", xdg.path())
        .env("TAYF_DISABLE_BG_DETECT", "1")
        .arg("--config")
        .arg(&cfg)
        .arg("--no-hot-reload")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn tayf");
    assert_eq!(
        out.status.code(),
        Some(64),
        "expected EX_USAGE for malformed styles key; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("capture-group key must be a positive decimal"),
        "diagnostic must point at the grammar violation; got: {stderr}"
    );
    assert!(stderr.contains("\"01\""), "diagnostic must echo the offending key; got: {stderr}");
}

// ---------------------------------------------------------------------------
// 10. Named-key user-config: `styles = { scheme = { fg = "cyan" } }` on the
//     `url` rule resolves via regex.capture_names() to group 1 (first branch
//     of `(?P<scheme>https?|ssh|ftp)(?P<sep>://)(?P<body>...)`). Output
//     should carry a cyan SGR (36 or 96, depending on the codebase's Cyan
//     vs BrightCyan resolution).
// ---------------------------------------------------------------------------
#[test]
fn theme_styles_named_scheme_renders_url_with_cyan_scheme() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    let cfg = write_user_config(
        xdg.path(),
        r#"
[[rules]]
name = "url"
styles = { scheme = { fg = "cyan" } }
"#,
    );

    let bytes = run_in_pty(
        xdg.path(),
        "'Visit https://example.com today'",
        &["--config", cfg.to_str().expect("utf-8 path")],
    );
    let s = String::from_utf8_lossy(&bytes);
    assert!(s.contains("https"), "scheme bytes must survive: {s:?}");
    let has_cyan = s.contains("\u{1b}[36m")
        || s.contains("\u{1b}[36;")
        || s.contains(";36m")
        || s.contains(";36;");
    assert!(has_cyan, "expected a cyan SGR (36) for the `scheme` group in: {s:?}");
}

// ---------------------------------------------------------------------------
// 11. Named-key user-config: `styles = { perm_owner = { fg = "red" } }` on
//     the `permission` rule resolves via regex.capture_names() to group 2
//     (`(?P<perm_type>...)(?P<perm_owner>[rwxsStT-]{3})...`). The user-rwx
//     triplet should carry a red SGR (31 or 91).
// ---------------------------------------------------------------------------
#[test]
fn theme_styles_named_perm_owner_renders_permission_with_red_owner() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    let cfg = write_user_config(
        xdg.path(),
        r#"
[[rules]]
name = "permission"
styles = { perm_owner = { fg = "red" } }
"#,
    );

    let bytes = run_in_pty(
        xdg.path(),
        "'drwxr-xr-x file.txt'",
        &["--config", cfg.to_str().expect("utf-8 path")],
    );
    let s = String::from_utf8_lossy(&bytes);
    assert!(s.contains("rwx"), "owner-rwx triplet must survive: {s:?}");
    let has_red = s.contains("\u{1b}[31m")
        || s.contains("\u{1b}[31;")
        || s.contains(";31m")
        || s.contains(";31;");
    assert!(has_red, "expected a red SGR (31) for the `perm_owner` group in: {s:?}");
}

// ---------------------------------------------------------------------------
// 12. Named-key user-config: `styles = { date = { fg = "yellow" } }` on the
//     `timestamp` rule resolves via regex.capture_names() to group 1 of the
//     ISO branch (`(?P<date>\d{4}-\d{2}-\d{2})...`). The YYYY-MM-DD prefix
//     should carry a yellow SGR (33 or 93).
// ---------------------------------------------------------------------------
#[test]
fn theme_styles_named_date_renders_timestamp_with_yellow_date() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    let cfg = write_user_config(
        xdg.path(),
        r#"
[[rules]]
name = "timestamp"
styles = { date = { fg = "yellow" } }
"#,
    );

    let bytes = run_in_pty(
        xdg.path(),
        "2026-05-25T12:30:45.123Z",
        &["--config", cfg.to_str().expect("utf-8 path")],
    );
    let s = String::from_utf8_lossy(&bytes);
    assert!(s.contains("2026-05-25"), "date prefix must survive: {s:?}");
    let has_yellow = s.contains("\u{1b}[33m")
        || s.contains("\u{1b}[33;")
        || s.contains(";33m")
        || s.contains(";33;");
    assert!(has_yellow, "expected a yellow SGR (33) for the `date` group in: {s:?}");
}

// ---------------------------------------------------------------------------
// 13. v0.5.1 §I-1 — theme TOML named-key happy path. `styles = { date = ... }`
//     on the `timestamp` rule, routed through a disk theme + `--theme` CLI
//     flag, must resolve via `regex.capture_names()` (ISO branch slot 1) and
//     style the YYYY-MM-DD prefix. Mirrors the user-config test 12 (line
//     494) but exercises the previously dead RuleSource::Theme dispatch arm.
// ---------------------------------------------------------------------------
#[test]
fn theme_toml_named_capture_group_renders_timestamp_with_yellow_date() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_disk_theme(
        xdg.path(),
        "named-key-smoke",
        r#"
[[rules]]
name = "timestamp"
styles = { date = { fg = "yellow" } }
"#,
    );

    let bytes = run_in_pty(xdg.path(), "2026-05-25T12:30:45.123Z", &["--theme", "named-key-smoke"]);
    let s = String::from_utf8_lossy(&bytes);
    assert!(s.contains("2026-05-25"), "date prefix must survive: {s:?}");
    let has_yellow = s.contains("\u{1b}[33m")
        || s.contains("\u{1b}[33;")
        || s.contains(";33m")
        || s.contains(";33;");
    assert!(has_yellow, "expected a yellow SGR (33) for the `date` group via theme TOML: {s:?}");
}

// ---------------------------------------------------------------------------
// 14. v0.5.1 §I-2 — theme TOML unknown capture-group name. `styles.bogus`
//     on the `timestamp` rule must surface as Error::ThemeValidation with
//     a single CaptureGroupNameUnknown entry. Byte-pinned Display:
//     `theme '<name>' (loaded from <path>) has 1 validation error:`
//     followed by
//     `  - rule 'timestamp': styles."bogus": rule's regex has no capture
//      group named 'bogus' (available: date, sep, time, ms, tz)`.
//     Exercises the previously dead dispatch arm at src/rules.rs:1067-1076.
// ---------------------------------------------------------------------------
#[test]
fn theme_toml_unknown_capture_group_name_byte_pinned_diagnostic() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_disk_theme(
        xdg.path(),
        "bogus-name",
        r#"
[[rules]]
name = "timestamp"
styles = { bogus = { fg = "red" } }
"#,
    );
    let out = Command::new(tayf_bin())
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env("XDG_CONFIG_HOME", xdg.path())
        .env("TAYF_DISABLE_BG_DETECT", "1")
        .arg("--theme")
        .arg("bogus-name")
        .arg("--no-hot-reload")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn tayf");
    assert_eq!(
        out.status.code(),
        Some(64),
        "expected EX_USAGE; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("theme 'bogus-name'"), "must quote theme name; got: {stderr}");
    assert!(stderr.contains("1 validation error:"), "must indicate single error; got: {stderr}");
    assert!(
        stderr.contains(
            "  - rule 'timestamp': styles.\"bogus\": rule's regex has no capture \
             group named 'bogus' (available: date, sep, time, ms, tz)"
        ),
        "expected byte-pinned NameUnknown line; got: {stderr}"
    );
    // Negative regression — must NOT surface as KeyMalformed or DuplicateTarget.
    assert!(
        !stderr.contains("capture-group key must be a positive decimal"),
        "must not surface as KeyMalformed: {stderr}"
    );
    assert!(
        !stderr.contains("target the same capture group"),
        "must not surface as DuplicateTarget: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// 15. v0.5.1 §I-2 — theme TOML duplicate-target diagnostic. `styles."1"`
//     (positional) AND `styles.date` (named, ISO branch slot 1) both
//     target the same capture group. Must surface as
//     Error::ThemeValidation with a single CaptureGroupDuplicateTarget
//     entry. Byte-pinned Display:
//     `  - rule 'timestamp': styles."1" and styles.date target the same
//      capture group (index 1); set exactly one`.
//     BTreeMap iteration order is lexicographic; "1" precedes "date",
//     so `positional="1"`, `named="date"` in the diagnostic.
//     Exercises the previously dead dispatch arm at src/rules.rs:1116-1124.
// ---------------------------------------------------------------------------
#[test]
fn theme_toml_duplicate_target_positional_and_named_byte_pinned_diagnostic() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_disk_theme(
        xdg.path(),
        "dup-target",
        r#"
[[rules]]
name = "timestamp"
styles = { "1" = { fg = "red" }, date = { fg = "blue" } }
"#,
    );
    let out = Command::new(tayf_bin())
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env("XDG_CONFIG_HOME", xdg.path())
        .env("TAYF_DISABLE_BG_DETECT", "1")
        .arg("--theme")
        .arg("dup-target")
        .arg("--no-hot-reload")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn tayf");
    assert_eq!(
        out.status.code(),
        Some(64),
        "expected EX_USAGE; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("theme 'dup-target'"), "must quote theme name; got: {stderr}");
    assert!(stderr.contains("1 validation error:"), "must indicate single error; got: {stderr}");
    assert!(
        stderr.contains(
            "  - rule 'timestamp': styles.\"1\" and styles.date target the same \
             capture group (index 1); set exactly one"
        ),
        "expected byte-pinned DuplicateTarget line; got: {stderr}"
    );
    // Negative regression — must NOT surface as NameUnknown or KeyMalformed.
    assert!(
        !stderr.contains("no capture group named"),
        "must not surface as NameUnknown: {stderr}"
    );
    assert!(
        !stderr.contains("capture-group key must be a positive decimal"),
        "must not surface as KeyMalformed: {stderr}"
    );
}
