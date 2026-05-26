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
//! - [`save`] — toml_edit roundtrip + atomic write + backup rotation.
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
pub(crate) mod debounce;
pub(crate) mod dump_cmd;
pub(crate) mod edit;
pub(crate) mod events;
pub(crate) mod render;
pub(crate) mod save;
pub(crate) mod snapshot;
pub(crate) mod status_cmd;
pub(crate) mod tabs;
pub(crate) mod widgets;

use std::process::ExitCode;

use crate::cli::{DumpKind, RunArgs};

/// Launch the interactive TUI. v0.5.4 stub — full impl lands in
/// Phase C tasks.
///
/// # Returns
/// `ExitCode::SUCCESS` (0) on clean exit; `ExitCode::from(70)`
/// (`EX_SOFTWARE`) on unrecoverable render/terminal errors.
#[allow(clippy::needless_pass_by_value, clippy::must_use_candidate)]
pub fn run(_args: RunArgs) -> ExitCode {
    // Phase C stub. Full impl lands in C2a/C2b/C2c + C3 + C4.
    eprintln!("tayf config: interactive TUI not yet implemented (v0.5.4 stub)");
    ExitCode::SUCCESS
}

/// `tayf config dump [--kind …]` — write built-in catalog to stdout.
/// v0.5.4 stub — full impl lands in B1.
#[allow(clippy::must_use_candidate)]
pub fn dump(_kind: Option<DumpKind>) -> ExitCode {
    eprintln!("tayf config dump: not yet implemented (v0.5.4 stub)");
    ExitCode::SUCCESS
}

/// `tayf config status` — resolved config state + reload event tail.
/// v0.5.4 stub — full impl lands in B2.
#[allow(clippy::needless_pass_by_value, clippy::must_use_candidate)]
pub fn status(_args: RunArgs) -> ExitCode {
    eprintln!("tayf config status: not yet implemented (v0.5.4 stub)");
    ExitCode::SUCCESS
}
