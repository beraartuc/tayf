//! v0.6.1 Group D — TUI polish coverage: V alias parity, search filter
//! application, save-diff modal scroll, help-modal canonical content
//! (spec §3.5 + §3.6 + §3.7).
//!
//! Drives the production `dispatch_key` entry-point through
//! `tayf::__test_api` helpers and asserts post-conditions on modal
//! state, scroll offset, and the canonical help-modal string.

mod common;

use tayf::__test_api::{
    boot_app_with_bound_empty_snapshot, builtin_rule_names, clear_modal, filter_names_lowercase,
    help_modal_content, is_full_preview_modal_open, open_save_diff_with_clean_body,
    save_diff_scroll, send_key, KeyCode, KeyEvent, KeyModifiers,
};

/// Single bare keypress without modifiers.
fn k(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

#[test]
fn v_keystroke_enters_full_preview_same_as_shift_p() {
    // v0.6.1 §3.5: V is a no-modifier alias for Shift+P. Both
    // keystrokes open Modal::FullPreview from the no-modal state.
    let tmp = tempfile::tempdir().expect("tempdir");
    let cfg_path = tmp.path().join("config.toml");
    let mut app = boot_app_with_bound_empty_snapshot(cfg_path);

    send_key(&mut app, k(KeyCode::Char('V')));
    assert!(is_full_preview_modal_open(&app), "V keystroke must open Modal::FullPreview");

    // Reset + Shift+P should hit the same state.
    clear_modal(&mut app);
    send_key(&mut app, KeyEvent::new(KeyCode::Char('P'), KeyModifiers::SHIFT));
    assert!(is_full_preview_modal_open(&app), "Shift+P keystroke must open Modal::FullPreview");
}

#[test]
fn search_filter_in_patterns_tab_renders_only_matching_rules_and_hides_nonmatching() {
    // v0.6.1 §3.6: filter_names_lowercase against BUILTIN_NAMES with
    // "ip" needle must keep IPv4 / IPv6 and exclude non-matching
    // entries. Negative regression guard per memory
    // feedback_test_assertion_specificity: assert an explicit
    // non-matching entry is absent (a bare positive-only assertion
    // would pass even if the filter were a no-op).
    let names = builtin_rule_names();
    let filtered = filter_names_lowercase(names, "ip");
    assert!(filtered.contains(&"ipv4"), "ipv4 present (substring match)");
    assert!(filtered.contains(&"ipv6"), "ipv6 present (substring match)");
    // Pick an entry guaranteed not to contain "ip": "url" (canonical
    // shipped builtin). If shipped catalog changes, swap to another
    // non-matching name.
    assert!(
        names.contains(&"url"),
        "sanity: builtin catalog ships a 'url' rule (test fixture invariant)"
    );
    assert!(!filtered.contains(&"url"), "'url' must be filtered out by the 'ip' substring needle");

    // Empty filter returns every name in canonical order.
    let all = filter_names_lowercase(names, "");
    assert_eq!(all.len(), names.len(), "empty filter returns every entry");
    assert_eq!(all[0], names[0], "order preserved");
}

#[test]
fn save_diff_down_arrow_advances_scroll_by_one() {
    // v0.6.1 §3.7: Down increments save_diff_scroll by 1; Up
    // decrements by 1 (saturating at 0).
    let tmp = tempfile::tempdir().expect("tempdir");
    let cfg_path = tmp.path().join("config.toml");
    let mut app = boot_app_with_bound_empty_snapshot(cfg_path);
    open_save_diff_with_clean_body(&mut app, "line1\nline2\nline3\nline4\nline5\n");
    assert_eq!(save_diff_scroll(&app), 0);

    send_key(&mut app, k(KeyCode::Down));
    assert_eq!(save_diff_scroll(&app), 1);
    send_key(&mut app, k(KeyCode::Down));
    assert_eq!(save_diff_scroll(&app), 2);
    send_key(&mut app, k(KeyCode::Up));
    assert_eq!(save_diff_scroll(&app), 1);
    // Saturating subtract: Up at 0 stays at 0.
    send_key(&mut app, k(KeyCode::Up));
    send_key(&mut app, k(KeyCode::Up));
    assert_eq!(save_diff_scroll(&app), 0, "Up saturates at 0");
}

#[test]
fn save_diff_page_down_advances_by_page_step_and_home_end_jump() {
    // v0.6.1 §3.7: PageDown / PageUp move by PAGE_STEP (= 10);
    // Home jumps to 0, End jumps to u16::MAX (Paragraph::scroll
    // clamps internally to the body length).
    let tmp = tempfile::tempdir().expect("tempdir");
    let cfg_path = tmp.path().join("config.toml");
    let mut app = boot_app_with_bound_empty_snapshot(cfg_path);
    let body: String = (0..50).map(|i| format!("line{i}\n")).collect();
    open_save_diff_with_clean_body(&mut app, body);

    send_key(&mut app, k(KeyCode::PageDown));
    assert_eq!(save_diff_scroll(&app), 10, "PageDown step = 10");
    send_key(&mut app, k(KeyCode::PageUp));
    assert_eq!(save_diff_scroll(&app), 0, "PageUp returns to 0");
    send_key(&mut app, k(KeyCode::End));
    assert_eq!(save_diff_scroll(&app), u16::MAX, "End → u16::MAX");
    send_key(&mut app, k(KeyCode::Home));
    assert_eq!(save_diff_scroll(&app), 0, "Home → 0");
}

#[test]
fn help_modal_content_lists_new_v0_6_1_keystrokes() {
    // v0.6.1: the HELP_MODAL_CONTENT canonical string must document
    // every new keybinding introduced in this minor (memory
    // feedback_consume_prior_review — explicit pin against drift).
    let help = help_modal_content();
    assert!(help.contains("Shift+P / V"), "V alias documented next to Shift+P");
    assert!(help.contains("Ctrl+R"), "Ctrl+R reload documented");
    assert!(help.contains("Shift+D"), "Shift+D init documented");
    assert!(help.contains("d / Delete"), "Delete keycode documented next to d");
}
