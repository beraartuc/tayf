//! Hot-reload orchestrator.
//!
//! Receives [`ReloadRequest`] events from the file watcher and the
//! signal thread, re-reads the config, re-compiles the rule set, and
//! atomically stores the new compiled set into the shared
//! `Arc<ArcSwap<Compiled>>`. Parse/compile failures preserve the
//! previous rule set and emit a `warn_msg!` to stderr.
//!
//! See `docs/superpowers/specs/2026-05-22-tayf-v0.2.1-hot-reload.md` §3.2.

/// Banner byte sequence written on successful reload when
/// `show_reload_banner = true`. See spec §1.3 for the per-byte semantics.
///
/// - `\x1b[s` (DECSC) save cursor — multi-line zsh ZLE prompts retain
///   their visual cursor position after the banner draws.
/// - `\r\x1b[K` return to col 0 and erase to EOL (overwrites half prompt).
/// - `\x1b[2m` enable dim; banner text; `\x1b[22m` cancel dim/bold ONLY
///   (NOT `\x1b[0m` ALL-reset, which would clobber prompt-side SGR state).
/// - `\n` advance one line.
/// - `\x1b[u` (DECRC) restore cursor to the saved position.
#[allow(dead_code)] // reason: consumed in Task 7 (lib.rs wires DevTtySink).
pub(crate) const BANNER_BYTES: &[u8] =
    b"\x1b[s\r\x1b[K\x1b[2mtayf: config reloaded\x1b[22m\n\x1b[u";

/// Sink for the reload banner. Production wires `DevTtySink` (writes to
/// `/dev/tty`); tests wire `VecSink` (captures bytes in-memory for
/// assertion). `None` injection from `Tayf::run` means banner disabled.
pub(crate) trait BannerSink: Send + 'static {
    /// Write the banner bytes. Best-effort: implementations swallow I/O
    /// errors silently — the banner is a side-channel and its absence
    /// does not affect session correctness.
    fn write_banner(&mut self, bytes: &[u8]);
}

/// Production banner sink. Opens `/dev/tty` per call (no fd caching),
/// writes the bytes, flushes, closes on Drop of the local `File`.
///
/// fd-leak audit (Rev2 I-8): `File` Drop is sync `close(2)`; `notify`
/// debounce is 200 ms (`src/watch.rs:23`), so the maximum reload rate is
/// 5/s. Even sustained 5/s reloads produce 5 open/close cycles per second
/// — well below any fd-budget threshold. Caching the fd at orchestrator
/// spawn was considered and rejected: holding a `/dev/tty` fd for the
/// entire session is a small surface increase, while reload events are
/// rare enough that per-event open is trivial.
#[allow(dead_code)] // reason: consumed in Task 7 (lib.rs wires DevTtySink).
pub(crate) struct DevTtySink;

impl BannerSink for DevTtySink {
    fn write_banner(&mut self, bytes: &[u8]) {
        use std::io::Write;
        if let Ok(mut tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty") {
            // Best-effort: ignore write errors. Banner is a side-channel.
            let _ = tty.write_all(bytes);
            let _ = tty.flush();
            // `tty` dropped at scope end → close(2) syncs.
        }
    }
}

/// Test-only in-memory banner sink. Captures written bytes for assertion.
#[cfg(test)]
pub(crate) struct VecSink {
    pub(crate) bytes: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
}

#[cfg(test)]
impl BannerSink for VecSink {
    fn write_banner(&mut self, bytes: &[u8]) {
        if let Ok(mut g) = self.bytes.lock() {
            g.extend_from_slice(bytes);
        }
    }
}

/// Source of a reload trigger. Both variants route to the same
/// orchestrator code path; the variant is preserved purely for
/// diagnostic logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReloadRequest {
    /// The file watcher observed a change to the config path.
    FileChanged,
    /// SIGHUP was delivered to the tayf process.
    SignalHup,
}

use std::path::Path;
use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::error::Result;
use crate::rules::Compiled;
use crate::terminfo::ColorDepth;

