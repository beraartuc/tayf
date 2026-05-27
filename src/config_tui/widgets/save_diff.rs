//! D1 conflict-aware save-diff modal. Spec §8.1 + §12.5.
//!
//! Two modes: Clean (single diff panel) and Conflict (dual diff panel
//! + merged-preview view after first 'y' per UX #5 fold + 'm' discard
//!   double-confirm per UX #4 fold).

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::config_tui::app::App;

// reason: `disk_now` on ConflictMergedPreview is carried through the
// state machine for v0.6+ merge reconciliation; v0.5.4 commits the
// TUI-side content unchanged because build_new_content is still
// pass-through (save.rs §C1c). Keep the field so the state shape
// matches the spec §8.4 D flow without a follow-up schema change.
#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum SaveDiffState {
    Clean { tui_diff: String },
    ConflictPending { tui_diff: String, manual_diff: String, disk_now: Vec<u8> },
    ConflictMergedPreview { merged_diff: String, disk_now: Vec<u8> },
    ConflictDiscardConfirm { disk_now: Vec<u8> },
}

#[derive(Debug)]
pub(crate) enum SaveDiffOutcome {
    StayOpen,
    CloseModal,
    Commit,
    DiscardAndReload(Vec<u8>),
}

/// Render the modal — `area` is the centered overlay rect.
pub(crate) fn render(frame: &mut Frame, area: Rect, app: &App) {
    frame.render_widget(Clear, area);
    let (title, body) = match app.save_diff.as_ref() {
        Some(SaveDiffState::Clean { tui_diff }) => {
            ("Save — Clean (y=commit, n/Esc=cancel)".to_owned(), tui_diff.clone())
        }
        Some(SaveDiffState::ConflictPending { tui_diff, manual_diff, .. }) => (
            "Save — CONFLICT (y=preview merge, m=discard TUI edits, n/Esc=cancel)".to_owned(),
            format!("TUI diff:\n{tui_diff}\n\nManual disk diff:\n{manual_diff}"),
        ),
        Some(SaveDiffState::ConflictMergedPreview { merged_diff, .. }) => {
            ("Save — merged preview (y=commit, n/Esc=cancel)".to_owned(), merged_diff.clone())
        }
        Some(SaveDiffState::ConflictDiscardConfirm { .. }) => (
            "Discard TUI edits and reload disk? [y/N]".to_owned(),
            "(destructive — default = N)".to_owned(),
        ),
        None => ("Save".to_owned(), "(no save state)".to_owned()),
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    frame.render_widget(Paragraph::new(body).block(block), area);
}

/// Key dispatch — returns the next state transition.
pub(crate) fn dispatch_key(app: &mut App, k: KeyEvent) -> SaveDiffOutcome {
    let Some(state) = app.save_diff.take() else {
        return SaveDiffOutcome::CloseModal;
    };
    match (state, k.code) {
        (SaveDiffState::Clean { .. }, KeyCode::Char('y')) => SaveDiffOutcome::Commit,
        (SaveDiffState::Clean { .. }, KeyCode::Char('n') | KeyCode::Esc) => {
            SaveDiffOutcome::CloseModal
        }
        (
            SaveDiffState::ConflictPending { tui_diff, manual_diff, disk_now },
            KeyCode::Char('y'),
        ) => {
            let merged_diff = format!(
                "(merged preview placeholder — TUI diff + manual disk diff reconciled)\n{tui_diff}\n---\n{manual_diff}"
            );
            app.save_diff = Some(SaveDiffState::ConflictMergedPreview { merged_diff, disk_now });
            SaveDiffOutcome::StayOpen
        }
        (SaveDiffState::ConflictPending { disk_now, .. }, KeyCode::Char('m')) => {
            app.save_diff = Some(SaveDiffState::ConflictDiscardConfirm { disk_now });
            SaveDiffOutcome::StayOpen
        }
        (SaveDiffState::ConflictMergedPreview { .. }, KeyCode::Char('y')) => {
            SaveDiffOutcome::Commit
        }
        (SaveDiffState::ConflictDiscardConfirm { disk_now }, KeyCode::Char('y')) => {
            SaveDiffOutcome::DiscardAndReload(disk_now)
        }
        (SaveDiffState::ConflictDiscardConfirm { .. }, KeyCode::Char('n') | KeyCode::Esc) => {
            // For v0.5.4 we close the modal in this case. v0.6+ may preserve
            // and bounce back to ConflictPending with the original diffs.
            SaveDiffOutcome::CloseModal
        }
        (_, KeyCode::Char('n') | KeyCode::Esc) => SaveDiffOutcome::CloseModal,
        (state, _) => {
            app.save_diff = Some(state);
            SaveDiffOutcome::StayOpen
        }
    }
}

/// Build the initial `SaveDiffState` from snapshot + edits — triggered by Ctrl+S.
pub(crate) fn build_initial_state(app: &App) -> SaveDiffState {
    let Some(cfg_path) = app.snapshot.source_path.as_deref() else {
        return SaveDiffState::Clean {
            tui_diff: "(first-run save — creating new config file)".to_owned(),
        };
    };
    let disk_now = std::fs::read(cfg_path).unwrap_or_default();
    let disk_hash = crate::config_tui::snapshot::sha256(&disk_now);
    let new_content = crate::config_tui::save::build_new_content(&app.snapshot, &app.edits);
    let tui_diff = build_diff(&app.snapshot.raw_bytes, new_content.as_bytes());
    if disk_hash == app.snapshot.source_hash {
        SaveDiffState::Clean { tui_diff }
    } else {
        let manual_diff = build_diff(&app.snapshot.raw_bytes, &disk_now);
        SaveDiffState::ConflictPending { tui_diff, manual_diff, disk_now }
    }
}

/// Tiny line-based diff (-/+ prefix). v0.5.4 inline impl — full
/// patience/Myers diff is overkill for the modal's display purpose.
fn build_diff(old: &[u8], new: &[u8]) -> String {
    let old_str = String::from_utf8_lossy(old);
    let new_str = String::from_utf8_lossy(new);
    let old_lines: std::collections::HashSet<&str> = old_str.lines().collect();
    let new_lines: std::collections::HashSet<&str> = new_str.lines().collect();
    let mut out = String::new();
    for line in old_str.lines() {
        if !new_lines.contains(line) {
            use std::fmt::Write;
            let _ = writeln!(out, "- {line}");
        }
    }
    for line in new_str.lines() {
        if !old_lines.contains(line) {
            use std::fmt::Write;
            let _ = writeln!(out, "+ {line}");
        }
    }
    if out.is_empty() {
        out.push_str("(no changes)\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_diff_no_change_returns_no_changes_marker() {
        let bytes = b"a\nb\nc\n";
        let d = build_diff(bytes, bytes);
        assert_eq!(d, "(no changes)\n");
    }

    #[test]
    fn build_diff_addition_emits_plus_prefix() {
        let d = build_diff(b"a\nb\n", b"a\nb\nc\n");
        assert!(d.contains("+ c"), "expected '+ c' line; got: {d}");
        assert!(!d.contains("- "), "expected no removals; got: {d}");
    }

    #[test]
    fn build_diff_removal_emits_minus_prefix() {
        let d = build_diff(b"a\nb\nc\n", b"a\nb\n");
        assert!(d.contains("- c"), "expected '- c' line; got: {d}");
        assert!(!d.contains("+ "), "expected no additions; got: {d}");
    }
}
