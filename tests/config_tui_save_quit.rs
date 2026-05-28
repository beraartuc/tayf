//! Integration tests for G2: pending_save_and_quit flag + reset
//! invariant on all SaveDiff non-commit exit paths.
//!
//! Spec §3.8.

mod common;

use tayf::__test_api::{
    boot_app_from_disk_path, boot_app_with_sample, help_modal_content, is_quit_confirm_modal_open,
    is_save_diff_modal_open, make_pending_edit, pending_save_and_quit, send_char, send_esc, send_q,
    should_quit,
};

/// Confirm that pressing `s` in the `QuitWithUnsavedEdits` modal sets the
/// `pending_save_and_quit` flag and opens the `SaveDiff` overlay.
///
/// G2 spec §3.8: `s` in quit modal sets flag then triggers save flow.
#[test]
fn quit_modal_s_sets_pending_save_and_quit_flag() {
    let mut app = boot_app_with_sample("hello world");
    make_pending_edit(&mut app);
    send_q(&mut app);
    assert!(is_quit_confirm_modal_open(&app), "precondition: quit modal open");

    send_char(&mut app, 's');
    assert!(pending_save_and_quit(&app), "s in quit modal must set pending_save_and_quit flag");
    assert!(is_save_diff_modal_open(&app), "s in quit modal must open SaveDiff overlay");
}

/// Confirm that a successful save with the flag set propagates `should_quit`.
///
/// G2 spec §3.8: commit success branch sets `should_quit` when flag is set.
#[test]
fn save_success_with_pending_quit_flag_sets_should_quit() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let cfg_path = tmp.path().join("config.toml");
    std::fs::write(&cfg_path, b"[general]\n").expect("seed config");
    let mut app = boot_app_from_disk_path(&cfg_path).expect("boot");

    make_pending_edit(&mut app);
    send_q(&mut app);
    assert!(is_quit_confirm_modal_open(&app), "precondition: quit modal open");

    send_char(&mut app, 's'); // sets flag + opens SaveDiff (Clean state)
    assert!(pending_save_and_quit(&app), "precondition: flag set");
    assert!(is_save_diff_modal_open(&app), "precondition: SaveDiff open");

    send_char(&mut app, 'y'); // Confirm Clean save → commit_save succeeds
    assert!(should_quit(&app), "successful save with pending flag must set should_quit");
}

/// Confirm that pressing `n` on the Clean SaveDiff modal resets the flag.
///
/// G2 spec §3.8: `CloseModal` exit path MUST reset `pending_save_and_quit`.
#[test]
fn save_close_modal_n_resets_pending_save_and_quit_flag() {
    let mut app = boot_app_with_sample("hello world");
    make_pending_edit(&mut app);
    send_q(&mut app);
    send_char(&mut app, 's'); // sets flag + opens SaveDiff
    assert!(pending_save_and_quit(&app), "precondition: flag set");

    send_char(&mut app, 'n'); // n on Clean → CloseModal
    assert!(
        !pending_save_and_quit(&app),
        "n on Clean SaveDiff must reset pending_save_and_quit flag"
    );
    assert!(!should_quit(&app), "n must not trigger quit");
}

/// Confirm that Esc on the SaveDiff modal resets the flag.
///
/// G2 spec §3.8: Esc-tier-2 SaveDiff close MUST reset `pending_save_and_quit`.
#[test]
fn save_esc_on_clean_resets_pending_save_and_quit_flag() {
    let mut app = boot_app_with_sample("hello world");
    make_pending_edit(&mut app);
    send_q(&mut app);
    send_char(&mut app, 's'); // sets flag + opens SaveDiff
    assert!(pending_save_and_quit(&app), "precondition: flag set");

    send_esc(&mut app); // Esc tier-2 closes SaveDiff
    assert!(!pending_save_and_quit(&app), "Esc on SaveDiff must reset pending_save_and_quit flag");
    assert!(!should_quit(&app), "Esc must not trigger quit");
}

/// Enumeration test: every non-commit `SaveDiff` exit path MUST reset
/// `pending_save_and_quit`. Covers the 3 paths reachable without Conflict
/// state in a single assertion sweep:
///
/// * Path 1 — `CloseModal` (`n` on Clean SaveDiff)
/// * Path 3 — SaveDiff Esc tier-2 (`handle_esc` → `Modal::SaveDiff` branch)
/// * Path 4 — commit-error (`y` when `source_path` is `None` → `Err`)
///
/// Path 2 (`DiscardAndReload`, reachable only via `ConflictDiscardConfirm`)
/// requires a disk file with a hash mismatch to enter Conflict state; it is
/// out-of-scope for this integration test. The reset at
/// `events.rs:SaveDiffOutcome::DiscardAndReload` is the direct enforcement.
///
/// Pinned by the doc-comment on `App::pending_save_and_quit` in `app.rs`.
#[test]
fn test_pending_save_and_quit_resets_on_every_non_commit_exit() {
    // Path 1: CloseModal (n on Clean SaveDiff)
    {
        let mut app = boot_app_with_sample("hello world");
        make_pending_edit(&mut app);
        send_q(&mut app);
        send_char(&mut app, 's');
        assert!(pending_save_and_quit(&app), "path 1 precondition: flag set after s");
        send_char(&mut app, 'n');
        assert!(
            !pending_save_and_quit(&app),
            "path 1: CloseModal (n on Clean) must reset pending_save_and_quit"
        );
        assert!(!should_quit(&app), "path 1: n must not trigger quit");
    }

    // Path 3: SaveDiff Esc tier-2 (handle_esc → Modal::SaveDiff branch)
    {
        let mut app = boot_app_with_sample("hello world");
        make_pending_edit(&mut app);
        send_q(&mut app);
        send_char(&mut app, 's');
        assert!(pending_save_and_quit(&app), "path 3 precondition: flag set after s");
        send_esc(&mut app);
        assert!(
            !pending_save_and_quit(&app),
            "path 3: SaveDiff Esc tier-2 must reset pending_save_and_quit"
        );
        assert!(!should_quit(&app), "path 3: Esc must not trigger quit");
    }

    // Path 4: commit-error (source_path is None → commit_save returns Err)
    // boot_app_with_sample produces source_path = None; 'y' on Clean triggers
    // commit_save which immediately errors, hitting the Err reset branch.
    {
        let mut app = boot_app_with_sample("hello world");
        make_pending_edit(&mut app);
        send_q(&mut app);
        send_char(&mut app, 's');
        assert!(pending_save_and_quit(&app), "path 4 precondition: flag set after s");
        send_char(&mut app, 'y'); // triggers commit_save → Err → resets flag
        assert!(
            !pending_save_and_quit(&app),
            "path 4: commit-error branch must reset pending_save_and_quit"
        );
        assert!(!should_quit(&app), "path 4: save error must not trigger quit");
    }
}

/// Confirm the help modal lists the save-then-quit binding.
///
/// G2 spec §3.8 + §3.5: Persistence section must include the exact entry.
#[test]
fn help_modal_content_lists_save_then_quit_in_quit_modal() {
    let content = help_modal_content();
    assert!(
        content.contains("s (in quit modal)  Save then quit"),
        "help modal must list save-then-quit Persistence entry exactly;\
         actual content:\n{content}"
    );
}
