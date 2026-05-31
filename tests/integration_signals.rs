//! Integration tests for signal forwarding.
//!
//! These tests verify the v0.2.0 signal path end-to-end: signals
//! delivered to the tayf process are correctly routed to either the
//! child process group (SIGINT/SIGTERM) or to the PTY master
//! (SIGWINCH) by the signal thread (see src/signals.rs).
//!
//! Adding these tests was a recommendation from the senior code
//! reviewer of the signal-hook 0.4 dependency bump (commit 7da37d0).

#![allow(clippy::expect_used)]

mod common;

use std::thread;
use std::time::Duration;

use portable_pty::PtySize;

#[test]
fn sigwinch_to_tayf_resizes_child_pty() {
    // Spawn tayf wrapping /bin/sh; resize the master fd; have the
    // shell report its window via `stty size`.
    let tayf = env!("CARGO_BIN_EXE_tayf");
    let (master, mut child) = common::spawn_for_interaction_with_env(
        tayf,
        &["--shell", "/bin/sh"],
        &[("TAYF_DISABLE_BG_DETECT", "1")],
        PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
    );

    // Give tayf time to install the signal handler and spawn the shell.
    thread::sleep(Duration::from_millis(200));

    // Resize the master. portable-pty propagates this as TIOCSWINSZ on
    // the master fd; the kernel translates to a SIGWINCH on the
    // foreground process group of the slave. tayf's signal thread
    // catches it and re-forwards via the resizer.
    master
        .resize(PtySize { rows: 50, cols: 132, pixel_width: 0, pixel_height: 0 })
        .expect("resize master");

    // Drive the shell to print its current window size and exit.
    use std::io::Write;
    let mut writer = master.take_writer().expect("take writer");
    writer.write_all(b"stty size; exit\n").expect("write stty");
    drop(writer);

    // Collect stdout until child exit or timeout.
    use std::io::Read;
    let mut reader = master.try_clone_reader().expect("clone reader");
    let mut out = Vec::new();
    let start = std::time::Instant::now();
    let mut buf = [0u8; 4096];
    while start.elapsed() < Duration::from_secs(5) {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
        if let Ok(Some(_status)) = child.try_wait() {
            if let Ok(n) = reader.read(&mut buf) {
                out.extend_from_slice(&buf[..n]);
            }
            break;
        }
    }
    let _ = child.kill();

    let text = String::from_utf8_lossy(&out);
    // tayf's colorizer can inject SGR escape sequences between digits
    // (e.g., the "132" cols matches a numeric rule), so strip
    // `ESC [ ... letter` CSI sequences before asserting on the
    // resize-reported dimensions.
    let stripped = strip_csi(&text);
    assert!(
        stripped.contains("50 132"),
        "expected '50 132' in stty output after SIGWINCH; got: {text:?} \
         (stripped: {stripped:?})"
    );
}

/// Remove ANSI CSI sequences (`ESC [ ... final-byte`) from a string. Only
/// used by the assertion above; not exposed beyond this test file.
fn strip_csi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Consume the next byte (usually `[`) and then everything up to
            // and including a final byte in 0x40..=0x7e.
            if chars.next() == Some('[') {
                for inner in chars.by_ref() {
                    if ('@'..='~').contains(&inner) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}
