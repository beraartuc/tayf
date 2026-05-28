//! Integration test: TUI editor end-to-end — `n` keystroke runs the
//! 3-phase new-pattern modal (Name → Regex → Style) and commits a draft
//! into `PendingEdits::added` (spec §12.4 D2).

mod common;
use common::tui_harness;

use tayf::__test_api::{
    has_modal_open, is_new_pattern_modal_open, pending_added_count, send_key, KeyCode, KeyEvent,
    KeyModifiers,
};

/// Helper: a single bare keypress (no modifiers).
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

#[test]
fn n_keystroke_opens_new_pattern_modal_and_commits_added_rule() {
    // Boot with a sample line containing a number so the committed `\d+`
    // pattern would match if a downstream test were to inspect spans.
    let (mut app, _terminal) = tui_harness::boot_tui_with_sample("see 42 here");

    // Sanity: no modal at boot; no pending added rules.
    assert!(!has_modal_open(&app), "no modal at boot");
    assert_eq!(pending_added_count(&app), 0, "no pending added rules at boot");

    // `n` on Patterns tab opens the new-pattern modal in Name phase.
    send_key(&mut app, key(KeyCode::Char('n')));
    assert!(is_new_pattern_modal_open(&app), "Modal::NewPattern opened by 'n'");

    // Name phase: type "test_rule" then Enter advances to Regex phase.
    for c in "test_rule".chars() {
        send_key(&mut app, key(KeyCode::Char(c)));
    }
    send_key(&mut app, key(KeyCode::Enter));
    // Still in NewPattern modal (now Regex phase).
    assert!(is_new_pattern_modal_open(&app), "still in NewPattern modal after Name → Regex");

    // Regex phase: type `\d+` then Enter advances to Style phase.
    send_key(&mut app, key(KeyCode::Char('\\')));
    send_key(&mut app, key(KeyCode::Char('d')));
    send_key(&mut app, key(KeyCode::Char('+')));
    send_key(&mut app, key(KeyCode::Enter));
    assert!(is_new_pattern_modal_open(&app), "still in NewPattern modal after Regex → Style");

    // Style phase: ColorPickerState defaults to Ansi16 / idx 0
    // (Color::Black). `selected_color()` returns Some(_) on the default
    // state, so Enter here commits the draft via the outer Enter
    // branch in `handle_new_pattern_key`. See
    // `src/config_tui/widgets/color_picker.rs::selected_color` +
    // `src/config_tui/events.rs::handle_new_pattern_key`.
    send_key(&mut app, key(KeyCode::Enter));

    // Modal must be dismissed (commit path runs `app.modal.take()`); the
    // draft must appear in `edits.added`.
    assert!(!has_modal_open(&app), "modal dismissed after commit");
    assert_eq!(pending_added_count(&app), 1, "draft committed to PendingEdits::added");
}
