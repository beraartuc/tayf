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

// reason: MergePending carries four DocumentMut clones and is much
// larger than the other variants — Box would add an extra heap hop on
// every state transition for no clarity gain since the only large
// variant is also the one we mutate most.
#[allow(clippy::large_enum_variant)]
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
    }
}

const MAX_DP_CELLS: usize = 100_000;

/// Line-diff over `&[u8]` inputs, output is +/- prefixed text used by the
/// save-diff modal. Hunt-McIlroy LCS-DP algorithm. v0.7 spec §4.
///
/// Returns `"(no changes)\n"` literal when inputs are line-equal.
/// Falls back to a literal removal+addition list when the DP table would
/// exceed `MAX_DP_CELLS` cells — defensive bound for accidentally-pathological
/// config sizes.
pub(crate) fn build_diff(old: &[u8], new: &[u8]) -> String {
    let old_str = String::from_utf8_lossy(old);
    let new_str = String::from_utf8_lossy(new);
    let old_lines: Vec<&str> = old_str.lines().collect();
    let new_lines: Vec<&str> = new_str.lines().collect();
    let n = old_lines.len();
    let m = new_lines.len();

    if n.saturating_mul(m) > MAX_DP_CELLS {
        return cap_fallback(&old_lines, &new_lines);
    }

    let dp = build_dp_table(&old_lines, &new_lines);
    let ops = trace_back(&old_lines, &new_lines, &dp);
    render_ops(&ops)
}

