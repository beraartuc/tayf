//! D1 conflict-aware save-diff modal. Spec §8.1 + §12.5.
//!
//! Two modes: Clean (single diff panel) and Conflict (dual diff panel
//! + merged-preview view after first 'y' per UX #5 fold + 'm' discard
//!   double-confirm per UX #4 fold).

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::config_tui::app::App;

// reason: ConflictDiscardConfirm reachable in v0.7+ via the Discard path;
// MergePending carries four DocumentMut clones and is much larger than
// the other variants — Box would add an extra heap hop on every state
// transition for no clarity gain since the only large variant is also
// the one we mutate most.
#[allow(dead_code, clippy::large_enum_variant)]
#[derive(Debug)]
pub(crate) enum SaveDiffState {
    Clean {
        tui_diff: String,
    },
    /// G8 §3.6: replaces the legacy `ConflictPending` /
    /// `ConflictMergedPreview` pair. Built by
    /// [`build_initial_state`] when the merge module reports per-key
    /// conflicts; drives the
    /// [`crate::config_tui::app::Modal::ConflictList`] UI.
    MergePending {
        base: toml_edit::DocumentMut,
        ours: toml_edit::DocumentMut,
        theirs: toml_edit::DocumentMut,
        auto_merged: toml_edit::DocumentMut,
        conflicts: Vec<crate::config_tui::merge::KeyConflict>,
        selection: Vec<ConflictChoice>,
        focused_row: usize,
        /// Raw disk bytes at the moment the save was triggered. The
        /// `DiscardAndReload` UX hook hands them off to the snapshot
        /// reload path. Carried for v0.7+ — currently only `Clean` and
        /// `MergePending` paths produce `SaveDiff` outcomes.
        disk_now: Vec<u8>,
    },
    /// v0.7+ reachable via a "Discard all TUI edits" affordance from
    /// `MergePending`. Currently no producer in the dispatcher — kept
    /// to preserve the state-machine shape from spec §8.4 D.
    ConflictDiscardConfirm {
        disk_now: Vec<u8>,
    },
    /// v0.5.5: reconcile.rs walk failed; render error message inline
    /// in modal. Spec §13.2 B2 + I13. Esc dismisses; commit refuses
    /// while in this state.
    ReconcileError {
        message: String,
    },
}

/// User pick for a single conflicting key — drives the per-row marker
/// in the conflict-list UI and the source selection in
/// `apply_conflict_selections`. `Skip` keeps the base value (auto-merged
/// representation) untouched; `Ours` and `Theirs` overwrite the merged
/// document via [`crate::config_tui::merge::write_to_path`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictChoice {
    Ours,
    Theirs,
    Skip,
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
        // MergePending is rendered by Modal::ConflictList (events.rs
        // routes there); the SaveDiff modal itself does not draw in
        // that state. If we somehow reach here, show a stub.
        Some(SaveDiffState::MergePending { conflicts, .. }) => (
            "Save — CONFLICT (use conflict list modal)".to_owned(),
            format!(
                "{} conflict(s) pending — press Enter on the conflict-list modal to apply.",
                conflicts.len()
            ),
        ),
        Some(SaveDiffState::ConflictDiscardConfirm { .. }) => (
            "Discard TUI edits and reload disk? [y/N]".to_owned(),
            "(destructive — default = N)".to_owned(),
        ),
        None => ("Save".to_owned(), "(no save state)".to_owned()),
        Some(SaveDiffState::ReconcileError { message }) => {
            let block =
                Block::default().borders(Borders::ALL).title("Reconcile error — fix and retry");
            frame.render_widget(
                Paragraph::new(message.as_str())
                    .style(Style::default().fg(Color::Red))
                    .wrap(Wrap { trim: false })
                    .block(block),
                area,
            );
            return;
        }
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    frame.render_widget(Paragraph::new(body).scroll((app.save_diff_scroll, 0)).block(block), area);
}

/// `PageUp` / `PageDown` step size (rows). Constant rather than a cached
/// inner-area height (v0.6.1 §3.7 lean simplification): `Paragraph::scroll`
/// clamps internally so over-scroll past EOF is harmless.
const PAGE_STEP: u16 = 10;

