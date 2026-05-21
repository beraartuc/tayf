//! End-to-end smoke tests for the tayf binary.
//!
//! These exercise the spawn -> output -> exit path with real PTY allocation.
//! Run with `cargo test --test integration_smoke`.

#![allow(clippy::expect_used)] // reason: tests, not library code

mod common;

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use common::spawn_capture;

fn tayf_bin() -> &'static str {
    env!("CARGO_BIN_EXE_tayf")
}

#[test]
fn version_flag_prints_banner() {
    let out = spawn_capture(tayf_bin(), &["--version"], Duration::from_secs(5));
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("tayf"), "missing tayf in: {s}");
    assert!(s.contains("rustc"), "missing rustc in: {s}");
}

#[test]
fn help_flag_prints_usage() {
    let out = spawn_capture(tayf_bin(), &["--help"], Duration::from_secs(5));
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("Usage:") || s.contains("USAGE:"), "missing usage: {s}");
    assert!(s.contains("--shell"));
    assert!(s.contains("--login"));
    assert!(s.contains("--no-color"));
}

/// The output thread polls the master fd with a 50ms timeout and calls
/// `Pipeline::tick` on every wake-up that produced no bytes. A partial line
/// emitted by the child (no trailing `\n`) must therefore be colorized
/// within roughly one tick (50ms) instead of waiting for a newline or the
/// 64KB line-buffer cap. This exercises the entire poll → tick → SGR path.
#[test]
fn partial_line_colorized_after_idle_tick() {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};

    // A trivial "shell": prints an IPv4-containing prompt with no newline,
    // then sleeps. The sleep is long enough that the only way the IPv4 can
    // reach our output is via the idle-tick flush path.
    let mut script =
        tempfile::Builder::new().prefix("tayf-tick-").suffix(".sh").tempfile().expect("tempfile");
    writeln!(script, "#!/bin/sh").expect("write");
    writeln!(script, "printf 'host 192.168.1.1 $ '").expect("write");
    writeln!(script, "sleep 2").expect("write");
    script.flush().expect("flush");
    let script_path = script.path().to_path_buf();
    // Make executable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_path).expect("meta").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).expect("chmod");
    }

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");

    let mut builder = CommandBuilder::new(tayf_bin());
    builder.arg("--shell");
    builder.arg(&script_path);
    let mut child = pair.slave.spawn_command(builder).expect("spawn");
    drop(pair.slave);

    // Read for 500ms — generous for the 50ms tick to fire, short enough
    // that the test stays fast. We stop early as soon as we see the IPv4
    // bytes so the common case is well under the budget.
    let mut reader = pair.master.try_clone_reader().expect("clone reader");
    let mut out = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(500);
    let mut buf = [0u8; 4096];
    while Instant::now() < deadline {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                out.extend_from_slice(&buf[..n]);
                if out.windows(11).any(|w| w == b"192.168.1.1") {
                    // Give the tick one more chance to wrap the match
                    // with SGR before we assert (poll → read race).
                    let small_deadline = Instant::now() + Duration::from_millis(120);
                    while Instant::now() < small_deadline {
                        match reader.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => out.extend_from_slice(&buf[..n]),
                            Err(_) => break,
                        }
                    }
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = child.kill();
    // Reap the signalled child so it does not linger as a zombie until
    // process exit. `wait` returns promptly here because the child was
    // just killed; we ignore the status because the kill makes it
    // meaningless for the assertion below.
    let _ = child.wait();

    let s = String::from_utf8_lossy(&out);
    assert!(
        out.windows(11).any(|w| w == b"192.168.1.1"),
        "IPv4 bytes never reached stdout (no tick?): {s:?}"
    );
    assert!(
        out.windows(2).any(|w| w == b"\x1b["),
        "expected at least one SGR introducer wrapping the partial line: {s:?}"
    );
}
