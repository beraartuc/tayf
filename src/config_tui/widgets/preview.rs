//! Mini-preview strip + full-preview overlay.
//!
//! Spec §9.5: mini-preview applies `app.preview.compiled` to
//! `app.sample_input.text` and renders the colorized result. Full
//! preview (`Shift+P`) renders the same in a full-screen modal overlay.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::config_tui::app::App;
use crate::config_tui::style_ratatui::to_ratatui;
use crate::pipeline::StyleSpan;

/// Render the 5-row mini-preview strip.
pub(crate) fn render_mini(frame: &mut Frame, area: Rect, app: &App) {
    let header = "─── live preview ─── [s] sample [P] hide [Shift+P] full ──";
    let block = Block::default().borders(Borders::TOP).title(header);
    let body = colorize_sample(app);
    frame.render_widget(Paragraph::new(body).block(block), area);
}

/// Render the full-preview modal overlay.
pub(crate) fn render_full_overlay(frame: &mut Frame, area: Rect, app: &App) {
    frame.render_widget(Clear, area);
    let body = colorize_sample(app);
    let block = Block::default().borders(Borders::ALL).title("Full preview — Esc to close");
    frame.render_widget(Paragraph::new(body).block(block), area);
}

fn colorize_sample(app: &App) -> Vec<Line<'_>> {
    let mut lines: Vec<Line> = Vec::new();
    if let Some(err) = &app.preview.compile_error {
        lines.push(Line::from(Span::raw(format!("⚠ {err}"))));
        lines.push(Line::from(""));
    }
    for (line_text, spans) in app.sample_input.text.lines().zip(&app.preview.runs) {
        lines.push(spans_to_line(line_text, spans));
    }
    lines
}

fn spans_to_line<'a>(line: &'a str, spans: &[StyleSpan]) -> Line<'a> {
    let mut out: Vec<Span<'a>> = Vec::new();
    let mut cursor = 0usize;
    for s in spans {
        debug_assert!(line.is_char_boundary(s.start), "span start not on char boundary");
        debug_assert!(line.is_char_boundary(s.end), "span end not on char boundary");
        if cursor < s.start {
            out.push(Span::raw(&line[cursor..s.start]));
        }
        out.push(Span::styled(&line[s.start..s.end], to_ratatui(s.style)));
        cursor = s.end;
    }
    if cursor < line.len() {
        out.push(Span::raw(&line[cursor..]));
    }
    Line::from(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{Color, Style};

    fn mk_span(start: usize, end: usize, color: Color) -> StyleSpan {
        StyleSpan { start, end, style: Style { fg: Some(color), ..Default::default() } }
    }

    #[test]
    fn spans_to_line_cursor_walk_emits_gap_span_styled_span_tail() {
        let line = "abc DEF ghi";
        let spans = vec![mk_span(4, 7, Color::Red)];
        let result = spans_to_line(line, &spans);
        assert_eq!(result.spans.len(), 3, "gap + styled + tail");
        assert_eq!(result.spans[0].content, "abc ");
        assert_eq!(result.spans[1].content, "DEF");
        assert_eq!(result.spans[2].content, " ghi");
    }

    #[test]
    fn spans_to_line_no_spans_yields_single_raw_span() {
        let line = "hello world";
        let spans: Vec<StyleSpan> = Vec::new();
        let result = spans_to_line(line, &spans);
        assert_eq!(result.spans.len(), 1);
        assert_eq!(result.spans[0].content, "hello world");
    }

    #[test]
    fn spans_to_line_contiguous_spans_no_gap_emit_two_styled_no_raw() {
        let line = "abcdef";
        let spans = vec![mk_span(0, 3, Color::Red), mk_span(3, 6, Color::Blue)];
        let result = spans_to_line(line, &spans);
        assert_eq!(result.spans.len(), 2, "two contiguous styled, no gap, no tail");
        assert_eq!(result.spans[0].content, "abc");
        assert_eq!(result.spans[1].content, "def");
    }

    #[test]
    fn colorize_sample_handles_multi_line_sample_strips_trailing_newline() {
        let sample = "first\nsecond\n";
        let lines: Vec<&str> = sample.lines().collect();
        assert_eq!(lines.len(), 2, "two non-empty lines, trailing \\n elided");
    }
}
