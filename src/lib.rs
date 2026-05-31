//! tayf — terminal-agnostic, PTY-based, regex-driven output colorizer.
//!
//! Public entry point: [`Tayf::run`]. See
//! `ARCHITECTURE.md` for the architecture overview.
//!
//! ## Capture-group styling (v0.3.5)
//!
//! A `[[rules]]` entry in either the user config or a theme TOML may set a
//! per-capture-group style overlay via the `styles` map. Two equivalent
//! TOML forms are accepted (inline-table for compactness; dotted-table
//! for multi-line clarity):
//!
//! ```toml
//! # Inline-table form — keep on one line.
//! [[rules]]
//! name = "timestamp"
//! style  = { fg = "bright_black" }
//! styles = { "1" = { fg = "yellow" }, "3" = { fg = "green" } }
//!
//! # Dotted-table form — equivalent, easier to grow.
//! [[rules]]
//! name = "timestamp"
//! style = { fg = "bright_black" }
//! [rules.styles."1"]
//! fg = "yellow"
//! [rules.styles."3"]
//! fg = "green"
//! ```
//!
//! Keys are 1-based capture-group indices encoded as positive-decimal
//! strings (grammar `^[1-9][0-9]*$`). Group 0 (the entire match) is
//! reserved for the `style` field. An empty `styles = {}` map is silently
//! accepted as a no-op. Range validation against the rule's regex
//! `captures_len()` happens at config load.

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
pub(crate) mod profiles;
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

/// Interactive `tayf config` TUI + dump + status. v0.5.4. Public so
/// `main.rs` can dispatch; subordinate items remain `pub(crate)`.
pub mod config_tui;

