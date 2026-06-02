//! Integration tests for the Themes-tab 'o' override-copy handler.
//!
//! Each test mutates the process-wide `XDG_CONFIG_HOME` environment
//! variable (the production `save::tayf_config_root` reads it), so a
//! binary-scoped mutex serializes them — other test binaries run as
//! separate processes and are unaffected.
//!
//! The Profiles-tab 'o' embedded-copy handler is retired (v0.12.0): the
//! embedded profile library is gone (the six domain rules are now built-ins),
//! so there is nothing to copy. Only the Themes-tab copy path remains here;
//! Profiles-tab management (create/delete) lands in the Profiles-tab rework.
//!
//! Spec §3.4 + §3.10. CLAUDE.md §3 symlink-traversal mandate covered by the
//! Themes-tab copy path's shared `save::check_safe_write_destination` guard.

#![allow(clippy::expect_used)] // integration-test convenience

use std::sync::Mutex;

use tayf::__test_api::{
    boot_app_with_sample, builtin_theme_idx, current_toast_message, embedded_theme_source,
    goto_themes_tab, send_char, set_selected_theme_idx,
};

/// Serializes `XDG_CONFIG_HOME` env mutation across this binary's tests.
/// `unwrap_or_else(|e| e.into_inner())` makes the gate poison-tolerant so
/// one failing test does not cascade-fail the rest with "poisoned mutex".
static SERIAL: Mutex<()> = Mutex::new(());

fn lock_serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn o_on_embedded_theme_writes_themes_directory_with_exact_source_bytes() {
    let _guard = lock_serial();
    let tmp = tempfile::tempdir().expect("tmpdir");
    std::env::set_var("XDG_CONFIG_HOME", tmp.path());

    let mut app = boot_app_with_sample("");
    goto_themes_tab(&mut app);
    let dark_idx = builtin_theme_idx(&app, "dark").expect("dark is a built-in theme");
    set_selected_theme_idx(&mut app, dark_idx);
    send_char(&mut app, 'o');

    let dest = tmp.path().join("tayf/themes/dark.toml");
    assert!(
        dest.exists(),
        "dark.toml copied to themes dir; toast was: {:?}",
        current_toast_message(&app)
    );
    let on_disk = std::fs::read_to_string(&dest).expect("read on-disk theme");
    let embedded = embedded_theme_source("dark").expect("dark embedded source");
    assert_eq!(on_disk, embedded);
}

#[test]
fn o_on_themes_tab_emits_path_explicit_already_on_disk_toast_when_dest_already_exists() {
    let _guard = lock_serial();
    let tmp = tempfile::tempdir().expect("tmpdir");
    std::env::set_var("XDG_CONFIG_HOME", tmp.path());
    let dest = tmp.path().join("tayf/themes/dark.toml");
    std::fs::create_dir_all(dest.parent().expect("parent")).expect("mkdir");
    std::fs::write(&dest, "user-tweaks").expect("write pre-existing");

    let mut app = boot_app_with_sample("");
    goto_themes_tab(&mut app);
    let dark_idx = builtin_theme_idx(&app, "dark").expect("dark idx");
    set_selected_theme_idx(&mut app, dark_idx);
    send_char(&mut app, 'o');

    assert_eq!(
        current_toast_message(&app).as_deref(),
        Some("Already on disk — edit ~/.config/tayf/themes/dark.toml"),
        "themes-tab skip-toast carries themes/ path (not profiles/)",
    );
    assert_eq!(std::fs::read_to_string(&dest).expect("read"), "user-tweaks");
}
