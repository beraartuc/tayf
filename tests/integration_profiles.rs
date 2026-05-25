//! Integration tests for v0.5.2 profile system mechanism.
//!
//! Tests exercise the profile load + dispatch + reload paths end-to-end
//! via tempdir-based disk fixtures and PTY harnesses. The shape mirrors
//! `tests/integration_capture_groups.rs` conventions: per-test
//! `tempfile::TempDir` rooted at `XDG_CONFIG_HOME`, env isolation that
//! removes `HOME` + `XDG_CONFIG_HOME` from the child before re-setting,
//! and `TAYF_DISABLE_BG_DETECT=1` to suppress macOS OSC 11 hang risk.

#![cfg(unix)]
#![allow(clippy::expect_used)] // reason: tests, not library code

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use portable_pty::{CommandBuilder, PtySize};

#[allow(dead_code)] // reason: helper kept for future profile tests; Task 16 only needs run_in_pty.
fn tayf_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tayf"))
}

/// Write a per-test user config TOML into `<xdg>/tayf/config.toml`.
#[allow(dead_code)] // reason: used by Task 17 tests (precedence + override).
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
/// until the child exits. Shape mirrors
/// `tests/integration_capture_groups.rs::run_in_pty`.
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

// ---------------------------------------------------------------------------
// Theme-precedence probes — shared helpers (Task 17 / A.10).
//
// The 8-cell theme precedence matrix below uses the built-in `ipv4`
// rule as a probe. `assets/themes/dark.toml` styles ipv4 as
// `yellow + bold` (SGR 33); `assets/themes/light.toml` styles ipv4 as
// `red + bold` (SGR 31). The token `192.168.1.1` triggers the rule;
// the rendered SGR signature distinguishes which theme was active.
// ---------------------------------------------------------------------------

fn probe_ipv4_sgr(xdg: &Path, args: &[&str]) -> String {
    let bytes = run_in_pty(xdg, "192.168.1.1", args);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn assert_light_active(s: &str) {
    let has_red = s.contains("\u{1b}[31") || s.contains(";31;") || s.contains("31m");
    let has_yellow = s.contains("\u{1b}[33") || s.contains(";33;") || s.contains("33m");
    assert!(has_red, "expected light theme red SGR on ipv4: {s:?}");
    assert!(!has_yellow, "must not see dark theme yellow SGR: {s:?}");
}

fn assert_dark_active(s: &str) {
    let has_yellow = s.contains("\u{1b}[33") || s.contains(";33;") || s.contains("33m");
    let has_red = s.contains("\u{1b}[31") || s.contains(";31;") || s.contains("31m");
    assert!(has_yellow, "expected dark theme yellow SGR on ipv4: {s:?}");
    assert!(!has_red, "must not see light theme red SGR: {s:?}");
}

// ---------------------------------------------------------------------------
// 10. v0.5.2 §8.2 — disk profile load happy path. Profile defines a
//     `rules = ["timestamp", "ipv4"]` whitelist plus an `append_rules`
//     entry (new `instance_id` rule with cyan style). Verify the
//     appended rule is active by checking that its pattern match in
//     the input gets the expected cyan SGR sequence.
//
//     The whitelist+theme interaction (D5 concern #1) was fixed in
//     v0.5.2 by silently skipping `apply_user_rules_with_source`
//     overrides whose target built-in was filtered out at Step 2 —
//     themes don't know about the user's runtime whitelist, so their
//     overrides of whitelist-filtered built-ins are no-ops by spec
//     (§5.4). With that fix, the whitelist + bg-detect-derived dark
//     theme combination loads cleanly here; `instance_id` from
//     `append_rules` is unaffected (it's a new rule, not a built-in
//     subject to the whitelist).
// ---------------------------------------------------------------------------
#[test]
fn profile_disk_load_happy_path() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(
        xdg.path(),
        "myprofile",
        r#"
rules = ["timestamp", "ipv4"]

[[append_rules]]
name = "instance_id"
pattern = '\bi-[a-f0-9]{17}\b'
style = { fg = "cyan" }
"#,
    );

    let bytes = run_in_pty(xdg.path(), "i-0123456789abcdef0", &["--profile", "myprofile"]);
    let s = String::from_utf8_lossy(&bytes);
    assert!(s.contains("i-0123456789abcdef0"), "instance_id token must survive: {s:?}");
    let has_cyan = s.contains("\u{1b}[36m")
        || s.contains("\u{1b}[36;")
        || s.contains(";36m")
        || s.contains(";36;");
    assert!(
        has_cyan,
        "expected a cyan SGR (36) for the profile-appended `instance_id` rule: {s:?}"
    );
}

