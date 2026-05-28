//! Integration tests for G3: ColorPicker bool-axis UI + `c` clear +
//! `NewStyle` tri-state migration.
//!
//! Spec §3.1 — pins:
//! 1. Default `axis_focus` is `None` (color-section focused on open).
//! 2. Tab cycles color sections then axes then wraps to Ansi16.
//! 3. `c` in TrueHex section with `axis_focus == None` is a hex digit
//!    (T-B2 regression — must NOT clear an axis).
//! 4. `c` with an axis focused writes `Some(None)` into that axis.
//! 5. Space toggles the focused axis: unedited/false/cleared → `Some(Some(true))`,
//!    `Some(Some(true))` → `Some(Some(false))`.
//! 6. Enter on an unedited axis leaves it at outer-`None` in PendingEdits
//!    (no write — R-I9).
//! 7. Esc on the modal does NOT stage any pending bool-axis edit into
//!    `app.edits`.
//! 8. `HELP_MODAL_CONTENT` lists the new Color Picker subsection keys.

mod common;

use tayf::__test_api::{
    boot_app_with_sample, color_picker_axis_focus_tag, color_picker_section_tag,
    color_picker_staged_axes, edits_are_dirty, help_modal_content, is_color_picker_modal_open,
    open_color_picker, pending_edits_first_builtin_axes, send_key, KeyCode, KeyEvent, KeyModifiers,
};

