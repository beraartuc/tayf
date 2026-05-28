//! `Modal::NewPattern` 3-phase wizard render (spec §12.4 D2).
//!
//! Phases share a single modal frame with two header rows (name, regex)
//! plus a body region that shows either the live pattern-syntax error
//! banner or the embedded `ColorPicker` (style phase).

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::config_tui::app::{NewPatternPhase, PatternDraft};

/// Render the 3-phase new-pattern modal into `area`.
pub(crate) fn render(frame: &mut Frame, area: Rect, phase: &NewPatternPhase, draft: &PatternDraft) {
    frame.render_widget(Clear, area);
    let title = match phase {
        NewPatternPhase::Name => "New pattern — name (Enter advances, Esc cancels)",
        NewPatternPhase::Regex => "New pattern — regex (Enter advances, Esc back)",
        NewPatternPhase::Style => "New pattern — pick color (Enter accepts, Esc back)",
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(2), Constraint::Min(0)])
        .split(inner);

    let name_active = matches!(phase, NewPatternPhase::Name);
    let mut name_spans: Vec<Span> = vec![
        Span::raw("Name: "),
        Span::styled(
            draft.name.as_str(),
            if name_active { Style::default().fg(Color::Yellow) } else { Style::default() },
        ),
    ];
    if name_active {
        name_spans.push(Span::raw("\u{2588}"));
    }
    frame.render_widget(Paragraph::new(Line::from(name_spans)), chunks[0]);

    let regex_active = matches!(phase, NewPatternPhase::Regex);
    let mut regex_spans: Vec<Span> = vec![
        Span::raw("Regex: "),
        Span::styled(
            draft.pattern.as_str(),
            if regex_active { Style::default().fg(Color::Yellow) } else { Style::default() },
        ),
    ];
    if regex_active {
        regex_spans.push(Span::raw("\u{2588}"));
    }
    frame.render_widget(Paragraph::new(Line::from(regex_spans)), chunks[1]);

    match phase {
        NewPatternPhase::Style => {
            crate::config_tui::widgets::color_picker::render(frame, chunks[2], &draft.picker_state);
        }
        NewPatternPhase::Name | NewPatternPhase::Regex => {
            if let Some(err) = &draft.pattern_error {
                let err_line = Line::from(vec![Span::styled(
                    format!("\u{26A0} {err}"),
                    Style::default().fg(Color::Red),
                )]);
                frame.render_widget(Paragraph::new(err_line), chunks[2]);
            }
        }
    }
}