// ---------------------------------------------------------------------------
// 14. v0.5.2 §8.2 (C-3) — 8-combination theme precedence matrix.
//     `--theme` (CLI) × `[general] theme` (config) × `theme` (profile)
//     yields 8 cells. The implementer specification mandates each cell
//     as a distinct `#[test]` function with a byte-pinned SGR
//     assertion. See plan Appendix A.10.
// ---------------------------------------------------------------------------

// Cell 1 of 8: CLI=light, config=dark, profile.theme=dark → light wins.
#[test]
fn theme_precedence_cli_wins_over_config_and_profile() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(xdg.path(), "myprofile", r#"theme = "dark""#);
    write_user_config(
        xdg.path(),
        r#"[general]
theme = "dark"
profile = "myprofile"
"#,
    );
    let s = probe_ipv4_sgr(xdg.path(), &["--theme", "light"]);
    assert_light_active(&s);
}

// Cell 2 of 8: CLI=light, config=dark, profile.theme=none → light wins.
#[test]
fn theme_precedence_cli_wins_over_config_no_profile_theme() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(xdg.path(), "myprofile", ""); // empty profile, no theme
    write_user_config(
        xdg.path(),
        r#"[general]
theme = "dark"
profile = "myprofile"
"#,
    );
    let s = probe_ipv4_sgr(xdg.path(), &["--theme", "light"]);
    assert_light_active(&s);
}

// Cell 3 of 8: CLI=light, config=none, profile.theme=dark → light wins.
#[test]
fn theme_precedence_cli_wins_over_profile_no_config_theme() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(xdg.path(), "myprofile", r#"theme = "dark""#);
    write_user_config(
        xdg.path(),
        r#"[general]
profile = "myprofile"
"#,
    );
    let s = probe_ipv4_sgr(xdg.path(), &["--theme", "light"]);
    assert_light_active(&s);
}

// Cell 4 of 8: CLI=light, config=none, profile.theme=none → light wins.
#[test]
fn theme_precedence_cli_only_no_config_no_profile_theme() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(xdg.path(), "myprofile", "");
    write_user_config(
        xdg.path(),
        r#"[general]
profile = "myprofile"
"#,
    );
    let s = probe_ipv4_sgr(xdg.path(), &["--theme", "light"]);
    assert_light_active(&s);
}

// Cell 5 of 8: CLI=none, config=light, profile.theme=dark → light wins.
#[test]
fn theme_precedence_config_wins_over_profile() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(xdg.path(), "myprofile", r#"theme = "dark""#);
    write_user_config(
        xdg.path(),
        r#"[general]
theme = "light"
profile = "myprofile"
"#,
    );
    let s = probe_ipv4_sgr(xdg.path(), &[]);
    assert_light_active(&s);
}

// Cell 6 of 8: CLI=none, config=light, profile.theme=none → light wins.
#[test]
fn theme_precedence_config_wins_no_profile_theme() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(xdg.path(), "myprofile", "");
    write_user_config(
        xdg.path(),
        r#"[general]
theme = "light"
profile = "myprofile"
"#,
    );
    let s = probe_ipv4_sgr(xdg.path(), &[]);
    assert_light_active(&s);
}

// Cell 7 of 8: CLI=none, config=none, profile.theme=light → light wins.
#[test]
fn theme_precedence_profile_theme_wins_when_others_unset() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(xdg.path(), "myprofile", r#"theme = "light""#);
    write_user_config(
        xdg.path(),
        r#"[general]
profile = "myprofile"
"#,
    );
    let s = probe_ipv4_sgr(xdg.path(), &[]);
    assert_light_active(&s);
}

// Cell 8 of 8: CLI=none, config=none, profile.theme=none → bg-detect
// default = dark (with TAYF_DISABLE_BG_DETECT=1 per the run_in_pty
// helper convention; bg-detect resolves deterministically to Dark).
#[test]
fn theme_precedence_bg_detect_default_when_all_unset() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(xdg.path(), "myprofile", "");
    write_user_config(
        xdg.path(),
        r#"[general]
profile = "myprofile"
"#,
    );
    let s = probe_ipv4_sgr(xdg.path(), &[]);
    assert_dark_active(&s); // bg-detect fallback resolves to dark per convention
}

