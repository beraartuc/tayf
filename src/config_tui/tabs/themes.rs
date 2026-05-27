//! Themes tab — built-in + disk list, active marker. v0.5.4 C2c stub.

use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::config_tui::app::App;

pub(crate) fn render(frame: &mut Frame, area: Rect, app: &App) {
    let n = app.catalog.builtin_theme_names.len();
    let body =
        format!("Themes ({n} built-in)\n\n(C3 wires list + active marker + Space-to-activate)");
    let block = Block::default().borders(Borders::ALL).title("Themes");
    frame.render_widget(Paragraph::new(body).block(block), area);
}

pub(crate) fn dispatch_key(_app: &mut App, _k: KeyEvent) {
    // C3 wires j/k/↑/↓ + Space activate + o override.
}
