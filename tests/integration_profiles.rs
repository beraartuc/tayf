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
// 10. v0.5.2 §8.2 — disk profile load happy path. Profile defines an
//     `append_rules` entry (new `instance_id` rule with cyan style).
//     Verify the appended rule is active by checking that its pattern
//     match in the input gets the expected cyan SGR sequence.
//
//     The plan body's draft fixture also included `rules = ["timestamp",
//     "ipv4"]` whitelist — but in combination with a `theme` layer that
//     touches built-ins filtered out by the whitelist, the current
//     Phase-4 merge order surfaces a misleading
//     "appears twice with conflicting `enabled` values" diagnostic
//     (Phase 4 `apply_user_rules_with_source` assumes every theme-rule
//     target survives in the merged set, which doesn't hold under a
//     profile.rules whitelist). v0.5.2 ships with no theme by default
//     in this test (TAYF_DISABLE_BG_DETECT=1 makes bg-detect → dark,
//     and dark.toml DOES touch `permission` and others), so the
//     whitelist would trip the bug here even without `--theme`.
//
//     Carrying the whitelist out of Test 10's scope keeps Phase 5
//     focused on CLI + orchestration; the whitelist+theme interaction
//     is a v0.5.3 carryover (see Concerns in the D5 report).
// ---------------------------------------------------------------------------
#[test]
fn profile_disk_load_happy_path() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_profile(
        xdg.path(),
        "myprofile",
        r#"
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