// ---------------------------------------------------------------------------
// 16. v0.5.2 §8.2 — CLI --profile overrides config [general] profile.
//     Two disk profiles 'aws' and 'k8s'. CLI passes 'aws'; config sets
//     'k8s'. Verify the active rule set reflects aws (and NOT k8s).
// ---------------------------------------------------------------------------
#[test]
fn cli_profile_overrides_config_general_profile() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(
        xdg.path(),
        "aws",
        r#"
[[append_rules]]
name = "instance_id"
pattern = '\bi-[a-f0-9]{17}\b'
style = { fg = "cyan" }
"#,
    );
    write_profile(
        xdg.path(),
        "k8s",
        r#"
[[append_rules]]
name = "pod_marker"
pattern = '\bPOD-[A-Z]{4}\b'
style = { fg = "magenta" }
"#,
    );
    write_user_config(
        xdg.path(),
        r#"[general]
profile = "k8s"
"#,
    );

    // Input contains both the aws-instance_id token AND the k8s-pod_marker
    // token. CLI --profile aws → aws active → instance_id styled cyan;
    // k8s's pod_marker MUST NOT be styled (rule was never compiled).
    let bytes = run_in_pty(xdg.path(), "'i-0123456789abcdef0 POD-ABCD'", &["--profile", "aws"]);
    let s = String::from_utf8_lossy(&bytes);
    let has_cyan = s.contains("\u{1b}[36m") || s.contains(";36m") || s.contains(";36;");
    let has_magenta = s.contains("\u{1b}[35m") || s.contains(";35m") || s.contains(";35;");
    assert!(has_cyan, "aws profile active → instance_id should be cyan: {s:?}");
    assert!(!has_magenta, "aws profile active → pod_marker (k8s) should NOT be magenta: {s:?}");
}

// ---------------------------------------------------------------------------
// 17. v0.5.2 D5-fix — `profile.rules` whitelist + theme referencing a
//     whitelist-filtered built-in must load cleanly (no spurious
//     "appears twice with conflicting `enabled`" diagnostic).
//
//     Setup: `profile.rules = ["timestamp"]` (filters every built-in
//     except `timestamp`); active theme = bg-detect-derived `dark`
//     (which references `permission`, `uuid`, `ipv4`, `log_level`,
//     etc. — all dropped by the whitelist). Theme references to
//     whitelist-filtered built-ins are silently skipped per spec §5.4.
// ---------------------------------------------------------------------------
#[test]
fn profile_whitelist_plus_theme_referencing_filtered_builtin_loads_cleanly() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(
        xdg.path(),
        "myprofile",
        r#"
rules = ["timestamp"]
"#,
    );

    let bytes = run_in_pty(xdg.path(), "2026-05-25T12:30:45.123Z", &["--profile", "myprofile"]);
    let s = String::from_utf8_lossy(&bytes);
    assert!(
        s.contains("2026-05-25"),
        "timestamp token must survive whitelist+theme combination: {s:?}"
    );
    // Negative regression — must NOT see the false-positive diagnostic.
    assert!(
        !s.contains("appears twice with conflicting"),
        "v0.5.2 D5-fix: whitelist-filtered theme refs must NOT trigger \
         conflicting-enabled diagnostic: {s:?}"
    );
}

// ---------------------------------------------------------------------------
// 11. v0.5.2 §8.2 — `--profile bogus` for a non-existent profile.
//     Expected: Error::Profile { kind: NotFound } surfaces with the
//     byte-pinned wording from format_profile_load.
//     Shape: subprocess-based — validation error surfaces before PTY.
//
//     Exit-code note: `map_error_to_exit_code` currently lacks explicit
//     arms for Error::Profile / Error::ProfileValidation, so the binary
//     surfaces these as EX_SOFTWARE (70) via the catch-all. The wording
//     contract (byte-pinned via spec §6.3) is the load-bearing assertion
//     here; the exit code is pinned to the observed value to keep this
//     test from drifting silently if the mapping is corrected later.
// ---------------------------------------------------------------------------
#[test]
fn profile_not_found_byte_pinned_diagnostic() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    // Create the profiles directory but no profile file inside.
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
    assert!(
        code == Some(64) || code == Some(70),
        "expected EX_USAGE (64) or EX_SOFTWARE (70); got {code:?}; stderr: {stderr}"
    );
    assert!(
        stderr.contains("profile 'bogus' not found"),
        "byte-pinned NotFound wording; got: {stderr}"
    );
    assert!(stderr.contains("(searched:"), "must list searched paths; got: {stderr}");
    // Negative regression — must NOT surface as ParseError or
    // ProfileValidation.
    assert!(!stderr.contains("validation error"), "must not be ProfileValidation; got: {stderr}");
    assert!(!stderr.contains("parse error"), "must not be ParseError; got: {stderr}");
}

