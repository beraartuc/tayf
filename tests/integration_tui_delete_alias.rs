//! v0.6.1 Group C — `Delete` keycode parallel to `d` in Patterns tab.
//!
//! Asserts that `KeyCode::Delete` opens the same `DeleteRule` confirm
//! modal as `KeyCode::Char('d')`, satisfying the file-manager / editor
//! UX convention where Delete = remove selected entry. Spec §3.4.

mod common;

use tayf::__test_api::{
    boot_app_with_bound_empty_snapshot, is_delete_rule_confirm_modal_open, send_key, KeyCode,
    KeyEvent, KeyModifiers,
};

#[test]
fn delete_keycode_opens_same_confirm_modal_as_d() {
    // App starts on Tab::Patterns with selected_idx = 0 (defaults).
    // builtin_rule_names is non-empty, so the first entry exists.
    let tmp = tempfile::tempdir().expect("tempdir");
    let cfg_path = tmp.path().join("config.toml");
    let mut app = boot_app_with_bound_empty_snapshot(cfg_path);

    send_key(&mut app, KeyEvent::new(KeyCode::Delete, KeyModifiers::empty()));

    assert!(
        is_delete_rule_confirm_modal_open(&app),
        "KeyCode::Delete must open a DeleteRule Confirm modal (same as 'd')",
    );
}
