//! Regression coverage for the v0.3.1 macOS portable-pty OSC 11 hang.
//!
//! Spawns the tayf binary inside a portable-pty subprocess WITHOUT the
//! v0.3.1 COLORFGBG=15;15 workaround and asserts that the binary's bg_detect
//! startup path completes within a generous 2-second budget. On macOS in the
//! v0.3.1 state this test hangs/times out — that failure is the v0.3.2 entry
//! point to either fix the root cause (Senaryo 1 or 2) or ship the
//! TAYF_DISABLE_BG_DETECT env-var bypass (Senaryo 3).
//!
//! See docs/superpowers/specs/2026-05-23-tayf-v0.3.2-pattern-polish-tech-debt.md §3.6, §4.4.

#![allow(clippy::expect_used)] // reason: tests

use std::io::Write;
use std::time::{Duration, Instant};

fn tayf_bin() -> &'static str {
    env!("CARGO_BIN_EXE_tayf")
}

#[test]
fn bg_detect_does_not_hang_in_portable_pty_subprocess() {
    // Spawn tayf with /bin/sh and immediately send `exit` so the binary's
    // happy-path shutdown sequence runs. The metric we care about is total
    // wall-clock time from spawn to child exit: bg_detect runs once at
    // startup, BEFORE TtyGuard::engage. If the OSC 11 path hangs, child
    // exit takes far longer than the 100 ms OSC11_READ_TIMEOUT.
    //
    // CRITICAL: We must scrub COLORFGBG AND TAYF_DISABLE_BG_DETECT from the
    // inherited env. Many developer terminals (iTerm2 in particular) set
    // COLORFGBG automatically, which would short-circuit bg_detect via the
    // env-var path and silently skip the OSC 11 code this test exists to
    // cover. The v0.3.1 CI hack set COLORFGBG=15;15 to force the same
    // shortcut; v0.3.2's whole purpose is to test the path WITHOUT it.
    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(portable_pty::PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");

    let mut builder = portable_pty::CommandBuilder::new(tayf_bin());
    builder.arg("--shell");
    builder.arg("/bin/sh");
    builder.env_remove("COLORFGBG");
    builder.env_remove("TAYF_DISABLE_BG_DETECT");

    let mut child = pair.slave.spawn_command(builder).expect("spawn");
    drop(pair.slave);
    let master = pair.master;

    let start = Instant::now();

    // Give tayf a moment for bg_detect + signal handler install.
    std::thread::sleep(Duration::from_millis(300));
    let mut writer = master.take_writer().expect("take writer");
    writer.write_all(b"exit\n").expect("write exit");
    drop(writer);

    // Wait up to 2 seconds for the child to exit.
    let budget = Duration::from_secs(2);
    let deadline = start + budget;
    let mut exited = false;
    while Instant::now() < deadline {
        if let Ok(Some(_status)) = child.try_wait() {
            exited = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let elapsed = start.elapsed();
    let _ = child.kill();
    let _ = child.wait();

    assert!(
        exited,
        "tayf did not exit within {budget:?} (elapsed {elapsed:?}); \
         bg_detect OSC 11 hang likely. Spawn examples/repro_osc11_hang via \
         portable-pty to isolate which phase stalls (spec §4.4)."
    );
}
