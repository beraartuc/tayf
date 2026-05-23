//! v0.3.4 integration tests that need a real PTY to observe SGR escape
//! sequences in tayf's output. Disk themes ship the same byte-for-byte
//! SGR injection path as built-in presets; these tests pin that
//! end-to-end behavior. Non-PTY assertions (error text, exit codes, help
//! text) live in `tests/integration_themes.rs`.
//!
//! portable-pty pattern: `--shell /bin/sh` forced + `TAYF_DISABLE_BG_DETECT=1`
//! env so macOS portable-pty does not hang on OSC 11 background queries.

#![cfg(unix)]
#![allow(clippy::expect_used)] // reason: tests, not library code

use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use portable_pty::{CommandBuilder, PtySize};

fn write_disk_theme(xdg: &Path, name: &str, body: &str) {
    let themes = xdg.join("tayf").join("themes");
    std::fs::create_dir_all(&themes).expect("create themes dir");
    std::fs::write(themes.join(format!("{name}.toml")), body).expect("write theme");
}

/// Spawn tayf under a real PTY with the supplied args. Returns the
/// captured stdout bytes within a 5-second window. The child shell is
/// forced to `/bin/sh` (portable across CI); `TAYF_DISABLE_BG_DETECT=1`
/// suppresses macOS OSC 11 query hangs. Rev2 I-2 env isolation applied.
fn run_in_pty(xdg: &Path, theme: &str) -> Vec<u8> {
    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_tayf"));
    cmd.env_remove("HOME");
    cmd.env_remove("XDG_CONFIG_HOME");
    cmd.env("XDG_CONFIG_HOME", xdg);
    cmd.env("TAYF_DISABLE_BG_DETECT", "1");
    cmd.arg("--shell");
    cmd.arg("/bin/sh");
    cmd.arg("--no-hot-reload");
    cmd.arg("--theme");
    cmd.arg(theme);

    let mut child = pair.slave.spawn_command(cmd).expect("spawn tayf");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("clone reader");
    let mut writer = pair.master.take_writer().expect("take writer");

    // Send a command that emits a token the theme should color.
    thread::sleep(Duration::from_millis(200)); // wait for shell init
    writer.write_all(b"echo ERROR\nexit\n").expect("write");
    drop(writer);

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let _reader_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        loop {
            match reader.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&tmp[..n]),
                Err(_) => break,
            }
        }
        let _ = tx.send(buf);
    });

    let _ = child.wait();
    rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default()
}

#[test]
fn disk_theme_overrides_built_in_styles_end_to_end() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_disk_theme(
        xdg.path(),
        "mine",
        r#"
[[rules]]
name = "log_level"
style = { fg = "cyan" }
"#,
    );

    let bytes = run_in_pty(xdg.path(), "mine");
    let s = String::from_utf8_lossy(&bytes);

    assert!(s.contains("ERROR"), "ERROR token must appear in PTY output: {s:?}");
    // Cyan FG = SGR 36. Check for the bytes \x1b[36m wrapping ERROR.
    let has_cyan =
        s.contains("\u{1b}[36m") || s.contains("\u{1b}[1;36m") || s.contains("\u{1b}[36;");
    assert!(has_cyan, "expected cyan SGR around ERROR; got: {s:?}");
}

#[test]
fn disk_theme_can_define_partial_overrides_without_failing() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    write_disk_theme(
        xdg.path(),
        "partial",
        r#"
[[rules]]
name = "log_level"
style = { fg = "magenta" }
"#,
    );

    let bytes = run_in_pty(xdg.path(), "partial");
    let s = String::from_utf8_lossy(&bytes);

    assert!(s.contains("ERROR"), "echo output missing: {s:?}");
    assert!(
        s.contains("\u{1b}[35m") || s.contains("\u{1b}[1;35m") || s.contains("\u{1b}[35;"),
        "expected magenta SGR around ERROR; got: {s:?}"
    );
    assert!(
        !s.contains("validation errors"),
        "partial override must not trigger validation: {s:?}"
    );
}
