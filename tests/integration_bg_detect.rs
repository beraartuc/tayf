//! Regression coverage for the TAYF_DISABLE_BG_DETECT bypass.
//!
//! Spawns the tayf binary inside a portable-pty subprocess with the
//! TAYF_DISABLE_BG_DETECT=1 env var set (and COLORFGBG scrubbed to prove
//! the bypass — not the env-var fast path — is what completes startup).
//! Asserts that bg_detect short-circuits to BgTheme::Dark and the binary
//! completes happy-path startup within a 2-second budget.
//!
//! The underlying v0.3.1 portable-pty OSC 11 hang on macOS is NOT fixed
//! in v0.3.2; the bypass is the documented escape hatch for test/CI
//! environments. Real RC investigation deferred — see CHANGELOG [0.3.2]
//! Fixed section and the D-2 diagnostic note for the reasoning.
//!
//! See docs/superpowers/specs/2026-05-23-tayf-v0.3.2-pattern-polish-tech-debt.md §3.6, §4.4.

#![allow(clippy::expect_used)] // reason: tests

use std::io::Write;
use std::time::{Duration, Instant};

fn tayf_bin() -> &'static str {
    env!("CARGO_BIN_EXE_tayf")
}

#[test]
fn bg_detect_with_disable_env_var_does_not_hang() {
    // Spawn tayf with /bin/sh and immediately send `exit` so the binary's
    // happy-path shutdown sequence runs. The bypass should make bg_detect
    // a no-op (returns BgTheme::Dark before opening /dev/tty), so total
    // wall-clock should be well under the 2-second budget.
    //
    // COLORFGBG is scrubbed from the child env so the env-var fast path
    // in detect_from_colorfgbg is NOT what gets us past bg_detect — only
    // the TAYF_DISABLE_BG_DETECT short-circuit makes this pass.
    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(portable_pty::PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");

    let mut builder = portable_pty::CommandBuilder::new(tayf_bin());
    builder.arg("--shell");
    builder.arg("/bin/sh");
    builder.env_remove("COLORFGBG");
    builder.env("TAYF_DISABLE_BG_DETECT", "1");

    let mut child = pair.slave.spawn_command(builder).expect("spawn");
    drop(pair.slave);
    let master = pair.master;

    let start = Instant::now();

    // Give tayf a moment for the (bypassed) bg_detect + signal handler install.
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
         TAYF_DISABLE_BG_DETECT bypass should have made bg_detect a no-op."
    );
}
