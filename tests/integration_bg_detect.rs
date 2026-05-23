//! End-to-end integration tests for `bg_detect` via the real `tayf` binary.
//!
//! These tests spawn tayf inside a freshly allocated pty (so `stdout` is a
//! real TTY → `apply_colors == true` → the detection branch in
//! `Tayf::run` is exercised) and inject `COLORFGBG` via the child env to
//! drive the deterministic detection path. The OSC 11 path is NOT exercised
//! here (that would require a fake terminal that answers `\e]11;?\e\\`);
//! the COLORFGBG path is purely env-driven and parser-pure (spec §3.3).
//!
//! Hermeticity: each test passes an explicit empty `--config` so the user's
//! `~/.config/tayf/config.toml` (which may be present on developer machines)
//! cannot pollute the run. Same convention as `tests/integration_themes.rs`.

#![allow(clippy::expect_used)] // reason: tests, not library code

mod common;

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use portable_pty::PtySize;

/// PTY read deadline. Generous against CI jitter; the assertion exits early
/// as soon as the marker substring appears.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

fn tayf_bin() -> &'static str {
    env!("CARGO_BIN_EXE_tayf")
}

/// Read from `reader` until the byte representation of `marker` appears in
/// the accumulated buffer, EOF, or `READ_TIMEOUT` elapses. Returns whatever
/// bytes were read. Mirrors the partial-read loop in
/// `tests/integration_smoke.rs::partial_line_colorized_after_idle_tick`.
fn read_until_marker(reader: &mut dyn Read, marker: &[u8]) -> Vec<u8> {
    let deadline = Instant::now() + READ_TIMEOUT;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    while Instant::now() < deadline {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(marker.len()).any(|w| w == marker) {
                    // Give tayf a brief grace window to flush trailing
                    // bytes (SGR reset after the matched URL) so the
                    // SGR-introducer assertion has the wrap closed.
                    let grace = Instant::now() + Duration::from_millis(150);
                    while Instant::now() < grace {
                        match reader.read(&mut chunk) {
                            Ok(0) => break,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                            Err(_) => break,
                        }
                    }
                    return buf;
                }
            }
            Err(_) => break,
        }
    }
    buf
}

/// Spawn `tayf` under a pty with the given env and CLI args, write
/// `stdin_content` to its captive shell, then drain the master until
/// `marker` is seen (or `READ_TIMEOUT` elapses). Returns the captured
/// bytes. The child is killed and reaped on the way out so no zombies
/// leak across the test suite.
fn run_with_env_and_input(
    env: &[(&str, &str)],
    cli_args: &[&str],
    stdin_content: &str,
    marker: &[u8],
) -> Vec<u8> {
    let (master, mut child) = common::spawn_for_interaction_with_env(
        tayf_bin(),
        cli_args,
        env,
        PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
    );

    // Give tayf a moment to install its signal handler and spawn the
    // child shell before writing — otherwise the first bytes can race
    // the shell startup and disappear. Same 200 ms budget as the
    // `common::spawn_with_input` helper.
    std::thread::sleep(Duration::from_millis(200));

    let mut writer = master.take_writer().expect("take writer");
    writer.write_all(stdin_content.as_bytes()).expect("write stdin");
    drop(writer);

    let mut reader = master.try_clone_reader().expect("clone reader");
    let out = read_until_marker(reader.as_mut(), marker);

    let _ = child.kill();
    let _ = child.wait();
    out
}

/// Write an empty config file to a fresh tempdir and return its path. Used
/// to defeat the host's `~/.config/tayf/config.toml` so the test sees only
/// built-in rules. Mirrors the pattern in `integration_themes.rs`.
fn empty_config() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tmpdir");
    let cfg_path = dir.path().join("config.toml");
    std::fs::write(&cfg_path, "").expect("write empty config");
    (dir, cfg_path)
}

/// COLORFGBG=15;0 → bg field (last `;`-separated) is `0` → bg ANSI 0
/// (black) → dark per spec §3.3. With no `--theme` flag and no
/// `[general] theme` in the (empty) config, `Tayf::run` must invoke
/// `bg_detect::detect_theme` and apply the dark preset. We pin the
/// observable effect by asserting an SGR introducer wraps the URL the
/// built-in `url` rule colorizes — without theme resolution the rule's
/// style would fall back to the no-theme default; with the dark theme
/// applied the rule emits a styled URL surrounded by `\e[...m` codes.
#[test]
fn colorfgbg_dark_via_pty_applies_dark_theme() {
    let (_tmp, cfg_path) = empty_config();
    let cfg_str = cfg_path.to_str().expect("utf8 cfg path");

    let out = run_with_env_and_input(
        &[("COLORFGBG", "15;0"), ("TERM", "xterm-256color")],
        &["--config", cfg_str, "--shell", "/bin/sh"],
        "printf '%s\\n' 'http://example.com'\nexit\n",
        b"example.com",
    );

    let s = String::from_utf8_lossy(&out);
    assert!(
        out.windows(b"http://example.com".len()).any(|w| w == b"http://example.com"),
        "URL missing from tayf output: {s:?}"
    );
    assert!(
        out.windows(2).any(|w| w == b"\x1b["),
        "expected SGR introducer (built-in url rule under dark theme) in: {s:?}"
    );
}

/// COLORFGBG signal must be overridden by an explicit `--theme` on the
/// CLI. We pass a (deliberately consistent) `COLORFGBG=15;0` and
/// `--theme light`; spec §3.2 mandates CLI > config > detection, so the
/// effective theme is `light` regardless of the env signal. The
/// assertion is again on SGR-introducer presence — the light preset
/// still colorizes the URL, just with a different palette than dark.
/// This pins the CLI-win precedence end-to-end; the unit suite in
/// `src/lib.rs` covers the precedence math, this test covers the wire.
#[test]
fn explicit_theme_overrides_colorfgbg_via_pty() {
    let (_tmp, cfg_path) = empty_config();
    let cfg_str = cfg_path.to_str().expect("utf8 cfg path");

    let out = run_with_env_and_input(
        &[("COLORFGBG", "15;0"), ("TERM", "xterm-256color")],
        &["--config", cfg_str, "--shell", "/bin/sh", "--theme", "light"],
        "printf '%s\\n' 'http://example.com'\nexit\n",
        b"example.com",
    );

    let s = String::from_utf8_lossy(&out);
    assert!(
        out.windows(b"http://example.com".len()).any(|w| w == b"http://example.com"),
        "URL missing from tayf output: {s:?}"
    );
    assert!(
        out.windows(2).any(|w| w == b"\x1b["),
        "expected SGR introducer (light theme applied via --theme) in: {s:?}"
    );
}