pub use cli::{Args, Cmd, ConfigAction, ConfigArgs, DumpArgs, DumpKind, RunArgs};
pub use error::{
    Error, ProfileErrorKind, ProfileRuleError, ProfileRuleErrorKind, Result, ThemeRuleError,
    ThemeRuleErrorKind,
};

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
    pub fn run(args: RunArgs) -> Result<ExitCode> {
        log::init_from_env();

        // Resolve bypass at process start; single-read guarantee (race-free).
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

        // v0.5.2 — effective profile name: CLI > config.
        let effective_profile_name: Option<String> =
            args.profile.clone().or_else(|| config_ref.and_then(|c| c.general.profile.clone()));

        // Load the profile (if any). Failures propagate as Error::Profile /
        // Error::ProfileValidation through the standard Tayf::run error
        // path. v0.5.2 ships no embedded profiles, so a None
        // loaded_profile just means "no profile active".
        let loaded_profile: Option<profiles::LoadedProfile> = match effective_profile_name {
            Some(ref name) => Some(profiles::load(name)?),
            None => None,
        };

        // v0.5.2 — 4-tier theme precedence:
        // CLI > config > profile.theme > bg-detect.
        let explicit_theme: Option<String> = args
            .theme
            .clone()
            .or_else(|| config_ref.and_then(|c| c.general.theme.clone()))
            .or_else(|| loaded_profile.as_ref().and_then(|lp| lp.profile.theme.clone()));

        // bg-detect is resolved ONCE at startup (querying the terminal via
        // OSC 11 is latency-sensitive). The result is the last-resort
        // theme fallback for both the startup compile AND every
        // subsequent hot reload — see `reload::reload_once` `bg_default`
        // argument. We retain the bg-detect result as a snapshot so the
        // reload thread can re-resolve the chain without re-querying.
        let bg_default: Option<String> =
            if apply_colors { Some(bg_detect::resolve().as_theme_name().to_owned()) } else { None };

        let effective_theme: Option<String> = explicit_theme.or_else(|| bg_default.clone());

        let compiled = rules::Compiled::load_with_theme(
            config_ref,
            config_path.as_deref(),
            effective_theme.as_deref(),
            loaded_profile.as_ref().map(|lp| &lp.profile),
            loaded_profile.as_ref().map(|lp| lp.path_label.as_str()),
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
        // ordering (mirrors the v0.3.2 invariant — orchestrator-declared-
        // LAST among threading scaffolding so that on any earlier `?`
        // returning Err, no orchestrator exists → no join-deadlock where
        // its Drop blocks on a channel still held by already-spawned
        // signal/watcher threads).
        let _orchestrator = if hot_reload_enabled {
            Some(reload::ReloadOrchestrator::spawn(
                Arc::clone(&rules),
                loaded.as_ref().map(|(_, p)| p.clone()),
                // v0.5.2: snapshot CLI --theme + --profile at startup;
                // every reload re-evaluates the full precedence chain
                // (CLI snapshot > config > profile.theme > bg_default).
                // bg_default is the bg-detect result resolved once at
                // startup — used as the last-resort fallback so reloads
                // don't re-query the terminal (OSC 11 latency).
                args.theme.clone(),
                args.profile.clone(),
                bg_default.clone(),
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

        // Drop the local sender now (preserves the v0.3.2 invariant —
        // local reload_tx is dropped before runtime::run so the only
        // remaining clones are the ones inside guard threads). Remaining
        // clones live in signal_guard (when hot_reload_enabled) and
        // watcher (when both hot_reload_enabled and config present).
        // When BOTH guards Drop at shutdown their clones go away, the
        // orchestrator's recv() returns Err, and its loop exits cleanly.
        // When hot_reload disabled, reload_tx here is the only sender
        // (none was cloned to signal thread); dropping is trivially safe
        // — receiver is already gone.
        drop(reload_tx);

        let exit_code = runtime::run(reader, writer, child, Arc::clone(&rules), apply_colors)?;

        // Explicit ordered shutdown (preserves the v0.3.2 invariant; see
        // spec §3.3 drop-ordering invariant):
        // - watcher first: closes notify's raw_tx, which lets the
        //   debounce thread observe Disconnected and exit, which drops
        //   the debounce thread's reload_tx clone.
        // - signal_guard next: closes signal_hook iterator, joins
        //   signal thread, which drops the signal thread's reload_tx
        //   clone (if any).
        // - implicit _orchestrator drop at end of scope: recv() returns
        //   Err (all senders gone), loop exits, join completes.
        // - guard last among explicit drops: Drop restores termios.
        //   _orchestrator's implicit Drop runs after this, but its only
        //   side-effect is joining the (already-exited) reload thread.
        drop(watcher);
        drop(signal_guard);

        drop(guard); // explicit; Drop would handle it but make ordering clear

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // reason: exit_code & 0xff is structurally in 0..=255
        let code = (exit_code & 0xff) as u8;
        Ok(ExitCode::from(code))
    }
}

/// Test-only adapters for `tests/` integration binaries. Not part of
/// the public API — hidden from rustdoc, no stability guarantees. Exposes
/// the minimum Config-TUI surface required by `tests/common/tui_harness.rs`
/// to drive headless TUI integration tests.
#[doc(hidden)]
pub mod __test_api {
    use ratatui::backend::Backend;

    /// Opaque newtype around the internal `App` so tests don't need to
    /// reach `pub(crate)` fields directly.
    pub struct AppHandle(pub(crate) crate::config_tui::app::App);

    /// Boot a fresh App on an empty snapshot, with the provided sample
    /// text seeded into the live-preview pipeline. Equivalent to the
    /// production `App::from_snapshot(ConfigSnapshot::empty())` path
    /// plus a sample swap + recompile.
    #[must_use]
    pub fn boot_app_with_sample(sample: &str) -> AppHandle {
        let snapshot = crate::config_tui::snapshot::ConfigSnapshot::empty();
        let mut app = crate::config_tui::app::App::from_snapshot(snapshot);
        sample.clone_into(&mut app.sample_input.text);
        app.preview.recompile(&app.sample_input.text);
        AppHandle(app)
    }

    /// Drive one frame through the internal `render::frame` entry-point.
    ///
    /// # Errors
    /// Forwards any backend `Error` produced by the backend's `draw`.
    pub fn draw_app<B: Backend>(
        app: &AppHandle,
        terminal: &mut ratatui::Terminal<B>,
    ) -> Result<(), B::Error> {
        terminal.draw(|f| crate::config_tui::render::frame(f, &app.0)).map(|_| ())
    }

    // Integration tests need to send keystrokes + observe modal and
    // selection state. The internal `Modal` enum stays `pub(crate)` —
    // tests interact through predicate helpers (`is_*_modal_open`)
    // rather than re-exporting the full variant set.
    pub use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// Dispatch a single key event into the App's event loop. Mirrors
    /// what the production `run_event_loop` does for each `KeyPress`.
    pub fn send_key(app: &mut AppHandle, key: KeyEvent) {
        crate::config_tui::events::dispatch_key(&mut app.0, key);
    }

    /// True iff any modal overlay is currently open.
    #[must_use]
    pub fn has_modal_open(app: &AppHandle) -> bool {
        app.0.modal.is_some()
    }

    /// True iff the Help overlay is currently open.
    #[must_use]
    pub fn is_help_modal_open(app: &AppHandle) -> bool {
        matches!(app.0.modal, Some(crate::config_tui::app::Modal::Help))
    }

    /// True iff the quit-confirm modal is currently open.
    #[must_use]
    pub fn is_quit_confirm_modal_open(app: &AppHandle) -> bool {
        matches!(app.0.modal, Some(crate::config_tui::app::Modal::QuitWithUnsavedEdits))
    }

    /// Read the App's current Patterns-tab selected index. Used by
    /// integration tests that assert a keystroke was discarded (selection
    /// unchanged) versus consumed (selection moved).
    #[must_use]
    pub fn current_selected_idx(app: &AppHandle) -> usize {
        app.0.focus.patterns.selected_idx
    }

    /// Count of pending newly-added rules in `PendingEdits::added`. Used by
    /// the editor integration test to assert that the `n` new-pattern modal
    /// commit path appended a draft to the edits set.
    #[must_use]
    pub fn pending_added_count(app: &AppHandle) -> usize {
        app.0.edits.added.len()
    }

    /// True iff the `Modal::NewPattern` overlay is currently open. Lets
    /// integration tests confirm modal open/dismiss transitions without
    /// reaching `pub(crate)` enum variants directly.
    #[must_use]
    pub fn is_new_pattern_modal_open(app: &AppHandle) -> bool {
        matches!(app.0.modal, Some(crate::config_tui::app::Modal::NewPattern { .. }))
    }

    /// Boot a fresh App with an EMPTY snapshot whose `source_path` is
    /// bound to the caller-provided `cfg_path` (the file does NOT need
    /// to exist; the `Shift+D` init-from-dump path expects it to be
    /// absent). v0.6.1 §3.3 integration test helper.
    #[must_use]
    pub fn boot_app_with_bound_empty_snapshot(cfg_path: std::path::PathBuf) -> AppHandle {
        let mut snap = crate::config_tui::snapshot::ConfigSnapshot::empty();
        snap.source_path = Some(cfg_path);
        AppHandle(crate::config_tui::app::App::from_snapshot(snap))
    }

    /// Boot a fresh App by reading the snapshot from `cfg_path` on disk.
    /// The file MUST exist; use this when the test seeds an existing
    /// config and expects the "Init dump only available when config file
    /// does not exist" branch.
    ///
    /// # Errors
    /// Returns any [`crate::error::Error`] surfaced by `read_from_disk`.
    pub fn boot_app_from_disk_path(
        cfg_path: &std::path::Path,
    ) -> Result<AppHandle, crate::error::Error> {
        let snap = crate::config_tui::snapshot::ConfigSnapshot::read_from_disk(Some(cfg_path))?;
        Ok(AppHandle(crate::config_tui::app::App::from_snapshot(snap)))
    }

    /// Boot a fresh App on an empty snapshot whose `parsed.rules` is
    /// pre-populated with synthetic `UserRule` entries (one per `(name,
    /// pattern)` tuple in `user_rules`), then seed the live-preview
    /// pipeline with `sample`. Used by v0.6.2 G5 integration tests
    /// asserting the Patterns tab union render (built-in + user-config
    /// rules under two DIM section headers).
    ///
    /// Each synthetic `UserRule` carries `enabled = true` and no style /
    /// styles / priority overrides — enough to surface in
    /// `patterns_list_layout` and the rendered list without affecting
    /// the live preview pipeline. Also builds a `TestBackend` terminal
    /// sized `cols × rows` so the caller can draw and inspect the buffer.
    ///
    /// Primitive `&str` signatures keep `pub(crate) UserRule` /
    /// `ConfigSnapshot` off the public boundary.
    #[must_use]
    #[allow(clippy::expect_used)]
    pub fn boot_app_with_user_config_and_sample(
        user_rules: &[(&str, &str)],
        sample: &str,
        cols: u16,
        rows: u16,
    ) -> (AppHandle, ratatui::Terminal<ratatui::backend::TestBackend>) {
        let mut snapshot = crate::config_tui::snapshot::ConfigSnapshot::empty();
        for (name, pattern) in user_rules {
            snapshot.parsed.rules.push(crate::config::UserRule {
                name: (*name).to_owned(),
                pattern: Some((*pattern).to_owned()),
                style: None,
                enabled: true,
                styles: None,
                priority: None,
            });
        }
        let mut app = crate::config_tui::app::App::from_snapshot(snapshot);
        sample.clone_into(&mut app.sample_input.text);
        app.preview.recompile(&app.sample_input.text);

        let backend = ratatui::backend::TestBackend::new(cols, rows);
        let terminal = ratatui::Terminal::new(backend)
            .expect("TestBackend init in boot_app_with_user_config_and_sample");
        (AppHandle(app), terminal)
    }

    /// Stage a foreground-color edit on a built-in rule so
    /// `app.edits.is_dirty()` becomes true. Used by `Ctrl+R` integration
    /// tests that need a dirty-edits precondition without driving the
    /// full `ColorPicker` modal path.
    pub fn stage_builtin_fg_edit(app: &mut AppHandle, builtin_name: &'static str) {
        use crate::config_tui::edit::{NewStyle, RuleEdit, RuleId, StyleKey};
        use std::collections::HashMap;
        let mut styles: HashMap<StyleKey, NewStyle> = HashMap::new();
        styles.insert(
            StyleKey::Default,
            NewStyle { fg: Some(Some(crate::style::Color::Red)), ..NewStyle::default() },
        );
        app.0.edits.rules.insert(RuleId::Builtin(builtin_name), RuleEdit { pattern: None, styles });
    }

    /// True iff `app.edits.is_dirty()` (any staged mutation present).
    #[must_use]
    pub fn edits_are_dirty(app: &AppHandle) -> bool {
        app.0.edits.is_dirty()
    }

    /// True iff a `Modal::Confirm` is currently open with the
    /// `DiscardEditsAndReload` action.
    #[must_use]
    pub fn is_discard_reload_confirm_modal_open(app: &AppHandle) -> bool {
        matches!(
            app.0.modal,
            Some(crate::config_tui::app::Modal::Confirm {
                action: crate::config_tui::app::ConfirmAction::DiscardEditsAndReload,
                ..
            })
        )
    }

    /// True iff a `Modal::Confirm` is currently open with the
    /// `InitFromDump` action.
    #[must_use]
    pub fn is_init_from_dump_confirm_modal_open(app: &AppHandle) -> bool {
        matches!(
            app.0.modal,
            Some(crate::config_tui::app::Modal::Confirm {
                action: crate::config_tui::app::ConfirmAction::InitFromDump,
                ..
            })
        )
    }

    /// True iff a `Modal::Confirm` is currently open with a
    /// `DeleteRule` action (any `RuleId`). Renamed from
    /// `is_delete_user_rule_confirm_modal_open` in v0.6.2 §3.3.
    #[must_use]
    pub fn is_delete_rule_confirm_modal_open(app: &AppHandle) -> bool {
        matches!(
            app.0.modal,
            Some(crate::config_tui::app::Modal::Confirm {
                action: crate::config_tui::app::ConfirmAction::DeleteRule(_),
                ..
            })
        )
    }

    /// True iff a `Modal::FullPreview` is currently open.
    #[must_use]
    pub fn is_full_preview_modal_open(app: &AppHandle) -> bool {
        matches!(app.0.modal, Some(crate::config_tui::app::Modal::FullPreview))
    }

    /// Static list of built-in rule names, in canonical order. Mirrors
    /// `crate::rules::BUILTIN_NAMES` for integration tests that need to
    /// assert "every shipped builtin is present in X" invariants.
    #[must_use]
    pub fn builtin_rule_names() -> &'static [&'static str] {
        crate::rules::BUILTIN_NAMES
    }

    /// Returns the current toast text (and `Ok` / `Warn` kind tag) if any.
    /// Used by integration tests to assert toast contents without
    /// re-exporting [`crate::config_tui::app::Toast`].
    #[must_use]
    pub fn current_toast(app: &AppHandle) -> Option<(String, &'static str)> {
        app.0.toast.as_ref().map(|t| {
            let kind = match t.kind {
                crate::config_tui::app::ToastKind::Ok => "ok",
                crate::config_tui::app::ToastKind::Warn => "warn",
            } as &'static str;
            (t.text.clone(), kind)
        })
    }

    /// Clear any active modal — test-only helper used to reset state
    /// between sub-assertions in a single test. v0.6.1 Group D parity
    /// check (V alias → reset → Shift+P).
    pub fn clear_modal(app: &mut AppHandle) {
        app.0.modal = None;
    }

    /// Read the App's current `save_diff_scroll` offset. v0.6.1 §3.7.
    #[must_use]
    pub fn save_diff_scroll(app: &AppHandle) -> u16 {
        app.0.save_diff_scroll
    }

    /// Open the save-diff modal seeded with a Clean diff body for
    /// scroll-key integration tests (v0.6.1 §3.7). Bypasses the
    /// `Ctrl+S` build-initial-state path so tests can assert scroll
    /// keystroke semantics without a real on-disk config.
    pub fn open_save_diff_with_clean_body(app: &mut AppHandle, tui_diff: impl Into<String>) {
        use crate::config_tui::widgets::save_diff::SaveDiffState;
        app.0.save_diff = Some(SaveDiffState::Clean { tui_diff: tui_diff.into() });
        app.0.modal = Some(crate::config_tui::app::Modal::SaveDiff);
    }

    /// Canonical help-modal string. Mirrors
    /// `crate::config_tui::events::HELP_MODAL_CONTENT` for integration
    /// tests asserting v0.6.1 keybindings are listed.
    #[must_use]
    pub fn help_modal_content() -> &'static str {
        crate::config_tui::events::HELP_MODAL_CONTENT
    }

    /// Wrap `crate::config_tui::search::filter_names_lowercase` for
    /// integration tests. v0.6.1 §3.6.
    #[must_use]
    pub fn filter_names_lowercase(names: &[&'static str], filter: &str) -> Vec<&'static str> {
        crate::config_tui::search::filter_names_lowercase(names.iter().copied(), filter)
    }

    // -----------------------------------------------------------------------
    // G1: debouncer-clear helpers (spec §3.7).
    // -----------------------------------------------------------------------

    /// True iff the preview debouncer has a pending edit mark. G1 integration
    /// tests use this to assert that Esc-cancel clears the pending mark.
    #[must_use]
    pub fn debouncer_pending(app: &AppHandle) -> bool {
        app.0.preview.debouncer.is_pending()
    }

    /// Open a `Modal::EditRegex` for the first builtin rule. Bypasses the
    /// `'e'` keystroke path so tests do not depend on tab focus order.
    /// G1 spec §3.7.
    pub fn open_edit_regex_modal_first_builtin(app: &mut AppHandle) {
        use crate::config_tui::app::Modal;
        use crate::config_tui::edit::RuleId;
        let rule_id = RuleId::Builtin(crate::rules::BUILTIN_NAMES[0]);
        let buffer = crate::config_tui::events::pattern_for_rule_id(&rule_id, &app.0);
        app.0.modal = Some(Modal::EditRegex { rule_id, buffer, error: None });
    }

    /// Open a `Modal::NewPattern` in the `Name` phase. Bypasses the `'n'`
    /// keystroke so tests do not depend on tab focus. G1 spec §3.7.
    pub fn open_new_pattern_modal(app: &mut AppHandle) {
        use crate::config_tui::app::{Modal, NewPatternPhase, PatternDraft};
        app.0.modal =
            Some(Modal::NewPattern { phase: NewPatternPhase::Name, draft: PatternDraft::new() });
    }

    /// True iff a `Modal::EditRegex` is currently open.
    #[must_use]
    pub fn is_edit_regex_modal_open(app: &AppHandle) -> bool {
        matches!(app.0.modal, Some(crate::config_tui::app::Modal::EditRegex { .. }))
    }

    /// Invoke one debounce tick as if the main loop's timer fired.
    ///
    /// Returns `true` if the pending mark was consumed by this tick
    /// (i.e., `was_pending` before the tick and `!is_pending` after).
    /// This does NOT guarantee that a recompile actually ran — use
    /// `debouncer_pending()` or inspect preview state for that. G1 spec §3.7.
    pub fn tick_debounce(app: &mut AppHandle) -> bool {
        let was_pending = app.0.preview.debouncer.is_pending();
        crate::config_tui::events::check_debounce(&mut app.0);
        was_pending && !app.0.preview.debouncer.is_pending()
    }

    // -----------------------------------------------------------------------
    // G2: save-quit flag helpers (spec §3.8).
    // -----------------------------------------------------------------------

    /// Stage a dirty edit so `app.edits.is_dirty()` is true, which causes
    /// the `q` keystroke to open `Modal::QuitWithUnsavedEdits`. Delegates
    /// to `stage_builtin_fg_edit` with the first builtin rule name.
    pub fn make_pending_edit(app: &mut AppHandle) {
        stage_builtin_fg_edit(app, crate::rules::BUILTIN_NAMES[0]);
    }

    /// Dispatch the `q` key (no modifiers) — opens `QuitWithUnsavedEdits`
    /// when edits are dirty, or sets `should_quit` directly. G2 §3.8.
    pub fn send_q(app: &mut AppHandle) {
        send_key(app, KeyEvent::new(KeyCode::Char('q'), KeyModifiers::empty()));
    }

    /// Dispatch a bare character keystroke (no modifiers). G2 §3.8.
    pub fn send_char(app: &mut AppHandle, c: char) {
        send_key(app, KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty()));
    }

    /// Dispatch the Esc key (no modifiers). G2 §3.8.
    pub fn send_esc(app: &mut AppHandle) {
        send_key(app, KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()));
    }

    /// Read `app.pending_save_and_quit`. G2 §3.8.
    #[must_use]
    pub fn pending_save_and_quit(app: &AppHandle) -> bool {
        app.0.pending_save_and_quit
    }

    /// Read `app.should_quit`. G2 §3.8.
    #[must_use]
    pub fn should_quit(app: &AppHandle) -> bool {
        app.0.should_quit
    }

    /// True iff a `Modal::SaveDiff` overlay is currently open. G2 §3.8.
    #[must_use]
    pub fn is_save_diff_modal_open(app: &AppHandle) -> bool {
        matches!(app.0.modal, Some(crate::config_tui::app::Modal::SaveDiff))
    }

    // -----------------------------------------------------------------------
    // G3: ColorPicker bool-axis helpers (spec §3.1).
    // -----------------------------------------------------------------------

    /// String tag for the picker's current `axis_focus`: `"none"`, `"bold"`,
    /// `"italic"`, or `"underline"`. Returns `None` if no `ColorPicker` modal
    /// is open. G3 §3.1.
    #[must_use]
    pub fn color_picker_axis_focus_tag(app: &AppHandle) -> Option<&'static str> {
        use crate::config_tui::app::Modal;
        use crate::config_tui::widgets::color_picker::AxisFocus;
        let Some(Modal::ColorPicker(state)) = &app.0.modal else { return None };
        Some(match state.axis_focus {
            AxisFocus::None => "none",
            AxisFocus::Bold => "bold",
            AxisFocus::Italic => "italic",
            AxisFocus::Underline => "underline",
        })
    }

    /// String tag for the picker's current `section`: `"ansi16"`,
    /// `"palette256"`, or `"truehex"`. Returns `None` if no `ColorPicker`
    /// modal is open. G3 §3.1.
    #[must_use]
    pub fn color_picker_section_tag(app: &AppHandle) -> Option<&'static str> {
        use crate::config_tui::app::Modal;
        use crate::config_tui::widgets::color_picker::PickerSection;
        let Some(Modal::ColorPicker(state)) = &app.0.modal else { return None };
        Some(match state.section {
            PickerSection::Ansi16 => "ansi16",
            PickerSection::Palette256 => "palette256",
            PickerSection::TrueHex => "truehex",
        })
    }

    /// One axis's staged tri-state: outer `None` = unedited, `Some(None)`
    /// = explicit clear, `Some(Some(b))` = explicit set. Mirrors
    /// `NewStyle::{bold,italic,underline}` for integration-test reads. G3 §3.1.
    //
    // reason: the `Option<Option<bool>>` shape is the load-bearing tri-state
    // on `NewStyle`; the alias only renames it for readability inside the
    // `__test_api` module without changing the semantic contract.
    #[allow(clippy::option_option)]
    pub type AxisTriState = Option<Option<bool>>;

    /// Read the picker's three staged bool-axis tri-states as a `(bold,
    /// italic, underline)` tuple. Each axis is the raw [`AxisTriState`] shape.
    /// Returns `None` if no `ColorPicker` modal is open. G3 §3.1.
    #[must_use]
    pub fn color_picker_staged_axes(
        app: &AppHandle,
    ) -> Option<(AxisTriState, AxisTriState, AxisTriState)> {
        use crate::config_tui::app::Modal;
        let Some(Modal::ColorPicker(state)) = &app.0.modal else { return None };
        Some((state.staged_bold, state.staged_italic, state.staged_underline))
    }

    /// True iff a `Modal::ColorPicker` is currently open. G3 §3.1.
    #[must_use]
    pub fn is_color_picker_modal_open(app: &AppHandle) -> bool {
        matches!(app.0.modal, Some(crate::config_tui::app::Modal::ColorPicker(_)))
    }

    /// Open a `Modal::ColorPicker` directly (bypassing the `c` keystroke on
    /// the Patterns tab) so integration tests do not depend on tab focus or
    /// existing-edit state. G3 §3.1.
    pub fn open_color_picker(app: &mut AppHandle) {
        use crate::config_tui::app::Modal;
        app.0.modal = Some(Modal::ColorPicker(
            crate::config_tui::widgets::color_picker::ColorPickerState::default(),
        ));
    }

    /// Read the `style` overlay staged on the first built-in rule's
    /// `StyleKey::Default` slot, after commit, as a `(bold, italic,
    /// underline)` tuple of raw [`AxisTriState`]. Returns `(None, None, None)`
    /// if no overlay exists. G3 §3.1.
    #[must_use]
    pub fn pending_edits_first_builtin_axes(
        app: &AppHandle,
    ) -> (AxisTriState, AxisTriState, AxisTriState) {
        use crate::config_tui::edit::{RuleId, StyleKey};
        let rule_id = RuleId::Builtin(crate::rules::BUILTIN_NAMES[0]);
        let Some(re) = app.0.edits.rules.get(&rule_id) else { return (None, None, None) };
        let Some(ns) = re.styles.get(&StyleKey::Default) else { return (None, None, None) };
        (ns.bold, ns.italic, ns.underline)
    }

    // -----------------------------------------------------------------------
    // G4: RuleId 4-variant delete + ResetOverride rename helpers.
    // Spec v0.6.2 §3.3.
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // G4: RuleId 4-variant delete + ResetOverride helpers.
    // All helpers use concrete primitives (&'static str / String) so that
    // `RuleId` (which is `pub(crate)`) does not need to appear in a `pub`
    // function signature. Spec v0.6.2 §3.3.
    // -----------------------------------------------------------------------

    /// Stage a `RuleId::Builtin(name)` delete in `app.edits.deleted`.
    pub fn stage_delete_builtin(app: &mut AppHandle, name: &'static str) {
        use crate::config_tui::edit::RuleId;
        app.0.edits.deleted.insert(RuleId::Builtin(name));
    }

    /// Stage a `RuleId::UserConfig(name)` delete in `app.edits.deleted`.
    pub fn stage_delete_user_config(app: &mut AppHandle, name: &str) {
        use crate::config_tui::edit::RuleId;
        app.0.edits.deleted.insert(RuleId::UserConfig(name.to_owned()));
    }

    /// Stage a `RuleId::Embedded { profile, rule }` delete.
    pub fn stage_delete_embedded(app: &mut AppHandle, profile: &'static str, rule: &str) {
        use crate::config_tui::edit::RuleId;
        app.0.edits.deleted.insert(RuleId::Embedded { profile, rule: rule.to_owned() });
    }

    /// Stage a `RuleId::DiskProfile { profile, rule }` delete.
    pub fn stage_delete_disk_profile(app: &mut AppHandle, profile: &str, rule: &str) {
        use crate::config_tui::edit::RuleId;
        app.0
            .edits
            .deleted
            .insert(RuleId::DiskProfile { profile: profile.to_owned(), rule: rule.to_owned() });
    }

    /// Run `compile_pending` and return the pattern strings of all compiled
    /// individuals. Allows tests to assert rule presence/absence without
    /// exposing `Compiled` (which is `pub(crate)`).
    ///
    /// # Panics
    /// Panics if `compile_pending` returns an error (test convenience).
    #[must_use]
    #[allow(clippy::expect_used)]
    pub fn compile_pending_individual_patterns(app: &AppHandle) -> Vec<String> {
        let theme = app.0.snapshot.parsed.theme.as_deref();
        let profile = app.0.snapshot.parsed.profile.as_deref();
        let compiled = crate::config_tui::compile_pending::compile_pending(
            &app.0.snapshot,
            &app.0.edits,
            theme,
            profile,
        )
        .expect("compile_pending_individual_patterns: unexpected compile error");
        compiled.individuals.iter().map(|r| r.as_str().to_owned()).collect()
    }

    /// True iff `edits.deleted` contains a `RuleId::Builtin(name)` entry.
    #[must_use]
    pub fn edits_deleted_has_builtin(app: &AppHandle, name: &'static str) -> bool {
        use crate::config_tui::edit::RuleId;
        app.0.edits.deleted.contains(&RuleId::Builtin(name))
    }

    /// True iff `edits.rules` contains a `RuleId::Builtin(name)` entry.
    #[must_use]
    pub fn edits_rules_has_builtin(app: &AppHandle, name: &'static str) -> bool {
        use crate::config_tui::edit::RuleId;
        app.0.edits.rules.contains_key(&RuleId::Builtin(name))
    }

    /// Apply a reset-override for a `RuleId::Builtin(name)`: clears both
    /// `edits.rules[Builtin(name)]` and `edits.deleted[Builtin(name)]`.
    pub fn apply_reset_override_builtin(app: &mut AppHandle, name: &'static str) {
        use crate::config_tui::edit::RuleId;
        let rid = RuleId::Builtin(name);
        app.0.edits.rules.remove(&rid);
        app.0.edits.deleted.remove(&rid);
    }

    // -----------------------------------------------------------------------
    // G6 — Item 4 helpers used by the override-copy integration tests.
    // All inputs are primitive types so `pub(crate) Tab` does not need to
    // appear in a `pub` signature.
    // -----------------------------------------------------------------------

    /// Switch the active tab to Profiles. Used by override-copy tests that
    /// need to drive `tabs::profiles::dispatch_key` directly.
    pub fn goto_profiles_tab(app: &mut AppHandle) {
        app.0.tab = crate::config_tui::app::Tab::Profiles;
    }

    /// Switch the active tab to Themes.
    pub fn goto_themes_tab(app: &mut AppHandle) {
        app.0.tab = crate::config_tui::app::Tab::Themes;
    }

    /// Set the focus selection on the Profiles tab to `idx`. Caller is
    /// responsible for ensuring `idx < catalog.embedded_profile_names.len()`.
    pub fn set_selected_profile_idx(app: &mut AppHandle, idx: usize) {
        app.0.focus.profiles.selected_idx = idx;
    }

    /// Set the focus selection on the Themes tab to `idx`.
    pub fn set_selected_theme_idx(app: &mut AppHandle, idx: usize) {
        app.0.focus.themes.selected_idx = idx;
    }

    /// Look up the catalog index for `name` in the Profiles embedded list.
    /// Returns `None` when the name is not in the embedded set.
    #[must_use]
    pub fn embedded_profile_idx(app: &AppHandle, name: &str) -> Option<usize> {
        app.0.catalog.embedded_profile_names.iter().position(|n| *n == name)
    }

    /// Look up the catalog index for `name` in the Themes built-in list.
    #[must_use]
    pub fn builtin_theme_idx(app: &AppHandle, name: &str) -> Option<usize> {
        app.0.catalog.builtin_theme_names.iter().position(|n| *n == name)
    }

    /// Read the current toast message text, if any. Convenience over
    /// [`current_toast`] for assertions that only need the body string.
    #[must_use]
    pub fn current_toast_message(app: &AppHandle) -> Option<String> {
        app.0.toast.as_ref().map(|t| t.text.clone())
    }

    /// Raw embedded TOML source for a built-in profile. Lets integration
    /// tests assert byte-equality between the copy-on-disk and the
    /// compile-time embedded source without exposing `pub(crate)
    /// crate::profiles::embedded_source` itself.
    #[must_use]
    pub fn embedded_profile_source(name: &str) -> Option<&'static str> {
        crate::profiles::embedded_source(name)
    }

    /// Raw embedded TOML source for a built-in theme — symmetric to
    /// [`embedded_profile_source`].
    #[must_use]
    pub fn embedded_theme_source(name: &str) -> Option<&'static str> {
        crate::themes::embedded_source(name)
    }

    // -----------------------------------------------------------------------
    // G8 conflict-list re-exports.
    // The pub(crate) `widgets` module is not directly reachable from
    // integration tests, so re-export the two types the render-side
    // tests need: the per-row choice enum and the render fn itself.
    // -----------------------------------------------------------------------
    pub use crate::config_tui::widgets::conflict_list::render_conflict_list;
    pub use crate::config_tui::widgets::save_diff::ConflictChoice;

    // -----------------------------------------------------------------------
    // v0.6.3 I2 re-export — `config_tui::merge` was demoted to
    // `pub(crate)` to drop the `toml_edit::DocumentMut` re-export from
    // the crate's public surface. These two pure-data types carry no
    // `toml_edit` types in their fields (verified in `merge.rs:59-85`)
    // and are needed by `tests/config_tui_conflict_list.rs` to fabricate
    // fixtures for the conflict-list render suite.
    // -----------------------------------------------------------------------
    pub use crate::config_tui::merge::{ConflictValueShape, KeyConflict};

    // -----------------------------------------------------------------------
    // Corpus-harness helpers (spec §5.3 + §5.4). Delegate to
    // `crate::rules::testing_*` shims so no logic lives here.
    // -----------------------------------------------------------------------

    /// Run a single built-in rule by name against `input`; return the leftmost
    /// match span as `String` if any. Per-rule isolation: no priority sort,
    /// no overlap suppression, no profile gating. Used for the corpus harness
    /// debugging primitive — production-level FP measurement uses
    /// `pipeline_spans` (spec §5.3, audit §0.2).
    ///
    /// Returns `None` when `rule_name` is not a known built-in or the pattern
    /// does not match.
    ///
    /// `SemVer` note: this lives in `__test_api` with "no stability guarantees"
    /// — signature may change without bump.
    #[must_use]
    pub fn match_named_rule(rule_name: &str, input: &str) -> Option<String> {
        crate::rules::testing_match_named_rule(rule_name, input)
    }

    /// Run the full production pipeline against `input` with optional
    /// `profile` activation. Returns post-priority post-overlap
    /// `(rule_name, matched_span)` pairs — exactly what tayf would color
    /// in production output. Used for corpus harness decision measurement
    /// (spec §5.3, §5.4).
    ///
    /// `profile` is the name of an embedded profile (e.g. `"aws"`, `"k8s"`).
    /// Pass `None` for built-ins only. Returns an empty `Vec` when the
    /// profile name is unknown or compilation fails.
    ///
    /// `SemVer` note: see [`match_named_rule`].
    #[must_use]
    pub fn pipeline_spans(input: &str, profile: Option<&str>) -> Vec<(String, String)> {
        crate::rules::testing_pipeline_spans(input, profile)
    }
}