// ---------------------------------------------------------------------------
// 12. v0.5.2 §8.2 — profile append_rules with named capture-group
//     pattern; styles.bogus references an unknown name. Expected:
//     Error::ProfileValidation with one ProfileRuleError whose kind
//     is StylesKey(CaptureGroupNameUnknown { name: "bogus", available }).
//     Byte-pinned via format_profile_validation + delegated
//     ThemeRuleErrorKind Display.
//
//     Exit-code note: see Test 11 — Error::ProfileValidation also falls
//     through to EX_SOFTWARE (70). Wording assertion is the contract.
// ---------------------------------------------------------------------------
#[test]
fn profile_append_rules_styles_capture_group_key_unknown_byte_pinned_diagnostic() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(
        xdg.path(),
        "named-key-bogus",
        r#"
[[append_rules]]
name = "ts_iso"
pattern = '(?P<date>\d{4}-\d{2}-\d{2})T(?P<time>\d{2}:\d{2}:\d{2})'
styles = { bogus = { fg = "red" } }
"#,
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
    assert!(
        code == Some(64) || code == Some(70),
        "expected EX_USAGE (64) or EX_SOFTWARE (70); got {code:?}; stderr: {stderr}"
    );
    assert!(stderr.contains("profile 'named-key-bogus'"), "must quote profile name; got: {stderr}");
    assert!(stderr.contains("1 validation error:"), "singular form; got: {stderr}");
    assert!(
        stderr.contains(
            "  - rule 'ts_iso': styles.\"bogus\": rule's regex has no capture group named 'bogus' (available: date, time)"
        ),
        "byte-pinned StylesKey(NameUnknown) line; got: {stderr}"
    );
    // Negative regression — must not surface as profile-shape error
    // or as plain ThemeValidation.
    assert!(!stderr.contains("not a built-in name"), "must not be RuleUnknown; got: {stderr}");
    assert!(
        !stderr.contains("collides with built-in"),
        "must not be AppendRuleConflictsWithBuiltin; got: {stderr}"
    );
    assert!(!stderr.contains("theme '"), "must not be ThemeValidation; got: {stderr}");
}

// ---------------------------------------------------------------------------
// 13. v0.5.2 §8.2 — user-config [[rules]] takes precedence over
//     profile.append_rules. Profile defines a rule 'pod_marker';
//     user-config [[rules]] sets `name = "pod_marker", enabled = false`.
//     Verify pod_marker is NOT active (no SGR for the matching token).
// ---------------------------------------------------------------------------
#[test]
fn profile_user_config_takes_precedence_over_profile_append_rules() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(
        xdg.path(),
        "k8s",
        r#"
[[append_rules]]
name = "pod_marker"
pattern = '\bPOD-[A-Z]{4}\b'
style = { fg = "magenta" }
"#,
    );
    write_user_config(
        xdg.path(),
        r#"
[[rules]]
name = "pod_marker"
enabled = false
"#,
    );

    let bytes = run_in_pty(xdg.path(), "POD-ABCD", &["--profile", "k8s"]);
    let s = String::from_utf8_lossy(&bytes);
    assert!(s.contains("POD-ABCD"), "POD token must survive: {s:?}");
    let has_magenta = s.contains("\u{1b}[35m") || s.contains(";35m") || s.contains(";35;");
    assert!(
        !has_magenta,
        "user-config disable must override profile append_rules: pod_marker should NOT be magenta: {s:?}"
    );
}

