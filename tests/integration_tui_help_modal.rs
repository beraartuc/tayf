//! Integration test: Help modal opens on `?`, dismisses on any key with
//! the dismissing key DISCARDED (vim/less convention, spec §12.4 D4).

mod common;
use common::tui_harness;

use tayf::__test_api::{
    current_selected_idx, has_modal_open, is_help_modal_open, is_quit_confirm_modal_open, send_key,
    KeyCode, KeyEvent, KeyModifiers,
};

#[test]
fn help_modal_opens_on_question_mark_and_dismisses_on_any_key() {
    // 80×100 backend: modal's 24% vertical share leaves ~22 content rows,
    // enough for the full HELP_MODAL_CONTENT (~28 lines) to surface its
    // upper section including the "Editing" header + 'n' keybind line.
    let (mut app, mut terminal) = tui_harness::boot_tui_with_sample_sized("hello world", 80, 100);

    // Initially no modal.
    assert!(!has_modal_open(&app), "no modal at boot");

    // Send `?` — opens Modal::Help.
    send_key(&mut app, KeyEvent::new(KeyCode::Char('?'), KeyModifiers::empty()));
    assert!(is_help_modal_open(&app), "Modal::Help opened by '?'");

    // Help content must be rendered: title + at least one specific keybind.
    let buf = tui_harness::draw_frame(&app, &mut terminal);
    assert!(
        tui_harness::find_text(&buf, "Keybindings").is_some(),
        "help title 'Keybindings' visible in buffer",
    );
    assert!(
        tui_harness::find_text(&buf, "New pattern modal").is_some(),
        "'n' keybind description visible in buffer",
    );

    // Snapshot the selection BEFORE the dismissing key. The dismissing key
    // (here: `j`, which would normally move selection DOWN) must be
    // DISCARDED — selection stays put.
    let selected_before = current_selected_idx(&app);

    send_key(&mut app, KeyEvent::new(KeyCode::Char('j'), KeyModifiers::empty()));

    assert!(!has_modal_open(&app), "Help dismissed by 'j' (any key)");
    assert_eq!(
        current_selected_idx(&app),
        selected_before,
        "dismissing key 'j' discarded — selection unchanged",
    );
}

#[test]
fn help_modal_opens_on_f1_and_can_be_dismissed_with_q_discarding_quit() {
    // F1 alt-binding parity with `?`. Also verifies the vim/less DISCARD
    // semantic for a key (`q`) that would otherwise have a powerful side
    // effect (the quit flow): the modal MUST dismiss without quitting.
    let (mut app, mut terminal) = tui_harness::boot_tui_with_sample_sized("hello world", 80, 100);
    assert!(!has_modal_open(&app), "no modal at boot");

    send_key(&mut app, KeyEvent::new(KeyCode::F(1), KeyModifiers::empty()));
    assert!(is_help_modal_open(&app), "Modal::Help opened by F1");

    // Re-draw so the second test path also exercises the render arm.
    let _ = tui_harness::draw_frame(&app, &mut terminal);

    send_key(&mut app, KeyEvent::new(KeyCode::Char('q'), KeyModifiers::empty()));
    assert!(!has_modal_open(&app), "Help dismissed by 'q'");
    // 'q' was DISCARDED — no quit flow, no quit-confirm modal.
    assert!(!is_quit_confirm_modal_open(&app), "dismissing 'q' must not initiate the quit flow",);
}
