//! Integration test: SIGINT forwarding to child process group.
//!
//! Pins the SIGINT/SIGTERM arm of `src/signals.rs` (lines ~81–91): when tayf
//! receives SIGINT it must call `killpg(child_pid, SIGINT)`, delivering the
//! signal to the **child's process group** — not just the child PID. This
//! prevents foreground programs launched by the shell from surviving `^C` as
//! orphans. Complements the existing SIGHUP forwarding test
//! (`tests/integration_signal_hup.rs`) and the SIGWINCH resize test
//! (`tests/integration_signals.rs`).
//!
//! macOS portable-pty timing note: per the v0.3.2 OSC 11 hang investigation
//! (CHANGELOG v0.3.2 Fixed entry), portable-pty subprocesses on macOS can
//! have multi-second stderr flush latency. Output read budget here is 5 s
//! defensively; `.github/workflows/ci.yml` retries once via
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

#[test]
fn sigint_is_forwarded_to_child_process_group() {
    // No --config flag: the signal thread has child_pid wired but no
    // reload_tx (None). The SIGINT/SIGTERM arm (src/signals.rs ~81–91)
    // calls killpg unconditionally when child_pid is Some. The shell INT
    // trap fires — proving the signal reached the process GROUP, not just
    // the tayf process itself.
    let pty = NativePtySystem::default()
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");

    let mut builder = CommandBuilder::new(tayf_bin());
    builder.env("TAYF_DISABLE_BG_DETECT", "1");
    // Force /bin/sh — predictable trap syntax.
    builder.arg("--shell");
    builder.arg("/bin/sh");

    let mut child = pty.slave.spawn_command(builder).expect("spawn tayf");
    drop(pty.slave);

    let tayf_pid = child.process_id().expect("tayf pid");

    let mut writer = pty.master.take_writer().expect("writer");
    let mut reader = pty.master.try_clone_reader().expect("reader");

    // Wait briefly for shell to be ready.
    std::thread::sleep(Duration::from_millis(TRAP_SETTLE_MS));

    // Install the INT trap; sleep long enough that we can deliver SIGINT.
    writer.write_all(b"trap 'echo INT_RECEIVED; exit 0' INT; sleep 5\n").expect("write trap");
    writer.flush().expect("flush");

    // Give the shell a moment to install the trap before signaling.
    std::thread::sleep(Duration::from_millis(TRAP_SETTLE_MS));

    // Deliver SIGINT to the tayf process.
    let pid_i32 = i32::try_from(tayf_pid).expect("pid fits in i32");
    kill(Pid::from_raw(pid_i32), Signal::SIGINT).expect("kill -INT tayf");

    // Read with budget until INT_RECEIVED appears or budget expires.
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
        if buf.windows(b"INT_RECEIVED".len()).any(|w| w == b"INT_RECEIVED") {
            break;
        }
        if let Ok(Some(_)) = child.try_wait() {
            // Drain residual output.
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

    let s = String::from_utf8_lossy(&buf);
    assert!(s.contains("INT_RECEIVED"), "SIGINT must reach child shell trap; output was:\n{s}");
}

// NOTE (v0.9 A6): There is intentionally no panic-time termios-restoration
// integration test. Under `panic = "abort"` (release profile) the RAII Drop
// guard is skipped (no unwind), so restoration relies solely on the
// `std::panic::set_hook` handler installed in src/tty_guard.rs
// (install_panic_hook / PANIC_RESTORE_STATE). std guarantees the hook runs
// under the aborting runtime before the process aborts. Triggering a real
// panic inside tayf's output thread would require a production fault-injection
// hook, which is forbidden (CLAUDE.md §4 — no test-only surface in shipped
// code), and tayf does not panic on any input (a fuzz-verified security
// property). The abort-path restore is therefore verified by reasoning + the
// std set_hook contract, and is an explicit verification item for the v0.9
// Phase C terminal-security audit (reading src/tty_guard.rs:93-135).