/// Single bare keypress without modifiers.
fn k(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

#[test]
fn color_picker_axis_focus_default_is_none() {
    // Spec §3.1: on open, the picker focuses the color sections (Ansi16) and
    // `axis_focus` is `None`. The three `staged_*` axes start outer-`None`.
    let mut app = boot_app_with_sample("see 192.168.1.1 here");
    open_color_picker(&mut app);
    assert!(is_color_picker_modal_open(&app));
    assert_eq!(color_picker_axis_focus_tag(&app), Some("none"));
    assert_eq!(color_picker_section_tag(&app), Some("ansi16"));
    let (b, i, u) = color_picker_staged_axes(&app).expect("modal open");
    assert_eq!(b, None);
    assert_eq!(i, None);
    assert_eq!(u, None);
}

#[test]
fn color_picker_tab_cycles_color_then_axes_then_wraps() {
    // Spec §3.1: 6-step Tab cycle:
    //   Ansi16 → Palette256 → TrueHex → Bold → Italic → Underline → wrap.
    let mut app = boot_app_with_sample("");
    open_color_picker(&mut app);
    assert_eq!(color_picker_section_tag(&app), Some("ansi16"));
    assert_eq!(color_picker_axis_focus_tag(&app), Some("none"));
    send_key(&mut app, k(KeyCode::Tab));
    assert_eq!(color_picker_section_tag(&app), Some("palette256"));
    assert_eq!(color_picker_axis_focus_tag(&app), Some("none"));
    send_key(&mut app, k(KeyCode::Tab));
    assert_eq!(color_picker_section_tag(&app), Some("truehex"));
    assert_eq!(color_picker_axis_focus_tag(&app), Some("none"));
    send_key(&mut app, k(KeyCode::Tab));
    assert_eq!(color_picker_axis_focus_tag(&app), Some("bold"));
    send_key(&mut app, k(KeyCode::Tab));
    assert_eq!(color_picker_axis_focus_tag(&app), Some("italic"));
    send_key(&mut app, k(KeyCode::Tab));
    assert_eq!(color_picker_axis_focus_tag(&app), Some("underline"));
    send_key(&mut app, k(KeyCode::Tab));
    assert_eq!(color_picker_section_tag(&app), Some("ansi16"), "wrap back to Ansi16");
    assert_eq!(color_picker_axis_focus_tag(&app), Some("none"));
}

#[test]
fn color_picker_c_in_truecolor_section_without_axis_focus_is_hex_digit() {
    // T-B2 regression: `c` without axis focus must fall through to the
    // TrueHex hex-input branch. The character must end up in `hex_buf`
    // (observable as the picker emitting Color::Rgb when buf becomes 6
    // chars), and `staged_bold` must remain `None`.
    let mut app = boot_app_with_sample("");
    open_color_picker(&mut app);
    // Tab twice to land on TrueHex with no axis focus.
    send_key(&mut app, k(KeyCode::Tab));
    send_key(&mut app, k(KeyCode::Tab));
    assert_eq!(color_picker_section_tag(&app), Some("truehex"));
    assert_eq!(color_picker_axis_focus_tag(&app), Some("none"));
    // Now press 'c' — must be absorbed as a hex digit, not cleared into an axis.
    send_key(&mut app, k(KeyCode::Char('c')));
    let (b, i, u) = color_picker_staged_axes(&app).expect("modal open");
    assert_eq!(b, None, "`c` without axis focus must NOT touch staged_bold");
    assert_eq!(i, None);
    assert_eq!(u, None);
}

#[test]
fn color_picker_c_with_axis_focus_clears_to_some_none() {
    // Spec §3.1: `c` with an axis focused writes `Some(None)` into that
    // axis's staged tri-state.
    let mut app = boot_app_with_sample("");
    open_color_picker(&mut app);
    // Tab forward 3 times to reach AxisFocus::Bold.
    for _ in 0..3 {
        send_key(&mut app, k(KeyCode::Tab));
    }
    assert_eq!(color_picker_axis_focus_tag(&app), Some("bold"));
    send_key(&mut app, k(KeyCode::Char('c')));
    let (b, i, u) = color_picker_staged_axes(&app).expect("modal open");
    assert_eq!(b, Some(None), "`c` clears bold to Some(None)");
    assert_eq!(i, None);
    assert_eq!(u, None);
}

#[test]
fn color_picker_space_toggles_focused_bool_axis() {
    // Spec §3.1: Space on a focused axis toggles its tri-state.
    // Unedited (None) → Some(Some(true)). Some(Some(true)) → Some(Some(false)).
    // Some(Some(false)) / Some(None) → Some(Some(true)).
    let mut app = boot_app_with_sample("");
    open_color_picker(&mut app);
    for _ in 0..3 {
        send_key(&mut app, k(KeyCode::Tab));
    }
    assert_eq!(color_picker_axis_focus_tag(&app), Some("bold"));

    // 1st Space: None → Some(Some(true))
    send_key(&mut app, k(KeyCode::Char(' ')));
    let (b, _, _) = color_picker_staged_axes(&app).expect("modal open");
    assert_eq!(b, Some(Some(true)));

    // 2nd Space: Some(Some(true)) → Some(Some(false))
    send_key(&mut app, k(KeyCode::Char(' ')));
    let (b, _, _) = color_picker_staged_axes(&app).expect("modal open");
    assert_eq!(b, Some(Some(false)));

    // 3rd Space: Some(Some(false)) → Some(Some(true))
    send_key(&mut app, k(KeyCode::Char(' ')));
    let (b, _, _) = color_picker_staged_axes(&app).expect("modal open");
    assert_eq!(b, Some(Some(true)));

    // After `c` clears to Some(None), one more Space goes back to Some(Some(true)).
    send_key(&mut app, k(KeyCode::Char('c')));
    let (b, _, _) = color_picker_staged_axes(&app).expect("modal open");
    assert_eq!(b, Some(None));
    send_key(&mut app, k(KeyCode::Char(' ')));
    let (b, _, _) = color_picker_staged_axes(&app).expect("modal open");
    assert_eq!(b, Some(Some(true)));
}

#[test]
fn color_picker_unedited_axis_stays_none_outer_after_enter() {
    // Spec §3.1 R-I9: if no Space/`c` ever fired on an axis, its staged
    // value is outer-`None` and Enter does NOT write `Some(_)` into
    // `app.edits.rules[..].styles[Default]`.
    let mut app = boot_app_with_sample("");
    open_color_picker(&mut app);
    // Pick a color (Ansi16 idx 0 is the default) and commit. No Space/`c`
    // fired on any bool axis.
    send_key(&mut app, k(KeyCode::Enter));
    assert!(!is_color_picker_modal_open(&app), "modal closes on Enter");
    let (b, i, u) = pending_edits_first_builtin_axes(&app);
    assert_eq!(b, None, "untouched bold axis must stay outer-None");
    assert_eq!(i, None);
    assert_eq!(u, None);
}

#[test]
fn color_picker_esc_does_not_stage_pending_bool_axis_edits() {
    // Spec §3.1: Esc discards the picker's staged_* edits — nothing leaks
    // into `app.edits` even when Space toggled an axis to Some(Some(true)).
    let mut app = boot_app_with_sample("");
    open_color_picker(&mut app);
    // Reach Bold focus and toggle to Some(Some(true)).
    for _ in 0..3 {
        send_key(&mut app, k(KeyCode::Tab));
    }
    send_key(&mut app, k(KeyCode::Char(' ')));
    let (b, _, _) = color_picker_staged_axes(&app).expect("modal open");
    assert_eq!(b, Some(Some(true)), "precondition: bold staged");
    // Esc discards.
    send_key(&mut app, k(KeyCode::Esc));
    assert!(!is_color_picker_modal_open(&app), "Esc closes the modal");
    let (b, i, u) = pending_edits_first_builtin_axes(&app);
    assert_eq!(b, None, "Esc must not stage the toggled axis into app.edits");
    assert_eq!(i, None);
    assert_eq!(u, None);
    assert!(!edits_are_dirty(&app), "no pending edits dirty after Esc-cancel");
}

#[test]
fn help_modal_content_lists_color_picker_axis_keys() {
    // Spec §3.1: the help modal Editing section gains a Color Picker
    // subsection listing the new keystrokes. Pin against exact wording so
    // future help-text refactors that drop these lines fail loudly.
    let s = help_modal_content();
    assert!(s.contains("Color Picker"), "Color Picker subsection header listed");
    assert!(
        s.contains("Cycle: ANSI16 → 256 → hex → bold → italic → underline"),
        "Tab cycle line listed"
    );
    assert!(s.contains("Toggle focused boolean axis"), "Space toggle line listed");
    assert!(
        s.contains("Clear focused boolean axis (bold/italic/underline only)"),
        "c clear line listed"
    );
}
