//! Integration test: TUI mini-preview renders colorized spans.

mod common;
use common::tui_harness;

#[test]
fn mini_preview_renders_ipv4_as_styled_span() {
    let (app, mut terminal) = tui_harness::boot_tui_with_sample("see 192.168.1.1 here");
    let buf = tui_harness::draw_frame(&app, &mut terminal);
    let pos = tui_harness::find_text(&buf, "192.168.1.1");
    assert!(pos.is_some(), "ipv4 substring rendered in TUI buffer");
    let (col, row) = pos.unwrap();
    let style = tui_harness::cell_style(&buf, col, row);
    assert!(
        style.fg.is_some() || !style.add_modifier.is_empty(),
        "ipv4 cells styled (fg or modifier set), got: {style:?}"
    );
}