/// Key dispatch — returns the next state transition.
pub(crate) fn dispatch_key(app: &mut App, k: KeyEvent) -> SaveDiffOutcome {
    // Scroll keys (v0.6.1 §3.7). These do not consume the saved
    // `app.save_diff` state — modal stays open and only the scroll
    // offset is mutated. `u16::MAX` for End is safe because
    // `Paragraph::scroll` clamps to the document length.
    match k.code {
        KeyCode::Up => {
            app.save_diff_scroll = app.save_diff_scroll.saturating_sub(1);
            return SaveDiffOutcome::StayOpen;
        }
        KeyCode::Down => {
            app.save_diff_scroll = app.save_diff_scroll.saturating_add(1);
            return SaveDiffOutcome::StayOpen;
        }
        KeyCode::PageUp => {
            app.save_diff_scroll = app.save_diff_scroll.saturating_sub(PAGE_STEP);
            return SaveDiffOutcome::StayOpen;
        }
        KeyCode::PageDown => {
            app.save_diff_scroll = app.save_diff_scroll.saturating_add(PAGE_STEP);
            return SaveDiffOutcome::StayOpen;
        }
        KeyCode::Home => {
            app.save_diff_scroll = 0;
            return SaveDiffOutcome::StayOpen;
        }
        KeyCode::End => {
            app.save_diff_scroll = u16::MAX;
            return SaveDiffOutcome::StayOpen;
        }
        _ => {}
    }
    let Some(state) = app.save_diff.take() else {
        return SaveDiffOutcome::CloseModal;
    };
    match (state, k.code) {
        (SaveDiffState::Clean { .. }, KeyCode::Char('y')) => SaveDiffOutcome::Commit,
        // G8: MergePending dispatch lives in `handle_conflict_list_key`
        // (events.rs) because the conflict-list modal owns its own
        // keymap. The SaveDiff modal sees no keys while we are in the
        // MergePending state — preserve and stay open.
        (state @ SaveDiffState::MergePending { .. }, _) => {
            app.save_diff = Some(state);
            SaveDiffOutcome::StayOpen
        }
        (SaveDiffState::ConflictDiscardConfirm { disk_now }, KeyCode::Char('y')) => {
            SaveDiffOutcome::DiscardAndReload(disk_now)
        }
        (_, KeyCode::Char('n') | KeyCode::Esc) => SaveDiffOutcome::CloseModal,
        (state, _) => {
            app.save_diff = Some(state);
            SaveDiffOutcome::StayOpen
        }
    }
}

/// Build the initial `SaveDiffState` from snapshot + edits — triggered by Ctrl+S.
///
/// G8 (§3.5 + §3.6): when the disk file diverged from the snapshot, run
/// the 3-way merge and produce [`SaveDiffState::MergePending`] (which
/// in turn opens `Modal::ConflictList`) instead of the pre-G8
/// `ConflictPending` two-pane diff. Reconcile / disk-parse failures
/// short-circuit to `ReconcileError`.
pub(crate) fn build_initial_state(app: &App) -> SaveDiffState {
    let Some(cfg_path) = app.snapshot.source_path.as_deref() else {
        return SaveDiffState::Clean {
            tui_diff: "(first-run save — creating new config file)".to_owned(),
        };
    };
    let disk_now = std::fs::read(cfg_path).unwrap_or_default();
    let disk_hash = crate::config_tui::snapshot::sha256(&disk_now);

    let ours_str = match crate::config_tui::save::build_new_content(&app.snapshot, &app.edits) {
        Ok(s) => s,
        Err(e) => return SaveDiffState::ReconcileError { message: format!("{e}") },
    };
    let tui_diff = build_diff(&app.snapshot.raw_bytes, ours_str.as_bytes());

    if disk_hash == app.snapshot.source_hash {
        return SaveDiffState::Clean { tui_diff };
    }

    // Disk diverged from the snapshot — run a structural 3-way merge.
    let base = app.snapshot.doc.clone();
    let ours = match std::str::FromStr::from_str(&ours_str) {
        Ok(d) => d,
        Err(e) => {
            return SaveDiffState::ReconcileError {
                message: format!("reparse of pending edits failed: {e}"),
            };
        }
    };
    let disk_str = String::from_utf8_lossy(&disk_now).to_string();
    let theirs = match std::str::FromStr::from_str(&disk_str) {
        Ok(d) => d,
        Err(e) => {
            return SaveDiffState::ReconcileError {
                message: format!("disk file no longer parses ({e}); fix and retry"),
            };
        }
    };

    let merge = crate::config_tui::merge::merge_three_way(&base, &ours, &theirs);

    if merge.conflicts.is_empty() {
        // Auto-merge resolved everything — single-pane Clean diff.
        return SaveDiffState::Clean { tui_diff };
    }

    let selection: Vec<ConflictChoice> = merge
        .conflicts
        .iter()
        .map(|c| match c.shape {
            crate::config_tui::merge::ConflictValueShape::Leaf => ConflictChoice::Ours,
            crate::config_tui::merge::ConflictValueShape::Block => ConflictChoice::Skip,
        })
        .collect();

    SaveDiffState::MergePending {
        base,
        ours,
        theirs,
        auto_merged: merge.auto_merged,
        conflicts: merge.conflicts,
        selection,
        focused_row: 0,
        disk_now,
    }
}

/// Tiny line-based diff (-/+ prefix). v0.5.4 inline impl — full
/// patience/Myers diff is overkill for the modal's display purpose.
///
/// Known limitation: the `HashSet` collapses duplicate lines, so removing
/// one copy of a repeated line (`a\na\nb\n` → `a\nb\n`) shows as "(no
/// changes)". v0.7+ may upgrade if the display becomes confusing.
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
