//! Hot-reload orchestrator.
//!
//! Receives [`ReloadRequest`] events from the file watcher and the
//! signal thread, re-reads the config, re-compiles the rule set, and
//! atomically stores the new compiled set into the shared
//! `Arc<ArcSwap<Compiled>>`. Parse/compile failures preserve the
//! previous rule set and emit a `warn_msg!` to stderr.
//!
//! See `ARCHITECTURE.md` for the hot-reload design.

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
use std::time::SystemTime;

use arc_swap::ArcSwap;

use crate::error::Result;
use crate::rules::Compiled;
use crate::terminfo::ColorDepth;

/// Re-load the config from `path` (or rebuild built-ins when `None`),
/// re-resolve the FULL precedence chain (profile + theme), re-compile
/// the rule set at `depth`, and atomically store the new
/// `Arc<Compiled>` into `handle`.
///
/// `theme` and `profile` are CLI snapshots taken at startup
/// (`--theme` and `--profile`). `bg_default` is the bg-detect result
/// resolved once at startup (or `None` when colors are disabled);
/// re-using the startup snapshot avoids querying the terminal on
/// every reload (OSC 11 latency / flicker) while keeping bg-detect's
/// fallback role intact. On every reload the precedence chain is
/// re-resolved:
///
/// 1. Re-read user-config (via [`crate::config::load`]).
/// 2. Effective profile = CLI snapshot OR `config.general.profile`.
/// 3. Load profile (if any) via [`crate::profiles::load`].
/// 4. Effective theme = CLI snapshot OR `config.general.theme` OR
///    `profile.theme` OR `bg_default`.
/// 5. Compile + atomic swap.
///
/// On failure (parse error, regex compile error, profile load
/// failure, etc.) the previous `Arc<Compiled>` remains in `handle`
/// untouched and the error is returned to the caller. The reload
/// thread's loop handles surfacing the error via `warn_msg!` and
/// suppressing the banner.
///
/// # Errors
/// Returns any error surfaced by [`crate::config::load`],
/// [`crate::profiles::load`], or
/// [`crate::rules::Compiled::load_with_theme`].
pub(crate) fn reload_once(
    handle: &ArcSwap<Compiled>,
    path: Option<&Path>,
    theme: Option<&str>,
    profile: Option<&str>,
    bg_default: Option<&str>,
    depth: ColorDepth,
) -> Result<()> {
    // 1. Re-read user-config using the same resolver the initial run
    //    used. `config::load` returns `Some((config, path_loaded_from))`
    //    when a file was read, or `None` when nothing applies.
    let loaded = crate::config::load(path)?;
    let cfg = loaded.as_ref().map(|(c, _)| c);
    let path_str = loaded.as_ref().map(|(_, p)| p.display().to_string());

    // 2. Effective profile name: CLI snapshot > config.
    let effective_profile_name: Option<&str> =
        profile.or_else(|| cfg.and_then(|c| c.general.profile.as_deref()));

    // 3. Resolve the active rule set + (for theme precedence) the profile's
    //    theme via the shared `profiles::resolve_active` helper. A named
    //    profile's `[[rules]]` REPLACE config.toml's; the built-ins remain the
    //    substrate; `[general]` always comes from config.toml. Failure
    //    propagates → reload thread warns + retains the prior valid Compiled.
    //    `resolve_active` clones the caller's `[general]`; pass the re-read
    //    config (or an empty default when nothing applied) as the base.
    let resolve_base = crate::config::Config {
        general: cfg.map(|c| c.general.clone()).unwrap_or_default(),
        rules: cfg.map(|c| c.rules.clone()).unwrap_or_default(),
    };
    let (effective_config, profile_path, rules_source, profile_theme) =
        crate::profiles::resolve_active(&resolve_base, effective_profile_name)?;
    // `resolve_active` returns `None` for the path on the no-profile path; the
    // active diagnostics path is then the re-read user-config path.
    let active_path = profile_path.or(path_str);

    // 4. Effective theme: CLI snapshot > config > profile.theme >
    //    bg-detect default (startup snapshot — see fn-level docs).
    let effective_theme: Option<&str> = theme
        .or_else(|| cfg.and_then(|c| c.general.theme.as_deref()))
        .or(profile_theme.as_deref())
        .or(bg_default);

    // 5. Compile + atomic swap.
    let compiled = Compiled::load_with_theme(
        Some(&effective_config),
        active_path.as_deref(),
        effective_theme,
        rules_source,
        depth,
    )?;
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
    /// `theme` is the CLI `--theme` snapshot taken at startup. It is
    /// re-applied on every reload so a config edit cannot silently
    /// drop the active CLI override.
    /// `profile` is the CLI `--profile` snapshot taken at startup
    /// (v0.5.2). Like `theme` it is a one-shot startup override that
    /// every reload re-evaluates against the current config.
    /// `bg_default` is the bg-detect result snapshotted at startup —
    /// used as the last-resort theme fallback in [`reload_once`] when
    /// every higher-precedence source resolves to `None`. We do not
    /// re-query the terminal on hot reload; the startup result is
    /// authoritative for the session.
    // reason: the parameter set mirrors the precedence chain
    // (`config_path`, `theme`, `profile`, `bg_default`) plus
    // structural plumbing (`rules_handle`, `depth`, `rx`,
    // `banner_sink`). Each is a load-bearing dimension of the reload
    // contract; collapsing into a struct would obscure the call-site
    // alignment with the spec's precedence rules.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn(
        rules_handle: Arc<ArcSwap<Compiled>>,
        config_path: Option<PathBuf>,
        theme: Option<String>,
        profile: Option<String>,
        bg_default: Option<String>,
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
                // v0.5.4 — additive side-effect sink. Constructed from
                // config_path's parent dir (the cfg base). Disabled
                // gracefully when no config file is loaded.
                let logger: Option<ReloadLogger> =
                    config_path.as_deref().and_then(|p| p.parent()).map(ReloadLogger::new);
                let mut reload_count: u64 = 0;
                while let Ok(req) = rx.recv() {
                    reload_count += 1;
                    let outcome_ts = SystemTime::now();
                    match reload_once(
                        &rules_handle,
                        config_path.as_deref(),
                        theme.as_deref(),
                        profile.as_deref(),
                        bg_default.as_deref(),
                        depth,
                    ) {
                        Ok(()) => {
                            crate::log::info_msg!("config reloaded ({req:?})");
                            if let Some(sink) = banner_sink.as_mut() {
                                sink.write_banner(BANNER_BYTES);
                            }
                            if let Some(l) = logger.as_ref() {
                                l.append(&ReloadEvent {
                                    timestamp: outcome_ts,
                                    reload_count,
                                    outcome: ReloadOutcome::Ok,
                                });
                            }
                        }
                        Err(e) => {
                            crate::log::warn_msg!(
                                "config reload failed ({req:?}): {e}; keeping previous rule set"
                            );
                            if let Some(l) = logger.as_ref() {
                                l.append(&ReloadEvent {
                                    timestamp: outcome_ts,
                                    reload_count,
                                    outcome: ReloadOutcome::Err(e.to_string()),
                                });
                            }
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

// ============================================================
// ReloadLogger — v0.5.4 additive plumbing (spec §8.5)
//
// Invariant (memory `feedback_reload_precedence_snapshot` + I-4
// fold): adding the logger does NOT change any precedence-chain
// snapshot. The reload thread continues snapshotting theme /
// profile / bg_default at startup exactly as today; the logger
// is purely an additive side-effect sink on the post-reload
// decision path.
// ============================================================

use std::fs;
use std::io::Write;
use std::sync::Mutex;

/// Append-only event log written to `<state_dir>/runtime/reload.log`.
/// Best-effort throughout — file-system errors are silently swallowed;
/// the reload thread NEVER blocks on logger I/O.
pub(crate) struct ReloadLogger {
    /// `<cfg_dir>/runtime` — created at construction time.
    state_dir: std::path::PathBuf,
    /// `false` when `create_dir_all` failed at construction; all
    /// subsequent `append` calls become no-ops.
    pub(crate) enabled: bool,
    /// Tracks the last file size for which the 1 MB warn fired so we
    /// emit it at most once total (when the file first crosses 1 MB).
    /// I-7 fold. v0.6+ rotation will reset this on rotate.
    last_warned_size: Mutex<Option<u64>>,
}

/// One reload event row. Serialized as
/// `epoch-ms=<ts> reload #<count> <outcome>\n`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReloadEvent {
    pub(crate) timestamp: SystemTime,
    pub(crate) reload_count: u64,
    pub(crate) outcome: ReloadOutcome,
}

/// Outcome of a reload attempt — `ok` or `err: <reason>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReloadOutcome {
    Ok,
    Err(String),
}

impl ReloadOutcome {
    fn render(&self) -> String {
        match self {
            Self::Ok => "ok".to_owned(),
            Self::Err(reason) => format!("err: {reason}"),
        }
    }
}

impl ReloadEvent {
    /// RFC3339-ish timestamp for human reading. Uses millisecond
    /// precision; falls back to `epoch-ms=0` on pre-epoch input
    /// (practically unreachable in production).
    fn ts_rfc3339(&self) -> String {
        ts_rfc3339_for_event(self.timestamp)
    }
}

/// Per-event epoch-ms timestamp helper. Pure fn for test fixture pinning.
///
/// We deliberately avoid pulling chrono — milliseconds since `UNIX_EPOCH`
/// suffice for the v0.5.4 reload log audience (operators tailing the
/// file). Format: `epoch-ms=<u128>` keeps it parser-friendly without a
/// date library; a future v0.6+ can switch to chrono if demand.
fn ts_rfc3339_for_event(t: SystemTime) -> String {
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => format!("epoch-ms={}", d.as_millis()),
        Err(_) => "epoch-ms=0".to_owned(),
    }
}

/// Size at which a one-shot warn fires (I-7 fold).
const RELOAD_LOG_WARN_BYTES: u64 = 1024 * 1024; // 1 MB

impl ReloadLogger {
    /// Construct a logger rooted at `cfg_dir`. Creates
    /// `<cfg_dir>/runtime/` on first use; sets `enabled = false`
    /// if creation fails (graceful degradation).
    pub(crate) fn new(cfg_dir: &std::path::Path) -> Self {
        let state_dir = cfg_dir.join("runtime");
        let enabled = fs::create_dir_all(&state_dir).is_ok();
        Self { state_dir, enabled, last_warned_size: Mutex::new(None) }
    }

    /// Best-effort append. Logger writes are NEVER allowed to block
    /// the reload thread; every error is swallowed.
    pub(crate) fn append(&self, event: &ReloadEvent) {
        if !self.enabled {
            return;
        }
        let log_path = self.state_dir.join("reload.log");
        let line = format!(
            "{} reload #{} {}\n",
            event.ts_rfc3339(),
            event.reload_count,
            event.outcome.render(),
        );
        // POSIX O_APPEND atomicity (I-7 fold): on a regular file,
        // each write(2) atomically seeks to EOF and writes — no size
        // ceiling applies (PIPE_BUF is the pipe/FIFO bound, not the
        // regular-file bound). Rust's `OpenOptions::append(true)` maps
        // to `O_APPEND | O_WRONLY` on Unix. Concurrent writers from
        // separate threads therefore never interleave bytes within a
        // single write call. Our lines are ~50-100 bytes — one write
        // each, no torn-line risk.
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&log_path) {
            let _ = f.write_all(line.as_bytes());
        }
        self.check_size_threshold(&log_path);
    }

    /// I-7 fold: per-append `stat(2)` to detect 1 MB growth.
    /// Cost ~50 ns; reload events are rare (minutes-to-hours apart).
    fn check_size_threshold(&self, log_path: &std::path::Path) {
        let size = match fs::metadata(log_path) {
            Ok(m) => m.len(),
            Err(_) => return,
        };
        if size < RELOAD_LOG_WARN_BYTES {
            return;
        }
        let Ok(mut last) = self.last_warned_size.lock() else {
            return; // poisoned — swallow
        };
        let already_warned_for_this_band =
            last.map(|prev| prev >= RELOAD_LOG_WARN_BYTES).unwrap_or(false);
        if !already_warned_for_this_band {
            crate::log::warn_msg!(
                "reload.log exceeded 1 MB ({size} bytes) at {}; v0.6+ adds rotation",
                log_path.display()
            );
            *last = Some(size);
        }
    }
}

