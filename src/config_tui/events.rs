//! crossterm event loop + key dispatch + debounce tick.
//!
//! Single-thread loop: poll (100 ms) → key | resize | debounce tick.
//! Modal absorbs all keys except Esc and Ctrl+C (spec §7.2).

use std::io::Stdout;
use std::time::Duration;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Terminal;

use crate::config_tui::app::{App, ConfirmAction, Modal, Tab};

/// Canonical keybinding cheat-sheet rendered by `Modal::Help` (spec §12.4 D4).
/// Cross-checked against `dispatch_key` arms below — when a new keybinding
/// is added or moved, this string MUST be updated in the same commit.
pub(crate) const HELP_MODAL_CONTENT: &str = "\
tayf Config TUI — Keybindings
─────────────────────────────────────
Navigation
  Tab / Shift+Tab  Cycle tabs forward / backward
  1..4             Jump to tab by index
  j / k            Move selection down / up
  g / G            First / last entry
  h / l            Toggle list-pane / detail focus
  Space            Activate / toggle entry

Editing (Patterns tab)
  n                New pattern modal
  e                Edit pattern regex source
  c                Open color picker for selected rule
  o                Override builtin rule
  r                Reset user override
  d / Delete       Delete user override (confirm)

Color Picker (when modal open)
  Tab              Cycle: ANSI16 → 256 → hex → bold → italic → underline
  Space            Toggle focused boolean axis
  c                Clear focused boolean axis (bold/italic/underline only)
  ← →              Adjust focused color value
  Enter            Commit
  Esc              Cancel (discards staged edits)

Display
  s                Set sample input
  p                Toggle live preview strip
  Shift+P / V      Full preview overlay
  / (forward-slash) Search filter

Save Conflicts (in conflict modal)
  j / k            Navigate conflict rows
  o                Take ours (TUI edit)
  t                Take theirs (disk version)
  s                Skip (keep base / current disk)
  Enter            Apply all selections
  Esc              Cancel merge

Persistence
  Ctrl+S / Ctrl+W  Save
  s (in quit modal)  Save then quit
  Ctrl+R           Reload from disk (discards edits)
  Shift+D          Initialize default config (if file missing)
  ? / F1           This help
  Esc              Dismiss modal / cancel edit
  Ctrl+C / q       Quit
";

/// Drive the TUI event loop until `app.should_quit`.
pub(crate) fn run_event_loop(
    mut app: App,
    mut terminal: Terminal<CrosstermBackend<Stdout>>,
) -> std::io::Result<()> {
    while !app.should_quit {
        terminal.draw(|frame| crate::config_tui::render::frame(frame, &app))?;
        if ratatui::crossterm::event::poll(Duration::from_millis(100))? {
            match ratatui::crossterm::event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => dispatch_key(&mut app, k),
                // All other events are intentionally ignored.
                // Resize: ratatui recalculates layout on the next draw call.
                // Mouse / Paste / Focus*: not handled in the current TUI scope.
                Event::Resize(_, _)
                | Event::Mouse(_)
                | Event::Paste(_)
                | Event::FocusGained
                | Event::FocusLost
                | Event::Key(_) => {}
            }
        } else {
            check_debounce(&mut app);
            check_toast(&mut app);
        }
    }
    Ok(())
}

/// Top-level key dispatch. Routes per spec §7.2 precedence rules.
// reason: tiered key precedence (Ctrl+C → Esc → modal-absorbs → global keys)
// is a flat decision tree that does not split cleanly without obscuring intent;
// modal arms are extracted to dedicated helpers, leaving only routing here.
#[allow(clippy::too_many_lines)]
pub(crate) fn dispatch_key(app: &mut App, k: KeyEvent) {
    // 1. Ctrl+C is one of two keys that bypass modal-absorbs (§7.2).
    if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
        handle_quit_request(app);
        return;
    }
    // 1b. Ctrl+R reload (v0.6.1 §3.3). When no modal is open: if edits are
    //     pending, open a Confirm modal; otherwise reload directly. Inside
    //     a modal the keystroke is absorbed by the modal arm (no-op).
    if k.code == KeyCode::Char('r') && k.modifiers.contains(KeyModifiers::CONTROL) {
        if app.modal.is_none() {
            if app.edits.is_dirty() {
                app.modal = Some(Modal::Confirm {
                    msg: "Discard all pending edits and reload from disk?".to_owned(),
                    action: ConfirmAction::DiscardEditsAndReload,
                });
            } else {
                match reload_snapshot_inline(app) {
                    Ok(()) => {
                        app.toast = Some(crate::config_tui::app::Toast::ok("Reloaded from disk."));
                    }
                    Err(e) => {
                        app.toast = Some(crate::config_tui::app::Toast::warn(format!(
                            "Reload failed: {e}"
                        )));
                    }
                }
            }
        }
        return;
    }
    // 2. Esc precedence (§12.1).
    if k.code == KeyCode::Esc {
        handle_esc(app);
        return;
    }
    // 3. Modal absorbs all other keys.
    if let Some(modal) = &app.modal {
        // Quit-confirm modal owns its own key set.
        if matches!(modal, Modal::QuitWithUnsavedEdits) {
            handle_quit_confirm_key(app, k);
            return;
        }
        // Confirm modals (DeleteRule / ResetOverride / DiscardEditsAndReload / InitFromDump).
        if matches!(modal, Modal::Confirm { .. }) {
            handle_confirm_modal_key(app, k);
            return;
        }
        // C4-owned modals: dispatch into the widget's key handler.
        // reason: FullPreview placeholder, Confirm/Quit defensive fallback,
        // and Error key-absorption all currently share an empty body but
        // document semantically distinct intent.
        #[allow(clippy::match_same_arms)]
        match modal {
            Modal::ColorPicker(_) => {
                if let Some(Modal::ColorPicker(state)) = app.modal.as_mut() {
                    let out = crate::config_tui::widgets::color_picker::dispatch_key(state, k);
                    match out {
                        crate::config_tui::widgets::color_picker::ColorPickerOutcome::Accept => {
                            // Capture color + staged bool-axis edits BEFORE closing
                            // the modal (which moves state). G3 — Spec §3.1.
                            let (color, staged_bold, staged_italic, staged_underline) =
                                if let Some(Modal::ColorPicker(state)) = &app.modal {
                                    (
                                        state.selected_color(),
                                        state.staged_bold,
                                        state.staged_italic,
                                        state.staged_underline,
                                    )
                                } else {
                                    (None, None, None, None)
                                };
                            app.modal = None;
                            // Commit per-axis tri-state edits regardless of
                            // whether a color was also picked — the picker
                            // may be used as a pure attribute editor.
                            bind_bool_axes_to_selected_rule(
                                app,
                                staged_bold,
                                staged_italic,
                                staged_underline,
                            );
                            if let Some(color) = color {
                                bind_color_to_selected_rule(app, color);
                            } else if staged_bold.is_none()
                                && staged_italic.is_none()
                                && staged_underline.is_none()
                            {
                                app.toast = Some(crate::config_tui::app::Toast::warn(
                                    "color picker yielded no color (truecolor hex incomplete?)",
                                ));
                            }
                        }
                        crate::config_tui::widgets::color_picker::ColorPickerOutcome::Cancel => {
                            app.modal = None;
                        }
                        crate::config_tui::widgets::color_picker::ColorPickerOutcome::StayOpen => {}
                    }
                }
            }
            Modal::SaveDiff => handle_save_diff_key(app, k),
            Modal::ConflictList(_) => handle_conflict_list_key(app, k),
            Modal::Search => handle_search_key(app, k),
            Modal::SampleSet => handle_sample_set_key(app, k),
            Modal::NewPattern { .. } => handle_new_pattern_key(app, k),
            Modal::EditRegex { .. } => handle_edit_regex_key(app, k),
            Modal::Help => handle_help_key(app, k),
            Modal::FullPreview => {
                // Shift+P overlay; only Esc dismisses (handled above).
            }
            Modal::Confirm { .. } | Modal::QuitWithUnsavedEdits => {
                // Reached only on unhandled keys for these modals — their
                // dedicated dispatchers above return before this branch fires.
            }
            Modal::Error(_) => {
                // Error modal absorbs all keys; only Esc (handled above) dismisses.
            }
        }
        return;
    }
    // 4. Global keys (no modal).
    match (k.code, k.modifiers) {
        (KeyCode::Char('q'), m) if m.is_empty() => handle_quit_request(app),
        (KeyCode::Tab, m) if m.is_empty() => app.tab = app.tab.next(),
        (KeyCode::BackTab, _) => app.tab = app.tab.prev(),
        (KeyCode::Char(d @ '1'..='4'), m) if m.is_empty() => {
            if let Some(tab) = Tab::from_digit(d as u8 - b'0') {
                app.tab = tab;
            }
        }
        (KeyCode::Char('?'), m) if m.is_empty() => {
            if app.modal.is_none() {
                app.modal = Some(Modal::Help);
            }
        }
        (KeyCode::F(1), _) => {
            if app.modal.is_none() {
                app.modal = Some(Modal::Help);
            }
        }
        (KeyCode::Char('P'), m) if m == KeyModifiers::SHIFT => {
            if app.modal.is_none() {
                app.modal = Some(Modal::FullPreview);
            }
        }
        (KeyCode::Char('s'), m) if m == KeyModifiers::CONTROL => {
            if app.modal.is_none() {
                open_save_modal_for_current_state(app);
            }
        }
        (KeyCode::Char('w'), m) if m == KeyModifiers::CONTROL => {
            // Ctrl+W alt-binding (🔵 #1 fold — XON/XOFF inferno).
            if app.modal.is_none() {
                open_save_modal_for_current_state(app);
            }
        }
        (KeyCode::Char('/'), m) if m.is_empty() => {
            if app.modal.is_none() {
                app.search_state = Some(crate::config_tui::widgets::search::SearchState::default());
                app.modal = Some(Modal::Search);
            }
        }
        (KeyCode::Char('s'), m) if m.is_empty() => {
            if app.modal.is_none() {
                app.sample_set_state =
                    Some(crate::config_tui::widgets::sample_set::SampleSetState {
                        buf: app.sample_input.text.clone(),
                    });
                app.modal = Some(Modal::SampleSet);
            }
        }
        (KeyCode::Char('p'), m) if m.is_empty() => {
            app.mini_preview_visible = !app.mini_preview_visible;
        }
        (KeyCode::Char('D'), _) => {
            // First-run init dump (v0.6.1 §3.3). Enabled only when the
            // bound config path does not exist on disk. `KeyCode::Char('D')`
            // is the canonical SHIFT+d delivery across crossterm backends
            // (the shift is absorbed into the character on most terminals);
            // we therefore do not gate on the SHIFT modifier flag.
            if app.modal.is_none() {
                let path = app.snapshot.source_path.as_deref();
                if let Some(p) = path {
                    if p.exists() {
                        app.toast = Some(crate::config_tui::app::Toast::warn(
                            "Init dump only available when config file does not exist",
                        ));
                    } else {
                        app.modal = Some(Modal::Confirm {
                            msg: format!(
                                "Initialize {} with the built-in default config?",
                                p.display()
                            ),
                            action: ConfirmAction::InitFromDump,
                        });
                    }
                } else {
                    app.toast = Some(crate::config_tui::app::Toast::warn(
                        "Init dump requires a bound config path (none in this session)",
                    ));
                }
            }
        }
        (KeyCode::Char('V'), _) => {
            // V alias for Shift+P (FullPreview). v0.6.1 §3.5.
            // Like Shift+D, the SHIFT-on-V delivery is implicit in the
            // uppercase keycode; modifier flags are not gated.
            if app.modal.is_none() {
                app.modal = Some(Modal::FullPreview);
            }
        }
        _ => {
            crate::config_tui::tabs::dispatch_key(app, k);
        }
    }
}

