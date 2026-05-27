//! Status tab — read-only resolved config + hot-reload event tail. v0.5.4 C2c stub.

use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::config_tui::app::App;

pub(crate) fn render(frame: &mut Frame, area: Rect, _app: &App) {
    let body = "Status\n\n(C3 wires resolved config view + reload.log tail)";
    let block = Block::default().borders(Borders::ALL).title("Status");
    frame.render_widget(Paragraph::new(body).block(block), area);
}

pub(crate) fn dispatch_key(_app: &mut App, _k: KeyEvent) {
    // C3 wires j/k/↑/↓ scroll only (read-only tab).
}
