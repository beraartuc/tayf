//! Shared fixtures for integration tests.
//!
//! Spawns the tayf binary inside a freshly allocated PTY so the binary sees a
//! real `tty(4)` and its `--version` / `--help` output reaches the test
//! process. Built on `portable-pty` 0.9 for cross-platform consistency.

// reason: integration-test helper — `expect` is the conventional shape here.
// Tests under `tests/` are not part of the library's production surface, but
// the crate-wide `clippy::pedantic` lint group still applies.
#![allow(clippy::expect_used)]
// reason: each integration-test binary compiles `common/mod.rs` independently
// and only sees the helpers it uses, so unused-fn warnings here are a
// false-positive of `tests/common` being shared.
#![allow(dead_code)]

pub mod tui_harness;

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

/// Spawn `cmd` inside a PTY and return the master plus the child handle so
/// the caller can write to stdin, read from stdout, send signals, and
/// observe exit. Caller is responsible for killing the child on test
/// failure paths.
pub fn spawn_for_interaction(
    cmd: &str,
    args: &[&str],
    size: portable_pty::PtySize,
) -> (Box<dyn portable_pty::MasterPty + Send>, Box<dyn portable_pty::Child + Send + Sync>) {
    spawn_for_interaction_with_env(cmd, args, &[], size)
}

/// Same as [`spawn_for_interaction`] but applies per-child env overrides.
/// The host process env is left untouched (no `std::env::set_var` calls),
/// which keeps cargo-test parallelism safe.
pub fn spawn_for_interaction_with_env(
    cmd: &str,
    args: &[&str],
    env: &[(&str, &str)],
    size: portable_pty::PtySize,
) -> (Box<dyn portable_pty::MasterPty + Send>, Box<dyn portable_pty::Child + Send + Sync>) {
    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system.openpty(size).expect("openpty");

    let mut builder = portable_pty::CommandBuilder::new(cmd);
    for a in args {
        builder.arg(a);
    }
    for (k, v) in env {
        builder.env(k, v);
    }
    let child = pair.slave.spawn_command(builder).expect("spawn");
    drop(pair.slave);

    (pair.master, child)
}

/// Spawn the tayf binary inside a PTY, write `stdin_content` to the child
/// shell's stdin, then drain the master until the child exits or `timeout`
/// elapses. Returns the captured PTY bytes (tayf's stdout).
///
/// Convenience wrapper around [`spawn_for_interaction_with_env`] for tests
/// that just need "feed these commands, get the bytes back" semantics.
/// `stdin_content` must terminate with `exit\n` (or otherwise cause the
/// shell to exit) so the read loop sees EOF on the master before `timeout`.
pub fn spawn_with_input(stdin_content: &str, timeout: Duration) -> Vec<u8> {
    spawn_with_input_and_args(stdin_content, &[], timeout)
}

/// Same as [`spawn_with_input`] but threads extra CLI args (e.g. `--config
/// <path>`) into the tayf invocation. The child shell is always
/// `/bin/sh`; tests that need a different shell should use
/// [`spawn_for_interaction_with_env`] directly.
pub fn spawn_with_input_and_args(
    stdin_content: &str,
    extra_args: &[&str],
    timeout: Duration,
) -> Vec<u8> {
    use std::io::{Read, Write};

    let tayf = env!("CARGO_BIN_EXE_tayf");
    let mut args: Vec<&str> = Vec::with_capacity(extra_args.len() + 2);
    args.extend_from_slice(extra_args);
    args.push("--shell");
    args.push("/bin/sh");

    let (master, mut child) = spawn_for_interaction(
        tayf,
        &args,
        portable_pty::PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
    );

    // Give tayf a moment to install its signal handler and spawn the
    // child shell before writing — otherwise the first bytes can race
    // the shell's startup and disappear into the void.
    std::thread::sleep(Duration::from_millis(200));

    let mut writer = master.take_writer().expect("take writer");
    writer.write_all(stdin_content.as_bytes()).expect("write stdin");
    drop(writer);

    let mut reader = master.try_clone_reader().expect("clone reader");
    let mut out = Vec::new();
    let start = std::time::Instant::now();
    let mut buf = [0u8; 4096];
    while start.elapsed() < timeout {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
        if let Ok(Some(_status)) = child.try_wait() {
            // Drain any remaining bytes the kernel still has buffered.
            if let Ok(n) = reader.read(&mut buf) {
                out.extend_from_slice(&buf[..n]);
            }
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    out
}
