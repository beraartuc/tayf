//! Modal render dispatcher.
//!
//! `render_modal` is called once per frame after the main pane + status
//! bar — overlays the active `Modal` variant onto a centered rect.

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::config_tui::app::{App, Modal};

pub(crate) mod color_picker;
pub(crate) mod edit_regex;
pub(crate) mod help;
pub(crate) mod new_pattern;
pub(crate) mod preview;
pub(crate) mod sample_set;
pub(crate) mod save_diff;
pub(crate) mod search;

/// Render the active modal as an overlay over the centered area.
pub(crate) fn render_modal(frame: &mut Frame, full: Rect, app: &App) {
    let Some(modal) = &app.modal else {
        return;
    };
    let area = centered_rect(80, 24, full);
    match modal {
        Modal::ColorPicker(state) => color_picker::render(frame, area, state),
        Modal::FullPreview => preview::render_full_overlay(frame, full, app),
        Modal::SaveDiff => save_diff::render(frame, area, app),
        Modal::QuitWithUnsavedEdits => render_quit_confirm(frame, area),
        Modal::Confirm { msg, .. } => render_confirm(frame, area, msg),
        Modal::Error(msg) => render_error(frame, area, msg),
        Modal::Search => {
            if let Some(state) = app.search_state.as_ref() {
                search::render(frame, area, state);
            }
        }
        Modal::SampleSet => {
            if let Some(state) = app.sample_set_state.as_ref() {
                sample_set::render(frame, area, state);
            }
        }
        Modal::NewPattern { phase, draft } => {
            new_pattern::render(frame, area, phase, draft);
        }
        Modal::EditRegex { rule_id, buffer, error } => {
            edit_regex::render(frame, area, rule_id, buffer.as_str(), error.as_deref());
        }
        Modal::Help => help::render(frame, area),
    }
}

fn centered_rect(width_pct: u16, height_pct: u16, area: Rect) -> Rect {
    use ratatui::layout::{Constraint, Layout};
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - height_pct) / 2),
        Constraint::Percentage(height_pct),
        Constraint::Percentage((100 - height_pct) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - width_pct) / 2),
        Constraint::Percentage(width_pct),
        Constraint::Percentage((100 - width_pct) / 2),
    ])
    .split(vertical[1])[1]
}

fn render_quit_confirm(frame: &mut Frame, area: Rect) {
    use ratatui::widgets::{Block, Borders, Clear, Paragraph};
    frame.render_widget(Clear, area);
    let body = "You have unsaved changes.\n\n  [n / Esc / Enter]  Cancel (return to editor)\n  [s]                Save and quit\n  [d]                Discard and quit";
    let block = Block::default().borders(Borders::ALL).title("Quit");
    frame.render_widget(Paragraph::new(body).block(block), area);
}

fn render_confirm(frame: &mut Frame, area: Rect, msg: &str) {
    use ratatui::widgets::{Block, Borders, Clear, Paragraph};
    frame.render_widget(Clear, area);
    let body = format!("{msg}\n\n[y] Yes    [n / Esc] No (default)");
    let block = Block::default().borders(Borders::ALL).title("Confirm");
    frame.render_widget(Paragraph::new(body).block(block), area);
}

fn render_error(frame: &mut Frame, area: Rect, msg: &str) {
    use ratatui::widgets::{Block, Borders, Clear, Paragraph};
    frame.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).title("Error — Esc to dismiss");
    frame.render_widget(Paragraph::new(msg.to_owned()).block(block), area);
}
