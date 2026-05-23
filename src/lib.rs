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

        // Detect terminal capabilities up-front so we can both gate the
        // runtime's no-color fast path AND pre-bake colors into the rule
        // set at their final depth.
        //
        // Note: the `depth != None` short-circuit here is for *performance*
        // — `runtime::run` uses `apply_colors == false` to bypass the
        // `Pipeline` entirely and stream raw PTY bytes to stdout. The
        // pre-bake (`Compiled::load_with_theme(_, _, _, ColorDepth::None)`) already
        // guarantees empty SGR output for correctness, so a future
        // maintainer simplifying this gate should preserve the runtime
        // bypass separately, not just delete the depth check here.
        let depth = terminfo::detect_depth();
        let apply_colors =
            !args.no_color && terminfo::stdout_is_tty() && depth != terminfo::ColorDepth::None;
        let effective_depth = if apply_colors { depth } else { terminfo::ColorDepth::None };

        let spec = shell::discover(args.shell.as_deref(), args.login)?;

        // Load TOML config before engaging the TTY guard — failures here
        // produce friendly stderr output without leaving the terminal in
        // raw mode. `config::load` returns the path it loaded from so we
        // can thread it into user-rule error messages without a second
        // resolve (which would race on $HOME / $XDG_CONFIG_HOME).
        let loaded = config::load(args.config.as_deref())?;
        let config_ref = loaded.as_ref().map(|(c, _)| c);
        let config_path: Option<String> = loaded.as_ref().map(|(_, p)| p.display().to_string());

        // Resolve the effective theme: CLI `--theme` wins over `[general] theme`;
        // both may be absent. Threaded through compile + reload so config edits
        // don't silently drop the active theme.
        let explicit_theme: Option<String> =
            args.theme.clone().or_else(|| config_ref.and_then(|c| c.general.theme.clone()));

        // Resolve background theme automatically when the user hasn't pinned one
        // AND we're going to emit color. Skips when:
        // - explicit CLI `--theme` or config `[general] theme` set
        // - `--no-color`, non-TTY stdout, or `TERM=dumb` (apply_colors == false)
        // Spec §3.6.
        let effective_theme: Option<String> = explicit_theme.or_else(|| {
            if apply_colors {
                Some(bg_detect::resolve().as_theme_name().to_owned())
            } else {
                None
            }
        });

        // Compile rules BEFORE engaging the TTY guard. Rule validation
        // (missing pattern, missing style, bad regex, duplicate names,
        // invalid color) lives inside `Compiled::load_with_theme` via
        // `config::apply_user_rules`; surfacing those errors before the
        // guard keeps the terminal in cooked mode and lets `Command::output`
        // callers (integration tests) observe exit code 64 cleanly.
        let compiled = rules::Compiled::load_with_theme(
            config_ref,
            config_path.as_deref(),
            effective_theme.as_deref(),
            effective_depth,
        )?;
        let rules: Arc<ArcSwap<rules::Compiled>> = Arc::new(ArcSwap::from_pointee(compiled));

        // The reload channel: senders go to the signal thread (always)
        // and the file-watcher debounce thread (when a config exists).
        // Receiver is moved into the orchestrator below. Both ends are
        // owned here until the orchestrator spawns; if any `?` returns
        // before that point, both ends drop with no observable effect
        // (no thread holds them yet).
        let (reload_tx, reload_rx) = std::sync::mpsc::channel::<reload::ReloadRequest>();

        let guard = tty_guard::TtyGuard::engage()?;

        let session = pty::PtySession::spawn(&spec)?;
        let (reader, writer, resizer, child) = session.into_parts()?;
        let child_pid = child.pid();

        let signal_guard = signals::spawn_handler(resizer, child_pid, Some(reload_tx.clone()))?;

        // Spawn the file watcher only when a config file was loaded.
        // Per spec §8 question 2: absent config → no watcher; SIGHUP
        // can still trigger a reload that re-resolves the config path.
        let watcher = loaded
            .as_ref()
            .map(|(_, p)| watch::ConfigWatcher::spawn(p, reload_tx.clone()))
            .transpose()?;

        // Orchestrator is declared LAST among the threading scaffolding.
        // If any `?` above had returned `Err`, no orchestrator would
        // have been spawned — preventing the join-deadlock where the
        // orchestrator's Drop blocks on a channel still holding live
        // senders in already-spawned signal/watcher threads.
        let _orchestrator = reload::ReloadOrchestrator::spawn(
            Arc::clone(&rules),
            loaded.as_ref().map(|(_, p)| p.clone()),
            effective_theme.clone(),
            effective_depth,
            reload_rx,
        );

        // Drop the local sender now. The only remaining `reload_tx`
        // clones live in the signal thread and (when present) the
        // watcher thread. When BOTH threads exit at shutdown, the
        // orchestrator's `recv()` returns Err and the reload thread
        // exits cleanly. Without this drop, the orchestrator's
        // channel would never reach a zero-sender state.
        drop(reload_tx);

        let exit_code = runtime::run(reader, writer, child, Arc::clone(&rules), apply_colors)?;

        // Explicit ordered shutdown — drop the watcher and signal
        // guard BEFORE the implicit `_orchestrator` drop at end of
        // scope. Each of those drops joins its thread, which in turn
        // drops the thread's `reload_tx` clone. Once both clones are
        // gone, `recv()` in the reload thread returns Err and its
        // loop exits, so the orchestrator's `Drop`-time `join()` is
        // unblocked.
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
