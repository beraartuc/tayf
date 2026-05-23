//! End-to-end integration tests for v0.2.1 hot reload.
//!
//! Test architecture: each test writes ALL its shell commands up-front
//! (interleaved with config-file modifications and signal delivery),
//! then drains tayf's stdout to EOF. Section boundaries between the
//! reload epochs are stamped with a unique `echo TAYFMARK_<id>` line
//! that tayf's built-in rule set never colorizes; the test splits the
//! captured stream on those markers and asserts each section
//! independently.
//!
//! This shape avoids the trap of reading tayf's stdout mid-session:
//! a `read()` on the PTY master blocks until more bytes arrive, with
//! no inherent timeout, so any "drain for 300ms" pattern that races
//! with the shell prompt loop deadlocks.

#![allow(clippy::expect_used)]

mod common;

use std::io::{Read, Write};
use std::thread;
use std::time::Duration;

use portable_pty::PtySize;

/// Env override that forces tayf to detect truecolor support even
/// inside the test PTY (where COLORTERM/TERM from the test runner
/// may be missing or boring). Applied per-child, not process-global.
const TRUECOLOR_ENV: &[(&str, &str)] = &[("TERM", "xterm-256color"), ("COLORTERM", "truecolor")];

fn yellow_sgr_present(haystack: &[u8]) -> bool {
    haystack.windows(5).any(|w| w == b"\x1b[33m") || haystack.windows(7).any(|w| w == b"\x1b[1;33m")
}

fn red_sgr_present(haystack: &[u8]) -> bool {
    haystack.windows(5).any(|w| w == b"\x1b[31m") || haystack.windows(7).any(|w| w == b"\x1b[1;31m")
}

/// Read until EOF or hard timeout. EOF happens when tayf's child shell
/// has exited (closing the slave PTY, which surfaces as Ok(0) on the
/// master). The hard timeout is a safety net for a genuinely stuck
/// run; production behavior shuts down well under one second.
fn drain_until_eof(reader: &mut Box<dyn Read + Send>, hard_timeout: Duration) -> Vec<u8> {
    let mut buf = [0u8; 4096];
    let mut out = Vec::new();
    let start = std::time::Instant::now();
    while start.elapsed() < hard_timeout {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }
    out
}

fn require_sh() -> bool {
    if std::path::Path::new("/bin/sh").exists() {
        true
    } else {
        eprintln!("skipping: /bin/sh not present");
        false
    }
}

#[test]
fn file_edit_swaps_log_level_color() {
    if !require_sh() {
        return;
    }
    let tayf = env!("CARGO_BIN_EXE_tayf");

    let dir = tempfile::tempdir().expect("tempdir");
    let cfg_path = dir.path().join("config.toml");
    std::fs::write(
        &cfg_path,
        r#"
[[rules]]
name = "log_level"
style = { fg = "yellow", bold = true }
"#,
    )
    .expect("write initial");

    let cfg_str = cfg_path.display().to_string();
    let (master, mut child) = common::spawn_for_interaction_with_env(
        tayf,
        &["--config", &cfg_str, "--shell", "/bin/sh"],
        TRUECOLOR_ENV,
        PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
    );

    let mut writer = master.take_writer().expect("take writer");
    let mut reader = master.try_clone_reader().expect("clone reader");

    // Settle: tayf spawns sh, signal/watch/reload threads come up.
    thread::sleep(Duration::from_millis(300));

    writer.write_all(b"echo ERROR before-reload\n").expect("write before");

    // Modify config — yellow → red. The notify watcher fires; the
    // 200ms debounce window has to close + the orchestrator has to
    // run reload_once + arc-swap before the next echo line is fed.
    std::fs::write(
        &cfg_path,
        r#"
[[rules]]
name = "log_level"
style = { fg = "red", bold = true }
"#,
    )
    .expect("write red");

    thread::sleep(Duration::from_millis(800));

    writer.write_all(b"echo TAYFMARK_RELOAD\n").expect("write marker");
    writer.write_all(b"echo ERROR after-reload\n").expect("write after");
    writer.write_all(b"exit\n").expect("write exit");
    drop(writer);

    let out = drain_until_eof(&mut reader, Duration::from_secs(10));
    let _ = child.wait();

    let marker = b"TAYFMARK_RELOAD";
    let pos = out
        .windows(marker.len())
        .position(|w| w == marker)
        .expect("marker missing — tayf never reloaded or shell never ran the marker");

    let (before, after) = out.split_at(pos);

    assert!(
        yellow_sgr_present(before),
        "expected yellow SGR before reload; got {:?}",
        String::from_utf8_lossy(before)
    );
    assert!(
        red_sgr_present(after),
        "expected red SGR after reload; got {:?}",
        String::from_utf8_lossy(after)
    );
}