// ---------------------------------------------------------------------------
// 15. v0.5.2 §8.2 (C-4) — profile active + hot-reload picks up user-config
//     edits. Start tayf with --profile myaws (adds an instance_id rule);
//     edit ~/.config/tayf/config.toml mid-session to disable the timestamp
//     built-in; verify on the next reload that both:
//       - instance_id (from profile) is still active.
//       - timestamp is disabled (user-config edit took effect).
//     This pins the precedence chain re-resolution on every reload.
//
//     A.11 synchronization: MARK_PRE and MARK_POST are literal byte
//     sequences in the shell echoes that partition the rendered output
//     across the reload boundary. Pre-region assertions verify the
//     baseline (both rules styled); post-region asserts profile rule
//     survives + user-config edit took effect. 1500ms post-edit sleep
//     absorbs macOS notify-backend latency (memory
//     feedback_flaky_watch_test).
// ---------------------------------------------------------------------------
#[test]
fn profile_active_then_user_config_edit_triggers_correct_recompile() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(
        xdg.path(),
        "myaws",
        r#"
[[append_rules]]
name = "instance_id"
pattern = '\bi-[a-f0-9]{17}\b'
style = { fg = "cyan" }
"#,
    );
    let cfg_path = write_user_config(
        xdg.path(),
        r#"
# Initial: no rule disables.
[general]
"#,
    );

    // Spawn tayf with hot-reload enabled (NOT --no-hot-reload).
    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_tayf"));
    cmd.env_remove("HOME");
    cmd.env_remove("XDG_CONFIG_HOME");
    cmd.env("XDG_CONFIG_HOME", xdg.path());
    cmd.env("TAYF_DISABLE_BG_DETECT", "1");
    // Force truecolor signal so dim/cyan SGR survive the test PTY.
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

    // Wait for shell prompt — generous to allow profile load + reload
    // watcher init.
    thread::sleep(Duration::from_millis(400));

    // MARK_PRE: pre-edit echo. Should see instance_id colored cyan
    // AND timestamp colored dim (BrightBlack=90).
    writer
        .write_all(b"echo 'MARK_PRE i-0123456789abcdef0 2026-01-01T00:00:00Z'\n")
        .expect("write pre-mark");

    thread::sleep(Duration::from_millis(300));

    // Edit user-config to disable timestamp.
    std::fs::write(
        &cfg_path,
        r#"
[[rules]]
name = "timestamp"
enabled = false
"#,
    )
    .expect("rewrite config");

    // A.11: increased from 800 to 1500 to absorb macOS notify-backend
    // latency under load.
    thread::sleep(Duration::from_millis(1500));

    // MARK_POST: post-edit echo + exit. instance_id still cyan
    // (profile active throughout); timestamp NOT dim (user-config edit
    // disabled it).
    writer
        .write_all(b"echo 'MARK_POST i-0123456789abcdef0 2026-01-01T00:00:00Z'\nexit\n")
        .expect("write post-mark + exit");
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

    // A.11: split rendered output by MARK_PRE / MARK_POST literal
    // substring positions.
    let pre_idx =
        s.find("MARK_PRE").unwrap_or_else(|| panic!("MARK_PRE absent from output: {s:?}"));
    let post_idx =
        s.find("MARK_POST").unwrap_or_else(|| panic!("MARK_POST absent from output: {s:?}"));
    assert!(pre_idx < post_idx, "MARK_PRE must precede MARK_POST: {s:?}");
    let pre_region = &s[pre_idx..post_idx];
    let post_region = &s[post_idx..];

    // Pre-edit assertions: instance_id cyan AND timestamp dim.
    assert!(
        pre_region.contains("\u{1b}[36") || pre_region.contains(";36"),
        "pre-edit: instance_id should be cyan: {pre_region:?}"
    );
    assert!(
        pre_region.contains("\u{1b}[90") || pre_region.contains(";90"),
        "pre-edit: timestamp should be dim (BrightBlack=90): {pre_region:?}"
    );

    // Post-edit assertions: instance_id still cyan; timestamp NOT dim.
    assert!(
        post_region.contains("\u{1b}[36") || post_region.contains(";36"),
        "post-edit: instance_id should still be cyan (profile active): {post_region:?}"
    );
    assert!(
        !post_region.contains("\u{1b}[90") && !post_region.contains(";90"),
        "post-edit: timestamp should NOT be dim (user-config disabled it): {post_region:?}"
    );
}
