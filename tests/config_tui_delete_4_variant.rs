//! Integration tests for G4: RuleId 4-variant delete + `ResetOverride` rename.
//! Spec v0.6.2 §3.3.
//!
//! Tests use the `__test_api` function-based harness pattern (no struct
//! `TuiHarness`). `compile_pending_individual_patterns` returns compiled
//! pattern strings so tests can assert rule presence / absence without
//! exposing the `pub(crate)` `Compiled` type.

mod common;

use tayf::__test_api::{
    apply_reset_override_builtin, boot_app_with_bound_empty_snapshot,
    compile_pending_individual_patterns, edits_deleted_has_builtin, edits_rules_has_builtin,
    send_key, stage_builtin_fg_edit, stage_delete_builtin, stage_delete_disk_profile,
    stage_delete_user_config, KeyCode, KeyEvent, KeyModifiers,
};

fn empty_app() -> tayf::__test_api::AppHandle {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cfg_path = tmp.path().join("config.toml");
    boot_app_with_bound_empty_snapshot(cfg_path)
}

// -----------------------------------------------------------------------
// G4-T1: Builtin delete masks the rule in compile_pending output.
// -----------------------------------------------------------------------

/// Stage `Builtin("ipv4")` delete; compiled individuals must not contain
/// the shipped ipv4 pattern. Verifies the `enabled = false` injection path
/// for the `RuleId::Builtin` arm in `compile_pending.rs` step 3.
#[test]
fn delete_builtin_rule_masks_in_compile_pending() {
    let mut app = empty_app();
    stage_delete_builtin(&mut app, "ipv4");
    let patterns = compile_pending_individual_patterns(&app);
    // The shipped ipv4 pattern contains specific octets notation.
    let has_ipv4 = patterns.iter().any(|p| p.contains("25[0-5]") || p.contains("2[0-4]\\d"));
    assert!(!has_ipv4, "ipv4 pattern must be absent after Builtin delete; patterns: {patterns:?}");
}

// -----------------------------------------------------------------------
// G4-T2: A second builtin still present after deleting one.
// -----------------------------------------------------------------------

/// Delete `ipv4`; `ipv6` (different builtin) must still be compiled.
/// Guards against an over-broad suppression that wipes all rules.
#[test]
fn delete_one_builtin_leaves_others_intact() {
    let mut app = empty_app();
    stage_delete_builtin(&mut app, "ipv4");
    let patterns = compile_pending_individual_patterns(&app);
    // ipv6 pattern includes the loopback literal ::1
    let has_ipv6 = patterns.iter().any(|p| p.contains("::1"));
    assert!(has_ipv6, "ipv6 must remain after deleting only ipv4; patterns: {patterns:?}");
}

// -----------------------------------------------------------------------
// G4-T3: UserConfig delete (unchanged behavior regression).
// -----------------------------------------------------------------------

/// `RuleId::UserConfig` delete path: a user-config rule absent from the
/// compiled output after staging a delete. Mirrors the pre-G4 behavior
/// (retain-based removal from `user_rules`).
#[test]
fn delete_user_config_rule_masks_in_compile_pending() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cfg_path = tmp.path().join("config.toml");
    // Write a UserRule to the config so the snapshot picks it up.
    std::fs::write(
        &cfg_path,
        "[[rules]]\nname = \"my-rule\"\npattern = \"FOOBAR_UNIQUE\"\n[rules.style]\nfg = \"blue\"\n",
    )
    .expect("write config");
    let mut app = tayf::__test_api::boot_app_from_disk_path(&cfg_path).expect("read snapshot");

    stage_delete_user_config(&mut app, "my-rule");
    let patterns = compile_pending_individual_patterns(&app);
    assert!(
        !patterns.iter().any(|p| p.contains("FOOBAR_UNIQUE")),
        "my-rule pattern must be absent after UserConfig delete"
    );
}

// -----------------------------------------------------------------------
// G4-T4: DiskProfile delete masks the rule.
// -----------------------------------------------------------------------

/// Stage a `DiskProfile` delete for a rule whose name matches a builtin
/// ("ipv4"). The `enabled = false` injection must suppress it.
/// This exercises the `RuleId::DiskProfile` arm (String-typed rule field).
#[test]
fn delete_disk_profile_rule_masks_matching_name_in_compile_pending() {
    let mut app = empty_app();
    stage_delete_disk_profile(&mut app, "custom", "ipv4");
    let patterns = compile_pending_individual_patterns(&app);
    let has_ipv4 = patterns.iter().any(|p| p.contains("25[0-5]") || p.contains("2[0-4]\\d"));
    assert!(!has_ipv4, "DiskProfile delete of 'ipv4' must suppress it from compiled output");
}

// -----------------------------------------------------------------------
// G4-T5: ResetOverride clears edits.rules + edits.deleted for the target.
// -----------------------------------------------------------------------

/// Stage a style edit AND a delete for `Builtin("ipv4")`, then call
/// `apply_reset_override_builtin`. Both `edits.rules` and `edits.deleted`
/// entries must be cleared.
#[test]
fn reset_override_builtin_clears_rules_and_deleted() {
    let mut app = empty_app();
    // Stage a style edit into edits.rules.
    stage_builtin_fg_edit(&mut app, "ipv4");
    assert!(edits_rules_has_builtin(&app, "ipv4"), "precondition: rules entry present");
    // Stage a delete into edits.deleted.
    stage_delete_builtin(&mut app, "ipv4");
    assert!(edits_deleted_has_builtin(&app, "ipv4"), "precondition: deleted entry present");

    apply_reset_override_builtin(&mut app, "ipv4");

    assert!(
        !edits_rules_has_builtin(&app, "ipv4"),
        "reset must clear edits.rules[Builtin(\"ipv4\")]"
    );
    assert!(
        !edits_deleted_has_builtin(&app, "ipv4"),
        "reset must clear edits.deleted[Builtin(\"ipv4\")]"
    );
}

// -----------------------------------------------------------------------
// G4-T6: 'd' keystroke on Patterns tab opens DeleteRule confirm modal.
// -----------------------------------------------------------------------

/// Press 'd' on the Patterns tab; confirm modal must open with
/// `ConfirmAction::DeleteRule` payload (formerly `DeleteUserRule`).
#[test]
fn d_keystroke_opens_delete_rule_confirm_modal() {
    let mut app = empty_app();
    send_key(&mut app, KeyEvent::new(KeyCode::Char('d'), KeyModifiers::empty()));
    assert!(
        tayf::__test_api::is_delete_rule_confirm_modal_open(&app),
        "'d' keystroke must open a DeleteRule Confirm modal"
    );
}