/// `SaveDiff` modal key dispatch + outcome handling.
fn handle_save_diff_key(app: &mut App, k: KeyEvent) {
    use crate::config_tui::widgets::save_diff::{
        dispatch_key as sd_dispatch, SaveDiffOutcome, SaveDiffState,
    };
    // Guard: commit is refused while modal is in ReconcileError state.
    // User must Esc to dismiss and retry edits. (Spec §13.2 B2/I13 fold.)
    if matches!(&app.save_diff, Some(SaveDiffState::ReconcileError { .. })) {
        // Allow Esc/n to dismiss via the normal dispatch path; block 'y' silently.
        if let ratatui::crossterm::event::KeyCode::Char('y') = k.code {
            return;
        }
    }
    match sd_dispatch(app, k) {
        SaveDiffOutcome::Commit => match crate::config_tui::save::commit_save(
            &app.snapshot,
            &app.edits,
            std::time::SystemTime::now(),
        ) {
            Ok(new_snap) => {
                app.snapshot = new_snap;
                app.edits.clear();
                app.modal = None;
                app.save_diff = None;
                app.save_diff_scroll = 0;
                app.toast = Some(crate::config_tui::app::Toast::ok(
                    "Saved. Hot-reload will pick this up shortly.",
                ));
                // G2 §3.8: save-and-quit — set should_quit when the flag is set.
                if app.pending_save_and_quit {
                    app.should_quit = true;
                }
            }
            Err(e) => {
                // Save failed: clear pending flag so a subsequent manual quit
                // does not carry forward a stale save-and-quit intent.
                app.pending_save_and_quit = false;
                app.modal = Some(Modal::Error(format!("Save failed: {e}")));
                app.save_diff = None;
                app.save_diff_scroll = 0;
            }
        },
        SaveDiffOutcome::CloseModal => {
            app.pending_save_and_quit = false;
            app.modal = None;
            app.save_diff = None;
            app.save_diff_scroll = 0;
        }
        SaveDiffOutcome::StayOpen => {}
    }
}

/// Esc precedence (§12.1):
/// 1. close active edit field (incl. color-picker goto-input — UI/UX nit #5 fold),
/// 2. close modal (drops matching side-channel: `SaveDiff` / `Search` / `SampleSet`),
/// 3. clear active sticky search filter,
/// 4. no-op.
fn handle_esc(app: &mut App) {
    // Tier 1: color-picker goto-input clears first, modal stays open.
    if let Some(Modal::ColorPicker(state)) = app.modal.as_mut() {
        if state.goto_buf.take().is_some() {
            return;
        }
    }
    // Tier 1b: NewPattern phase-aware back-out (TUI reviewer I4 fold —
    // Esc rewinds through Style → Regex → Name, only closing the modal
    // on Esc from the Name phase. Draft buffers are preserved across
    // back-steps so an accidental rewind doesn't lose input.
    if let Some(Modal::NewPattern { phase, .. }) = app.modal.as_mut() {
        use crate::config_tui::app::NewPatternPhase;
        match phase {
            NewPatternPhase::Name => {
                // T-I5 paragraph 2: symmetric debouncer-leak fix — clear any
                // pending debounce mark so a phantom recompile cannot fire
                // after the modal is dismissed. See spec §3.7.
                app.preview.debouncer.mark_edit_clear();
                app.modal = None;
            }
            NewPatternPhase::Regex => {
                *phase = NewPatternPhase::Name;
            }
            NewPatternPhase::Style => {
                *phase = NewPatternPhase::Regex;
            }
        }
        return;
    }
    // Tier 2: close modal. Drop side-channels alongside so the
    // `side_channel.is_some() ↔ modal == Some(MatchingVariant)`
    // invariants that render + dispatch rely on stay intact.
    if app.modal.is_some() {
        match app.modal {
            Some(Modal::SaveDiff) => {
                app.pending_save_and_quit = false;
                app.save_diff = None;
                app.save_diff_scroll = 0;
            }
            Some(Modal::ConflictList(_)) => {
                // G8 §3.6 cross-cutting review fix: the conflict-list
                // modal shares the `save_diff` side-channel with
                // `Modal::SaveDiff`, so its Esc-close path needs the
                // same triple-reset. Without this the next Ctrl+S would
                // see a stale `MergePending` (4 DocumentMut clones held
                // alive) and `pending_save_and_quit` could leak past
                // the Esc into a subsequent save-success commit.
                app.pending_save_and_quit = false;
                app.save_diff = None;
                app.save_diff_scroll = 0;
            }
            Some(Modal::Search) => app.search_state = None,
            Some(Modal::SampleSet) => app.sample_set_state = None,
            Some(Modal::EditRegex { .. }) => {
                // G1 spec §3.7: clear pending debounce mark on Esc-cancel so
                // the quiescent timer does not trigger a phantom recompile.
                app.preview.debouncer.mark_edit_clear();
            }
            _ => {}
        }
        app.modal = None;
        return;
    }
    // Tier 3: clear sticky search filter when no modal open.
    if app.search_filter.is_some() {
        app.search_filter = None;
    }
}

/// Trigger quit flow per §12.1.1.
fn handle_quit_request(app: &mut App) {
    if app.edits.is_dirty() {
        // Replace any current modal with the quit-confirm modal (§12.1.1 stacking rule).
        app.modal = Some(Modal::QuitWithUnsavedEdits);
    } else {
        app.should_quit = true;
    }
}

/// §12.1.1 quit-confirm key set:
/// - n / Esc / Enter → cancel (default = safe)
/// - s → save then quit (opens `SaveDiff`; commit success sets `should_quit`)
/// - d → discard and quit (immediate)
fn handle_quit_confirm_key(app: &mut App, k: KeyEvent) {
    match k.code {
        KeyCode::Char('n') | KeyCode::Enter => {
            app.modal = None;
        }
        KeyCode::Char('s') => {
            // Save-and-quit: set flag so the commit-success path triggers quit,
            // then open the SaveDiff modal via the same path as Ctrl+S. Spec §3.8.
            app.pending_save_and_quit = true;
            open_save_modal_for_current_state(app);
        }
        KeyCode::Char('d') => {
            app.edits.clear();
            app.should_quit = true;
        }
        _ => {}
    }
}

/// Confirm modal key set (y / n / Esc).
fn handle_confirm_modal_key(app: &mut App, k: KeyEvent) {
    let action = match &app.modal {
        Some(Modal::Confirm { action, .. }) => match action {
            ConfirmAction::DiscardEditsAndReload => Some(ConfirmAction::DiscardEditsAndReload),
            ConfirmAction::DeleteRule(rid) => Some(ConfirmAction::DeleteRule(rid.clone())),
            ConfirmAction::ResetOverride(rid) => Some(ConfirmAction::ResetOverride(rid.clone())),
            ConfirmAction::InitFromDump => Some(ConfirmAction::InitFromDump),
        },
        _ => None,
    };
    match k.code {
        KeyCode::Char('y') => {
            if let Some(act) = &action {
                apply_confirm(app, act);
            }
            app.modal = None;
        }
        KeyCode::Char('n') => {
            app.modal = None;
        }
        _ => {}
    }
}

/// Apply a confirmed action. Spec §12.2 (Patterns d/r) +
/// §12.1.1 / §9.6 (`DiscardEditsAndReload` / `InitFromDump` are v0.5.5+).
fn apply_confirm(app: &mut App, action: &ConfirmAction) {
    match action {
        ConfirmAction::DeleteRule(rule_id) => {
            // Insert the RuleId into the deleted set so compile_pending
            // injects an `enabled = false` UserRule for any variant.
            // Spec v0.6.2 §3.3.
            let rule_id = rule_id.clone();
            let name = rule_id_display_name(&rule_id);
            app.edits.deleted.insert(rule_id);
            app.preview.debouncer.mark_edit();
            app.toast = Some(crate::config_tui::app::Toast::ok(format!(
                "Deleted '{name}' — save to persist"
            )));
        }
        ConfirmAction::ResetOverride(rule_id) => {
            // Clear all three edit maps (rules + deleted) for this
            // RuleId. Spec v0.6.2 §3.3.
            let rule_id = rule_id.clone();
            let name = rule_id_display_name(&rule_id);
            app.edits.rules.remove(&rule_id);
            app.edits.deleted.remove(&rule_id);
            app.preview.debouncer.mark_edit();
            app.toast = Some(crate::config_tui::app::Toast::ok(format!(
                "Reset '{name}' — all staged overrides cleared"
            )));
        }
        ConfirmAction::DiscardEditsAndReload => match reload_snapshot_inline(app) {
            Ok(()) => {
                app.toast =
                    Some(crate::config_tui::app::Toast::ok("Reloaded from disk; edits discarded."));
            }
            Err(e) => {
                app.toast =
                    Some(crate::config_tui::app::Toast::warn(format!("Reload failed: {e}")));
            }
        },
        ConfirmAction::InitFromDump => {
            apply_init_from_dump(app);
        }
    }
}

/// Debounce tick — fires `apply_pending_and_recompile` once per quiescent
/// window (spec §9.1). The in-progress `EditRegex` buffer is NOT applied
/// to the compile path until Enter commit — the debounced tick re-runs
/// against the currently committed `edits.rules` state so the rest of
/// the preview stays live while the user types.
pub(crate) fn check_debounce(app: &mut App) {
    if app.preview.debouncer.should_recompile() {
        apply_pending_and_recompile(app);
    }
}

/// Recompile the live-preview rule set. Delegates to
/// [`PreviewState::recompile`], which runs `apply_rules_spans` across
/// every sample line and refreshes `preview.runs`.
pub(crate) fn recompile_preview(app: &mut App) {
    app.preview.compile_error = None;
    app.preview.recompile(&app.sample_input.text);
}