/// Smoke tests for the two `__test_api` corpus-harness helpers added in
/// Task 16. Kept separate from the existing `__test_api` module tests so
/// they are easy to filter via `cargo test --lib __test_api_smoke`.
#[cfg(test)]
mod __test_api_smoke {
    #[test]
    fn match_named_rule_returns_some_for_builtin_ipv4_hit() {
        let r = super::__test_api::match_named_rule("ipv4", "see 192.168.1.1 here");
        assert_eq!(r, Some("192.168.1.1".to_owned()));
    }

    #[test]
    fn match_named_rule_returns_none_for_unknown_rule_name() {
        let r = super::__test_api::match_named_rule("nonexistent", "anything");
        assert_eq!(r, None);
    }

    #[test]
    fn pipeline_spans_returns_priority_resolved_spans() {
        let spans = super::__test_api::pipeline_spans("192.168.1.1", None);
        let rules: Vec<&str> = spans.iter().map(|(n, _)| n.as_str()).collect();
        assert!(rules.contains(&"ipv4"), "ipv4 fires; got {rules:?}");
    }
}

/// Smoke tests for the `__bench__::BenchPipeline` shim. Kept separate from
/// the existing `__test_api_smoke` module so they are easy to filter via
/// `cargo test --lib __bench_pipeline_smoke`.
#[cfg(test)]
mod __bench_pipeline_smoke {
    #[test]
    fn bench_pipeline_feeds_and_emits_sgr_for_ipv4() {
        let mut p = crate::__bench__::BenchPipeline::with_builtins();
        let mut out: Vec<u8> = Vec::new();
        p.feed(b"connect 192.168.1.1 now\n", &mut out).expect("feed ok");
        let s = String::from_utf8(out).expect("utf8");
        assert!(s.contains("192.168.1.1"), "payload must survive: {s:?}");
        assert!(s.contains("\x1b["), "ipv4 builtin must inject an SGR: {s:?}");
    }

