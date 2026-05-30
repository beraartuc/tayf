//! Integration test: --bypass and TAYF_DISABLE bypass mode.
//!
//! Pins:
//! 1. With --bypass, tayf wraps the shell but does NOT inject SGR
//!    sequences into the output (output bytes contain no ANSI SGR
//!    around what the built-in `log_level` rule would normally
//!    colorize).
//! 2. With TAYF_DISABLE=1 (env), same as #1 (env var path).
//! 3. CLI --bypass wins over TAYF_DISABLE=0 (precedence).
//! 4. --bypass --no-hot-reload combined remains passthrough (Decision 14
//!    documentation gap closed — Rev2 N-3).
//!
//! All tests set TAYF_DISABLE_BG_DETECT=1 defensively (v0.3.2 pattern:
//! avoids macOS portable-pty hang on the OSC 11 query path) and force
//! `--shell /bin/sh` to keep the user's $SHELL prompt (zsh/bash with
//! colored PS1) from contaminating the SGR-free assertion.

#![cfg(unix)]
#![allow(clippy::expect_used)] // reason: tests, not library code

use std::io::{Read, Write};
use std::time::Duration;

use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};

const PROMPT_SETTLE_MS: u64 = 300;
const READ_BUDGET_MS: u64 = 5_000;

fn tayf_bin() -> std::path::PathBuf {
    // Cargo provides CARGO_BIN_EXE_<name> for the binary target.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_tayf"))
}

/// Spawn `tayf <args>` in a portable-pty pseudoterminal, send `cmd`,
/// then `exit\n`, drain stdout up to READ_BUDGET_MS, and return the
/// collected bytes plus the child's exit status.
///
/// Always forces `--shell /bin/sh` (prepended to `extra_args`) so the
/// user's interactive shell prompt cannot inject SGR into the captured
/// stream. The bypass assertion is "tayf injected no SGR"; a colored
/// PS1 from zsh/bash would be a false positive.
fn run_in_pty(
    extra_args: &[&str],
    extra_env: &[(&str, &str)],
    cmd: &str,
) -> (Vec<u8>, portable_pty::ExitStatus) {
    let pty = NativePtySystem::default()
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");

    let mut builder = CommandBuilder::new(tayf_bin());
    // Force /bin/sh to avoid interactive-shell prompt color contamination.
    builder.arg("--shell");
    builder.arg("/bin/sh");
    for arg in extra_args {
        builder.arg(arg);
    }
    builder.env("TAYF_DISABLE_BG_DETECT", "1");
    for (k, v) in extra_env {
        builder.env(*k, *v);
    }

    let mut child = pty.slave.spawn_command(builder).expect("spawn tayf");
    drop(pty.slave); // master keeps the pty open

    let mut writer = pty.master.take_writer().expect("writer");
    let mut reader = pty.master.try_clone_reader().expect("reader");

    std::thread::sleep(Duration::from_millis(PROMPT_SETTLE_MS));

    writer.write_all(cmd.as_bytes()).expect("write cmd");
    writer.write_all(b"\n").expect("write LF");
    writer.write_all(b"exit\n").expect("write exit");
    writer.flush().expect("flush");

    // Read until child exits or budget expires.
    let mut buf = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_millis(READ_BUDGET_MS);
    let mut chunk = [0u8; 1024];
    loop {
        if std::time::Instant::now() >= deadline {
            break;
        }
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
        if let Ok(Some(_)) = child.try_wait() {
            // Drain anything left after the child died.
            while let Ok(n) = reader.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            break;
        }
    }

    let status = child.wait().expect("wait");
    (buf, status)
}

/// Returns true if `bytes` contains any ANSI SGR (CSI ... m) escape.
fn contains_sgr(bytes: &[u8]) -> bool {
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == 0x1B && bytes[i + 1] == b'[' {
            // Scan forward for an `m` terminator within the CSI.
            let mut j = i + 2;
            while j < bytes.len() && bytes[j] != b'm' && bytes[j] < 0x40 {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'm' {
                return true;
            }
        }
        i += 1;
    }
    false
}

#[test]
fn bypass_flag_skips_pattern_matching() {
    // The built-in log_level rule normally colors "ERROR". With --bypass
    // we expect zero SGR escapes around the user echo output.
    let (out, _status) = run_in_pty(&["--bypass"], &[], "echo error: failed");
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("error: failed"), "echo output present; got:\n{s}");
    // Bypass guarantees no SGR injection on user output.
    assert!(!contains_sgr(&out), "bypass mode must not inject SGR; got bytes:\n{out:?}");
}

#[test]
fn tayf_disable_env_skips_pattern_matching() {
    let (out, _status) = run_in_pty(&[], &[("TAYF_DISABLE", "1")], "echo error: failed");
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("error: failed"));
    assert!(!contains_sgr(&out), "TAYF_DISABLE=1 must not inject SGR; got:\n{out:?}");
}

#[test]
fn cli_bypass_overrides_env_disable_zero() {
    // CLI --bypass true; env TAYF_DISABLE=0 (falsy). CLI wins.
    let (out, _status) = run_in_pty(&["--bypass"], &[("TAYF_DISABLE", "0")], "echo error: failed");
    assert!(!contains_sgr(&out), "CLI --bypass must win over env TAYF_DISABLE=0; got:\n{out:?}");
}

#[test]
fn bypass_combined_with_no_hot_reload_is_passthrough() {
    // Rev2 N-3 — Decision 14 combined-flag coverage.
    let (out, _status) = run_in_pty(&["--bypass", "--no-hot-reload"], &[], "echo error: failed");
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("error: failed"));
    assert!(
        !contains_sgr(&out),
        "--bypass --no-hot-reload combined must be passthrough; got:\n{out:?}"
    );
}