/// Re-load the config from `path` (or rebuild built-ins when `None`),
/// re-compile the rule set at `depth`, and atomically store the new
/// `Arc<Compiled>` into `handle`.
///
/// On failure (parse error, regex compile error, etc.) the previous
/// `Arc<Compiled>` remains in `handle` untouched and the error is
/// returned to the caller.
///
/// # Errors
/// Returns any error surfaced by [`crate::config::load`] (with the
/// caller's explicit `path`) or [`crate::rules::Compiled::load_with_theme`].
pub(crate) fn reload_once(
    handle: &ArcSwap<Compiled>,
    path: Option<&Path>,
    theme: Option<&str>,
    depth: ColorDepth,
) -> Result<()> {
    // Load using the same resolver the initial run used. `config::load`
    // returns `Some((config, path_loaded_from))` when a file was read,
    // or `None` when nothing applies.
    let loaded = crate::config::load(path)?;
    let cfg = loaded.as_ref().map(|(c, _)| c);
    let path_str = loaded.as_ref().map(|(_, p)| p.display().to_string());

    let compiled = Compiled::load_with_theme(cfg, path_str.as_deref(), theme, depth)?;
    handle.store(Arc::new(compiled));
    Ok(())
}

use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::thread::{self, JoinHandle};

/// Owns the reload thread. Drop joins the thread; the thread exits
/// when its `Receiver` returns `Err` (sender side closed).
pub(crate) struct ReloadOrchestrator {
    handle: Option<JoinHandle<()>>,
}

impl ReloadOrchestrator {
    /// Spawn the orchestrator thread.
    ///
    /// `rules_handle` is shared with `Pipeline` (read side) and is the
    /// target of all `store` operations performed here.
    /// `config_path` is the path resolved at startup; it is re-used on
    /// every reload (we do NOT re-walk XDG fallbacks at runtime —
    /// avoids env-race surprises mid-session).
    /// `theme` is the effective theme resolved at startup (CLI `--theme`
    /// or `[general] theme`); it is re-applied on every reload so a
    /// config edit cannot silently drop the active preset.
    pub(crate) fn spawn(
        rules_handle: Arc<ArcSwap<Compiled>>,
        config_path: Option<PathBuf>,
        theme: Option<String>,
        depth: ColorDepth,
        rx: Receiver<ReloadRequest>,
        // F3 (v0.3.3): None = banner disabled; Some(sink) = enabled.
        // The sink is moved into the reload thread.
        banner_sink: Option<Box<dyn BannerSink>>,
    ) -> Self {
        // reason: thread::Builder::spawn fails only on OS resource
        // exhaustion. As with the runtime threads (see
        // src/runtime.rs::spawn_output_thread), we accept the panic
        // on that path; the TtyGuard's Drop restores the terminal
        // during unwind.
        let handle = thread::Builder::new()
            .name("tayf-reload".into())
            .spawn(move || {
                let mut banner_sink = banner_sink;
                while let Ok(req) = rx.recv() {
                    match reload_once(
                        &rules_handle,
                        config_path.as_deref(),
                        theme.as_deref(),
                        depth,
                    ) {
                        Ok(()) => {
                            crate::log::info_msg!("config reloaded ({req:?})");
                            if let Some(sink) = banner_sink.as_mut() {
                                sink.write_banner(BANNER_BYTES);
                            }
                        }
                        Err(e) => {
                            crate::log::warn_msg!(
                                "config reload failed ({req:?}): {e}; keeping previous rule set"
                            );
                            // Failure path: no banner (intentional — see spec §1.3).
                        }
                    }
                }
                // Sender side closed — exit cleanly.
            })
            .expect("reload thread must spawn");
        ReloadOrchestrator { handle: Some(handle) }
    }
}

