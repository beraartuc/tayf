//! Integration tests for v0.6.2 G6 — Item 4: 'o' override-copy on the
//! Profiles / Themes tabs.
//!
//! Each test mutates the process-wide `XDG_CONFIG_HOME` environment
//! variable (the production `save::tayf_config_root` reads it), so a
//! binary-scoped mutex serializes them — other test binaries run as
//! separate processes and are unaffected.
//!
//! Spec §3.4 + §3.10. CLAUDE.md §3 symlink-traversal mandate covered by
//! the `*_refuses_*` tests.

#![allow(clippy::expect_used)] // integration-test convenience

use std::sync::Mutex;

use tayf::__test_api::{
    boot_app_with_sample, builtin_theme_idx, current_toast_message, embedded_profile_idx,
    embedded_profile_source, embedded_theme_source, goto_profiles_tab, goto_themes_tab, send_char,
    set_selected_profile_idx, set_selected_theme_idx,
};

/// Serializes `XDG_CONFIG_HOME` env mutation across this binary's tests.
/// `unwrap_or_else(|e| e.into_inner())` makes the gate poison-tolerant so
/// one failing test does not cascade-fail the rest with "poisoned mutex".
static SERIAL: Mutex<()> = Mutex::new(());

fn lock_serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn o_copies_embedded_profile_to_disk_with_exact_source_bytes() {
    let _guard = lock_serial();
    let tmp = tempfile::tempdir().expect("tmpdir");
    std::env::set_var("XDG_CONFIG_HOME", tmp.path());

    let mut app = boot_app_with_sample("");
    goto_profiles_tab(&mut app);
    let aws_idx = embedded_profile_idx(&app, "aws").expect("aws is an embedded profile");
    set_selected_profile_idx(&mut app, aws_idx);
    send_char(&mut app, 'o');

    let dest = tmp.path().join("tayf/profiles/aws.toml");
    assert!(dest.exists(), "aws.toml copied to disk; toast was: {:?}", current_toast_message(&app));

    let on_disk = std::fs::read_to_string(&dest).expect("read on-disk copy");
    let embedded = embedded_profile_source("aws").expect("aws embedded source");
    assert_eq!(on_disk, embedded, "on-disk bytes match the compile-time embedded source");

    assert_eq!(
        current_toast_message(&app).as_deref(),
        Some("Copied 'aws' to disk; now editable"),
        "success toast wording pinned",
    );
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
fn o_emits_already_on_disk_skip_toast_with_pinned_wording_when_dest_already_exists() {
    let _guard = lock_serial();
    let tmp = tempfile::tempdir().expect("tmpdir");
    std::env::set_var("XDG_CONFIG_HOME", tmp.path());
    // Pre-seed a file at the override-copy destination.
    let dest = tmp.path().join("tayf/profiles/aws.toml");
    std::fs::create_dir_all(dest.parent().expect("parent")).expect("mkdir");
    std::fs::write(&dest, "user-edits").expect("write pre-existing");

    let mut app = boot_app_with_sample("");
    goto_profiles_tab(&mut app);
    let aws_idx = embedded_profile_idx(&app, "aws").expect("aws idx");
    set_selected_profile_idx(&mut app, aws_idx);
    send_char(&mut app, 'o');

    assert_eq!(
        current_toast_message(&app).as_deref(),
        Some("Already on disk — edit ~/.config/tayf/profiles/aws.toml"),
        "exact toast wording for already-on-disk skip path",
    );
    // The pre-existing file MUST NOT be overwritten.
    assert_eq!(
        std::fs::read_to_string(&dest).expect("read post-skip"),
        "user-edits",
        "user edits preserved when 'o' is a no-op on an already-on-disk profile",
    );
}

#[test]
fn o_refuses_when_dest_is_symlink_pointing_outside_tayf_root() {
    let _guard = lock_serial();
    let tmp = tempfile::tempdir().expect("tmpdir");
    std::env::set_var("XDG_CONFIG_HOME", tmp.path());
    let profiles_dir = tmp.path().join("tayf/profiles");
    std::fs::create_dir_all(&profiles_dir).expect("mkdir");
    let outside = tmp.path().join("attacker-target.toml");
    std::fs::write(&outside, "attacker-controlled").expect("write outside");
    // Craft ~/.config/tayf/profiles/aws.toml as a symlink to outside.
    let dest = profiles_dir.join("aws.toml");
    std::os::unix::fs::symlink(&outside, &dest).expect("symlink");

    let mut app = boot_app_with_sample("");
    goto_profiles_tab(&mut app);
    let aws_idx = embedded_profile_idx(&app, "aws").expect("aws idx");
    set_selected_profile_idx(&mut app, aws_idx);
    send_char(&mut app, 'o');

    let toast = current_toast_message(&app).expect("toast present after refused 'o'");
    assert!(
        toast.starts_with("Override refused:") && toast.contains("symlink"),
        "symlink dest must be refused with 'Override refused:' + 'symlink' wording; got: {toast}"
    );
    // Outside file must not have been overwritten by the rejected write.
    assert_eq!(
        std::fs::read_to_string(&outside).expect("read outside"),
        "attacker-controlled",
        "attacker target untouched (CLAUDE.md §3 mandate)",
    );
}

#[test]
fn o_refuses_when_parent_dir_canonicalizes_outside_tayf_root() {
    let _guard = lock_serial();
    let tmp = tempfile::tempdir().expect("tmpdir");
    std::env::set_var("XDG_CONFIG_HOME", tmp.path());
    let tayf_root = tmp.path().join("tayf");
    std::fs::create_dir_all(&tayf_root).expect("mkdir tayf root");
    let outside_dir = tmp.path().join("outside-config");
    std::fs::create_dir_all(&outside_dir).expect("mkdir outside dir");
    // Replace ~/.config/tayf/profiles with a symlink to a directory
    // outside the canonical tayf root.
    let profiles_link = tayf_root.join("profiles");
    std::os::unix::fs::symlink(&outside_dir, &profiles_link).expect("symlink dir");

    let mut app = boot_app_with_sample("");
    goto_profiles_tab(&mut app);
    let aws_idx = embedded_profile_idx(&app, "aws").expect("aws idx");
    set_selected_profile_idx(&mut app, aws_idx);
    send_char(&mut app, 'o');

    let toast = current_toast_message(&app).expect("toast present after refused 'o'");
    assert!(
        toast.starts_with("Override refused:") && toast.to_lowercase().contains("outside"),
        "parent-outside-root must be refused with 'Override refused:' + 'outside' wording; got: {toast}"
    );
    // The outside directory must not have received a new file.
    assert!(
        outside_dir.read_dir().expect("read outside dir").next().is_none(),
        "outside dir remains empty (no aws.toml written through the symlink)",
    );
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
