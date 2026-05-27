//! Mini-preview strip + full-preview overlay.
//!
//! Spec §9.5: mini-preview applies `app.preview.compiled` to
//! `app.sample_input.text` and renders the colorized result. Full
//! preview (`Shift+P`) renders the same in a full-screen modal overlay.
//!
//! v0.5.4 simplification (spec §5.4 DOKUNULMAZ src/pipeline.rs): the
//! existing `apply_rules` byte-emit path cannot be reused here without
//! plumbing a span-emitting variant. So v0.5.4 ships the raw sample
//! text + compile-error banner; true colorized preview lands in v0.6+.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::config_tui::app::App;

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

/// v0.5.4: sample lines verbatim + compile-error banner when applicable.
/// True colorized preview is deferred to v0.6+ (see module doc).
fn colorize_sample(app: &App) -> Vec<Line<'_>> {
    let mut lines: Vec<Line> = Vec::new();
    if let Some(err) = &app.preview.compile_error {
        lines.push(Line::from(Span::raw(format!("⚠ {err}"))));
        lines.push(Line::from(""));
    }
    for raw_line in app.sample_input.text.lines() {
        lines.push(Line::from(raw_line.to_owned()));
    }
    lines
}
