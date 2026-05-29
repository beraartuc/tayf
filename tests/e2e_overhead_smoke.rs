//! Mechanism smoke test for the e2e overhead harness (spec §7.2).
//!
//! Proves the drive shape works end-to-end at toy scale: tayf wraps a shell
//! running `cat <tmpfile>`, the file's bytes stream through tayf's colorize
//! path (real TTY), and the child exits without hanging. NO perf threshold
//! is asserted — wall-clock is noisy and the self-hosted runner is PTY-flaky;
//! the heavy measurement is the local `cargo bench --bench e2e_overhead` run.
//!
//! PTY-captured streams are SGR-fragmented, so we assert on a marker token
//! that no built-in matches (so it passes through verbatim), via a window
//! scan rather than a whole-line `contains`.

mod common;

use std::io::{Read, Write};
use std::time::{Duration, Instant};

/// A token no built-in rule matches, so tayf emits it byte-for-byte.
const MARKER: &[u8] = b"ZZE2EMARKERZZ";

#[test]
fn tayf_streams_corpus_through_and_exits() {
    // Toy corpus: a few hundred lines, each ending in the inert marker.
    let marker = std::str::from_utf8(MARKER).expect("ascii marker");
    let mut corpus = tempfile::NamedTempFile::new().expect("temp corpus");
    for _ in 0..300 {
        writeln!(corpus, "10.0.0.1 ERROR line {marker}").expect("write corpus");
    }
    corpus.flush().expect("flush");
    let path = corpus.path().display().to_string();

    let start = Instant::now();
    let out = common::spawn_with_input(&format!("cat {path}\nexit\n"), Duration::from_secs(20));
    let tayf_elapsed = start.elapsed();

    // (a) bytes streamed through tayf: the inert marker appears verbatim.
    assert!(
        out.windows(MARKER.len()).any(|w| w == MARKER),
        "tayf did not stream the corpus marker through (mechanism broken)"
    );

    // (b) no hang: the helper returned before its own 20s timeout.
    assert!(
        tayf_elapsed < Duration::from_secs(20),
        "tayf run hit the drain timeout (possible hang)"
    );

    // (c) ratio is computable (finite, positive) — shape guard only, not perf.
    let bare = bare_shell_elapsed(&path);
    let ratio = tayf_elapsed.as_secs_f64() / bare.as_secs_f64();
    assert!(ratio.is_finite() && ratio > 0.0, "overhead ratio not computable: {ratio}");
}

/// Time a bare `/bin/sh` running the same `cat`, mirroring the harness drive.
fn bare_shell_elapsed(path: &str) -> Duration {
    use portable_pty::PtySize;
    let (master, mut child) = common::spawn_for_interaction(
        "/bin/sh",
        &[],
        PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
    );
    std::thread::sleep(Duration::from_millis(200));
    // Mirror the bench's `timed_run` shape exactly: acquire writer + reader
    // before the clock starts, and drain remaining buffered bytes after the
    // child exits, so this baseline faithfully matches the harness it models.
    let mut writer = master.take_writer().expect("take writer");
    let mut reader = master.try_clone_reader().expect("clone reader");
    let start = Instant::now();
    writer.write_all(format!("cat {path}\nexit\n").as_bytes()).expect("write");
    drop(writer);
    let mut buf = [0u8; 65536];
    while start.elapsed() < Duration::from_secs(20) {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        if let Ok(Some(_)) = child.try_wait() {
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
            }
            break;
        }
    }
    let e = start.elapsed();
    let _ = child.kill();
    let _ = child.wait();
    e
}