#[derive(Debug)]
enum DiffOp<'src> {
    Same(&'src str),
    Add(&'src str),
    Remove(&'src str),
}

/// Flat row-major LCS table. Cells indexed as `cells[i * cols + j]`.
/// Single allocation, cache-friendly. `u16` per cell suffices: for any
/// `dp[i][j] <= min(i, j) <= floor(sqrt(n*m)) <= floor(sqrt(MAX_DP_CELLS))
/// = 316`, well within `u16::MAX`.
struct DpTable {
    cols: usize,
    cells: Vec<u16>,
}

impl DpTable {
    fn new(rows: usize, cols: usize) -> Self {
        Self { cols, cells: vec![0_u16; rows * cols] }
    }
    fn get(&self, i: usize, j: usize) -> u16 {
        self.cells[i * self.cols + j]
    }
    fn set(&mut self, i: usize, j: usize, v: u16) {
        self.cells[i * self.cols + j] = v;
    }
}

fn build_dp_table(old: &[&str], new: &[&str]) -> DpTable {
    let n = old.len();
    let m = new.len();
    let mut dp = DpTable::new(n + 1, m + 1);
    for i in 1..=n {
        for j in 1..=m {
            let v = if old[i - 1] == new[j - 1] {
                dp.get(i - 1, j - 1) + 1
            } else {
                std::cmp::max(dp.get(i - 1, j), dp.get(i, j - 1))
            };
            debug_assert!(v < u16::MAX, "LCS dp cell overflow");
            dp.set(i, j, v);
        }
    }
    dp
}

// reason: the `i == 0` boundary arm and the tied-dp non-matching final
// arm have identical bodies (Add(new[j-1]); j -= 1) but reach the body
// via DIFFERENT preconditions — the boundary guard must execute before
// any indexing into `dp` (which would underflow at i=0). Merging the
// guards yields an unreadable composite condition.
#[allow(clippy::if_same_then_else)]
fn trace_back<'src>(old: &[&'src str], new: &[&'src str], dp: &DpTable) -> Vec<DiffOp<'src>> {
    // Walk backwards from (n, m) emitting ops; reverse at the end so the
    // forward-reading order matches the canonical `diff -u` output:
    // Same-then-Remove-then-Add at modification points, and removals
    // preferred at the LATEST position when the LCS has multiple equally-
    // optimal traces (e.g. `"a\na\nb"` vs `"a\nb"` removes the SECOND `a`).
    //
    // The check order is significant — strictly-greater dp neighbours
    // win before the equality short-circuit so that backwards `Remove`
    // emits BEFORE the matching `Same`, yielding `Same/Remove` in forward
    // order. Tied dp with matching cell → Same. Tied dp with non-matching
    // cell → Add (convention, pinned by §4.5 #4 and #6).
    let mut ops = Vec::new();
    let mut i = old.len();
    let mut j = new.len();
    while i > 0 || j > 0 {
        if i == 0 {
            ops.push(DiffOp::Add(new[j - 1]));
            j -= 1;
        } else if j == 0 {
            ops.push(DiffOp::Remove(old[i - 1]));
            i -= 1;
        } else if dp.get(i - 1, j) > dp.get(i, j - 1) {
            ops.push(DiffOp::Remove(old[i - 1]));
            i -= 1;
        } else if dp.get(i, j - 1) > dp.get(i - 1, j) {
            ops.push(DiffOp::Add(new[j - 1]));
            j -= 1;
        } else if old[i - 1] == new[j - 1] {
            ops.push(DiffOp::Same(old[i - 1]));
            i -= 1;
            j -= 1;
        } else {
            ops.push(DiffOp::Add(new[j - 1]));
            j -= 1;
        }
    }
    ops.reverse();
    ops
}

fn render_ops(ops: &[DiffOp<'_>]) -> String {
    if ops.iter().all(|op| matches!(op, DiffOp::Same(_))) {
        return "(no changes)\n".to_owned();
    }
    let mut out = String::new();
    for op in ops {
        use std::fmt::Write;
        let _ = match op {
            DiffOp::Same(line) => writeln!(out, "  {line}"),
            DiffOp::Add(line) => writeln!(out, "+ {line}"),
            DiffOp::Remove(line) => writeln!(out, "- {line}"),
        };
    }
    out
}

fn cap_fallback(old_lines: &[&str], new_lines: &[&str]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "(diff too large for inline display: {} lines removed, {} lines added)",
        old_lines.len(),
        new_lines.len(),
    );
    for line in old_lines {
        let _ = writeln!(out, "- {line}");
    }
    for line in new_lines {
        let _ = writeln!(out, "+ {line}");
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
    fn build_diff_pure_addition_emits_plus_prefix() {
        let d = build_diff(b"a\nb\n", b"a\nb\nc\n");
        assert_eq!(d, "  a\n  b\n+ c\n");
    }

    #[test]
    fn build_diff_pure_removal_emits_minus_prefix() {
        let d = build_diff(b"a\nb\nc\n", b"a\nb\n");
        assert_eq!(d, "  a\n  b\n- c\n");
    }

    #[test]
    fn build_diff_empty_old_emits_only_additions() {
        let d = build_diff(b"", b"a\nb\n");
        assert_eq!(d, "+ a\n+ b\n");
    }

    #[test]
    fn build_diff_empty_new_emits_only_removals() {
        let d = build_diff(b"a\nb\n", b"");
        assert_eq!(d, "- a\n- b\n");
    }

    #[test]
    fn build_diff_modification_emits_minus_then_plus_exact() {
        // Tied dp + non-matching cell → Add (backward), so forward order
        // is Same/Remove/Add. Spec §4.5 #4.
        let d = build_diff(b"a\nb\n", b"a\nc\n");
        assert_eq!(d, "  a\n- b\n+ c\n");
    }

    #[test]
    fn build_diff_duplicate_line_removal_visible() {
        // Audit doc bug-fix: HashSet impl collapsed "a\na\nb\n" → "a\nb\n"
        // to "(no changes)". LCS-DP correctly shows the SECOND `a` removed
        // (strictly-greater dp neighbour rule keeps the late match). Spec §4.5 #5.
        let d = build_diff(b"a\na\nb\n", b"a\nb\n");
        assert_eq!(d, "  a\n- a\n  b\n");
    }

    #[test]
    fn build_diff_interleaved_changes_exact_with_remove_tie_convention() {
        // Mid-line replacement with shared anchor "b" in the middle.
        // Tied-dp Add convention pins forward order Remove-before-Add at
        // both modification points. Spec §4.5 #6.
        let d = build_diff(b"a\nb\nc\n", b"x\nb\ny\n");
        assert_eq!(d, "- a\n+ x\n  b\n- c\n+ y\n");
    }

    #[test]
    fn build_diff_cap_threshold_just_under_passes_lcs() {
        // 316 × 316 = 99 856 < 100 000 → LCS path active.
        let mut old = String::new();
        let mut new = String::new();
        for i in 0..316_u32 {
            use std::fmt::Write;
            let _ = writeln!(old, "line{i}");
            let _ = writeln!(new, "line{i}");
        }
        let d = build_diff(old.as_bytes(), new.as_bytes());
        assert_eq!(d, "(no changes)\n", "LCS path active under threshold");
    }

    #[test]
    fn build_diff_cap_fallback_at_oversize_input() {
        // 400 × 400 = 160 000 > 100 000 → cap_fallback fires.
        let mut old = String::new();
        let mut new = String::new();
        for i in 0..400_u32 {
            use std::fmt::Write;
            let _ = writeln!(old, "old{i}");
            let _ = writeln!(new, "new{i}");
        }
        let d = build_diff(old.as_bytes(), new.as_bytes());
        let first_line = d.lines().next().expect("cap fallback has banner line");
        assert_eq!(
            first_line,
            "(diff too large for inline display: 400 lines removed, 400 lines added)",
        );
    }

    #[test]
    fn build_diff_crlf_old_vs_lf_new_reports_no_false_diff_on_matching_bodies() {
        // str::lines() splits on both \n and \r\n. CRLF old vs LF new with
        // identical line bodies → "(no changes)".
        let d = build_diff(b"a\r\nb\r\n", b"a\nb\n");
        assert_eq!(d, "(no changes)\n");
    }

    #[test]
    fn build_diff_trailing_newline_normalization_is_no_change() {
        // str::lines() strips the trailing newline equally. "a\nb" and
        // "a\nb\n" both iterate ["a", "b"].
        let d = build_diff(b"a\nb", b"a\nb\n");
        assert_eq!(d, "(no changes)\n");
    }
}
