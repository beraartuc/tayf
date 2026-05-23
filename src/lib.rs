//! tayf — terminal-agnostic, PTY-based, regex-driven output colorizer.
//!
//! Public entry point: [`Tayf::run`]. See
//! `docs/superpowers/specs/2026-05-21-tayf-v0.1-design.md` for the full design.

// Crate-wide policy: unsafe is permitted only with a SAFETY comment;
// reviewer enforces. See `src/terminfo.rs::winsize` (TIOCGWINSZ ioctl) and
// `src/runtime.rs::borrow_master_fd` (PTY master fd borrow for `poll(2)`)
// for the only v0.1.1 use sites.
#![warn(unsafe_code)]
#![warn(clippy::pedantic, clippy::cargo)]
// reason: `module_name_repetitions` is idiomatic for our file-per-concept layout
// (e.g., `tty_guard::TtyGuard`); `missing_errors_doc` is satisfied at the
// public-API boundary (`Tayf::run`) and tracked in the spec, not duplicated
// on every `pub(crate)` fn.
#![allow(clippy::module_name_repetitions, clippy::missing_errors_doc)]
#![allow(clippy::multiple_crate_versions)] // reason: portable-pty 0.9 still pulls thiserror 1.x (via filedescriptor) while our direct dep is on thiserror 2.x; toml 0.9 transitively pulls winnow 0.7 alongside winnow 1.0 (via toml_parser). All from upstream crates we don't control.

pub(crate) mod cli;
pub(crate) mod config;
pub(crate) mod error;
pub(crate) mod style;
pub(crate) mod version;

pub(crate) mod ansi;
pub(crate) mod bg_detect;
pub(crate) mod line_buffer;
pub(crate) mod log;
pub(crate) mod pipeline;
pub(crate) mod pty;
pub(crate) mod reload;
pub(crate) mod rules;
pub(crate) mod runtime;
pub(crate) mod shell;
pub(crate) mod signals;
pub(crate) mod terminfo;
pub(crate) mod themes;
pub(crate) mod tty_guard;
pub(crate) mod watch;

pub use cli::Args;
pub use error::{Error, Result};

/// Truthy env-var values: "1", "true", "yes" (case-insensitive). Shared by:
/// - `bg_detect::resolve` (`TAYF_DISABLE_BG_DETECT` test-only bypass, v0.3.2)
/// - `Tayf::run` (`TAYF_DISABLE` whole-binary bypass, v0.3.3)
///
/// Single-source utility — duplicated parsing would risk semantic drift
/// between the two escape hatches.
pub(crate) fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

use std::process::ExitCode;
use std::sync::Arc;

use arc_swap::ArcSwap;

/// Top-level facade. Wires logging, shell discovery, TTY guard, PTY spawn,
/// signal handling, and the I/O loop into a single entry point.
pub struct Tayf;