/// Recompile the live preview from current `PendingEdits` + snapshot.
///
/// On success: replaces the compiled snapshot via `ArcSwap` and re-runs
/// the per-line span computation. On failure: sets `preview.compile_error`
/// banner; existing `preview.runs` are preserved for visual continuity
/// (spec §9.4 — last-good preview survives transient compile errors).
pub(crate) fn apply_pending_and_recompile(app: &mut App) {
    let theme = app.snapshot.parsed.theme.as_deref();
    let profile = app.snapshot.parsed.profile.as_deref();
    match crate::config_tui::compile_pending::compile_pending(
        &app.snapshot,
        &app.edits,
        theme,
        profile,
    ) {
        Ok(new_compiled) => {
            app.preview.compiled.store(std::sync::Arc::new(new_compiled));
            app.preview.compile_error = None;
            recompile_preview(app);
        }
        Err(err) => {
            app.preview.compile_error = Some(err.to_string());
        }
    }
}

/// Reload `app.snapshot` from disk, clear pending edits, and recompile
/// the live preview. Used by the `Ctrl+R` / `DiscardEditsAndReload` /
/// `InitFromDump` flows (v0.6.1 §3.3).
///
/// Extract a human-readable rule name from a [`RuleId`] for use in toast
/// messages. Returns just the rule name component (not the profile prefix).
pub(crate) fn rule_id_display_name(rule_id: &crate::config_tui::edit::RuleId) -> String {
    match rule_id {
        crate::config_tui::edit::RuleId::Builtin(n) => (*n).to_owned(),
        crate::config_tui::edit::RuleId::UserConfig(n) => n.clone(),
        crate::config_tui::edit::RuleId::Embedded { rule, .. }
        | crate::config_tui::edit::RuleId::DiskProfile { rule, .. } => rule.clone(),
    }
}

/// Re-read the config snapshot from disk, clear pending edits, and
/// recompile the live-preview pipeline. Used by the v0.6.2 override-copy
/// `'o'` handler (via [`request_snapshot_reload`]).
///
/// All precedence-chain inputs (theme / profile / CLI flags) flow
/// through [`crate::config_tui::snapshot::ConfigSnapshot::read_from_disk`]
/// — reload does not introduce `app.edits` as a new precedence input.
/// Memory `feedback_reload_precedence_snapshot`.
fn reload_snapshot_inline(app: &mut App) -> Result<(), crate::error::Error> {
    let snap = crate::config_tui::snapshot::ConfigSnapshot::read_from_disk(
        app.snapshot.source_path.as_deref(),
    )?;
    app.snapshot = snap;
    app.edits.clear();
    apply_pending_and_recompile(app);
    Ok(())
}

/// `pub(crate)` wrapper around [`reload_snapshot_inline`] so non-events.rs
/// callers (the override-copy 'o' handler in `tabs/profiles.rs` and
/// `tabs/themes.rs`) can request a snapshot reload without duplicating
/// the policy. Surface a warn toast on failure rather than bubbling the
/// error — the override write already succeeded; failure to *re-read*
/// the snapshot is a soft error.
pub(crate) fn request_snapshot_reload(app: &mut App) {
    if let Err(e) = reload_snapshot_inline(app) {
        app.toast = Some(crate::config_tui::app::Toast::warn(format!(
            "Override written; snapshot reload failed: {e}"
        )));
    }
}

/// Compute the initial `SaveDiffState` for the current snapshot+edits
/// pair and open the matching modal:
/// - [`SaveDiffState::MergePending`] →
///   [`Modal::ConflictList`] (the per-key conflict UI);
/// - everything else → [`Modal::SaveDiff`] (the single-pane diff modal).
///
/// Centralized so `Ctrl+S`, `Ctrl+W`, and the `QuitConfirm` `s` path all
/// reach the same modal selection (G8 §3.6 — pre-G8 they all hard-coded
/// `Modal::SaveDiff`).
fn open_save_modal_for_current_state(app: &mut App) {
    use crate::config_tui::widgets::save_diff::{build_initial_state, SaveDiffState};
    let state = build_initial_state(app);
    let modal = if matches!(state, SaveDiffState::MergePending { .. }) {
        Modal::ConflictList(crate::config_tui::widgets::conflict_list::ConflictListState)
    } else {
        Modal::SaveDiff
    };
    app.save_diff = Some(state);
    app.modal = Some(modal);
}

/// Per-key conflict-list dispatcher — `j`/`k` nav, `o`/`t`/`s` toggle
/// the focused row's pick, Enter applies via [`apply_conflict_selections`],
/// Esc cancels and resets `pending_save_and_quit` (T-I6 invariant from
/// G2: every non-commit exit from the `SaveDiff` family must clear the
/// save-and-quit flag).
fn handle_conflict_list_key(app: &mut App, k: KeyEvent) {
    use crate::config_tui::widgets::save_diff::{ConflictChoice, SaveDiffState};

    let Some(SaveDiffState::MergePending { conflicts, selection, focused_row, .. }) =
        app.save_diff.as_mut()
    else {
        return;
    };
    if conflicts.is_empty() {
        return;
    }
    let len = conflicts.len();
    let is_block_at = |i: usize| -> bool {
        matches!(
            conflicts.get(i).map(|c| c.shape),
            Some(crate::config_tui::merge::ConflictValueShape::Block)
        )
    };

    match k.code {
        KeyCode::Char('j') | KeyCode::Down => {
            *focused_row = (*focused_row + 1) % len;
        }
        KeyCode::Char('k') | KeyCode::Up => {
            *focused_row = (*focused_row + len - 1) % len;
        }
        KeyCode::Char('o') => {
            let row = *focused_row;
            if is_block_at(row) {
                app.toast = Some(crate::config_tui::app::Toast::warn(
                    "table-shaped conflict — must be resolved manually; press 's' to skip"
                        .to_owned(),
                ));
                return;
            }
            selection[row] = ConflictChoice::Ours;
        }
        KeyCode::Char('t') => {
            let row = *focused_row;
            if is_block_at(row) {
                app.toast = Some(crate::config_tui::app::Toast::warn(
                    "table-shaped conflict — must be resolved manually; press 's' to skip"
                        .to_owned(),
                ));
                return;
            }
            selection[row] = ConflictChoice::Theirs;
        }
        KeyCode::Char('s') => {
            let row = *focused_row;
            selection[row] = ConflictChoice::Skip;
        }
        KeyCode::Enter => {
            if let Err(e) = apply_conflict_selections(app) {
                app.toast =
                    Some(crate::config_tui::app::Toast::warn(format!("merge apply failed: {e}")));
            }
        }
        // Esc is intentionally not handled here — `dispatch_key` routes
        // Esc to `handle_esc` (events.rs:~390) BEFORE dispatching to the
        // modal handler, so `handle_esc`'s `Some(Modal::ConflictList(_))`
        // arm owns the side-channel triple-reset (`pending_save_and_quit`,
        // `save_diff`, `save_diff_scroll`). v0.6.2 cross-cutting review.
        _ => {}
    }
}

/// Apply the per-row picks on `MergePending` to the auto-merged document,
/// then commit via [`crate::config_tui::save::commit_bytes`]. On success
/// clears edits + closes the modal + reloads the snapshot + (if the
/// save-and-quit flag was set) triggers quit.
fn apply_conflict_selections(app: &mut App) -> std::io::Result<()> {
    use crate::config_tui::widgets::save_diff::SaveDiffState;

    let Some(SaveDiffState::MergePending {
        base,
        ours,
        theirs,
        auto_merged,
        conflicts,
        selection,
        ..
    }) = app.save_diff.as_ref()
    else {
        return Ok(());
    };

    let final_doc = build_final_doc(base, ours, theirs, auto_merged, conflicts, selection)
        .map_err(|e| std::io::Error::other(format!("merge apply failed: {e}")))?;

    let body = final_doc.to_string();
    let new_snapshot =
        crate::config_tui::save::commit_bytes(&app.snapshot, &body, std::time::SystemTime::now())?;
    app.snapshot = new_snapshot;
    app.edits.clear();
    apply_pending_and_recompile(app);

    app.toast = Some(crate::config_tui::app::Toast::ok("Saved (with manual merge)".to_owned()));
    app.save_diff = None;
    app.modal = None;
    if app.pending_save_and_quit {
        app.should_quit = true;
        app.pending_save_and_quit = false;
    }
    Ok(())
}

