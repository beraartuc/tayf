//! Shared fixtures for integration tests.
//!
//! Spawns the tayf binary inside a freshly allocated PTY so the binary sees a
//! real `tty(4)` and its `--version` / `--help` output reaches the test
//! process. Built on `portable-pty` 0.9 for cross-platform consistency.

// reason: integration-test helper — `expect` is the conventional shape here.
// Tests under `tests/` are not part of the library's production surface, but
// the crate-wide `clippy::pedantic` lint group still applies.
#![allow(clippy::expect_used)]

use std::time::Duration;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

/// Spawn `cmd` inside a PTY and return its stdout bytes once the process
/// exits or `timeout` elapses.
///
/// The child is killed on timeout so a hung binary cannot wedge the test
/// suite. EOF on the master side (child closed its tty) ends the read loop
/// promptly.
pub fn spawn_capture(cmd: &str, args: &[&str], timeout: Duration) -> Vec<u8> {
    use std::io::Read;

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");

    let mut builder = CommandBuilder::new(cmd);
    for a in args {
        builder.arg(a);
    }
    let mut child = pair.slave.spawn_command(builder).expect("spawn");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("clone reader");
    let mut out = Vec::new();
    let start = std::time::Instant::now();
    let mut buf = [0u8; 4096];
    while start.elapsed() < timeout {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }
    let _ = child.kill();
    out
}
