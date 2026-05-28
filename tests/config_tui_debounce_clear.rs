//! Integration tests for G1: mark_edit_clear debouncer cleanup on
//! Modal::EditRegex and Modal::NewPattern Esc-cancel.
//!
//! Spec §3.7 — pins that Esc on regex/new-pattern modals does NOT
//! trigger a phantom recompile after the debounce interval elapses.

mod common;

use tayf::__test_api::{
    boot_app_with_sample, debouncer_pending, is_edit_regex_modal_open, is_new_pattern_modal_open,
    open_edit_regex_modal_first_builtin, open_new_pattern_modal, send_key, tick_debounce, KeyCode,
    KeyEvent, KeyModifiers,
};

/// Single bare keypress without modifiers.
fn k(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

#[test]
fn edit_regex_esc_clears_pending_debounce_mark() {
    // G1 spec §3.7: Esc on EditRegex modal must clear the debouncer mark
    // set by typing into the regex buffer, so no phantom recompile fires.
    let mut app = boot_app_with_sample("see 192.168.1.1 here");
    open_edit_regex_modal_first_builtin(&mut app);
    assert!(is_edit_regex_modal_open(&app), "modal should be open");

    // Type a char — dispatched to handle_edit_regex_key which calls mark_edit.
    send_key(&mut app, k(KeyCode::Char('a')));
    assert!(debouncer_pending(&app), "modal edit should mark debouncer");

    // Esc — dispatched to handle_esc Tier 2 EditRegex arm, calls mark_edit_clear.
    send_key(&mut app, k(KeyCode::Esc));
    assert!(!is_edit_regex_modal_open(&app), "modal must close after Esc");
    assert!(!debouncer_pending(&app), "Esc on EditRegex must clear debounce mark");
}

#[test]
fn new_pattern_esc_clears_pending_debounce_mark() {
    // G1 spec §3.7: Esc on NewPattern::Name modal must clear any existing
    // debouncer pending mark (T-I5 paragraph 2 — symmetric debouncer-leak
    // fix). NewPattern itself does not call mark_edit, but a prior EditRegex
    // session may have left the mark set. This test sets the mark explicitly
    // via EditRegex, cancels, then opens NewPattern and Escs to assert the
    // mark stays cleared.
    let mut app = boot_app_with_sample("see 192.168.1.1 here");

    // Manually mark the debouncer (as if a prior EditRegex typed a char).
    open_edit_regex_modal_first_builtin(&mut app);
    send_key(&mut app, k(KeyCode::Char('x')));
    assert!(debouncer_pending(&app), "precondition: debouncer marked");
    send_key(&mut app, k(KeyCode::Esc));
    assert!(!debouncer_pending(&app), "EditRegex Esc cleared the mark");

    // Open NewPattern. Mark the debouncer again to simulate a leaked mark.
    // We open EditRegex, type, then directly open NewPattern without Esc
    // (bypassing mark_edit_clear) to create the leaked state.
    open_edit_regex_modal_first_builtin(&mut app);
    send_key(&mut app, k(KeyCode::Char('y')));
    assert!(debouncer_pending(&app), "leaked mark set");
    // Close EditRegex without going through the Esc path (direct modal swap).
    open_new_pattern_modal(&mut app);
    assert!(is_new_pattern_modal_open(&app), "NewPattern modal open");
    assert!(debouncer_pending(&app), "precondition: mark still leaked before NewPattern Esc");

    // Esc on NewPattern::Name — Tier 1b handles, mark_edit_clear runs.
    send_key(&mut app, k(KeyCode::Esc));
    assert!(!is_new_pattern_modal_open(&app), "NewPattern modal must close on Name-phase Esc");
    assert!(!debouncer_pending(&app), "Esc on NewPattern::Name must clear debounce mark");
}

#[test]
fn post_edit_regex_esc_recompile_not_triggered_after_interval() {
    // G1 spec §3.7: After Esc clears the debounce mark, tick_debounce must
    // return false (no recompile) regardless of elapsed time. Since
    // mark_edit_clear sets pending=false and last_edit=None, should_recompile
    // is structurally blocked — this is stronger than a timing-based assertion.
    let mut app = boot_app_with_sample("see 192.168.1.1 here");
    open_edit_regex_modal_first_builtin(&mut app);
    send_key(&mut app, k(KeyCode::Char('a')));
    assert!(debouncer_pending(&app), "precondition: debouncer marked");

    send_key(&mut app, k(KeyCode::Esc));
    assert!(!debouncer_pending(&app), "Esc cleared the mark");

    // tick_debounce with pending=false and last_edit=None: should_recompile
    // short-circuits on the None check and returns false.
    let triggered = tick_debounce(&mut app);
    assert!(!triggered, "no phantom recompile after Esc-cleared debounce mark");
    assert!(!debouncer_pending(&app), "debouncer stays idle after tick with cleared mark");
}

#[test]
fn debouncer_still_fires_on_normal_edit_path_after_clear() {
    // G1 spec §3.7: mark_edit_clear does not permanently disable the
    // debouncer. A subsequent EditRegex modal edit must re-mark it.
    let mut app = boot_app_with_sample("see 192.168.1.1 here");

    // First session: type, Esc (clears mark).
    open_edit_regex_modal_first_builtin(&mut app);
    send_key(&mut app, k(KeyCode::Char('a')));
    send_key(&mut app, k(KeyCode::Esc));
    assert!(!debouncer_pending(&app), "mark cleared after first Esc");

    // Second session: type into a new EditRegex (mark_edit re-fires).
    open_edit_regex_modal_first_builtin(&mut app);
    send_key(&mut app, k(KeyCode::Char('b')));
    assert!(debouncer_pending(&app), "normal edit path must re-mark debouncer after a prior clear");
}