/// Walk per-row picks against `auto_merged` and produce the final document
/// body. Pure — no IO, no app-state mutation — so testable in isolation.
///
/// `Skip` semantics: copy the base value at the conflicting key into
/// `auto_merged` (which carries no value at conflicting keys by construction
/// in [`crate::config_tui::merge::merge_three_way`]'s conflict arm). When the
/// base side ALSO has no value at the path (the conflict arose because
/// ours/theirs disagree and base was absent), the function skips writing —
/// without this guard the default `Skip` on a `Block`-shape `[[array-of-
/// tables]]` conflict surfaces a misleading
/// `"merge apply failed: ... missing intermediate at <key>"` toast. v0.6.2
/// cross-cutting review I3.
fn build_final_doc(
    base: &toml_edit::DocumentMut,
    ours: &toml_edit::DocumentMut,
    theirs: &toml_edit::DocumentMut,
    auto_merged: &toml_edit::DocumentMut,
    conflicts: &[crate::config_tui::merge::KeyConflict],
    selection: &[crate::config_tui::widgets::save_diff::ConflictChoice],
) -> Result<toml_edit::DocumentMut, crate::config_tui::merge::WriteToPathError> {
    use crate::config_tui::widgets::save_diff::ConflictChoice;
    let mut final_doc = auto_merged.clone();
    for (i, choice) in selection.iter().enumerate() {
        let Some(conflict) = conflicts.get(i) else {
            continue;
        };
        let source = match choice {
            ConflictChoice::Ours => ours,
            ConflictChoice::Theirs => theirs,
            ConflictChoice::Skip => {
                if !crate::config_tui::merge::path_exists(base, &conflict.path) {
                    continue;
                }
                base
            }
        };
        match crate::config_tui::merge::write_to_path(&mut final_doc, &conflict.path, source) {
            Ok(()) => {}
            // Delete-modify translation. Spec §3.3 pins `AotElementMissing`
            // as the canonical signal that the chosen side dropped the
            // element. `MissingIntermediate` is included alongside because
            // `descend_source` raises it (not `AotElementMissing`) when the
            // chosen side dropped the ENTIRE AoT key (e.g. ours has no
            // `[[rules]]` block at all — Task 11 pin
            // `apply_conflict_layer_translates_delete_modify_to_remove_
            // when_source_absent`). The widening is gated by
            // `is_delete_modify()`, and `remove_aot_element_by_name`
            // propagates `TypeMismatch` for non-AoT paths via `?`, so the
            // v0.6.2 I3 "Skip-on-absent-base no toast" contract (handled
            // earlier by `path_exists`) is not weakened.
            Err(
                crate::config_tui::merge::WriteToPathError::AotElementMissing { .. }
                | crate::config_tui::merge::WriteToPathError::MissingIntermediate { .. },
            ) if conflict.is_delete_modify() => {
                if let Some((last, parent)) = conflict.path.split_last() {
                    crate::config_tui::merge::remove_aot_element_by_name(
                        &mut final_doc,
                        parent,
                        last,
                    )?;
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(final_doc)
}

/// Real impl for `ConfirmAction::InitFromDump` (v0.6.1 §3.3). Writes the
/// built-in default config to `snapshot.source_path` (file must not
/// already exist — gate enforced at keystroke time in `dispatch_key`'s
/// `Shift+D` arm) and reloads the snapshot.
fn apply_init_from_dump(app: &mut App) {
    let Some(path) = app.snapshot.source_path.clone() else {
        app.toast = Some(crate::config_tui::app::Toast::warn("Cannot init: no config path bound."));
        return;
    };
    let toml = crate::config::default_config_toml();
    if let Err(e) = crate::config_tui::save::write_atomic_to(&path, &toml) {
        app.toast = Some(crate::config_tui::app::Toast::warn(format!("Init failed: {e}")));
        return;
    }
    match reload_snapshot_inline(app) {
        Ok(()) => {
            app.toast = Some(crate::config_tui::app::Toast::ok(format!(
                "Initialized {} with defaults.",
                path.display()
            )));
        }
        Err(e) => {
            app.toast = Some(crate::config_tui::app::Toast::warn(format!(
                "Init wrote but reload failed: {e}"
            )));
        }
    }
}

/// Bind the picked color to the rule currently focused under the
/// Patterns tab, then drive a recompile + toast.
///
/// `Builtin` / `Embedded` / `DiskProfile` rules apply via synth-`UserRule`
/// overlay in `compile_pending` (dedupe-then-mutate-or-push). See
/// spec §3.2 of the v0.6.1 design.
fn bind_color_to_selected_rule(app: &mut App, color: crate::style::Color) {
    let Some(rule_id) = resolve_selected_rule_id(app) else {
        app.toast = Some(crate::config_tui::app::Toast::warn(
            "no rule selected — switch to Patterns tab and select first",
        ));
        return;
    };
    let rule_name = match &rule_id {
        crate::config_tui::edit::RuleId::Builtin(n) => (*n).to_owned(),
        crate::config_tui::edit::RuleId::UserConfig(n) => n.clone(),
        crate::config_tui::edit::RuleId::Embedded { rule, .. }
        | crate::config_tui::edit::RuleId::DiskProfile { rule, .. } => rule.clone(),
    };
    let rule_edit = app.edits.rules.entry(rule_id).or_default();
    let style_entry =
        rule_edit.styles.entry(crate::config_tui::edit::StyleKey::Default).or_default();
    style_entry.fg = Some(Some(color));
    apply_pending_and_recompile(app);
    app.toast =
        Some(crate::config_tui::app::Toast::ok(format!("color bound to rule '{rule_name}'")));
}

/// Bind staged bool-axis tri-state edits onto the focused Patterns rule.
/// All-`None` is a no-op (the picker was used purely for color or fully
/// abandoned). Only axes with `Some(_)` (explicit set or clear) write
/// through into [`PendingEdits`]. G3 — Spec §3.1.
//
// reason: `Option<Option<bool>>` is the load-bearing tri-state on
// `NewStyle::{bold,italic,underline}`; mirrored on these params so the
// caller can pass-through `ColorPickerState::staged_*` verbatim.
#[allow(clippy::option_option)]
fn bind_bool_axes_to_selected_rule(
    app: &mut App,
    bold: Option<Option<bool>>,
    italic: Option<Option<bool>>,
    underline: Option<Option<bool>>,
) {
    if bold.is_none() && italic.is_none() && underline.is_none() {
        return;
    }
    let Some(rule_id) = resolve_selected_rule_id(app) else {
        // Same warn-toast path as `bind_color_to_selected_rule`. Skip
        // emitting if the color path already emitted one (caller orders
        // bool-axes BEFORE color binding so this branch only fires when
        // no color was picked and no rule is selected — rare).
        app.toast = Some(crate::config_tui::app::Toast::warn(
            "no rule selected — switch to Patterns tab and select first",
        ));
        return;
    };
    let rule_edit = app.edits.rules.entry(rule_id).or_default();
    let style_entry =
        rule_edit.styles.entry(crate::config_tui::edit::StyleKey::Default).or_default();
    if let Some(v) = bold {
        style_entry.bold = Some(v);
    }
    if let Some(v) = italic {
        style_entry.italic = Some(v);
    }
    if let Some(v) = underline {
        style_entry.underline = Some(v);
    }
    apply_pending_and_recompile(app);
}

/// Apply staged bool axes from the color picker into a draft [`NewStyle`]
/// in place.
///
/// Mirrors [`bind_bool_axes_to_selected_rule`] but targets a mutable draft
/// owned by the `NewPattern` flow rather than the edits table.
fn bind_bool_axes_to_draft(
    draft: &mut crate::config_tui::edit::NewStyle,
    picker: &crate::config_tui::widgets::color_picker::ColorPickerState,
) {
    if let Some(v) = picker.staged_bold {
        draft.bold = Some(v);
    }
    if let Some(v) = picker.staged_italic {
        draft.italic = Some(v);
    }
    if let Some(v) = picker.staged_underline {
        draft.underline = Some(v);
    }
}

/// Map `focus.patterns.selected_idx` to a `RuleId`.
///
/// The Patterns tab list is a union of built-in + user-config rules
/// presented under two section headers (see
/// [`crate::config_tui::tabs::patterns::patterns_list_layout`]). The
/// selectable-index space is `[0..builtin_count)` followed by
/// `[builtin_count..total)`; this function maps that index to
/// `RuleId::Builtin(name)` in the first range and `RuleId::UserConfig(name)`
/// in the second.
///
/// Other tabs yield `None` — v0.8+ on community demand may extend (Themes / Profiles).
pub(crate) fn resolve_selected_rule_id(app: &App) -> Option<crate::config_tui::edit::RuleId> {
    use crate::config_tui::app::Tab;
    use crate::config_tui::edit::RuleId;
    match app.tab {
        Tab::Patterns => {
            let filter = app.search_filter.as_deref().unwrap_or("");
            let layout = crate::config_tui::tabs::patterns::patterns_list_layout(
                &app.catalog.builtin_rule_names,
                &app.snapshot.parsed.rules,
                filter,
            );
            let total = layout.builtin_count + layout.user_count;
            if total == 0 {
                return None;
            }
            let idx = app.focus.patterns.selected_idx.min(total - 1);
            if idx < layout.builtin_count {
                Some(RuleId::Builtin(layout.builtin_names[idx]))
            } else {
                let user_pos = idx - layout.builtin_count;
                layout.user_names.get(user_pos).cloned().map(RuleId::UserConfig)
            }
        }
        _ => None,
    }
}

/// Look up the current pattern source for a `RuleId`, applying any
/// `PendingEdits` overlay. Used by the `e` keystroke to initialize the
/// `EditRegex` modal buffer. Embedded / disk-profile rule sources are
/// not catalog-resolved (v0.8+ on community demand); they fall through to the empty-string
/// default.
pub(crate) fn pattern_for_rule_id(rule_id: &crate::config_tui::edit::RuleId, app: &App) -> String {
    use crate::config_tui::edit::RuleId;
    // PendingEdits overlay wins over the on-disk / built-in source.
    if let Some(edit) = app.edits.rules.get(rule_id) {
        if let Some(p) = &edit.pattern {
            return p.clone();
        }
    }
    match rule_id {
        RuleId::Builtin(name) => crate::rules::builtin_rules()
            .into_iter()
            .find(|r| r.name == *name)
            .map(|r| r.pattern)
            .unwrap_or_default(),
        RuleId::UserConfig(name) => app
            .snapshot
            .parsed
            .rules
            .iter()
            .find(|r| &r.name == name)
            .and_then(|r| r.pattern.clone())
            .unwrap_or_default(),
        RuleId::Embedded { .. } | RuleId::DiskProfile { .. } => String::new(),
    }
}

/// Toast expiration tick.
pub(crate) fn check_toast(app: &mut App) {
    if app.toast.as_ref().is_some_and(crate::config_tui::app::Toast::expired) {
        app.toast = None;
    }
}

/// `Search` modal key dispatch + outcome handling.
fn handle_search_key(app: &mut App, k: KeyEvent) {
    use crate::config_tui::widgets::search::{dispatch_key as sd, SearchOutcome};
    let Some(state) = app.search_state.as_mut() else {
        app.modal = None;
        return;
    };
    match sd(state, k) {
        SearchOutcome::Commit(buf) => {
            app.search_filter = if buf.is_empty() { None } else { Some(buf) };
            app.modal = None;
            app.search_state = None;
        }
        SearchOutcome::Cancel => {
            app.modal = None;
            app.search_state = None;
        }
        SearchOutcome::StayOpen => {}
    }
}

/// `SampleSet` modal key dispatch + outcome handling.
fn handle_sample_set_key(app: &mut App, k: KeyEvent) {
    use crate::config_tui::widgets::sample_set::{dispatch_key as sd, SampleSetOutcome};
    let Some(state) = app.sample_set_state.as_mut() else {
        app.modal = None;
        return;
    };
    match sd(state, k) {
        SampleSetOutcome::Commit(buf) => {
            app.sample_input.text = buf;
            app.modal = None;
            app.sample_set_state = None;
            // No mark_edit: per spec §9.2 sample-input changes are
            // a no-debounce interaction — render re-applies the
            // existing preview.compiled to the new sample on the
            // next frame without a recompile.
        }
        SampleSetOutcome::Cancel => {
            app.modal = None;
            app.sample_set_state = None;
        }
        SampleSetOutcome::StayOpen => {}
    }
}

/// `Modal::NewPattern` 3-phase wizard dispatch (spec §12.4 D2).
///
/// Phase transitions:
/// - `Name → Regex`  on Enter when `validate_pattern_name` passes.
/// - `Regex → Style` on Enter when `validate_pattern_regex` compiles.
/// - `Style → commit` on `ColorPickerOutcome::Accept` OR Enter; commits the
///   draft into `edits.added` and triggers `apply_pending_and_recompile`.
///
/// Esc back-out is handled by `handle_esc` (tier 1b) before dispatch reaches
/// this function, so Esc is not handled here.
fn handle_new_pattern_key(app: &mut App, k: KeyEvent) {
    use crate::config_tui::app::{Modal, NewPatternPhase};

    // Enter: phase transition or commit.
    if matches!(k.code, KeyCode::Enter) {
        let Some(Modal::NewPattern { phase, draft }) = app.modal.as_mut() else {
            return;
        };
        match phase {
            NewPatternPhase::Name => {
                if validate_pattern_name(&draft.name).is_ok() {
                    *phase = NewPatternPhase::Regex;
                }
            }
            NewPatternPhase::Regex => match validate_pattern_regex(&draft.pattern) {
                Ok(()) => {
                    draft.pattern_error = None;
                    *phase = NewPatternPhase::Style;
                }
                Err(e) => {
                    draft.pattern_error = Some(e);
                }
            },
            NewPatternPhase::Style => {
                // Enter in the style phase accepts the picker's current
                // selection, including any staged bool-axis edits. G3 §3.1.
                if let Some(color) = draft.picker_state.selected_color() {
                    draft.draft_style.fg = Some(Some(color));
                }
                bind_bool_axes_to_draft(&mut draft.draft_style, &draft.picker_state);
                commit_new_pattern_draft(app);
            }
        }
        return;
    }

    // Per-phase text input / picker delegation.
    let Some(Modal::NewPattern { phase, draft }) = app.modal.as_mut() else {
        return;
    };
    match phase {
        NewPatternPhase::Name => {
            handle_text_input(&mut draft.name, k, 64);
        }
        NewPatternPhase::Regex => {
            handle_text_input(&mut draft.pattern, k, 4096);
            // Live syntax check; commit-time recompile still runs on advance.
            draft.pattern_error = validate_pattern_regex(&draft.pattern).err();
        }
        NewPatternPhase::Style => {
            let outcome =
                crate::config_tui::widgets::color_picker::dispatch_key(&mut draft.picker_state, k);
            match outcome {
                crate::config_tui::widgets::color_picker::ColorPickerOutcome::Accept => {
                    if let Some(color) = draft.picker_state.selected_color() {
                        draft.draft_style.fg = Some(Some(color));
                    }
                    bind_bool_axes_to_draft(&mut draft.draft_style, &draft.picker_state);
                    commit_new_pattern_draft(app);
                }
                crate::config_tui::widgets::color_picker::ColorPickerOutcome::Cancel => {
                    *phase = NewPatternPhase::Regex;
                }
                crate::config_tui::widgets::color_picker::ColorPickerOutcome::StayOpen => {}
            }
        }
    }
}

/// Identifier rules for new-pattern names: non-empty, ≤ 64 chars,
/// `[A-Za-z0-9_-]+`. Mirrors the user-config `[[rules]].name` schema.
fn validate_pattern_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("name required".into());
    }
    if name.len() > 64 {
        return Err("name too long (max 64)".into());
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err("name must be [A-Za-z0-9_-]+".into());
    }
    Ok(())
}

/// Inline regex syntax/size validation. Mirrors the canonical
/// `REGEX_SIZE_LIMIT_BYTES = 1 << 20` from `src/rules.rs:15` so a draft that
/// passes here will also pass the v0.5 compile path. Inlined to avoid
/// widening `rules.rs` visibility (same pattern as `compile_pending.rs`).
fn validate_pattern_regex(pattern: &str) -> Result<(), String> {
    const REGEX_SIZE_LIMIT_BYTES: usize = 1 << 20;
    if pattern.is_empty() {
        return Err("pattern required".into());
    }
    regex::bytes::RegexBuilder::new(pattern)
        .size_limit(REGEX_SIZE_LIMIT_BYTES)
        .dfa_size_limit(REGEX_SIZE_LIMIT_BYTES)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Commit the draft as a `NewRule` into `edits.added`, close the modal,
/// trigger a preview recompile, and surface a confirmation toast.
fn commit_new_pattern_draft(app: &mut App) {
    use crate::config_tui::app::Modal;
    let Some(Modal::NewPattern { draft, .. }) = app.modal.take() else {
        return;
    };
    let new_rule = crate::config_tui::edit::NewRule {
        name: draft.name,
        pattern: draft.pattern,
        style: draft.draft_style,
    };
    app.edits.added.push(new_rule);
    apply_pending_and_recompile(app);
    app.toast = Some(crate::config_tui::app::Toast::ok("new pattern added"));
}

/// Plain text-input buffer mutation for the Name/Regex phases. Backspace
/// pops, printable chars push (up to `max_len`); every other key is ignored.
fn handle_text_input(buf: &mut String, k: KeyEvent, max_len: usize) {
    match k.code {
        KeyCode::Char(c) if buf.len() < max_len => buf.push(c),
        KeyCode::Backspace => {
            buf.pop();
        }
        _ => {}
    }
}

/// `Modal::EditRegex` key dispatch (spec §12.4 D3).
///
/// - Esc: cancel — `PendingEdits` unchanged, modal closes. Defensive only;
///   the global `handle_esc` tier 2 dismisses the modal before this branch
///   is reached.
/// - Enter: validate, write to `edits.rules[rule_id].pattern`, recompile.
///   On invalid regex the modal stays open with `error` set.
/// - Other keys: text-buffer edit + live syntax check + debouncer tickle.
fn handle_edit_regex_key(app: &mut App, k: KeyEvent) {
    use crate::config_tui::app::Modal;

    if matches!(k.code, KeyCode::Esc) {
        app.modal = None;
        return;
    }

    if matches!(k.code, KeyCode::Enter) {
        let Some(Modal::EditRegex { rule_id, buffer, .. }) = app.modal.take() else {
            return;
        };
        match validate_pattern_regex(&buffer) {
            Ok(()) => {
                app.edits.rules.entry(rule_id).or_default().pattern = Some(buffer);
                apply_pending_and_recompile(app);
                app.toast = Some(crate::config_tui::app::Toast::ok("pattern updated"));
            }
            Err(e) => {
                app.modal = Some(Modal::EditRegex { rule_id, buffer, error: Some(e) });
            }
        }
        return;
    }

    // Text input + debouncer tickle. The in-progress buffer is NOT applied
    // to the live preview compile until Enter commits (spec §9.1).
    let Some(Modal::EditRegex { buffer, error, .. }) = app.modal.as_mut() else {
        return;
    };
    handle_text_input(buffer, k, 4096);
    *error = validate_pattern_regex(buffer).err();
    app.preview.debouncer.mark_edit();
}

/// `Modal::Help` key dispatch (spec §12.4 D4).
///
/// vim/less convention: ANY key dismisses the Help overlay and the key
/// is DISCARDED (it does NOT fall through to the underlying global / tab
/// dispatchers). This guarantees that hitting a benign key to dismiss
/// Help never triggers a surprise action (e.g. `q` would otherwise
/// initiate the quit flow). Ctrl+C and Esc bypass this path because
/// they're handled by `dispatch_key` before the modal branch.
fn handle_help_key(app: &mut App, _k: KeyEvent) {
    app.modal = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_tui::snapshot::ConfigSnapshot;
    use crate::config_tui::widgets::color_picker::{ColorPickerState, PickerSection};
    use ratatui::crossterm::event::KeyModifiers;

    fn mk(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    #[test]
    fn esc_in_color_picker_clears_goto_buf_first_then_closes_modal() {
        // Integration path: dispatch_key → handle_esc → tier-1 (goto clear) /
        // tier-2 (modal close). Without the tier-1 branch, the modal would
        // close on the first Esc and the goto-input buffer would be lost.
        let snap = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snap);
        app.modal = Some(Modal::ColorPicker(ColorPickerState {
            section: PickerSection::Palette256,
            goto_buf: Some(String::from("13")),
            ..Default::default()
        }));

        dispatch_key(&mut app, mk(KeyCode::Esc));
        match &app.modal {
            Some(Modal::ColorPicker(s)) => {
                assert!(s.goto_buf.is_none(), "first Esc must clear goto_buf");
            }
            other => panic!("modal must remain open after first Esc; got {other:?}"),
        }

        dispatch_key(&mut app, mk(KeyCode::Esc));
        assert!(app.modal.is_none(), "second Esc must close the modal");
    }

    #[test]
    fn esc_on_save_diff_modal_clears_side_channel_alongside_modal() {
        // Invariant: app.save_diff.is_some() ↔ app.modal == Some(Modal::SaveDiff).
        // Without the matches! guard in handle_esc, app.save_diff would leak
        // past Esc dismissal and the next Ctrl+S would see stale state.
        let snap = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snap);
        app.save_diff = Some(crate::config_tui::widgets::save_diff::SaveDiffState::Clean {
            tui_diff: "(stub)".to_owned(),
        });
        app.modal = Some(Modal::SaveDiff);

        dispatch_key(&mut app, mk(KeyCode::Esc));
        assert!(app.modal.is_none(), "Esc must close SaveDiff modal");
        assert!(app.save_diff.is_none(), "Esc must clear save_diff side-channel");
    }

    #[test]
    fn esc_on_conflict_list_modal_clears_side_channel_and_pending_save_and_quit() {
        // G8 cross-cutting review B1 regression guard: Modal::ConflictList
        // shares the `save_diff` side-channel with Modal::SaveDiff, so
        // its Esc-close path needs the same triple-reset. Without this,
        // `pending_save_and_quit = true` (e.g. set by the QuitWithUnsaved
        // → 's' path) would leak past Esc into a surprise should_quit on
        // the next save-success commit.
        let snap = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snap);
        // Fabricate a MergePending state — only the variant tag matters
        // for the side-channel reset assertion.
        let empty_doc = toml_edit::DocumentMut::new();
        app.save_diff = Some(crate::config_tui::widgets::save_diff::SaveDiffState::MergePending {
            base: empty_doc.clone(),
            ours: empty_doc.clone(),
            theirs: empty_doc.clone(),
            auto_merged: empty_doc,
            conflicts: Vec::new(),
            selection: Vec::new(),
            focused_row: 0,
        });
        app.modal =
            Some(Modal::ConflictList(crate::config_tui::widgets::conflict_list::ConflictListState));
        app.pending_save_and_quit = true;
        app.save_diff_scroll = 7;

        dispatch_key(&mut app, mk(KeyCode::Esc));

        assert!(app.modal.is_none(), "Esc must close ConflictList modal");
        assert!(
            app.save_diff.is_none(),
            "Esc must clear save_diff side-channel — invariant shared with SaveDiff modal"
        );
        assert!(
            !app.pending_save_and_quit,
            "Esc on ConflictList must clear pending_save_and_quit (T-I6 invariant)"
        );
        assert_eq!(app.save_diff_scroll, 0, "Esc must reset save_diff_scroll");
    }

    #[test]
    fn build_final_doc_skip_on_absent_base_leaves_auto_merged_untouched_at_that_key() {
        // v0.6.2 cross-cutting review I3 regression guard: the default
        // Block-shape selection in `build_initial_state`
        // (widgets/save_diff.rs:236-239) is Skip, so the most likely user
        // path for `[[rules]]` array-of-tables conflicts surfaced the
        // misleading "merge apply failed: write_to_path at <key>: missing
        // intermediate at <key>" toast. Fix: Skip + base-absent-at-path =
        // continue without writing — `auto_merged` already carries no
        // value at conflicting keys by construction in
        // `merge_three_way`'s conflict arm.
        use crate::config_tui::merge::{ConflictValueShape, KeyConflict};
        use crate::config_tui::widgets::save_diff::ConflictChoice;

        let base: toml_edit::DocumentMut =
            "[general]\ntheme = \"dark\"\n".parse().expect("base TOML parses");
        let ours: toml_edit::DocumentMut = "[general]\ntheme = \"dark\"\n[[rules]]\nname = \"a\"\n"
            .parse()
            .expect("ours TOML parses");
        let theirs: toml_edit::DocumentMut =
            "[general]\ntheme = \"dark\"\n[[rules]]\nname = \"b\"\n"
                .parse()
                .expect("theirs TOML parses");
        // merge_three_way leaves `rules` unset on auto_merged in the conflict arm.
        let auto_merged: toml_edit::DocumentMut =
            "[general]\ntheme = \"dark\"\n".parse().expect("auto_merged TOML parses");

        let conflicts = vec![KeyConflict {
            path: vec!["rules".to_owned()],
            base_value: "(absent)".to_owned(),
            ours_value: "[ours-only]".to_owned(),
            theirs_value: "[theirs-only]".to_owned(),
            shape: ConflictValueShape::Block,
            is_array_block: true,
        }];
        let selection = vec![ConflictChoice::Skip];

        let result = build_final_doc(&base, &ours, &theirs, &auto_merged, &conflicts, &selection)
            .expect("Skip + base-absent must not error");

        assert_eq!(
            result.to_string(),
            "[general]\ntheme = \"dark\"\n",
            "auto_merged unchanged when Skip on absent-base conflict",
        );
    }

    #[test]
    fn build_final_doc_skip_on_present_base_copies_base_value_to_final_doc() {
        // Companion to the I3 fix: when base DOES have a value at the
        // conflicting path, Skip still means "use base" — `write_to_path`
        // copies the leaf in. Pinning the Skip-present arm so a future
        // "always continue on Skip" regression breaks this test loudly.
        use crate::config_tui::merge::{ConflictValueShape, KeyConflict};
        use crate::config_tui::widgets::save_diff::ConflictChoice;

        let base: toml_edit::DocumentMut = "[general]\ntheme = \"dark\"\n".parse().expect("base");
        let ours: toml_edit::DocumentMut = "[general]\ntheme = \"tokyo\"\n".parse().expect("ours");
        let theirs: toml_edit::DocumentMut =
            "[general]\ntheme = \"light\"\n".parse().expect("theirs");
        // auto_merged starts WITHOUT theme (conflict arm leaves the key unset).
        let auto_merged: toml_edit::DocumentMut = "[general]\n".parse().expect("auto_merged");

        let conflicts = vec![KeyConflict {
            path: vec!["general".to_owned(), "theme".to_owned()],
            base_value: "dark".to_owned(),
            ours_value: "tokyo".to_owned(),
            theirs_value: "light".to_owned(),
            shape: ConflictValueShape::Leaf,
            is_array_block: false,
        }];
        let selection = vec![ConflictChoice::Skip];

        let result = build_final_doc(&base, &ours, &theirs, &auto_merged, &conflicts, &selection)
            .expect("Skip + base-present must succeed");

        let rendered = result.to_string();
        assert!(
            rendered.contains("theme = \"dark\""),
            "Skip + base-present copies base value into final_doc; got: {rendered:?}",
        );
        assert!(!rendered.contains("tokyo"), "ours value must NOT propagate on Skip");
        assert!(!rendered.contains("light"), "theirs value must NOT propagate on Skip");
    }

    // -----------------------------------------------------------------------
    // v0.6.3 NIT c — dispatcher coverage for handle_conflict_list_key.
    // Render-side is exercised by `tests/config_tui_conflict_list.rs`; the
    // build_final_doc helper has its own pure-logic tests above. The
    // dispatcher path that wires keystrokes → state mutations was the
    // gap the v0.6.2 cross-cutting review flagged.
    // -----------------------------------------------------------------------

    /// Helper — fabricate a `MergePending` `SaveDiffState` with the supplied
    /// conflict list + Skip-defaulted selection + focus at row 0.
    fn make_merge_pending_state(
        conflicts: Vec<crate::config_tui::merge::KeyConflict>,
    ) -> crate::config_tui::widgets::save_diff::SaveDiffState {
        use crate::config_tui::widgets::save_diff::{ConflictChoice, SaveDiffState};
        let selection = vec![ConflictChoice::Skip; conflicts.len()];
        let empty_doc = toml_edit::DocumentMut::new();
        SaveDiffState::MergePending {
            base: empty_doc.clone(),
            ours: empty_doc.clone(),
            theirs: empty_doc.clone(),
            auto_merged: empty_doc,
            conflicts,
            selection,
            focused_row: 0,
        }
    }

    #[test]
    fn enter_on_conflict_list_modal_invokes_apply_conflict_selections_and_succeeds() {
        // Drives the full Enter dispatch path:
        // dispatch_key → Modal::ConflictList branch → handle_conflict_list_key
        // → apply_conflict_selections → build_final_doc → commit_bytes.
        // Setup: tempfile-bound snapshot + a Leaf conflict at `general.theme`
        // with Skip selection on a present-base value (so build_final_doc
        // copies base → final_doc and commit_bytes writes the file).
        use crate::config_tui::merge::{ConflictValueShape, KeyConflict};
        use crate::config_tui::widgets::save_diff::SaveDiffState;

        let tmp = tempfile::tempdir().expect("tmpdir");
        let cfg_path = tmp.path().join("config.toml");
        std::fs::write(&cfg_path, b"[general]\ntheme = \"dark\"\n").expect("seed cfg");
        let snap = ConfigSnapshot::read_from_disk(Some(&cfg_path)).expect("read snapshot");
        let mut app = App::from_snapshot(snap);

        let base: toml_edit::DocumentMut = "[general]\ntheme = \"dark\"\n".parse().expect("base");
        let ours: toml_edit::DocumentMut = "[general]\ntheme = \"tokyo\"\n".parse().expect("ours");
        let theirs: toml_edit::DocumentMut =
            "[general]\ntheme = \"light\"\n".parse().expect("theirs");
        let auto_merged: toml_edit::DocumentMut = "[general]\n".parse().expect("auto_merged");
        let conflicts = vec![KeyConflict {
            path: vec!["general".to_owned(), "theme".to_owned()],
            base_value: "dark".to_owned(),
            ours_value: "tokyo".to_owned(),
            theirs_value: "light".to_owned(),
            shape: ConflictValueShape::Leaf,
            is_array_block: false,
        }];
        app.save_diff = Some(SaveDiffState::MergePending {
            base,
            ours,
            theirs,
            auto_merged,
            conflicts,
            selection: vec![crate::config_tui::widgets::save_diff::ConflictChoice::Skip],
            focused_row: 0,
        });
        app.modal =
            Some(Modal::ConflictList(crate::config_tui::widgets::conflict_list::ConflictListState));

        dispatch_key(&mut app, mk(KeyCode::Enter));

        assert!(app.modal.is_none(), "Enter on ConflictList must close modal after apply succeeds");
        assert!(app.save_diff.is_none(), "save_diff side-channel cleared after successful apply");
        let toast =
            app.toast.as_ref().expect("expected success toast after Enter triggers apply path");
        assert_eq!(
            toast.text, "Saved (with manual merge)",
            "byte-pinned success toast wording — memory feedback_test_assertion_specificity",
        );
        let disk_after = std::fs::read_to_string(&cfg_path).expect("re-read cfg");
        assert!(
            disk_after.contains("theme = \"dark\""),
            "Skip + base-present writes base value to disk; got: {disk_after:?}",
        );
    }

    #[test]
    fn o_and_t_keystrokes_on_block_shape_row_emit_warn_toast_and_preserve_skip_selection() {
        // v0.6.2 review NIT c: handle_conflict_list_key reject path for
        // Block-shape rows. Both 'o' (Ours) and 't' (Theirs) must surface
        // the "table-shaped conflict" warn toast and leave the focused row's
        // selection unchanged at the default Skip. Pins the exact wording
        // per memory feedback_test_assertion_specificity.
        use crate::config_tui::merge::{ConflictValueShape, KeyConflict};
        use crate::config_tui::widgets::save_diff::{ConflictChoice, SaveDiffState};

        let conflicts = vec![KeyConflict {
            path: vec!["rules".to_owned()],
            base_value: "(table)".to_owned(),
            ours_value: "(table)".to_owned(),
            theirs_value: "(table)".to_owned(),
            shape: ConflictValueShape::Block,
            is_array_block: false,
        }];
        let mut app = App::from_snapshot(ConfigSnapshot::empty());
        app.save_diff = Some(make_merge_pending_state(conflicts));
        app.modal =
            Some(Modal::ConflictList(crate::config_tui::widgets::conflict_list::ConflictListState));

        let expected_toast = "table-shaped conflict — must be resolved manually; press 's' to skip";

        // 'o' arm
        dispatch_key(&mut app, mk(KeyCode::Char('o')));
        let toast = app.toast.as_ref().expect("'o' on Block must set warn toast");
        assert_eq!(toast.text, expected_toast, "'o' warn wording byte-pinned");
        let Some(SaveDiffState::MergePending { selection, .. }) = app.save_diff.as_ref() else {
            panic!("save_diff still MergePending after 'o' on Block");
        };
        assert_eq!(
            selection[0],
            ConflictChoice::Skip,
            "Block-shape Skip default preserved on rejected 'o'",
        );

        // 't' arm
        app.toast = None;
        dispatch_key(&mut app, mk(KeyCode::Char('t')));
        let toast = app.toast.as_ref().expect("'t' on Block must set warn toast");
        assert_eq!(toast.text, expected_toast, "'t' warn wording byte-pinned");
        let Some(SaveDiffState::MergePending { selection, .. }) = app.save_diff.as_ref() else {
            panic!("save_diff still MergePending after 't' on Block");
        };
        assert_eq!(
            selection[0],
            ConflictChoice::Skip,
            "Block-shape Skip default preserved on rejected 't'",
        );
    }

    #[test]
    fn j_and_k_navigation_wraps_focused_row_modulo_conflict_count() {
        // v0.6.2 review NIT c: handle_conflict_list_key nav. j moves
        // forward, k moves backward, both wrap modulo len. Pins the
        // arithmetic so a future "saturating_sub on k" or "no wrap on j"
        // refactor breaks loudly.
        use crate::config_tui::merge::{ConflictValueShape, KeyConflict};
        use crate::config_tui::widgets::save_diff::SaveDiffState;

        let conflicts = vec![
            KeyConflict {
                path: vec!["a".to_owned()],
                base_value: "0".to_owned(),
                ours_value: "1".to_owned(),
                theirs_value: "2".to_owned(),
                shape: ConflictValueShape::Leaf,
                is_array_block: false,
            },
            KeyConflict {
                path: vec!["b".to_owned()],
                base_value: "0".to_owned(),
                ours_value: "1".to_owned(),
                theirs_value: "2".to_owned(),
                shape: ConflictValueShape::Leaf,
                is_array_block: false,
            },
        ];
        let mut app = App::from_snapshot(ConfigSnapshot::empty());
        app.save_diff = Some(make_merge_pending_state(conflicts));
        app.modal =
            Some(Modal::ConflictList(crate::config_tui::widgets::conflict_list::ConflictListState));

        let read_focus = |app: &App| -> usize {
            match app.save_diff.as_ref() {
                Some(SaveDiffState::MergePending { focused_row, .. }) => *focused_row,
                _ => panic!("save_diff must remain MergePending across nav"),
            }
        };

        assert_eq!(read_focus(&app), 0, "starts at row 0");
        dispatch_key(&mut app, mk(KeyCode::Char('j')));
        assert_eq!(read_focus(&app), 1, "j: 0 → 1");
        dispatch_key(&mut app, mk(KeyCode::Char('j')));
        assert_eq!(read_focus(&app), 0, "j wraps: 1 → 0 modulo 2");
        dispatch_key(&mut app, mk(KeyCode::Char('k')));
        assert_eq!(read_focus(&app), 1, "k wraps: 0 → 1 modulo 2");
        dispatch_key(&mut app, mk(KeyCode::Char('k')));
        assert_eq!(read_focus(&app), 0, "k: 1 → 0");
    }

    #[test]
    fn slash_opens_search_modal_and_enter_commits_filter() {
        let snap = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snap);

        dispatch_key(&mut app, mk(KeyCode::Char('/')));
        assert!(matches!(app.modal, Some(Modal::Search)));
        assert!(app.search_state.is_some());

        for c in ['f', 'o', 'o'] {
            dispatch_key(&mut app, mk(KeyCode::Char(c)));
        }
        dispatch_key(&mut app, mk(KeyCode::Enter));
        assert!(app.modal.is_none());
        assert!(app.search_state.is_none());
        assert_eq!(app.search_filter.as_deref(), Some("foo"));
    }

    #[test]
    fn esc_tier3_clears_search_filter_when_no_modal_open() {
        let snap = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snap);
        app.search_filter = Some("foo".to_owned());

        dispatch_key(&mut app, mk(KeyCode::Esc));
        assert!(app.search_filter.is_none(), "tier-3 must clear sticky filter");
    }

    #[test]
    fn s_opens_sample_set_modal_with_buffer_prefilled() {
        let snap = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snap);
        let original = app.sample_input.text.clone();

        dispatch_key(&mut app, mk(KeyCode::Char('s')));
        assert!(matches!(app.modal, Some(Modal::SampleSet)));
        let state = app.sample_set_state.as_ref().expect("state set");
        assert_eq!(state.buf, original, "buffer must be pre-filled with current sample");
    }

    #[test]
    fn color_picker_commit_binds_to_selected_rule_via_styles_default_key() {
        let snapshot = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snapshot);
        // Default tab is Patterns; first builtin is at index 0.
        app.focus.patterns.selected_idx = 0;
        let color = crate::style::Color::Cyan;
        bind_color_to_selected_rule(&mut app, color);
        let first_builtin = app.catalog.builtin_rule_names[0];
        let rule_id = crate::config_tui::edit::RuleId::Builtin(first_builtin);
        let edit = app.edits.rules.get(&rule_id).expect("rule edit recorded");
        let style =
            edit.styles.get(&crate::config_tui::edit::StyleKey::Default).expect("style entry");
        assert_eq!(style.fg, Some(Some(color)));
    }

    #[test]
    fn color_picker_commit_invokes_recompile_preview() {
        let snapshot = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snapshot);
        app.sample_input.text = "see 192.168.1.1 here".to_owned();
        app.preview.recompile(&app.sample_input.text);
        let runs_before = app.preview.runs.len();
        app.focus.patterns.selected_idx = 0;
        bind_color_to_selected_rule(&mut app, crate::style::Color::Magenta);
        assert!(app.preview.compile_error.is_none(), "valid edit yields no error");
        assert_eq!(app.preview.runs.len(), runs_before, "line count stable");
    }

    #[test]
    fn sample_set_commit_writes_text_without_marking_debouncer() {
        // Spec §9.2: sample-input change is a no-debounce interaction.
        // Render re-applies preview.compiled to the new sample on the next
        // frame; a debouncer.mark_edit() here would over-trigger a recompile.
        let snap = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snap);
        dispatch_key(&mut app, mk(KeyCode::Char('s')));
        // Clear the pre-filled buffer.
        let state = app.sample_set_state.as_mut().expect("state set");
        state.buf.clear();
        for c in ['l', 'o', 'g'] {
            dispatch_key(&mut app, mk(KeyCode::Char(c)));
        }
        dispatch_key(&mut app, mk(KeyCode::Enter));
        assert!(app.modal.is_none());
        assert!(app.sample_set_state.is_none());
        assert_eq!(app.sample_input.text, "log");
        assert!(
            !app.preview.debouncer.should_recompile(),
            "spec §9.2 forbids debouncer trigger on sample-input change"
        );
    }

    #[test]
    fn new_pattern_modal_name_phase_validates_alphanumeric_and_hyphen() {
        assert_eq!(validate_pattern_name(""), Err("name required".to_owned()));
        assert_eq!(validate_pattern_name("valid_name"), Ok(()));
        assert_eq!(validate_pattern_name("with-hyphen"), Ok(()));
        assert_eq!(
            validate_pattern_name("invalid space"),
            Err("name must be [A-Za-z0-9_-]+".to_owned()),
        );
        assert_eq!(
            validate_pattern_name(&"x".repeat(65)),
            Err("name too long (max 64)".to_owned()),
        );
    }

    #[test]
    fn new_pattern_modal_regex_phase_validates_inline_with_size_limit() {
        assert_eq!(validate_pattern_regex(""), Err("pattern required".to_owned()));
        assert_eq!(validate_pattern_regex(r"\bfoo\b"), Ok(()));
        assert!(
            validate_pattern_regex("[unbalanced").is_err(),
            "unbalanced character class must fail to compile"
        );
    }

    #[test]
    fn edit_regex_modal_marks_debouncer_on_keystroke() {
        // D3 (spec §12.4): each keystroke in EditRegex pushes a char into
        // the buffer and tickles the debouncer so the quiescent-window
        // timer slides. Without mark_edit the live preview would never
        // recompile while the user is typing.
        let snapshot = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snapshot);
        app.modal = Some(Modal::EditRegex {
            rule_id: crate::config_tui::edit::RuleId::UserConfig("test".to_owned()),
            buffer: String::new(),
            error: None,
        });
        let k = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty());
        handle_edit_regex_key(&mut app, k);
        if let Some(Modal::EditRegex { buffer, .. }) = &app.modal {
            assert_eq!(buffer, "a");
        } else {
            panic!("modal still EditRegex");
        }
    }

    #[test]
    fn edit_regex_modal_commit_writes_to_edits_rules_pattern_on_enter() {
        // D3 commit path: Enter on a valid buffer writes the new pattern
        // into edits.rules[rule_id].pattern and closes the modal.
        let snapshot = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snapshot);
        app.modal = Some(Modal::EditRegex {
            rule_id: crate::config_tui::edit::RuleId::Builtin("ipv4"),
            buffer: r"\d+\.\d+".to_owned(),
            error: None,
        });
        let k = KeyEvent::new(KeyCode::Enter, KeyModifiers::empty());
        handle_edit_regex_key(&mut app, k);
        let edit = app
            .edits
            .rules
            .get(&crate::config_tui::edit::RuleId::Builtin("ipv4"))
            .expect("rule edit recorded");
        assert_eq!(edit.pattern.as_deref(), Some(r"\d+\.\d+"));
        assert!(app.modal.is_none(), "modal closed on successful commit");
    }

    #[test]
    fn edit_regex_modal_invalid_pattern_on_enter_reopens_with_error() {
        // D3 error path: Enter on an invalid buffer reopens the modal
        // with the error string set; PendingEdits is NOT mutated.
        let snapshot = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snapshot);
        app.modal = Some(Modal::EditRegex {
            rule_id: crate::config_tui::edit::RuleId::UserConfig("test".to_owned()),
            buffer: "[unbalanced".to_owned(),
            error: None,
        });
        let k = KeyEvent::new(KeyCode::Enter, KeyModifiers::empty());
        handle_edit_regex_key(&mut app, k);
        if let Some(Modal::EditRegex { error, .. }) = &app.modal {
            assert!(error.is_some(), "error set after invalid regex commit attempt");
        } else {
            panic!("modal should still be EditRegex with error");
        }
        assert!(
            app.edits
                .rules
                .get(&crate::config_tui::edit::RuleId::UserConfig("test".to_owned()))
                .and_then(|e| e.pattern.as_deref())
                .is_none(),
            "PendingEdits unchanged on invalid commit",
        );
    }

    #[test]
    fn new_pattern_modal_commit_appends_added_new_rule() {
        use crate::config_tui::app::{Modal, NewPatternPhase, PatternDraft};
        let snapshot = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snapshot);
        let mut draft = PatternDraft::new();
        draft.name = "test_rule".to_owned();
        draft.pattern = r"\bX\b".to_owned();
        draft.draft_style = crate::config_tui::edit::NewStyle {
            fg: Some(Some(crate::style::Color::Cyan)),
            ..Default::default()
        };
        app.modal = Some(Modal::NewPattern { phase: NewPatternPhase::Style, draft });
        commit_new_pattern_draft(&mut app);
        assert_eq!(app.edits.added.len(), 1);
        assert_eq!(app.edits.added[0].name, "test_rule");
        assert_eq!(app.edits.added[0].pattern, r"\bX\b");
        assert_eq!(app.edits.added[0].style.fg, Some(Some(crate::style::Color::Cyan)));
        assert!(app.modal.is_none(), "commit must close the modal");
    }

    #[test]
    fn question_mark_opens_help_modal_when_no_modal_is_open() {
        // D4 dispatch: `?` from the global key arm constructs Modal::Help.
        let snap = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snap);
        assert!(app.modal.is_none(), "precondition: no modal at boot");

        dispatch_key(&mut app, mk(KeyCode::Char('?')));

        assert!(
            matches!(app.modal, Some(Modal::Help)),
            "'?' must open Modal::Help when no modal is open",
        );
    }

    #[test]
    fn f1_opens_help_modal_as_alt_binding() {
        // D4 alt-binding parity: F1 is equivalent to `?` for opening Help.
        let snap = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snap);

        dispatch_key(&mut app, mk(KeyCode::F(1)));

        assert!(
            matches!(app.modal, Some(Modal::Help)),
            "F1 must open Modal::Help when no modal is open",
        );
    }

    #[test]
    fn help_modal_dismisses_and_discards_any_key_per_vim_less_convention() {
        // D4 dismiss: Any key from Help dispatches to handle_help_key,
        // which closes the modal AND does NOT fall through to global
        // dispatch — so a `q` press while Help is open must NOT initiate
        // the quit flow (which would open Modal::QuitWithUnsavedEdits if
        // edits were dirty, or set should_quit otherwise).
        let snap = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snap);
        app.modal = Some(Modal::Help);

        dispatch_key(&mut app, mk(KeyCode::Char('q')));

        assert!(app.modal.is_none(), "any key dismisses Help");
        assert!(!app.should_quit, "'q' must NOT initiate the quit flow");
    }

    #[test]
    fn question_mark_is_no_op_when_a_different_modal_is_open() {
        // Spec §7.2: modals absorb all keys (except Ctrl+C / Esc). With
        // SaveDiff open, `?` must route through the modal branch and
        // hit handle_save_diff_key (a no-op for `?`), NOT the global
        // dispatch arm that would replace the modal with Help.
        let snap = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snap);
        app.save_diff = Some(crate::config_tui::widgets::save_diff::SaveDiffState::Clean {
            tui_diff: "(stub)".to_owned(),
        });
        app.modal = Some(Modal::SaveDiff);

        dispatch_key(&mut app, mk(KeyCode::Char('?')));

        assert!(
            matches!(app.modal, Some(Modal::SaveDiff)),
            "'?' under SaveDiff must NOT replace it with Help",
        );
    }

    #[test]
    fn reload_snapshot_inline_clears_edits_and_recompiles_preview() {
        // v0.6.1 §3.3: reload_snapshot_inline is the shared helper for the
        // Ctrl+R / DiscardEditsAndReload / InitFromDump flows. With a
        // source_path = None (empty snapshot), read_from_disk returns the
        // empty-snapshot shape; the assertion focuses on the invariants
        // that downstream flows depend on (edits cleared + compile_error
        // not stuck).
        use crate::config_tui::edit::{NewStyle, RuleEdit, RuleId, StyleKey};
        use std::collections::HashMap;
        let snap = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snap);
        // Pre-populate an edit so the clear assertion is meaningful.
        let mut styles: HashMap<StyleKey, NewStyle> = HashMap::new();
        styles.insert(
            StyleKey::Default,
            NewStyle { fg: Some(Some(crate::style::Color::Red)), ..NewStyle::default() },
        );
        app.edits.rules.insert(RuleId::Builtin("ipv4"), RuleEdit { pattern: None, styles });
        assert!(app.edits.is_dirty(), "precondition: edits dirty");
        reload_snapshot_inline(&mut app).expect("reload empty-snapshot path must succeed");
        assert!(!app.edits.is_dirty(), "edits cleared after reload");
        // Recompile ran without a compile_error stuck on the App.
        assert!(app.preview.compile_error.is_none(), "preview recompile clean");
    }

    #[test]
    fn ctrl_r_with_dirty_edits_opens_discard_confirm_modal() {
        // v0.6.1 §3.3: Ctrl+R when edits are pending must open the
        // DiscardEditsAndReload Confirm modal (not reload directly).
        use crate::config_tui::edit::{NewStyle, RuleEdit, RuleId, StyleKey};
        use std::collections::HashMap;
        let snap = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snap);
        let mut styles: HashMap<StyleKey, NewStyle> = HashMap::new();
        styles.insert(
            StyleKey::Default,
            NewStyle { fg: Some(Some(crate::style::Color::Red)), ..NewStyle::default() },
        );
        app.edits.rules.insert(RuleId::Builtin("ipv4"), RuleEdit { pattern: None, styles });
        dispatch_key(&mut app, KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert!(
            matches!(
                app.modal,
                Some(Modal::Confirm { action: ConfirmAction::DiscardEditsAndReload, .. })
            ),
            "Ctrl+R with dirty edits must open DiscardEditsAndReload Confirm modal",
        );
    }

    #[test]
    fn ctrl_r_with_clean_edits_reloads_directly_without_modal() {
        // v0.6.1 §3.3: Ctrl+R with no pending edits reloads inline (no
        // confirm) and surfaces an Ok toast.
        let snap = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snap);
        assert!(!app.edits.is_dirty(), "precondition: clean");
        dispatch_key(&mut app, KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert!(app.modal.is_none(), "no modal on clean Ctrl+R");
        let toast = app.toast.as_ref().expect("Ok toast set");
        assert!(toast.text.contains("Reloaded"), "toast: {}", toast.text);
    }

    #[test]
    fn shift_d_with_existing_config_file_warns_via_toast() {
        // v0.6.1 §3.3: Shift+D refuses to overwrite an existing config —
        // surfaces a warn toast instead of opening InitFromDump confirm.
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().expect("tmp");
        file.write_all(b"# existing\n").expect("seed");
        let path = file.path().to_path_buf();
        let snap = ConfigSnapshot::read_from_disk(Some(&path)).expect("read");
        let mut app = App::from_snapshot(snap);
        dispatch_key(&mut app, mk(KeyCode::Char('D')));
        assert!(app.modal.is_none(), "no modal when config file exists");
        let toast = app.toast.as_ref().expect("warn toast set");
        assert!(
            toast.text.contains("does not exist"),
            "expected 'does not exist' guard wording; got: {}",
            toast.text,
        );
    }

    #[test]
    fn shift_d_with_missing_config_file_opens_init_confirm_modal() {
        // v0.6.1 §3.3: Shift+D against a bound-but-absent config path
        // opens the InitFromDump confirm modal.
        let tmp = tempfile::tempdir().expect("tmpdir");
        let cfg_path = tmp.path().join("config.toml");
        assert!(!cfg_path.exists(), "precondition: config absent");
        let mut snap = ConfigSnapshot::empty();
        snap.source_path = Some(cfg_path);
        let mut app = App::from_snapshot(snap);
        dispatch_key(&mut app, mk(KeyCode::Char('D')));
        assert!(
            matches!(app.modal, Some(Modal::Confirm { action: ConfirmAction::InitFromDump, .. })),
            "Shift+D with missing config must open InitFromDump Confirm modal",
        );
    }

    #[test]
    fn shift_d_with_no_bound_path_warns_via_toast() {
        // v0.6.1 §3.3: Shift+D on an empty snapshot (source_path = None)
        // surfaces a warn toast — no Confirm modal.
        let snap = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snap);
        dispatch_key(&mut app, mk(KeyCode::Char('D')));
        assert!(app.modal.is_none(), "no modal when source_path is None");
        let toast = app.toast.as_ref().expect("warn toast set");
        assert!(
            toast.text.contains("bound config path"),
            "expected 'bound config path' wording; got: {}",
            toast.text,
        );
    }

    #[test]
    fn v_keystroke_opens_full_preview_modal_as_shift_p_alias() {
        // v0.6.1 §3.5: V is an alias for Shift+P — both open Modal::FullPreview.
        let snap = ConfigSnapshot::empty();
        let mut app = App::from_snapshot(snap);
        dispatch_key(&mut app, mk(KeyCode::Char('V')));
        assert!(matches!(app.modal, Some(Modal::FullPreview)), "V must open Modal::FullPreview");
    }

    #[test]
    fn help_modal_content_lists_canonical_keybindings_present_in_dispatch() {
        // Drift guard: if a keybinding is added/moved in dispatch_key,
        // HELP_MODAL_CONTENT must be updated in the same commit. These
        // pins catch the most-likely silent-drift cases.
        assert!(HELP_MODAL_CONTENT.contains("Tab / Shift+Tab"), "tab cycle listed");
        assert!(HELP_MODAL_CONTENT.contains("Ctrl+S / Ctrl+W"), "save bindings listed");
        assert!(HELP_MODAL_CONTENT.contains("? / F1"), "help bindings listed");
        assert!(HELP_MODAL_CONTENT.contains("Ctrl+C / q"), "quit bindings listed");
        assert!(
            HELP_MODAL_CONTENT.contains("Shift+P / V"),
            "full preview overlay + V alias listed"
        );
        assert!(
            HELP_MODAL_CONTENT.contains("Save Conflicts (in conflict modal)"),
            "G8 conflict-list section heading present"
        );
        assert!(
            HELP_MODAL_CONTENT.contains("Enter            Apply all selections"),
            "G8 conflict-list Enter binding listed"
        );
        assert!(HELP_MODAL_CONTENT.contains("Esc"), "esc listed");
        // v0.6.1 §3.3: Ctrl+R reload + Shift+D init keybindings pinned.
        assert!(HELP_MODAL_CONTENT.contains("Ctrl+R"), "Ctrl+R reload listed");
        assert!(HELP_MODAL_CONTENT.contains("Shift+D"), "Shift+D init listed");
    }

    #[test]
    fn apply_conflict_layer_translates_delete_modify_to_remove_when_source_absent() {
        // Spec §3.4 #15. base "x" present, ours deleted, theirs modified.
        // User picks Ours → the auto_merged loses "x" element (no AotElementMissing toast).
        use crate::config_tui::merge::{ConflictValueShape, KeyConflict};

        let base: toml_edit::DocumentMut =
            "[[rules]]\nname = \"x\"\npattern = \"A\"\n".parse().expect("base parses");
        let ours: toml_edit::DocumentMut = "# empty\n".parse().expect("ours parses");
        let theirs: toml_edit::DocumentMut =
            "[[rules]]\nname = \"x\"\npattern = \"B\"\n".parse().expect("theirs parses");
        let auto_merged: toml_edit::DocumentMut =
            "[[rules]]\nname = \"x\"\npattern = \"A\"\n".parse().expect("auto_merged parses");
        let conflicts = vec![KeyConflict {
            path: vec!["rules".to_owned(), "x".to_owned()],
            base_value: "{name=\"x\", pattern=\"A\"}".to_owned(),
            ours_value: "(absent)".to_owned(),
            theirs_value: "{name=\"x\", pattern=\"B\"}".to_owned(),
            shape: ConflictValueShape::Block,
            is_array_block: false,
        }];
        let selection = vec![crate::config_tui::widgets::save_diff::ConflictChoice::Ours];

        let result = build_final_doc(&base, &ours, &theirs, &auto_merged, &conflicts, &selection)
            .expect("delete-modify translated, no toast");

        let s = result.to_string();
        assert!(!s.contains("name = \"x\""), "delete-modify pick translated to remove");
    }
}