    #[test]
    fn bench_pipeline_passes_plain_text_unchanged() {
        let mut p = crate::__bench__::BenchPipeline::with_builtins();
        let mut out: Vec<u8> = Vec::new();
        p.feed(b"the quick brown fox\n", &mut out).expect("feed ok");
        assert_eq!(out, b"the quick brown fox\n", "no match => byte-identical");
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

    /// Bench-only scratch container. Wraps `crate::pipeline::PipelineScratch`
    /// so the `benches/` crate (an external crate from rustc's perspective)
    /// can hoist scratch allocation outside `b.iter` loops, matching the
    /// production Pipeline's per-call PipelineScratch-surface zero-allocation
    /// contract. Not part of the public API.
    #[derive(Default)]
    pub struct BenchScratch(crate::pipeline::PipelineScratch);

    /// Bench-only wrapper exposing the `pub(crate)` `Pipeline::feed` hot path
    /// to external bench crates without widening `src/pipeline.rs` visibility
    /// (an off-limits hot-path module). Constructs a pipeline over the
    /// built-in rule set. Behavior-neutral: forwards verbatim to `Pipeline`.
    pub struct BenchPipeline(crate::pipeline::Pipeline);

    impl BenchPipeline {
        /// Build a pipeline over the default built-in rule set.
        ///
        /// # Panics
        /// Panics if the built-in rule set fails to compile — impossible in
        /// practice (the built-ins are compile-tested) and acceptable in a
        /// bench-only constructor.
        #[must_use]
        pub fn with_builtins() -> Self {
            let compiled =
                crate::rules::Compiled::load_builtins().expect("built-in rules must compile");
            let rules = std::sync::Arc::new(arc_swap::ArcSwap::from_pointee(compiled));
            BenchPipeline(crate::pipeline::Pipeline::new(rules))
        }

        /// Feed a chunk through the pipeline, writing styled output to `out`.
        ///
        /// # Errors
        /// Propagates any write error from `out`.
        pub fn feed<W: Write>(&mut self, chunk: &[u8], out: &mut W) -> std::io::Result<()> {
            self.0.feed(chunk, out)
        }
    }

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

    /// Compile the rule set with an embedded profile active. Mirrors the
    /// production startup flow's profile-active branch (`crate::lib::Tayf::run`
    /// → `crate::profiles::load_with` → `crate::rules::Compiled::load_with_theme`),
    /// but stubs out the env-var lookups so disk discovery in `load_with`
    /// finds nothing and falls through to the embedded library.
    ///
    /// The bench crate is an external crate from rustc's perspective and
    /// cannot reach the `pub(crate)` profile/rules constructors directly,
    /// so this adapter wraps them. v0.5.3 only — see `benches/throughput.rs`
    /// `bench_profile_*` for the call sites.
    ///
    /// # Errors
    /// Forwards any [`crate::Error::Profile`], [`crate::Error::ProfileValidation`],
    /// or [`crate::Error::RegexCompile`] surfaced by the underlying load/compile.
    pub fn load_profile_rules(name: &str) -> crate::Result<CompiledRules> {
        // Force disk discovery to miss → embedded library wins. Empty
        // closures keep the disk path off, regardless of how the test
        // environment has `$XDG_CONFIG_HOME` / `$HOME` set.
        let loaded = crate::profiles::load_with(name, || None, || None)?;
        let compiled = crate::rules::Compiled::load_with_theme(
            None,
            None,
            None,
            Some(&loaded.profile),
            Some(loaded.path_label.as_str()),
            crate::terminfo::ColorDepth::Truecolor,
        )?;
        Ok(CompiledRules(std::sync::Arc::new(arc_swap::ArcSwap::from_pointee(compiled))))
    }

    /// Drive the rule scanner against `line`. Scratch is caller-owned and
    /// MUST be hoisted outside any `b.iter` loop so the measurement reflects
    /// the scanner, not the allocator.
    ///
    /// # Errors
    /// Forwards any `std::io::Error` produced by `out`.
    pub fn apply_rules<W: Write>(
        line: &[u8],
        rules: &CompiledRules,
        scratch: &mut BenchScratch,
        out: &mut W,
    ) -> std::io::Result<()> {
        crate::pipeline::apply_rules(line, &rules.0, &mut scratch.0, out)
    }
}

/// Fuzz-only adapters over `pub(crate)` internals. Compiled ONLY under
/// `--cfg fuzzing` (cargo-fuzz sets this across the path-dep graph), so it
/// is absent from normal and `cargo publish` builds — zero public API
/// surface, clean `cargo metadata`/SBOM. See `fuzz/fuzz_targets/`.
#[cfg(fuzzing)]
#[doc(hidden)]
pub mod __fuzz__ {
    /// Drive the ANSI state machine byte-by-byte. Invariant: no panic; the
    /// SM consumes every byte and the sequence cap keeps internal state bounded.
    pub fn ansi_sm(data: &[u8]) {
        let mut sm = crate::ansi::AnsiSm::new();
        for &b in data {
            let _ = sm.step(b);
        }
    }

