//! Interactive `tayf config` TUI + non-interactive `dump` / `status`
//! sub-subcommands. v0.5.4 entry point.
//!
//! Module layout (see `docs/superpowers/specs/2026-05-26-tayf-v0.5.4-config-tui.md` §5.1):
//! - [`app`] — App state struct + Tab enum + edit-mode FSM.
//! - [`events`] — crossterm event loop + key dispatch + debounce tick.
//! - [`render`] — frame composition (Layout split).
//! - [`tabs`] — per-tab dispatch (patterns / themes / profiles / status).
//! - [`widgets`] — color_picker, save_diff, preview overlays.
//! - [`edit`] — PendingEdits aggregator + RuleEdit / StyleKey.
//! - [`save`] — atomic write + backup rotation (top-level entry into [`reconcile`] for the toml_edit walk).
//! - [`reconcile`] — PendingEdits → DocumentMut walk + serialize.
//! - [`snapshot`] — ConfigSnapshot: disk read + SHA256 + DocumentMut.
//! - [`debounce`] — 200 ms debouncer for live preview recompile.
//! - [`dump_cmd`] — `tayf config dump` impl (no ratatui).
//! - [`status_cmd`] — `tayf config status` impl (no ratatui).
//!
//! Naming convention (spec §5.1 I-8 fold): non-interactive
//! subcommand modules suffixed `_cmd` to avoid shadowing the
//! `tabs/status.rs` Status TAB module.
//!
//! Public entry points (called from `src/main.rs` dispatch):
//! - [`run`] — interactive TUI.
//! - [`dump`] — TOML catalog dump to stdout.
//! - [`status`] — resolved config state to stdout.

pub(crate) mod app;
pub(crate) mod compile_pending;
pub(crate) mod debounce;
pub(crate) mod dump_cmd;
pub(crate) mod edit;
pub(crate) mod events;
pub(crate) mod reconcile;
pub(crate) mod render;
pub(crate) mod save;
pub(crate) mod search;
pub(crate) mod snapshot;
pub(crate) mod status_cmd;
pub(crate) mod style_ratatui;
pub(crate) mod tabs;
pub(crate) mod widgets;

use std::io::stdout;
use std::process::ExitCode;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::Terminal;

use crate::cli::{DumpKind, RunArgs};

/// RAII guard around raw mode + alt-screen. Drop restores both.
/// Mirrors `src/tty_guard.rs` pattern (CLAUDE.md §3 — restore on every exit path).
struct TuiGuard;

/// One-shot gate so the panic hook is installed at most once per process.
static PANIC_HOOK_INSTALLED: std::sync::Once = std::sync::Once::new();

impl TuiGuard {
    fn enter() -> std::io::Result<Self> {
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen)?;
        // Panic-safety: install a one-shot hook that restores the
        // terminal on unwind, then chains to the previous hook.
        PANIC_HOOK_INSTALLED.call_once(|| {
            let prev = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                let _ = execute!(stdout(), LeaveAlternateScreen);
                let _ = disable_raw_mode();
                prev(info);
            }));
        });
        Ok(Self)
    }
}

impl Drop for TuiGuard {
    fn drop(&mut self) {
        let _ = execute!(stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

/// Launch the interactive TUI.
///
/// # Returns
/// `ExitCode::SUCCESS` (0) on clean exit; `ExitCode::from(70)`
/// (`EX_SOFTWARE`) on unrecoverable render/terminal errors;
/// `ExitCode::from(64)` on config parse error.
// reason: ExitCode is returned for the caller (main.rs) to propagate; needless_pass_by_value
// is acceptable here since RunArgs is the CLI contract type.
#[allow(clippy::needless_pass_by_value, clippy::must_use_candidate)]
pub fn run(args: RunArgs) -> ExitCode {
    // 1. Load snapshot from disk (the user's existing config).
    let snapshot = match crate::config::load(args.config.as_deref()) {
        Ok(Some((_, path))) => match snapshot::ConfigSnapshot::read_from_disk(Some(&path)) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("tayf config: snapshot read failed: {e}");
                return ExitCode::from(70);
            }
        },
        Ok(None) => snapshot::ConfigSnapshot::empty(),
        Err(e) => {
            eprintln!("tayf config: config parse error: {e}");
            return ExitCode::from(64);
        }
    };

    // 2. Engage TUI guard + ratatui terminal.
    let _guard = match TuiGuard::enter() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("tayf config: terminal init failed: {e}");
            return ExitCode::from(70);
        }
    };
    let backend = CrosstermBackend::new(stdout());
    let terminal = match Terminal::new(backend) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("tayf config: ratatui Terminal init failed: {e}");
            return ExitCode::from(70);
        }
    };

    // 3. Build App + run loop.
    let app = app::App::from_snapshot(snapshot);
    match events::run_event_loop(app, terminal) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tayf config: event loop error: {e}");
            ExitCode::from(70)
        }
    }
    // _guard.drop() → disable_raw_mode + LeaveAlternateScreen
}

/// `tayf config dump [--kind …]` — write built-in catalog to stdout.
#[allow(clippy::must_use_candidate)]
// reason: ExitCode is returned for the caller (main.rs) to propagate; the
// function's primary effect is writing to stdout, so #[must_use] adds noise.
pub fn dump(kind: Option<DumpKind>) -> ExitCode {
    dump_cmd::run(kind)
}

/// `tayf config status` — resolved config state + hot-reload event log tail.
#[allow(clippy::must_use_candidate)]
// reason: ExitCode is returned for the caller (main.rs) to propagate; the
// function's primary effect is writing to stdout, so #[must_use] adds noise.
pub fn status(args: RunArgs) -> ExitCode {
    status_cmd::run(args)
}