impl Drop for ReloadOrchestrator {
    fn drop(&mut self) {
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reload_request_variants_are_distinct() {
        assert_ne!(ReloadRequest::FileChanged, ReloadRequest::SignalHup);
    }

    #[test]
    fn reload_request_is_copy() {
        // Compile-time guarantee — a copy through assignment.
        let a = ReloadRequest::FileChanged;
        let b = a;
        let _ = a; // would fail to compile if non-Copy
        assert_eq!(a, b);
    }

    use std::path::PathBuf;
    use std::sync::Arc;

    use arc_swap::ArcSwap;
    use tempfile::TempDir;

    use crate::rules::Compiled;
    use crate::terminfo::ColorDepth;

    fn write(dir: &TempDir, body: &str) -> PathBuf {
        let p = dir.path().join("config.toml");
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn reload_once_swaps_on_valid_config() {
        let handle = Arc::new(ArcSwap::from_pointee(Compiled::load_builtins().unwrap()));
        let before = handle.load_full();

        let dir = tempfile::tempdir().unwrap();
        let path = write(
            &dir,
            r#"
[[rules]]
name = "log_level"
style = { fg = "yellow", bold = true }
"#,
        );

        super::reload_once(&handle, Some(&path), None, ColorDepth::Truecolor).unwrap();

        let after = handle.load_full();
        assert!(!Arc::ptr_eq(&before, &after), "reload_once must replace the Arc on success");
    }

    #[test]
    fn reload_once_preserves_old_arc_on_parse_error() {
        let handle = Arc::new(ArcSwap::from_pointee(Compiled::load_builtins().unwrap()));
        let before = handle.load_full();

        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "this is = not valid = toml\n");

        let err = super::reload_once(&handle, Some(&path), None, ColorDepth::Truecolor)
            .expect_err("invalid toml must fail reload");
        assert!(matches!(err, crate::error::Error::Config { .. }));

        let after = handle.load_full();
        assert!(Arc::ptr_eq(&before, &after), "reload_once must NOT swap the Arc on failure");
    }

    #[test]
    fn reload_once_preserves_theme() {
        // Build a handle from no-theme defaults; reload with Some("light") and
        // verify the resulting styles match the light theme (permission becomes
        // Color::Black + dim). Regression guard against the wiring dropping the
        // theme through reload.
        use crate::rules::BUILTIN_NAMES;
        use crate::style::Color;

        let handle =
            Arc::new(ArcSwap::from_pointee(Compiled::load_builtins().expect("builtins compile")));
        super::reload_once(&handle, None, Some("light"), ColorDepth::Truecolor)
            .expect("reload with theme must succeed");

        let compiled = handle.load();
        let idx = BUILTIN_NAMES.iter().position(|n| *n == "permission").unwrap();
        assert_eq!(compiled.styles[idx].fg, Some(Color::Black));
        assert!(compiled.styles[idx].dim);
    }

    // Deliberately NO `reload_once_with_none_path_*` test here — see plan
    // rationale (the test would be host-dependent on $XDG_CONFIG_HOME and
    // $HOME). The production code path always passes Some(path) when a
    // config was loaded at startup.

    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn orchestrator_swaps_on_file_changed_event() {
        let handle = Arc::new(ArcSwap::from_pointee(Compiled::load_builtins().unwrap()));
        let before = handle.load_full();

        let dir = tempfile::tempdir().unwrap();
        let path = write(
            &dir,
            r#"
[[rules]]
name = "log_level"
style = { fg = "yellow" }
"#,
        );

        let (tx, rx) = mpsc::channel::<super::ReloadRequest>();
        let _orchestrator = super::ReloadOrchestrator::spawn(
            Arc::clone(&handle),
            Some(path.clone()),
            None,
            ColorDepth::Truecolor,
            rx,
            None, // banner_sink: disabled (v0.3.2 default behavior)
        );

        tx.send(super::ReloadRequest::FileChanged).unwrap();
        // 200ms is generous; the actual work is reading a tiny file +
        // compiling one regex.
        std::thread::sleep(Duration::from_millis(200));

        let after = handle.load_full();
        assert!(!Arc::ptr_eq(&before, &after));

        drop(tx); // lets orchestrator's recv return Err on next iteration
    }

    #[test]
    fn orchestrator_preserves_old_arc_on_bad_config() {
        let handle = Arc::new(ArcSwap::from_pointee(Compiled::load_builtins().unwrap()));
        let before = handle.load_full();

        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "broken toml = = =\n");

        let (tx, rx) = mpsc::channel::<super::ReloadRequest>();
        let _orchestrator = super::ReloadOrchestrator::spawn(
            Arc::clone(&handle),
            Some(path),
            None,
            ColorDepth::Truecolor,
            rx,
            None, // banner_sink: disabled (v0.3.2 default behavior)
        );

        tx.send(super::ReloadRequest::SignalHup).unwrap();
        std::thread::sleep(Duration::from_millis(200));

        let after = handle.load_full();
        assert!(
            Arc::ptr_eq(&before, &after),
            "orchestrator must keep the old Arc when reload_once fails"
        );

        drop(tx);
    }