    /// Drive the line buffer with arbitrary chunking. Invariant: no panic;
    /// UTF-8 splits never cause invalid slicing (`regex::bytes` operates on raw
    /// bytes); the buffer cap bounds memory.
    pub fn line_buffer(data: &[u8]) {
        let mut lb = crate::line_buffer::LineBuffer::new();
        for chunk in data.chunks(7) {
            // arbitrary small stride to vary chunk boundaries
            let _ = lb.feed_data_run(chunk);
        }
        let _ = lb.feed_with_overflow(data);
    }

    /// Differential passthrough oracle at `apply_rules` granularity: with an
    /// EMPTY rule set, applying rules to any single line is byte-identical to
    /// the input. (NOT `Pipeline::feed`-level byte-identity — that injects a
    /// synthetic ST on cap-overflow; see spec §4 A1.3.)
    ///
    /// # Panics
    /// Panics (the fuzzer's crash signal) iff the byte-identity oracle breaks.
    pub fn pipeline_apply_rules_identity(line: &[u8]) {
        use std::sync::Arc;
        let empty = crate::rules::Compiled::empty();
        let rules = Arc::new(arc_swap::ArcSwap::from_pointee(empty));
        let mut scratch = crate::pipeline::PipelineScratch::default();
        let mut out: Vec<u8> = Vec::new();
        crate::pipeline::apply_rules(line, &rules, &mut scratch, &mut out)
            .expect("Vec write is infallible");
        assert_eq!(out, line, "empty-rules apply_rules must be byte-identical");
    }

    /// Full pipeline feed over the built-in rule set. Crash-finder only
    /// (no oracle): invariant is no panic on arbitrary chunked input.
    pub fn pipeline_feed_builtins(data: &[u8]) {
        let mut p = crate::__bench__::BenchPipeline::with_builtins();
        let mut out: Vec<u8> = Vec::new();
        for chunk in data.chunks(13) {
            // arbitrary small stride to vary chunk boundaries
            let _ = p.feed(chunk, &mut out);
        }
    }

    /// Compile an arbitrary user pattern under the production size limits.
    /// Invariant: returns (Ok or clean Err) without panic/OOM/timeout.
    pub fn regex_compile(pattern: &str) {
        let _ = crate::rules::fuzz_compile_pattern(pattern);
    }
}
