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
        // Confirm modals (DeleteUserRule / ResetUserOverride / DiscardEditsAndReload / InitFromDump).
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
                            app.modal = None;
                            app.toast = Some(crate::config_tui::app::Toast::ok(
                                "color accepted (binding to selected rule lands in v0.6+)",
                            ));
                        }
                        crate::config_tui::widgets::color_picker::ColorPickerOutcome::Cancel => {
                            app.modal = None;
                        }
                        crate::config_tui::widgets::color_picker::ColorPickerOutcome::StayOpen => {}
                    }
                }
            }
            Modal::SaveDiff => handle_save_diff_key(app, k),
            Modal::Search => handle_search_key(app, k),
            Modal::SampleSet => handle_sample_set_key(app, k),
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
            app.toast = Some(crate::config_tui::app::Toast::warn(
                "help overlay lands in v0.5.5+ (Modal::Help wiring deferred)",
            ));
        }
        (KeyCode::F(1), _) => {
            app.toast = Some(crate::config_tui::app::Toast::warn(
                "help overlay lands in v0.5.5+ (Modal::Help wiring deferred)",
            ));
        }
        (KeyCode::Char('P'), m) if m == KeyModifiers::SHIFT => {
            if app.modal.is_none() {
                app.modal = Some(Modal::FullPreview);
            }
        }
        (KeyCode::Char('s'), m) if m == KeyModifiers::CONTROL => {
            if app.modal.is_none() {
                app.save_diff =
                    Some(crate::config_tui::widgets::save_diff::build_initial_state(app));
                app.modal = Some(Modal::SaveDiff);
            }
        }
        (KeyCode::Char('w'), m) if m == KeyModifiers::CONTROL => {
            // Ctrl+W alt-binding (🔵 #1 fold — XON/XOFF inferno).
            if app.modal.is_none() {
                app.save_diff =
                    Some(crate::config_tui::widgets::save_diff::build_initial_state(app));
                app.modal = Some(Modal::SaveDiff);
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
        // Shift+D first-run init (§9.6) lands in v0.5.5+.
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
                app.toast = Some(crate::config_tui::app::Toast::ok(
                    "Saved. Hot-reload will pick this up shortly.",
                ));
            }
            Err(e) => {
                app.modal = Some(Modal::Error(format!("Save failed: {e}")));
                app.save_diff = None;
            }
        },
        SaveDiffOutcome::DiscardAndReload(_disk_now) => {
            if let Ok(snap) = crate::config_tui::snapshot::ConfigSnapshot::read_from_disk(
                app.snapshot.source_path.as_deref(),
            ) {
                app.snapshot = snap;
            }
            app.edits.clear();
            app.modal = None;
            app.save_diff = None;
            app.toast =
                Some(crate::config_tui::app::Toast::ok("Reloaded from disk; TUI edits discarded."));
        }
        SaveDiffOutcome::CloseModal => {
            app.modal = None;
            app.save_diff = None;
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
    // Tier 2: close modal. Drop side-channels alongside so the
    // `side_channel.is_some() ↔ modal == Some(MatchingVariant)`
    // invariants that render + dispatch rely on stay intact.
    if app.modal.is_some() {
        match app.modal {
            Some(Modal::SaveDiff) => app.save_diff = None,
            Some(Modal::Search) => app.search_state = None,
            Some(Modal::SampleSet) => app.sample_set_state = None,
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
/// - s → save then quit (C4 wires `SaveDiff` inline; C2a stub closes modal + sets toast)
/// - d → discard and quit (immediate)
fn handle_quit_confirm_key(app: &mut App, k: KeyEvent) {
    match k.code {
        KeyCode::Char('n') | KeyCode::Enter => {
            app.modal = None;
        }
        KeyCode::Char('s') => {
            // Save-and-quit: open SaveDiff inline; commit will set
            // should_quit on success via a separate flag (deferred to v0.6+
            // because event-loop reentrancy from here is non-trivial). For
            // v0.5.4 we open SaveDiff and let user commit-then-quit manually.
            app.save_diff = Some(crate::config_tui::widgets::save_diff::build_initial_state(app));
            app.modal = Some(Modal::SaveDiff);
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
            ConfirmAction::DeleteUserRule(n) => Some(ConfirmAction::DeleteUserRule(n.clone())),
            ConfirmAction::ResetUserOverride(n) => {
                Some(ConfirmAction::ResetUserOverride(n.clone()))
            }
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
        ConfirmAction::DeleteUserRule(name) => {
            let removed = app
                .edits
                .rules
                .remove(&crate::config_tui::edit::RuleId::UserConfig(name.clone()))
                .is_some();
            let msg = if removed {
                format!("Removed staged override of '{name}' (built-in restored)")
            } else {
                format!("No staged override of '{name}' to remove; built-in remains active")
            };
            app.toast = Some(crate::config_tui::app::Toast::ok(msg));
        }
        ConfirmAction::ResetUserOverride(name) => {
            let removed = app
                .edits
                .rules
                .remove(&crate::config_tui::edit::RuleId::UserConfig(name.clone()))
                .is_some();
            let msg = if removed {
                format!("Reset '{name}' to built-in (override discarded)")
            } else {
                format!("No staged override of '{name}' to reset")
            };
            app.toast = Some(crate::config_tui::app::Toast::ok(msg));
        }
        ConfirmAction::DiscardEditsAndReload | ConfirmAction::InitFromDump => {
            app.toast = Some(crate::config_tui::app::Toast::warn(
                "this action lands in v0.5.5+ (init-from-dump and discard-reload deferred)",
            ));
        }
    }
}

/// Debounce tick — fires `recompile_preview` once per quiescent window.
pub(crate) fn check_debounce(app: &mut App) {
    if app.preview.debouncer.should_recompile() {
        recompile_preview(app);
    }
}

/// Recompile the live-preview rule set. v0.5.4 stub — true Compiled
/// rebuild lands in v0.6+ when the span-emitting preview pipeline is
/// ready. For now this just clears any stale compile-error indicator.
fn recompile_preview(app: &mut App) {
    app.preview.compile_error = None;
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
}
