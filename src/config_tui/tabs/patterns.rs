//! Patterns tab — built-in + user list, detail/edit. v0.5.4 C2c stub;
//! C3 ships real list rendering, override (`o`), delete (`d`), reset (`r`),
//! new (`n`), color picker (`c`), edit (`e`), and Vim hjkl navigation.

use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::config_tui::app::App;

pub(crate) fn render(frame: &mut Frame, area: Rect, app: &App) {
    let n = app.catalog.builtin_rule_names.len();
    let body = format!("Patterns ({n} built-in)\n\n(C3 wires list + detail + edit)");
    let block = Block::default().borders(Borders::ALL).title("Patterns");
    frame.render_widget(Paragraph::new(body).block(block), area);
}

pub(crate) fn dispatch_key(_app: &mut App, _k: KeyEvent) {
    // C3 wires j/k/↑/↓ navigation + Enter/Space + o/d/r/n/e/c.
}