    #[test]
    fn orchestrator_writes_banner_on_success_when_sink_present() {
        let handle = Arc::new(ArcSwap::from_pointee(Compiled::load_builtins().unwrap()));

        let dir = tempfile::tempdir().unwrap();
        let path = write(
            &dir,
            r#"
[[rules]]
name = "log_level"
style = { fg = "yellow" }
"#,
        );

        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let sink: Box<dyn super::BannerSink> =
            Box::new(super::VecSink { bytes: Arc::clone(&captured) });

        let (tx, rx) = mpsc::channel::<super::ReloadRequest>();
        let _orchestrator = super::ReloadOrchestrator::spawn(
            Arc::clone(&handle),
            Some(path),
            None,
            ColorDepth::Truecolor,
            rx,
            Some(sink),
        );

        tx.send(super::ReloadRequest::FileChanged).unwrap();
        // 200 ms covers reload_once + sink write under normal load.
        std::thread::sleep(Duration::from_millis(200));

        let got = captured.lock().unwrap();
        assert_eq!(
            got.as_slice(),
            super::BANNER_BYTES,
            "banner bytes must match BANNER_BYTES exactly"
        );

        drop(tx);
    }

    #[test]
    fn orchestrator_no_banner_when_sink_absent() {
        // Mirrors orchestrator_swaps_on_file_changed_event but explicit:
        // no sink → swap happens, no banner write.
        let handle = Arc::new(ArcSwap::from_pointee(Compiled::load_builtins().unwrap()));
        let before = handle.load_full();

        let dir = tempfile::tempdir().unwrap();
        let path = write(
            &dir,
            r#"
[[rules]]
name = "log_level"
style = { fg = "yellow" }
"#,
        );

        let (tx, rx) = mpsc::channel::<super::ReloadRequest>();
        let _orchestrator = super::ReloadOrchestrator::spawn(
            Arc::clone(&handle),
            Some(path),
            None,
            ColorDepth::Truecolor,
            rx,
            None,
        );

        tx.send(super::ReloadRequest::FileChanged).unwrap();
        std::thread::sleep(Duration::from_millis(200));

        let after = handle.load_full();
        assert!(!Arc::ptr_eq(&before, &after), "reload should still swap the Arc");

        drop(tx);
    }

    #[test]
    fn orchestrator_no_banner_on_reload_failure_when_sink_present() {
        let handle = Arc::new(ArcSwap::from_pointee(Compiled::load_builtins().unwrap()));

        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "broken toml = = =\n");

        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let sink: Box<dyn super::BannerSink> =
            Box::new(super::VecSink { bytes: Arc::clone(&captured) });

        let (tx, rx) = mpsc::channel::<super::ReloadRequest>();
        let _orchestrator = super::ReloadOrchestrator::spawn(
            Arc::clone(&handle),
            Some(path),
            None,
            ColorDepth::Truecolor,
            rx,
            Some(sink),
        );

        tx.send(super::ReloadRequest::SignalHup).unwrap();
        std::thread::sleep(Duration::from_millis(200));

        let got = captured.lock().unwrap();
        assert!(got.is_empty(), "failed reload must not write banner; got {} bytes", got.len());

        drop(tx);
    }
}