impl Tayf {
    /// Run tayf with the given arguments. Returns the exit code that should
    /// be propagated to the OS.
    ///
    /// # Errors
    /// Returns any [`Error`] encountered during setup or execution. Failures
    /// after the TTY guard is engaged are still surfaced; the guard's Drop
    /// restores the terminal.
    #[allow(clippy::needless_pass_by_value)]
    // reason: `Args` is the parsed CLI surface and this is the process entry
    // point; taking ownership is the conventional shape even though we
    // currently only read individual fields.
    pub fn run(args: Args) -> Result<ExitCode> {
        log::init_from_env();

        // Resolve bypass at process start; tek read garantisi (race-free).
        // CLI flag wins over env var; both default to false.
        let bypass = args.bypass || crate::env_truthy("TAYF_DISABLE");

        if bypass {
            // ============================================================
            // BYPASS BRANCH (spec §1.1 + §3.3, Rev2 C-3 enumeration)
            //
            // RUNS:    shell::discover, tty_guard::TtyGuard::engage,
            //          pty::PtySession::spawn, signals::spawn_handler
            //          (with reload_tx = None), runtime::run
            //          (apply_colors = false).
            //
            // SKIPS:   terminfo::detect_depth, terminfo::stdout_is_tty,
            //          config::load, bg_detect::resolve, theme resolution,
            //          rules::Compiled::load_with_theme,
            //          watch::ConfigWatcher::spawn,
            //          reload::ReloadOrchestrator::spawn.
            //
            // Pipeline IS constructed inside runtime::run (Rev2 C-2),
            // but apply_colors=false short-circuits all feed/tick/drain
            // calls in the output thread. Compiled::empty() satisfies
            // the Arc<ArcSwap<Compiled>> signature with a structurally
            // valid (but never iterated) empty rule set.
            // ============================================================

            // Rev2 I-10: diagnostic so users debugging with TAYF_LOG=info
            // see why no colorization is happening.
            crate::log::info_msg!(
                "bypass active (CLI={}, TAYF_DISABLE={:?})",
                args.bypass,
                std::env::var("TAYF_DISABLE").ok()
            );

            let spec = shell::discover(args.shell.as_deref(), args.login)?;
            let empty_rules: Arc<ArcSwap<rules::Compiled>> =
                Arc::new(ArcSwap::from_pointee(rules::Compiled::empty()));

            let guard = tty_guard::TtyGuard::engage()?;
            let session = pty::PtySession::spawn(&spec)?;
            let (reader, writer, resizer, child) = session.into_parts()?;
            let child_pid = child.pid();

            // No reload pipeline in bypass — pass None to signal thread.
            // SIGHUP is still forwarded to the child PG (signals.rs
            // handler body, v0.3.3 F2-b fix).
            let signal_guard = signals::spawn_handler(resizer, child_pid, None)?;

            let exit_code = runtime::run(reader, writer, child, Arc::clone(&empty_rules), false)?;

            // Explicit ordered shutdown for symmetry with the non-bypass
            // branch: signal_guard joined first, tty_guard restored last.
            drop(signal_guard);
            drop(guard);

            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            // reason: exit_code & 0xff is structurally in 0..=255
            let code = (exit_code & 0xff) as u8;
            return Ok(ExitCode::from(code));
        }

        // ================================================================
        // NON-BYPASS BRANCH (v0.3.2 flow + --no-hot-reload gating)
        // ================================================================

        let depth = terminfo::detect_depth();
        let apply_colors =
            !args.no_color && terminfo::stdout_is_tty() && depth != terminfo::ColorDepth::None;
        let effective_depth = if apply_colors { depth } else { terminfo::ColorDepth::None };

        let spec = shell::discover(args.shell.as_deref(), args.login)?;

        let loaded = config::load(args.config.as_deref())?;
        let config_ref = loaded.as_ref().map(|(c, _)| c);
        let config_path: Option<String> = loaded.as_ref().map(|(_, p)| p.display().to_string());

        let explicit_theme: Option<String> =
            args.theme.clone().or_else(|| config_ref.and_then(|c| c.general.theme.clone()));

        let effective_theme: Option<String> = explicit_theme.or_else(|| {
            if apply_colors {
                Some(bg_detect::resolve().as_theme_name().to_owned())
            } else {
                None
            }
        });

        let compiled = rules::Compiled::load_with_theme(
            config_ref,
            config_path.as_deref(),
            effective_theme.as_deref(),
            effective_depth,
        )?;
        let rules: Arc<ArcSwap<rules::Compiled>> = Arc::new(ArcSwap::from_pointee(compiled));

        // F2: --no-hot-reload gates BOTH the file watcher AND the reload
        // orchestrator. Channel still constructed; receiver dropped on
        // the else branch so no orphan rx lives past the decision point.
        let hot_reload_enabled = !args.no_hot_reload;

        // Snapshot the banner-opt-in flag while config_ref is still live.
        let show_reload_banner = config_ref.is_some_and(|c| c.general.show_reload_banner);

        let (reload_tx, reload_rx) = std::sync::mpsc::channel::<reload::ReloadRequest>();

        let guard = tty_guard::TtyGuard::engage()?;

        let session = pty::PtySession::spawn(&spec)?;
        let (reader, writer, resizer, child) = session.into_parts()?;
        let child_pid = child.pid();

        // F2: signal thread always receives child_pid; reload_tx only
        // when hot-reload is wired. The SIGHUP handler in signals.rs
        // forwards to the child PG unconditionally (v0.3.3 F2-b), and
        // additionally sends to reload_tx when Some.
        let signal_reload_tx = if hot_reload_enabled { Some(reload_tx.clone()) } else { None };
        let signal_guard = signals::spawn_handler(resizer, child_pid, signal_reload_tx)?;

        // Spawn the file watcher only when (a) a config file was loaded
        // AND (b) hot-reload is enabled. Either condition false → no
        // watcher, and the corresponding reload_tx clone is never created.
        let watcher = if hot_reload_enabled {
            loaded
                .as_ref()
                .map(|(_, p)| watch::ConfigWatcher::spawn(p, reload_tx.clone()))
                .transpose()?
        } else {
            None
        };

        // Orchestrator: spawned only when hot-reload is enabled. Drop
        // ordering (mirrors lib.rs:152-156 invariant): orchestrator is
        // declared LAST so that on any earlier `?` returning Err, no
        // orchestrator exists → no join-deadlock where its Drop blocks
        // on a channel still held by already-spawned signal/watcher
        // threads.
        let _orchestrator = if hot_reload_enabled {
            Some(reload::ReloadOrchestrator::spawn(
                Arc::clone(&rules),
                loaded.as_ref().map(|(_, p)| p.clone()),
                effective_theme.clone(),
                effective_depth,
                reload_rx,
                // F3: inject banner sink; production = DevTtySink when
                // banner enabled, None when disabled.
                if show_reload_banner {
                    Some(Box::new(reload::DevTtySink) as Box<dyn reload::BannerSink>)
                } else {
                    None
                },
            ))
        } else {
            // Hot-reload off: receiver is never consumed. Drop it
            // explicitly so the channel reaches zero-receivers
            // immediately; any stray future `reload_tx.send(...)`
            // returns Err (intended). Note: there are no clones live —
            // signal_reload_tx was None and watcher was not spawned,
            // so dropping `reload_rx` here is safe and explicit.
            drop(reload_rx);
            None
        };

        // Drop the local sender now (mirrors lib.rs:171). Remaining
        // clones live in signal_guard (when hot_reload_enabled) and
        // watcher (when both hot_reload_enabled and config present).
        // When BOTH guards Drop at shutdown their clones go away, the
        // orchestrator's recv() returns Err, and its loop exits cleanly.
        // When hot_reload disabled, reload_tx here is the only sender
        // (none was cloned to signal thread); dropping is trivially safe
        // — receiver is already gone.
        drop(reload_tx);

        let exit_code = runtime::run(reader, writer, child, Arc::clone(&rules), apply_colors)?;

        // Explicit ordered shutdown (mirrors lib.rs:175-184 invariant):
        // - watcher first: closes notify's raw_tx, which lets the
        //   debounce thread observe Disconnected and exit, which drops
        //   the debounce thread's reload_tx clone.
        // - signal_guard next: closes signal_hook iterator, joins
        //   signal thread, which drops the signal thread's reload_tx
        //   clone (if any).
        // - implicit _orchestrator drop at end of scope: recv() returns
        //   Err (all senders gone), loop exits, join completes.
        // - guard last: explicit for ordering clarity; Drop restores
        //   termios.
        drop(watcher);
        drop(signal_guard);

        drop(guard); // explicit; Drop would handle it but make ordering clear

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // reason: exit_code & 0xff is structurally in 0..=255
        let code = (exit_code & 0xff) as u8;
        Ok(ExitCode::from(code))
    }
}

