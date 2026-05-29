//! Single-screen conflict-resolution UI for save-time merge conflicts.
//!
//! Renders a `ratatui::List` where each row corresponds to one
//! [`KeyConflict`](crate::config_tui::merge::KeyConflict) from
//! [`merge_three_way`](crate::config_tui::merge::merge_three_way). The
//! user picks Ours, Theirs, or Skip for each conflict; Enter bulk-
//! applies all choices via
//! [`commit_bytes`](crate::config_tui::save::commit_bytes); Esc cancels
//! the entire merge.
//!
//! Spec v0.6.2 §3.6.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::config_tui::merge::{key_path_display, ConflictValueShape, KeyConflict};
use crate::config_tui::widgets::save_diff::ConflictChoice;

/// Marker type for `Modal::ConflictList`. The list contents, per-row
/// selection, focused row index, and underlying documents all live on
/// [`crate::config_tui::widgets::save_diff::SaveDiffState::MergePending`] —
/// this struct exists solely so the modal-routing enum can dispatch
/// `Modal::ConflictList(_)` without redundantly mirroring those fields.
#[derive(Debug, Clone, Default)]
pub(crate) struct ConflictListState;

fn truncate_for_display(value: &str, max_chars: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= max_chars {
        return value.to_owned();
    }
    let mut out: String = chars.iter().take(max_chars).collect();
    out.push('…');
    out
}

fn render_row(conflict: &KeyConflict, choice: ConflictChoice, focused: bool) -> ListItem<'static> {
    let marker = match choice {
        ConflictChoice::Ours => "[O]",
        ConflictChoice::Theirs => "[T]",
        ConflictChoice::Skip => "[S]",
    };
    let path = key_path_display(&conflict.path);
    let (ours_short, theirs_short) = match conflict.shape {
        ConflictValueShape::Leaf => (
            truncate_for_display(&conflict.ours_value, 16),
            truncate_for_display(&conflict.theirs_value, 16),
        ),
        ConflictValueShape::Block => ("(table)".to_owned(), "(table)".to_owned()),
    };
    let arrow = if focused { "▶ " } else { "  " };
    let suffix = if conflict.is_array_block { "  ⚠ array merge v0.7+" } else { "" };
    let text =
        format!("{arrow}{marker} {path}    ours:{ours_short}  theirs:{theirs_short}{suffix}");
    let style =
        if focused { Style::default().add_modifier(Modifier::REVERSED) } else { Style::default() };
    ListItem::new(text).style(style)
}

/// Render the conflict-resolution modal over `area`.
pub fn render_conflict_list(
    frame: &mut Frame,
    area: Rect,
    conflicts: &[KeyConflict],
    selection: &[ConflictChoice],
    focused_row: usize,
) {
    frame.render_widget(Clear, area);
    let n = conflicts.len();
    let plural = if n == 1 { "" } else { "s" };
    let title = format!("Save Conflicts ({n} key{plural})");

    let items: Vec<ListItem> = conflicts
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let choice = selection.get(i).copied().unwrap_or(ConflictChoice::Skip);
            render_row(c, choice, i == focused_row)
        })
        .collect();
    let list = List::new(items).block(Block::default().title(title).borders(Borders::ALL));

    let chunks = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Min(3),
        ratatui::layout::Constraint::Length(2),
    ])
    .split(area);
    frame.render_widget(list, chunks[0]);

    let help = "j/k nav · o ours · t theirs · s skip\nEnter apply · Esc cancel";
    frame.render_widget(Paragraph::new(help), chunks[1]);
}