#[test]
fn parse_failure_preserves_previous_rules() {
    if !require_sh() {
        return;
    }
    let tayf = env!("CARGO_BIN_EXE_tayf");
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg_path = dir.path().join("config.toml");

    std::fs::write(
        &cfg_path,
        r#"
[[rules]]
name = "log_level"
style = { fg = "yellow", bold = true }
"#,
    )
    .expect("write initial");

    let cfg_str = cfg_path.display().to_string();
    let (master, mut child) = common::spawn_for_interaction_with_env(
        tayf,
        &["--config", &cfg_str, "--shell", "/bin/sh"],
        TRUECOLOR_ENV,
        PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
    );

    let mut writer = master.take_writer().expect("take writer");
    let mut reader = master.try_clone_reader().expect("clone reader");

    thread::sleep(Duration::from_millis(300));

    writer.write_all(b"echo ERROR first\n").expect("write first");

    // Write broken TOML — orchestrator should log warn + KEEP yellow.
    std::fs::write(&cfg_path, "this is = = not valid toml\n").expect("write broken");
    thread::sleep(Duration::from_millis(800));

    writer.write_all(b"echo TAYFMARK_BROKEN\n").expect("write marker");
    writer.write_all(b"echo ERROR second\n").expect("write second");
    writer.write_all(b"exit\n").expect("write exit");
    drop(writer);

    let out = drain_until_eof(&mut reader, Duration::from_secs(10));
    let _ = child.wait();

    let marker = b"TAYFMARK_BROKEN";
    let pos = out.windows(marker.len()).position(|w| w == marker).expect("marker missing");
    let (before, after) = out.split_at(pos);

    assert!(yellow_sgr_present(before), "before: {:?}", String::from_utf8_lossy(before));
    assert!(
        yellow_sgr_present(after),
        "after parse failure: rule must still be yellow; got {:?}",
        String::from_utf8_lossy(after)
    );
    assert!(
        !red_sgr_present(after),
        "no new rule should have been installed; got {:?}",
        String::from_utf8_lossy(after)
    );
}

#[test]
#[cfg(unix)]
fn sighup_forces_reload() {
    if !require_sh() {
        return;
    }
    let tayf = env!("CARGO_BIN_EXE_tayf");
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg_path = dir.path().join("config.toml");

    std::fs::write(
        &cfg_path,
        r#"
[[rules]]
name = "log_level"
style = { fg = "yellow", bold = true }
"#,
    )
    .expect("write initial");

    let cfg_str = cfg_path.display().to_string();
    let (master, mut child) = common::spawn_for_interaction_with_env(
        tayf,
        &["--config", &cfg_str, "--shell", "/bin/sh"],
        TRUECOLOR_ENV,
        PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
    );

    let mut writer = master.take_writer().expect("take writer");
    let mut reader = master.try_clone_reader().expect("clone reader");

    thread::sleep(Duration::from_millis(300));

    // v0.3.3 BEHAVIOR CHANGE: tayf now forwards SIGHUP to the child
    // process group unconditionally (fixing a v0.2.1 silent-drop
    // regression — see CHANGELOG v0.3.3). /bin/sh's default HUP
    // disposition is termination, which would kill the shell before
    // we could exercise the reload-still-happened assertion. Install
    // a trap that ignores HUP so the shell stays alive; the reload
    // pipeline still fires independently via the mpsc channel.
    writer.write_all(b"trap '' HUP\n").expect("write hup trap");
    writer.write_all(b"echo ERROR before-sighup\n").expect("write before");

    std::fs::write(
        &cfg_path,
        r#"
[[rules]]
name = "log_level"
style = { fg = "red", bold = true }
"#,
    )
    .expect("write red");

    // Synchronization barrier: the `trap '' HUP` bytes above were just
    // handed to the PTY master fd, but the child shell still has to
    // read them, lex the line, and actually install the trap. Without
    // a wait here, tayf's signal thread can deliver SIGHUP to the
    // process group (via forward_to_pgid) BEFORE sh has processed the
    // trap line — and sh's default HUP disposition is termination, so
    // the shell dies with "Input/output error" and the rest of the
    // test fails. 200ms is generous on every platform we target; bump
    // to 250ms if this resurfaces as flaky.
    thread::sleep(Duration::from_millis(200));

    // SIGHUP — bypasses the file watcher debounce window. The
    // orchestrator runs reload_once immediately. (The shell also
    // receives the HUP per v0.3.3 forwarding, but ignores it via
    // the trap installed above.)
    let pid = i32::try_from(child.process_id().expect("child pid")).expect("pid fits i32");
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), nix::sys::signal::Signal::SIGHUP)
        .expect("kill SIGHUP");

    thread::sleep(Duration::from_millis(400));

    writer.write_all(b"echo TAYFMARK_SIGHUP\n").expect("write marker");
    writer.write_all(b"echo ERROR after-sighup\n").expect("write after");
    writer.write_all(b"exit\n").expect("write exit");
    drop(writer);

    let out = drain_until_eof(&mut reader, Duration::from_secs(10));
    let _ = child.wait();

    let marker = b"TAYFMARK_SIGHUP";
    let pos = out.windows(marker.len()).position(|w| w == marker).expect("marker missing");
    let (before, after) = out.split_at(pos);

    assert!(yellow_sgr_present(before), "before: {:?}", String::from_utf8_lossy(before));
    assert!(
        red_sgr_present(after),
        "expected red SGR after SIGHUP reload; got {:?}",
        String::from_utf8_lossy(after)
    );
}
