//! Integration tests for G5: Patterns tab union render (built-in +
//! user-config rules) with DIM section headers.
//!
//! Spec v0.6.2 §3.2. The pure layout / selectable-idx mapping is unit-
//! tested in `src/config_tui/tabs/patterns.rs`; this file covers the
//! render output the user actually sees in the terminal.

mod common;

use ratatui::style::Modifier;

use tayf::__test_api::boot_app_with_user_config_and_sample;

#[test]
fn patterns_tab_renders_section_headers_with_dim_modifier_and_user_rule() {
    let (app, mut terminal) = boot_app_with_user_config_and_sample(
        &[("unique-user-foo", "FOO")],
        "sample line\n",
        80,
        // Tall enough that the full union list (18 built-ins + both section
        // headers + the user rule) fits without scrolling — the built-in
        // catalog grew to 18 in v0.12.0, so 24 rows no longer shows the
        // User section at the initial (top) selection.
        44,
    );
    let buf = common::tui_harness::draw_frame(&app, &mut terminal);

    let (col, row) = common::tui_harness::find_text(&buf, "── Builtin ──")
        .expect("'── Builtin ──' section header rendered in list");
    let style = common::tui_harness::cell_style(&buf, col, row);
    assert!(
        style.add_modifier.contains(Modifier::DIM),
        "Builtin section header cell carries DIM modifier (got {:?})",
        style.add_modifier,
    );

    let (col, row) = common::tui_harness::find_text(&buf, "── User ──")
        .expect("'── User ──' section header rendered in list");
    let style = common::tui_harness::cell_style(&buf, col, row);
    assert!(
        style.add_modifier.contains(Modifier::DIM),
        "User section header cell carries DIM modifier (got {:?})",
        style.add_modifier,
    );

    assert!(
        common::tui_harness::find_text(&buf, "unique-user-foo").is_some(),
        "user-config rule name appears in the rendered list",
    );
}
