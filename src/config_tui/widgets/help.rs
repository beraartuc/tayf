//! `Modal::Help` overlay render (spec §12.4 D4).
//!
//! Renders the canonical [`HELP_MODAL_CONTENT`] keybinding cheat-sheet
//! inside the modal's centered rect. The content string lives in
//! `events.rs` next to the dispatch arms so cross-references stay
//! co-located.
//!
//! [`HELP_MODAL_CONTENT`]: crate::config_tui::events::HELP_MODAL_CONTENT

use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::config_tui::events::HELP_MODAL_CONTENT;

/// Draw the Help overlay: clear the area, frame it, and write the
/// keybinding list. Any key dismisses; see `events::handle_help_key`.
pub(crate) fn render(frame: &mut Frame, area: Rect) {
    frame.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).title("Help (press any key to dismiss)");
    let body = Paragraph::new(HELP_MODAL_CONTENT).block(block);
    frame.render_widget(body, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_tui::app::{App, Modal};
    use crate::config_tui::test_support::assert_render_snapshot;

    #[test]
    fn render_modal_help_matches_snapshot() {
        let mut app = App::default_for_test();
        app.modal = Some(Modal::Help);
        assert_render_snapshot(80, 24, &app, |frame, area, _app| render(frame, area), "modal_help");
    }
}
