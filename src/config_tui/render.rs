//! Frame composition — Layout split per spec §7.3 + narrow-terminal
//! degradation gate §7.4.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::config_tui::app::{App, Tab};

/// Default sample input shown in the live-preview strip. Spec §9.3
/// (Unicode coverage + collision-avoidance N-7 fold, all-18-builtins coverage).
///
/// Token → built-in rule map:
/// - `timestamp`    — `[2026-05-26T…Z]`
/// - `log_level`    — `INFO` / `WARN` / `ERROR` / `DEBUG`
/// - `ipv4`         — `192.168.1.42`
/// - `ipv6`         — `2001:db8::1`
/// - `mac`          — `aa:bb:cc:dd:ee:ff`
/// - `url`          — `https://api.example.com`
/// - `duration`     — `12ms`
/// - `region`       — `us-east-1`          (default-off; present for preview when enabled)
/// - `instance_id`  — `i-0abcd1234567890ef`
/// - `email`        — `ops@example.com`
/// - `fqdn`         — `gateway.internal`
/// - `filename`     — `config.toml`
/// - `arn`          — `arn:aws:iam:::role/Admin`
/// - `pod_name`     — `nginx-deployment-7c79c4bf97-9hk6r`  (default-off)
/// - `image_tag`    — `gcr.io/app/api:1.21`                (default-off)
/// - `container_id` — `abc123def456`                       (default-off)
/// - `uuid`         — `550e8400-e29b-41d4-a716-446655440000`
/// - `permission`   — ` -rw-r--r-- ` (requires surrounding whitespace)
pub(crate) const DEFAULT_PREVIEW_SAMPLE: &str =
    "[2026-05-26T17:18:42Z] INFO 192.168.1.42 2001:db8::1 aa:bb:cc:dd:ee:ff GET https://api.example.com 12ms\n\
[2026-05-26T17:18:43Z] WARN us-east-1 i-0abcd1234567890ef ops@example.com via gateway.internal config.toml\n\
[2026-05-26T17:18:44Z] ERROR arn:aws:iam:::role/Admin nginx-deployment-7c79c4bf97-9hk6r gcr.io/app/api:1.21\n\
[2026-05-26T17:18:45Z] DEBUG abc123def456 550e8400-e29b-41d4-a716-446655440000 -rw-r--r-- ñame façade 完了\n";

/// Top-level draw. Dispatches narrow-term degradation gate first.
pub(crate) fn frame(frame: &mut Frame, app: &App) {
    let size = frame.area();
    if size.width < 60 || size.height < 16 {
        render_too_small(frame, size);
        return;
    }
    let preview_visible = app.mini_preview_visible && size.height >= 24;
    let preview_rows = if preview_visible { 5 } else { 0 };
    let chunks = Layout::vertical([
        Constraint::Length(1),            // tab strip
        Constraint::Min(10),              // main pane
        Constraint::Length(preview_rows), // mini-preview (collapsible)
        Constraint::Length(1),            // status bar
    ])
    .split(size);
    render_tab_strip(frame, chunks[0], app, size.width);
    render_main_pane(frame, chunks[1], app);
    if preview_visible {
        render_mini_preview_placeholder(frame, chunks[2], app);
    }
    render_status_bar(frame, chunks[3], app, size.width, preview_visible);
    crate::config_tui::widgets::render_modal(frame, size, app);
}

/// < 60 × 16 hard block (spec §7.4).
fn render_too_small(frame: &mut Frame, size: Rect) {
    let msg = format!("Resize to ≥60×16 (currently {}×{})", size.width, size.height);
    let p = Paragraph::new(msg).block(Block::default().borders(Borders::NONE));
    frame.render_widget(p, size);
}

/// 1-row tab strip. Three-letter labels at 60-79 col (spec §7.4 short-tab pin).
fn render_tab_strip(frame: &mut Frame, area: Rect, app: &App, width: u16) {
    let labels: &[(&str, Tab)] = if width < 80 {
        &[
            ("Pat", Tab::Patterns),
            ("Thm", Tab::Themes),
            ("Pro", Tab::Profiles),
            ("Sta", Tab::Status),
        ]
    } else {
        &[
            ("Patterns", Tab::Patterns),
            ("Themes", Tab::Themes),
            ("Profiles", Tab::Profiles),
            ("Status", Tab::Status),
        ]
    };
    let accent = app.tui_env.accent;
    let mut spans: Vec<Span> = Vec::new();
    for (i, (label, t)) in labels.iter().enumerate() {
        let style = if *t == app.tab { accent.tab_active() } else { accent.tab_inactive() };
        if i > 0 {
            spans.push(Span::raw(" │ "));
        }
        spans.push(Span::styled((*label).to_owned(), style));
    }
    let line = Line::from(spans);
    let p = Paragraph::new(line);
    frame.render_widget(p, area);
}

/// Main pane — routes to the per-tab renderer via the tabs facade.
fn render_main_pane(frame: &mut Frame, area: Rect, app: &App) {
    crate::config_tui::tabs::render(frame, area, app);
}

/// 5-row mini-preview — delegates to `widgets::preview::render_mini`.
fn render_mini_preview_placeholder(frame: &mut Frame, area: Rect, app: &App) {
    crate::config_tui::widgets::preview::render_mini(frame, area, app);
}

