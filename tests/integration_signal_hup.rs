//! Integration test: SIGHUP forwarding to child process group.
//!
//! Pins the v0.3.3 BEHAVIOR CHANGE: SIGHUP is always forwarded to the
//! child process group, regardless of hot-reload state. Spawns tayf
//! under portable-pty (no config file → reload_tx is None at the
//! signal thread level), sends SIGHUP to the tayf process, and verifies
//! the shell's HUP trap fires.
//!
//! Pre-v0.3.3 behavior: this test would time out — SIGHUP was silently
//! dropped by the signal thread when reload_tx was None.
//!
//! macOS portable-pty timing note: per the v0.3.2 OSC 11 hang investigation
//! (CHANGELOG v0.3.2 Fixed entry), portable-pty subprocesses on macOS can
//! have multi-second stderr flush latency. Output read budget here is 5 s
//! defensively (Rev2 I-9); .github/workflows/ci.yml retry once via
//! `gh run rerun --failed` (memory `tayf release workflow` rule).

#![cfg(unix)]
#![allow(clippy::expect_used)] // reason: tests, not library code

use std::io::{Read, Write};
use std::time::Duration;

use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};

const TRAP_SETTLE_MS: u64 = 500;
const READ_BUDGET_MS: u64 = 5_000;

fn tayf_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_tayf"))
}

/// Spawn tayf with `extra_args`, install a HUP trap in the child shell,
/// send SIGHUP to the tayf process, return captured stdout bytes.
fn run_hup_trap_test(extra_args: &[&str]) -> Vec<u8> {
    let pty = NativePtySystem::default()
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");

    let mut builder = CommandBuilder::new(tayf_bin());
    builder.env("TAYF_DISABLE_BG_DETECT", "1");
    // Force /bin/sh — predictable trap syntax.
    builder.arg("--shell");
    builder.arg("/bin/sh");
    for a in extra_args {
        builder.arg(a);
    }

    let mut child = pty.slave.spawn_command(builder).expect("spawn tayf");
    drop(pty.slave);

    let tayf_pid = child.process_id().expect("tayf pid");

    let mut writer = pty.master.take_writer().expect("writer");
    let mut reader = pty.master.try_clone_reader().expect("reader");

    // Wait briefly for shell to be ready.
    std::thread::sleep(Duration::from_millis(TRAP_SETTLE_MS));

    // Install the trap; sleep long enough that we can deliver HUP.
    writer.write_all(b"trap 'echo HUP_RECEIVED; exit 0' HUP; sleep 5\n").expect("write trap");
    writer.flush().expect("flush");

    // Give the shell a moment to install the trap before signaling.
    std::thread::sleep(Duration::from_millis(TRAP_SETTLE_MS));

    // Deliver SIGHUP to tayf.
    let pid_i32 = i32::try_from(tayf_pid).expect("pid fits in i32");
    kill(Pid::from_raw(pid_i32), Signal::SIGHUP).expect("kill -HUP tayf");

    // Read with budget until HUP_RECEIVED appears or budget expires.
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
        if buf.windows(b"HUP_RECEIVED".len()).any(|w| w == b"HUP_RECEIVED") {
            break;
        }
        if let Ok(Some(_)) = child.try_wait() {
            // Drain residual.
            while let Ok(n) = reader.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            break;
        }
    }

    let _ = child.wait();
    buf
}

#[test]
fn sighup_is_forwarded_to_child_process_group_without_hot_reload() {
    // No --config flag, no --no-hot-reload flag explicitly: reload_tx
    // is None at signal thread level because loaded is None (no config
    // discovered in temp env). Pre-v0.3.3 behavior would silently drop
    // SIGHUP here; v0.3.3 forwards to child PG so the trap fires.
    let out = run_hup_trap_test(&[]);
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("HUP_RECEIVED"), "SIGHUP must reach child shell trap; output was:\n{s}");
}

#[test]
fn sighup_is_forwarded_to_child_process_group_with_bypass() {
    // Bypass also has reload_tx = None at signal thread level. SIGHUP
    // forwarding must still work.
    let out = run_hup_trap_test(&["--bypass"]);
    let s = String::from_utf8_lossy(&out);
    assert!(
        s.contains("HUP_RECEIVED"),
        "SIGHUP must reach child shell trap under --bypass; output was:\n{s}"
    );
}
