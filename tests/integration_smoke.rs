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

/// clap's default exit code on parse errors is 2. The v0.1 spec promises
/// BSD `EX_USAGE` (64); this test pins `main`'s `ErrorKind` mapping so a
/// future refactor cannot silently regress to clap's default. We invoke the
/// binary without a PTY because the assertion is purely on the exit status,
/// and clap writes its error to stderr regardless of whether stdout is a
/// terminal.
#[test]
fn invalid_flag_exits_with_64() {
    use std::process::Command;

    let out = Command::new(tayf_bin())
        .arg("--bogus-flag")
        .output()
        .expect("spawn tayf with --bogus-flag");
    assert_eq!(
        out.status.code(),
        Some(64),
        "expected EX_USAGE (64), got {:?}; stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn help_flag_prints_usage() {
    let out = spawn_capture(tayf_bin(), &["--help"], Duration::from_secs(5));
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("Usage:") || s.contains("USAGE:"), "missing usage: {s}");
    assert!(s.contains("--shell"));
    assert!(s.contains("--login"));
    assert!(s.contains("--no-color"));
    assert!(s.contains("--config"), "--config flag must appear in --help output");
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
    let mut tmp =
        tempfile::Builder::new().prefix("tayf-tick-").suffix(".sh").tempfile().expect("tempfile");
    writeln!(tmp, "#!/bin/sh").expect("write");
    writeln!(tmp, "printf 'host 192.168.1.1 $ '").expect("write");
    writeln!(tmp, "sleep 2").expect("write");
    tmp.flush().expect("flush");
    // Make executable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tmp.as_file().metadata().expect("meta").permissions();
        perms.set_mode(0o755);
        tmp.as_file().set_permissions(perms).expect("chmod");
    }
    // Close the writer fd before spawning tayf. On Linux, `execve(2)` returns
    // ETXTBSY when any process holds an O_RDWR descriptor for the target
    // file; macOS does not enforce this. `into_temp_path` drops the `File`
    // (closing the fd) while keeping the unlink-on-drop responsibility on
    // the returned `TempPath`, which must outlive the spawned child.
    let script = tmp.into_temp_path();
    let script_path = script.to_path_buf();

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

/// The input thread used to block forever on `stdin.read()` and was simply
/// detached at shutdown; the OS reaped it on process exit (spec §3.4 step
/// 7, v0.1 limit). v0.1.1 wires a self-pipe: after `child.wait()` returns,
/// the runtime writes one byte to the pipe, the input thread wakes from
/// `poll(2)`, exits its loop, and is `join`ed.
///
/// This test pins the wake-up path. It runs tayf against a shell that exits
/// immediately, so the only thing standing between `child.wait()` and tayf
/// returning is the input-thread join. The test's PTY harness keeps the
/// master side alive (so the slave stdin never gets EOF) — without the
/// self-pipe wake-up the join would block indefinitely and the test would
/// hit the deadline. The 2 s budget is generous against CI jitter; the
/// actual time on a quiet host is well under 100 ms.
#[test]
fn input_thread_joins_promptly_after_child_exit() {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};

    // A trivial "shell" that exits immediately. We do not use `/bin/true`
    // directly because the path is not guaranteed across all Unix targets
    // tayf supports; a tempfile script with `#!/bin/sh` and `exit 0` is
    // portable and self-documenting.
    let mut tmp = tempfile::Builder::new()
        .prefix("tayf-quickexit-")
        .suffix(".sh")
        .tempfile()
        .expect("tempfile");
    writeln!(tmp, "#!/bin/sh").expect("write");
    writeln!(tmp, "exit 0").expect("write");
    tmp.flush().expect("flush");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tmp.as_file().metadata().expect("meta").permissions();
        perms.set_mode(0o755);
        tmp.as_file().set_permissions(perms).expect("chmod");
    }
    // Close the writer fd before spawning tayf. See the sibling test above
    // for why this matters on Linux (ETXTBSY from `execve(2)`).
    let script = tmp.into_temp_path();
    let script_path = script.to_path_buf();

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");

    let mut builder = CommandBuilder::new(tayf_bin());
    builder.arg("--shell");
    builder.arg(&script_path);

    let start = Instant::now();
    let mut child = pair.slave.spawn_command(builder).expect("spawn");
    drop(pair.slave);

    // Drain the master in a background thread so its buffer cannot block
    // the tayf binary; we do not assert on its contents here.
    let mut reader = pair.master.try_clone_reader().expect("clone reader");
    let drain = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    // Poll `try_wait` for tayf's exit with a 2 s deadline. Without the
    // self-pipe wake-up, the runtime's `input_handle.join()` blocks forever
    // and tayf never exits; we'd hit the deadline. With the wake-up, exit
    // is well under 100 ms on a quiet host; 2 s is generous against CI
    // jitter. We poll rather than calling `child.wait()` directly so we
    // can bound the wait and produce a useful failure message instead of
    // hanging the test runner.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut exited = None;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("try_wait") {
            exited = Some(status);
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let elapsed = start.elapsed();

    if exited.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = drain.join();

    assert!(
        exited.is_some(),
        "tayf failed to exit within 2 s after a quick-exit shell; the runtime's input-thread join is blocked. elapsed={elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "tayf exited but took longer than 2 s; elapsed={elapsed:?}"
    );
}
