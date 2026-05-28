//! v0.6.1 Group B — integration coverage for `Ctrl+R` / `Shift+D` /
//! `DiscardEditsAndReload` / `InitFromDump` real implementations
//! (spec §3.3 of the v0.6.1 design).
//!
//! These tests drive the production `dispatch_key` entry-point through
//! the `tayf::__test_api` helpers and assert post-conditions on modal
//! state, toast surfacing, and on-disk effects.

mod common;

use tayf::__test_api::{
    boot_app_from_disk_path, boot_app_with_bound_empty_snapshot, builtin_rule_names, current_toast,
    edits_are_dirty, has_modal_open, is_discard_reload_confirm_modal_open,
    is_init_from_dump_confirm_modal_open, send_key, stage_builtin_fg_edit, KeyCode, KeyEvent,
    KeyModifiers,
};

/// Single bare keypress without modifiers.
fn k(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

#[test]
fn ctrl_r_with_pending_edits_opens_discard_confirm_modal() {
    // v0.6.1 §3.3: Ctrl+R when edits.is_dirty() opens the
    // DiscardEditsAndReload confirm modal (does NOT reload directly).
    let tmp = tempfile::tempdir().expect("tempdir");
    let cfg_path = tmp.path().join("config.toml");
    let mut app = boot_app_with_bound_empty_snapshot(cfg_path);
    stage_builtin_fg_edit(&mut app, "ipv4");
    assert!(edits_are_dirty(&app), "precondition: edits.is_dirty()");

    send_key(&mut app, KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));

    assert!(
        is_discard_reload_confirm_modal_open(&app),
        "Ctrl+R with dirty edits must open DiscardEditsAndReload Confirm modal",
    );
}

#[test]
fn ctrl_r_with_clean_edits_reloads_directly_without_modal() {
    // v0.6.1 §3.3: Ctrl+R when edits are clean reloads inline (no
    // confirm modal). Surfaces an Ok toast on disk-read success, a
    // warn toast on disk-read failure. We seed an on-disk config so
    // the happy path is exercised; the warn-toast branch is covered
    // by the shift_d_with_no_bound_path / events.rs lib tests.
    let tmp = tempfile::tempdir().expect("tempdir");
    let cfg_path = tmp.path().join("config.toml");
    std::fs::write(&cfg_path, "[general]\ntheme = \"dark\"\n").expect("seed");
    let mut app = boot_app_from_disk_path(&cfg_path).expect("read snapshot");
    assert!(!edits_are_dirty(&app), "precondition: clean");

    send_key(&mut app, KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));

    assert!(!has_modal_open(&app), "no modal on clean Ctrl+R");
    let (text, kind) = current_toast(&app).expect("toast surfaced");
    assert_eq!(kind, "ok", "Ok toast kind on successful reload");
    assert!(text.contains("Reloaded"), "toast: {text}");
}

#[test]
fn shift_d_creates_default_config_when_path_missing_and_reloads() {
    // v0.6.1 §3.3 (full happy path): Shift+D opens the InitFromDump
    // confirm modal; pressing `y` writes the built-in default config
    // body via write_atomic_to and reloads the snapshot from disk.
    let tmp = tempfile::tempdir().expect("tempdir");
    let cfg_path = tmp.path().join("config.toml");
    assert!(!cfg_path.exists(), "precondition: config absent");
    let mut app = boot_app_with_bound_empty_snapshot(cfg_path.clone());

    // Shift+D opens the InitFromDump confirm modal.
    send_key(&mut app, k(KeyCode::Char('D')));
    assert!(
        is_init_from_dump_confirm_modal_open(&app),
        "Shift+D with missing config opens InitFromDump Confirm modal",
    );

    // Confirm with `y`: writes the file, reloads, surfaces ok toast.
    send_key(&mut app, k(KeyCode::Char('y')));
    assert!(!has_modal_open(&app), "modal dismissed after y");
    assert!(cfg_path.exists(), "config file written by InitFromDump");

    // Every BUILTIN_NAMES entry is present in the written body.
    let body = std::fs::read_to_string(&cfg_path).expect("read created config");
    for name in builtin_rule_names() {
        let needle = format!("name = \"{name}\"");
        assert!(body.contains(&needle), "builtin '{name}' present in default_config_toml body",);
    }

    // Ok toast text mentions "Initialized" + the path's filename.
    let (text, kind) = current_toast(&app).expect("ok toast");
    assert_eq!(kind, "ok");
    assert!(text.contains("Initialized"), "toast: {text}");
}

#[test]
fn shift_d_warns_when_config_file_exists() {
    // v0.6.1 §3.3: Shift+D against an EXISTING config refuses to
    // overwrite — surfaces a warn toast, opens no modal.
    let tmp = tempfile::tempdir().expect("tempdir");
    let cfg_path = tmp.path().join("config.toml");
    std::fs::write(&cfg_path, "# existing\n").expect("seed existing");
    let mut app = boot_app_from_disk_path(&cfg_path).expect("read snapshot");

    send_key(&mut app, k(KeyCode::Char('D')));

    assert!(!has_modal_open(&app), "no modal when config exists");
    let (text, kind) = current_toast(&app).expect("warn toast");
    assert_eq!(kind, "warn", "warn toast kind");
    assert!(
        text.contains("does not exist"),
        "expected 'does not exist' guard wording; got: {text}",
    );
}