/// Bench-only adapters around `pub(crate)` internals so the `benches/`
/// crate (an external crate from rustc's perspective) can drive the hot
/// path directly.
///
/// Not part of the public API — hidden from rustdoc, no stability
/// guarantees, may change or vanish between minor releases. See
/// `benches/throughput.rs`.
#[doc(hidden)]
pub mod __bench__ {
    use std::io::Write;

    /// Opaque newtype carrying the compiled built-in rule set. Constructed
    /// via [`load_builtin_rules`] and passed back into [`apply_rules`].
    pub struct CompiledRules(std::sync::Arc<arc_swap::ArcSwap<crate::rules::Compiled>>);

    /// Compile the v0.1 built-in rule set (same path the production runtime
    /// uses). See `src/rules.rs::Compiled::load_builtins`.
    ///
    /// # Errors
    /// Returns [`crate::Error::RegexCompile`] if any built-in pattern fails
    /// to compile. In practice this never fires — the patterns are tested.
    pub fn load_builtin_rules() -> crate::Result<CompiledRules> {
        crate::rules::Compiled::load_builtins()
            .map(|c| CompiledRules(std::sync::Arc::new(arc_swap::ArcSwap::from_pointee(c))))
    }

    /// Run the per-line rule scanner against `line`, emitting the SGR-wrapped
    /// output to `out`. Mirrors `src/pipeline.rs::apply_rules`.
    ///
    /// # Errors
    /// Forwards any `std::io::Error` produced by `out`.
    pub fn apply_rules<W: Write>(
        line: &[u8],
        rules: &CompiledRules,
        out: &mut W,
    ) -> std::io::Result<()> {
        crate::pipeline::apply_rules(line, &rules.0, out)
    }
}