/// Status bar: bottom 1-row line with dirty marker, narrow-term hint, compile
/// error, and toast. Truncated to `width` with trailing `…` (UX 🔵 #10).
fn render_status_bar(frame: &mut Frame, area: Rect, app: &App, width: u16, preview_visible: bool) {
    let mut bits: Vec<String> = Vec::new();
    if app.edits.is_dirty() {
        bits.push("[unsaved]".to_owned());
    }
    if let Some(filter) = &app.search_filter {
        bits.push(format!("filter: \"{filter}\""));
    }
    // Narrow-term auto-hide marker (§7.4 + UX #1 fold).
    if !preview_visible && app.mini_preview_visible {
        bits.push("[preview hidden — press P to force show]".to_owned());
    }
    if let Some(err) = &app.preview.compile_error {
        // Priority marker — error always visible.
        bits.push(format!("⚠ pattern won't compile: {err}"));
    }
    if let Some(t) = &app.toast {
        let prefix = match t.kind {
            crate::config_tui::app::ToastKind::Ok => "",
            crate::config_tui::app::ToastKind::Warn => "⚠ ",
        };
        bits.push(format!("[{prefix}{}]", t.text));
    }
    let line = bits.join("  ");
    // Truncate for narrow terminal (UX 🔵 #10).
    let line_truncated = if line.len() > width as usize {
        let max_len = (width as usize).saturating_sub(1);
        let truncated: String = line.chars().take(max_len).collect();
        format!("{truncated}…")
    } else {
        line
    };
    let p = Paragraph::new(line_truncated);
    frame.render_widget(p, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_tab_cell_carries_brass_accent_bg() {
        use ratatui::backend::TestBackend;
        use ratatui::style::Color;
        use ratatui::Terminal;
        let app = App::default_for_test(); // Dark env
        let mut term = Terminal::new(TestBackend::new(80, 24)).expect("backend");
        term.draw(|f| frame(f, &app)).expect("draw");
        let buf = term.backend().buffer();
        // The active tab is "Patterns" at the start of row 0.
        let cell = &buf[(0, 0)];
        assert_eq!(cell.style().bg, Some(Color::Rgb(0xd8, 0xa6, 0x57)), "active tab brass bg");
    }

    #[test]
    fn navigation_does_not_leave_a_selection_ghost() {
        // Repro for the manual-review "trail" (issue 4): after moving the
        // selection, only ONE list row may carry the brass highlight bg. A
        // ghost shows as >1 row retaining it.
        use ratatui::backend::TestBackend;
        use ratatui::style::Color;
        use ratatui::Terminal;
        let brass = Color::Rgb(0xd8, 0xa6, 0x57);
        let mut app = App::default_for_test();
        let mut term = Terminal::new(TestBackend::new(124, 35)).expect("backend");
        app.focus.patterns.selected_idx = 8;
        term.draw(|f| frame(f, &app)).expect("draw 1");
        app.focus.patterns.selected_idx = 2;
        term.draw(|f| frame(f, &app)).expect("draw 2");
        let buf = term.backend().buffer();
        let mut rows_with_brass = std::collections::BTreeSet::new();
        for y in 2..33u16 {
            for x in 1..48u16 {
                if buf[(x, y)].style().bg == Some(brass) {
                    rows_with_brass.insert(y);
                }
            }
        }
        assert_eq!(
            rows_with_brass.len(),
            1,
            "exactly one selected row keeps the brass highlight; ghost = >1, rows={rows_with_brass:?}"
        );
    }

    #[test]
    fn default_preview_sample_has_4_lines_unicode_coverage() {
        // Spec §9.3 + 🔵 #9 fold — 4 lines, line 4 holds Unicode probes.
        let lines: Vec<&str> = DEFAULT_PREVIEW_SAMPLE.lines().collect();
        assert_eq!(lines.len(), 4, "sample must have 4 lines");
        assert!(
            lines[3].contains("ñame") && lines[3].contains("façade") && lines[3].contains("完了"),
            "line 4 must carry combining diacritics + CJK wide chars; got: {}",
            lines[3]
        );
    }

    #[test]
    fn default_preview_sample_avoids_v0_5_5_collision_n7() {
        // N-7 fold: `10.0.0.5:5432` shape is the v0.5.5 fqdn-vs-image_tag
        // collision example — must NOT appear in the default first-impression
        // preview. Power-users can paste it via `s` sample-set modal.
        assert!(
            !DEFAULT_PREVIEW_SAMPLE.contains("10.0.0.5:5432"),
            "default sample must not include the v0.5.5 collision shape"
        );
    }

    /// Strong oracle: every built-in rule's pattern must have at least one
    /// matching example token in `DEFAULT_PREVIEW_SAMPLE`. This guarantees
    /// full 18-of-18 coverage and will fail loudly if patterns change.
    #[test]
    fn default_preview_sample_covers_all_builtin_rules() {
        use regex::bytes::Regex;
        let rules = crate::rules::builtin_rules();
        let bytes = DEFAULT_PREVIEW_SAMPLE.as_bytes();
        let mut failures: Vec<String> = Vec::new();
        for rule in &rules {
            let re = Regex::new(&rule.pattern).unwrap_or_else(|e| {
                panic!("builtin '{}' pattern failed to compile: {e}", rule.name)
            });
            if !re.is_match(bytes) {
                failures.push(rule.name.clone());
            }
        }
        assert!(
            failures.is_empty(),
            "DEFAULT_PREVIEW_SAMPLE has no example for built-in rule(s): {failures:?}\n\
             Add a matching token to the sample string."
        );
    }
}