/// Read the last `n` parseable lines of `<state_dir>/reload.log`.
/// Returned in **reverse-chronological order** (most recent first).
/// Returns an empty Vec if the log doesn't exist (no wrapper has run yet).
/// Malformed lines are silently skipped.
pub(crate) fn read_recent_events(state_dir: &std::path::Path, n: usize) -> Vec<ReloadEvent> {
    let log_path = state_dir.join("reload.log");
    let Ok(content) = fs::read_to_string(&log_path) else {
        return Vec::new();
    };
    content.lines().rev().take(n).filter_map(parse_log_line).collect()
}

/// Parse one log line back to a `ReloadEvent`. Returns `None` on any
/// malformed shape — defensive against partial-write or corruption.
fn parse_log_line(line: &str) -> Option<ReloadEvent> {
    // Shape: `epoch-ms=<u128> reload #<count> <outcome>`
    let rest = line.strip_prefix("epoch-ms=")?;
    let (ts_str, rest) = rest.split_once(' ')?;
    let ts_ms: u128 = ts_str.parse().ok()?;
    let rest = rest.strip_prefix("reload #")?;
    let (count_str, outcome_str) = rest.split_once(' ')?;
    let reload_count: u64 = count_str.parse().ok()?;
    let outcome = if outcome_str == "ok" {
        ReloadOutcome::Ok
    } else if let Some(reason) = outcome_str.strip_prefix("err: ") {
        ReloadOutcome::Err(reason.to_owned())
    } else {
        return None;
    };
    let timestamp =
        SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(u64::try_from(ts_ms).ok()?);
    Some(ReloadEvent { timestamp, reload_count, outcome })
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

        super::reload_once(&handle, Some(&path), None, None, None, ColorDepth::Truecolor).unwrap();

        let after = handle.load_full();
        assert!(!Arc::ptr_eq(&before, &after), "reload_once must replace the Arc on success");
    }

    #[test]
    fn reload_once_preserves_old_arc_on_parse_error() {
        let handle = Arc::new(ArcSwap::from_pointee(Compiled::load_builtins().unwrap()));
        let before = handle.load_full();

        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "this is = not valid = toml\n");

        let err = super::reload_once(&handle, Some(&path), None, None, None, ColorDepth::Truecolor)
            .expect_err("invalid toml must fail reload");
        assert!(matches!(err, crate::error::Error::Config { .. }));

        let after = handle.load_full();
        assert!(Arc::ptr_eq(&before, &after), "reload_once must NOT swap the Arc on failure");
    }

    #[test]
    fn reload_once_preserves_theme() {
        // Build a handle from no-theme defaults; reload with Some("light") and
        // verify the resulting styles match the light theme (permission becomes
        // Neon-light slate #475569). Regression guard against the wiring
        // dropping the theme through reload.
        use crate::rules::BUILTIN_NAMES;
        use crate::style::Color;

        // Hermetic: pass an explicit empty temp config so `config::load` does
        // NOT read the developer's real ~/.config/tayf/config.toml. A user
        // `permission` override there beats the theme (correct precedence:
        // user config > theme), which would make this test host-dependent —
        // exactly the trap the no-`None`-path note below warns about.
        let dir = tempfile::tempdir().expect("tmpdir");
        let cfg = write(&dir, "# no user rules\n");

        let handle =
            Arc::new(ArcSwap::from_pointee(Compiled::load_builtins().expect("builtins compile")));
        super::reload_once(&handle, Some(&cfg), Some("light"), None, None, ColorDepth::Truecolor)
            .expect("reload with theme must succeed");

        let compiled = handle.load();
        let idx = BUILTIN_NAMES.iter().position(|n| *n == "permission").unwrap();
        assert_eq!(compiled.styles[idx].fg, Some(Color::Rgb(0x47, 0x55, 0x69)));
        assert!(!compiled.styles[idx].dim, "permission must not be dim in Neon-light theme");
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
            None,
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
            None,
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
            None,
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
            None,
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
            None,
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

    #[test]
    fn reload_logger_create_writes_runtime_dir_and_appends_line() {
        use std::time::SystemTime;
        let tmpdir = tempfile::tempdir().expect("tmpdir");
        let logger = super::ReloadLogger::new(tmpdir.path());
        assert!(logger.enabled, "logger must enable when state_dir create succeeds");

        let event = super::ReloadEvent {
            timestamp: SystemTime::UNIX_EPOCH,
            reload_count: 1,
            outcome: super::ReloadOutcome::Ok,
        };
        logger.append(&event);

        let log_path = tmpdir.path().join("runtime").join("reload.log");
        let body = std::fs::read_to_string(&log_path).expect("reload.log must exist");
        // Timestamp is UNIX_EPOCH → deterministic `epoch-ms=0`; pin
        // the full line shape rather than loose substrings (per
        // memory `feedback_test_assertion_specificity`).
        assert_eq!(
            body, "epoch-ms=0 reload #1 ok\n",
            "appended line must be byte-identical; got: {body:?}"
        );
    }

    #[test]
    fn reload_logger_disabled_when_state_dir_create_fails() {
        use std::time::SystemTime;
        let tmp = tempfile::tempdir().expect("tmpdir");
        let blocker = tmp.path().join("blocker");
        std::fs::write(&blocker, b"i am a file, not a dir").unwrap();
        let logger = super::ReloadLogger::new(&blocker); // blocker/runtime → fails
        assert!(!logger.enabled, "logger must disable when state_dir create fails");
        let event = super::ReloadEvent {
            timestamp: SystemTime::UNIX_EPOCH,
            reload_count: 1,
            outcome: super::ReloadOutcome::Ok,
        };
        logger.append(&event); // no-op
        assert!(!blocker.join("runtime").join("reload.log").exists());
    }

    #[test]
    fn read_recent_events_parses_appended_lines() {
        use std::time::SystemTime;
        let tmp = tempfile::tempdir().expect("tmpdir");
        let logger = super::ReloadLogger::new(tmp.path());
        for i in 1..=5u64 {
            logger.append(&super::ReloadEvent {
                timestamp: SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(i * 100),
                reload_count: i,
                outcome: if i % 2 == 0 {
                    super::ReloadOutcome::Ok
                } else {
                    super::ReloadOutcome::Err(format!("synthetic #{i}"))
                },
            });
        }
        let state_dir = tmp.path().join("runtime");
        let events = super::read_recent_events(&state_dir, 3);
        assert_eq!(events.len(), 3, "asked for 3, got {}", events.len());
        assert_eq!(events[0].reload_count, 5);
        assert_eq!(events[1].reload_count, 4);
        assert_eq!(events[2].reload_count, 3);
    }

    #[test]
    fn read_recent_events_skips_malformed_lines_silently() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let state_dir = tmp.path().join("runtime");
        std::fs::create_dir_all(&state_dir).unwrap();
        let log_path = state_dir.join("reload.log");
        std::fs::write(
            &log_path,
            "epoch-ms=100 reload #1 ok\nGARBAGE GARBAGE GARBAGE\nepoch-ms=200 reload #2 ok\n",
        )
        .unwrap();
        let events = super::read_recent_events(&state_dir, 10);
        assert_eq!(events.len(), 2, "malformed line must be skipped, leaving 2");
    }

    #[test]
    fn concurrent_appends_do_not_interleave_within_line() {
        use std::sync::Arc;
        use std::thread;
        use std::time::SystemTime;
        let tmp = tempfile::tempdir().expect("tmpdir");
        let logger_a = Arc::new(super::ReloadLogger::new(tmp.path()));
        let logger_b = Arc::clone(&logger_a);
        let t1 = thread::spawn(move || {
            for i in 0..100u64 {
                logger_a.append(&super::ReloadEvent {
                    timestamp: SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(i),
                    reload_count: 1000 + i,
                    outcome: super::ReloadOutcome::Ok,
                });
            }
        });
        let t2 = thread::spawn(move || {
            for i in 0..100u64 {
                logger_b.append(&super::ReloadEvent {
                    timestamp: SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(i),
                    reload_count: 2000 + i,
                    outcome: super::ReloadOutcome::Ok,
                });
            }
        });
        t1.join().unwrap();
        t2.join().unwrap();
        let state_dir = tmp.path().join("runtime");
        let events = super::read_recent_events(&state_dir, 1000);
        assert_eq!(
            events.len(),
            200,
            "expected 200 well-formed lines (100+100); torn writes would yield fewer"
        );
    }

    #[test]
    fn reload_logger_does_not_affect_precedence_snapshot() {
        // I-4 fold + memory feedback_reload_precedence_snapshot:
        // adding the logger must NOT change any v0.2.1/v0.5.1/v0.5.2
        // precedence-chain snapshot. The orchestrator continues
        // taking the same (theme, profile, bg_default) snapshot
        // parameters; this test pins the orchestrator-construction
        // signature against silent regression.
        use std::sync::mpsc::channel;
        let rules: Arc<ArcSwap<Compiled>> =
            Arc::new(ArcSwap::from_pointee(Compiled::load_builtins().expect("builtins compile")));
        let (tx, rx) = channel();
        let orch = super::ReloadOrchestrator::spawn(
            Arc::clone(&rules),
            None,
            Some("light".to_owned()), // theme snapshot
            None,                     // profile snapshot
            Some("dark".to_owned()),  // bg_default snapshot
            ColorDepth::Truecolor,
            rx,
            None,
        );
        drop(tx); // close sender → orchestrator recv returns Err → thread exits
        drop(orch); // Drop impl joins the thread — guarantees the Arc clone is released
        assert_eq!(Arc::strong_count(&rules), 1);
    }
}
