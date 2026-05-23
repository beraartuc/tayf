//! Integration test: --no-hot-reload prevents file watcher + reload
//! orchestrator from spawning.
//!
//! Approach: write a minimal config that colors "error" red. Spawn
//! `tayf --config <path> --no-hot-reload`. Edit the config file to
//! color "error" green. Wait > DEBOUNCE_WINDOW (200 ms). Send an
//! `echo error: ...` and verify the output still uses the ORIGINAL
//! red color — proof that no reload occurred.
//!
//! Sensitive to timing — uses generous waits. macOS portable-pty
//! flush latency tolerated via the 5s read budget.

#![cfg(unix)]
#![allow(clippy::expect_used)] // reason: tests, not library code

use std::io::{Read, Write};
use std::time::Duration;

use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use tempfile::TempDir;

const SETTLE_MS: u64 = 500;
const READ_BUDGET_MS: u64 = 5_000;

fn tayf_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_tayf"))
}

#[test]
fn no_hot_reload_does_not_reload_after_config_edit() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        r#"
[[rules]]
name = "log_level"
style = { fg = "red", bold = true }
"#,
    )
    .expect("write config");

    let pty = NativePtySystem::default()
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");

    let mut builder = CommandBuilder::new(tayf_bin());
    builder.env("TAYF_DISABLE_BG_DETECT", "1");
    builder.arg("--shell");
    builder.arg("/bin/sh");
    builder.arg("--config");
    builder.arg(&config_path);
    builder.arg("--no-hot-reload");

    let mut child = pty.slave.spawn_command(builder).expect("spawn tayf");
    drop(pty.slave);

    let mut writer = pty.master.take_writer().expect("writer");
    let mut reader = pty.master.try_clone_reader().expect("reader");

    std::thread::sleep(Duration::from_millis(SETTLE_MS));

    // Mutate the config: switch red → green. With hot-reload enabled
    // this would, after a 200 ms debounce, take effect on the next
    // `error: ...` line. With --no-hot-reload, it must NOT.
    std::fs::write(
        &config_path,
        r#"
[[rules]]
name = "log_level"
style = { fg = "green", bold = true }
"#,
    )
    .expect("rewrite config");

    // Wait > DEBOUNCE_WINDOW (200 ms) + a margin.
    std::thread::sleep(Duration::from_millis(SETTLE_MS));

    // NOTE: built-in `log_level` pattern matches uppercase tokens
    // (`ERROR|FAIL|FATAL|...`) — lowercase `error` would not trigger
    // colorization, defeating the assertion below.
    writer.write_all(b"echo ERROR still_old_rules\n").expect("write echo");
    writer.write_all(b"exit\n").expect("write exit");
    writer.flush().expect("flush");

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

    // SGR code 31 = red foreground; SGR code 32 = green foreground.
    // The output must contain a red SGR around "error" and NOT a green
    // SGR. Use simple substring search — robust to ordering of bold/fg.
    let has_red = buf.windows(2).any(|w| w == b"31");
    let has_green = buf.windows(2).any(|w| w == b"32");
    assert!(
        has_red,
        "expected red SGR (31) from original config; output:\n{:?}",
        String::from_utf8_lossy(&buf)
    );
    assert!(
        !has_green,
        "must NOT contain green SGR (32) — config edit should not have reloaded; output:\n{:?}",
        String::from_utf8_lossy(&buf)
    );
}
