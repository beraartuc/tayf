//! `Modal::NewProfile` render — the Profiles-tab `n` name prompt (spec §6.1).
//!
//! A single-field name prompt with a clone/empty toggle. Enter writes
//! `profiles/<name>.toml` (cloning the active rule set when `clone_rules`,
//! else an empty file); Tab toggles the clone flag; Esc cancels. In-TUI
//! editing of a profile's rules is deferred (spec §6.3).

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

/// Render the new-profile name prompt into `area`.
pub(crate) fn render(
    frame: &mut Frame,
    area: Rect,
    buffer: &str,
    clone_rules: bool,
    error: Option<&str>,
) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title("New profile — name (Enter creates, Tab toggles clone, Esc cancels)");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(2), Constraint::Min(0)])
        .split(inner);

    let name_spans: Vec<Span> = vec![
        Span::raw("Name: "),
        Span::styled(buffer, Style::default().fg(Color::Yellow)),
        Span::raw("\u{2588}"),
    ];
    frame.render_widget(Paragraph::new(Line::from(name_spans)), chunks[0]);

    let clone_label = if clone_rules {
        "Contents: clone current active rules  (Tab → empty)"
    } else {
        "Contents: empty profile  (Tab → clone current rules)"
    };
    frame.render_widget(Paragraph::new(Line::from(Span::raw(clone_label))), chunks[1]);

    if let Some(err) = error {
        let err_line = Line::from(vec![Span::styled(
            format!("\u{26A0} {err}"),
            Style::default().fg(Color::Red),
        )]);
        frame.render_widget(Paragraph::new(err_line), chunks[2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_tui::app::App;
    use crate::config_tui::test_support::assert_render_snapshot;

    #[test]
    fn render_modal_new_profile_clone_matches_snapshot() {
        let app = App::default_for_test();
        assert_render_snapshot(
            80,
            24,
            &app,
            move |frame, area, _app| render(frame, area, "work", true, None),
            "modal_new_profile_clone",
        );
    }
}
