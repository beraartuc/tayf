//! Integration tests for v0.6.2 G8 — Item 6: per-key conflict-list UI.
//!
//! Render-side coverage on `render_conflict_list` (title pluralization,
//! help footer, leaf vs block row shapes). The save-side commit_bytes
//! invariant is covered by lib-level tests in `src/config_tui/save.rs`;
//! the merge algorithm itself is covered by `#[cfg(test)] mod tests`
//! inline in `src/config_tui/merge.rs` (moved from the prior
//! `tests/config_tui_merge_3way.rs` integration suite when the module
//! was demoted to `pub(crate)` in v0.6.3 I2).

#![allow(clippy::expect_used)] // integration-test convenience

use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;

use tayf::__test_api::{render_conflict_list, ConflictChoice, ConflictValueShape, KeyConflict};

fn leaf_conflict(path: &[&str], ours: &str, theirs: &str) -> KeyConflict {
    KeyConflict {
        path: path.iter().map(|s| (*s).to_owned()).collect(),
        base_value: "(absent)".to_owned(),
        ours_value: ours.to_owned(),
        theirs_value: theirs.to_owned(),
        shape: ConflictValueShape::Leaf,
        is_array_block: false,
    }
}

fn block_conflict(path: &[&str]) -> KeyConflict {
    KeyConflict {
        path: path.iter().map(|s| (*s).to_owned()).collect(),
        base_value: "(table)".to_owned(),
        ours_value: "(table)".to_owned(),
        theirs_value: "(table)".to_owned(),
        shape: ConflictValueShape::Block,
        is_array_block: false,
    }
}

fn draw(
    conflicts: &[KeyConflict],
    selection: &[ConflictChoice],
    focused_row: usize,
) -> ratatui::buffer::Buffer {
    let mut term = Terminal::new(TestBackend::new(80, 20)).expect("backend");
    term.draw(|f| {
        let area = Rect::new(0, 0, 80, 20);
        render_conflict_list(f, area, conflicts, selection, focused_row);
    })
    .expect("draw");
    term.backend().buffer().clone()
}

fn row_text(buf: &ratatui::buffer::Buffer, row: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, row)].symbol().to_owned()).collect()
}

#[test]
fn conflict_list_title_singularizes_one_key() {
    let conflicts = vec![leaf_conflict(&["general", "theme"], "tokyo", "light")];
    let selection = vec![ConflictChoice::Ours];
    let buf = draw(&conflicts, &selection, 0);
    let row0 = row_text(&buf, 0);
    assert!(row0.contains("Save Conflicts (1 key)"), "1 conflict → singular 'key'; got: {row0}");
    assert!(!row0.contains("1 keys"), "no plural-with-1 in title");
}

#[test]
fn conflict_list_title_pluralizes_two_keys() {
    let conflicts = vec![
        leaf_conflict(&["general", "theme"], "tokyo", "light"),
        leaf_conflict(&["general", "profile"], "aws", "k8s"),
    ];
    let selection = vec![ConflictChoice::Ours, ConflictChoice::Theirs];
    let buf = draw(&conflicts, &selection, 0);
    let row0 = row_text(&buf, 0);
    assert!(row0.contains("Save Conflicts (2 keys)"), "2 conflicts → 'keys' plural; got: {row0}");
}

#[test]
fn conflict_list_help_footer_exact_wording_pinned() {
    let conflicts = vec![leaf_conflict(&["x"], "1", "2")];
    let selection = vec![ConflictChoice::Ours];
    let buf = draw(&conflicts, &selection, 0);
    // Help footer takes the last 2 rows of the 20-row test backend.
    let footer_line_1 = row_text(&buf, 18);
    let footer_line_2 = row_text(&buf, 19);
    assert!(
        footer_line_1.contains("j/k nav")
            && footer_line_1.contains("o ours")
            && footer_line_1.contains("t theirs")
            && footer_line_1.contains("s skip"),
        "first help line missing expected fragments; got: {footer_line_1:?}"
    );
    assert!(
        footer_line_2.contains("Enter apply") && footer_line_2.contains("Esc cancel"),
        "second help line missing expected fragments; got: {footer_line_2:?}"
    );
}

#[test]
fn conflict_list_block_shape_row_renders_table_placeholder() {
    let conflicts = vec![block_conflict(&["rules"])];
    let selection = vec![ConflictChoice::Skip];
    let buf = draw(&conflicts, &selection, 0);
    // Row 1 is the first list entry (row 0 is the title border).
    let mut concat = String::new();
    for r in 1u16..18 {
        concat.push_str(&row_text(&buf, r));
        concat.push('\n');
    }
    assert!(
        concat.contains("(table)"),
        "block-shape row must surface '(table)' placeholder; rendered rows:\n{concat}"
    );
    assert!(
        concat.contains("[S]"),
        "block-shape default selection is Skip → '[S]' marker; rendered rows:\n{concat}"
    );
}

#[test]
fn conflict_list_focused_row_carries_arrow_marker() {
    let conflicts = vec![leaf_conflict(&["a"], "1", "2"), leaf_conflict(&["b"], "3", "4")];
    let selection = vec![ConflictChoice::Ours, ConflictChoice::Ours];
    let buf = draw(&conflicts, &selection, 1);
    let mut concat = String::new();
    for r in 1u16..18 {
        concat.push_str(&row_text(&buf, r));
        concat.push('\n');
    }
    // The second row carries the ▶ focus marker; the first does not.
    let lines: Vec<&str> = concat.lines().filter(|l| !l.trim().is_empty()).collect();
    let row_a = lines.iter().find(|l| l.contains(" a    ")).copied().unwrap_or("");
    let row_b = lines.iter().find(|l| l.contains(" b    ")).copied().unwrap_or("");
    assert!(
        !row_a.contains('▶'),
        "non-focused row 'a' must not carry the ▶ marker; got: {row_a:?}"
    );
    assert!(row_b.contains('▶'), "focused row 'b' must carry the ▶ marker; got: {row_b:?}");
}

#[test]
fn conflict_list_array_block_row_surfaces_v0_7_array_merge_warning() {
    let conflict = KeyConflict {
        path: vec!["rules".to_owned()],
        base_value: "(array)".to_owned(),
        ours_value: "(array)".to_owned(),
        theirs_value: "(array)".to_owned(),
        shape: ConflictValueShape::Block,
        is_array_block: true,
    };
    let conflicts = vec![conflict];
    let selection = vec![ConflictChoice::Skip];
    let buf = draw(&conflicts, &selection, 0);
    let mut concat = String::new();
    for r in 1u16..18 {
        concat.push_str(&row_text(&buf, r));
        concat.push('\n');
    }
    assert!(
        concat.contains("array merge v0.7+"),
        "array-of-tables row must carry the v0.7+ banner; rendered rows:\n{concat}"
    );
}
