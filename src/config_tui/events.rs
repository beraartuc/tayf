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
        // C4-owned modals: events.rs dispatches the global keys
        // (Esc, Ctrl+C handled above); per-modal key sets land in
        // widgets/color_picker.rs / widgets/save_diff.rs / etc.
        // For C2a we no-op modal-absorbed keys.
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
            // Help overlay placeholder — C4 wires real help modal.
            app.toast =
                Some(crate::config_tui::app::Toast::ok("help overlay (C4 wires real impl)"));
        }
        (KeyCode::F(1), _) => {
            app.toast =
                Some(crate::config_tui::app::Toast::ok("help overlay (C4 wires real impl)"));
        }
        (KeyCode::Char('P'), m) if m == KeyModifiers::SHIFT => {
            // Shift+P — full-preview overlay (C4 wires real impl).
            app.modal = Some(Modal::FullPreview);
        }
        (KeyCode::Char('p'), m) if m.is_empty() => {
            app.mini_preview_visible = !app.mini_preview_visible;
        }
        // C3 / C4 wire the rest (Ctrl+S save, s sample, / search, Shift+D init).
        _ => {
            crate::config_tui::tabs::dispatch_key(app, k);
        }
    }
}

/// Esc precedence (§12.1):
/// 1. close active edit field (C4 owns this — placeholder),
/// 2. close modal,
/// 3. clear active search filter (C3 owns this),
/// 4. no-op.
fn handle_esc(app: &mut App) {
    if app.modal.is_some() {
        app.modal = None;
    }
    // C3/C4 hook search-clear + edit-field-close in front of modal close.
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
            // C2a stub: close modal + toast "save not wired yet".
            // C4 replaces this with SaveDiff modal open inline.
            app.modal = None;
            app.toast = Some(crate::config_tui::app::Toast::warn(
                "save flow lands in C4; discarding instead",
            ));
            app.edits.clear();
            app.should_quit = true;
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

/// C2a stub — C3 / C4 wire real action execution.
fn apply_confirm(app: &mut App, action: &ConfirmAction) {
    app.toast = Some(crate::config_tui::app::Toast::ok(format!(
        "confirm action: {action:?} (impl lands in C3 / C4)"
    )));
}

/// Debounce tick — C4 wires real recompile.
pub(crate) fn check_debounce(_app: &mut App) {
    // C4: if app.preview.debounce_pending && elapsed > 200ms { recompile }
}

/// Toast expiration tick.
pub(crate) fn check_toast(app: &mut App) {
    if app.toast.as_ref().is_some_and(crate::config_tui::app::Toast::expired) {
        app.toast = None;
    }
}
