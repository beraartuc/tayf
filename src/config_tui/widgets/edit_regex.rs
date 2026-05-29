//! `Modal::EditRegex` inline regex source modal render (spec §12.4 D3).
//!
//! Single-line text buffer with a yellow cursor block. On invalid regex,
//! a red warning line is rendered immediately below the buffer (provided
//! the inner area is tall enough). Enter commits, Esc cancels — handled
//! by `events::handle_edit_regex_key`.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::config_tui::edit::RuleId;

/// Render the `EditRegex` modal into `area`.
pub(crate) fn render(
    frame: &mut Frame,
    area: Rect,
    rule_id: &RuleId,
    buffer: &str,
    error: Option<&str>,
) {
    frame.render_widget(Clear, area);
    let rule_name = match rule_id {
        RuleId::Builtin(n) => format!("(builtin) {n}"),
        RuleId::UserConfig(n) => format!("(user) {n}"),
        RuleId::Embedded { rule, profile } => format!("(profile {profile}) {rule}"),
        RuleId::DiskProfile { rule, profile } => format!("(profile {profile}) {rule}"),
    };
    let title = format!("Edit regex - {rule_name} (Enter commits, Esc cancels)");
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let buf_line = Line::from(vec![
        Span::raw(buffer.to_owned()),
        Span::styled("\u{2588}", Style::default().fg(Color::Yellow)),
    ]);
    frame.render_widget(Paragraph::new(buf_line), inner);

    if let Some(err) = error {
        if inner.height >= 2 {
            let err_area = Rect { x: inner.x, y: inner.y + 1, width: inner.width, height: 1 };
            let err_line =
                Line::from(vec![Span::styled(format!("! {err}"), Style::default().fg(Color::Red))]);
            frame.render_widget(Paragraph::new(err_line), err_area);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_tui::app::{App, Modal};
    use crate::config_tui::edit::RuleId;
    use crate::config_tui::test_support::assert_render_snapshot;

    #[test]
    fn render_modal_edit_log_level_matches_snapshot() {
        let mut app = App::default_for_test();
        let rule_id = RuleId::Builtin("log_level");
        let buffer = r"(?i)\b(ERROR|WARN|INFO|DEBUG)\b".to_owned();
        app.modal = Some(Modal::EditRegex {
            rule_id: rule_id.clone(),
            buffer: buffer.clone(),
            error: None,
        });
        assert_render_snapshot(
            80,
            24,
            &app,
            move |frame, area, _app| render(frame, area, &rule_id, &buffer, None),
            "modal_edit_log_level",
        );
    }

    #[test]
    fn render_modal_edit_regex_valid_matches_snapshot() {
        let mut app = App::default_for_test();
        let rule_id = RuleId::UserConfig("my_rule".to_owned());
        let buffer = "^(foo|bar)$".to_owned();
        app.modal = Some(Modal::EditRegex {
            rule_id: rule_id.clone(),
            buffer: buffer.clone(),
            error: None,
        });
        assert_render_snapshot(
            80,
            24,
            &app,
            move |frame, area, _app| render(frame, area, &rule_id, &buffer, None),
            "modal_edit_regex_valid",
        );
    }

    #[test]
    fn render_modal_edit_regex_error_matches_snapshot() {
        let mut app = App::default_for_test();
        let rule_id = RuleId::UserConfig("my_rule".to_owned());
        let buffer = "^[invalid".to_owned();
        let error = "unclosed character class".to_owned();
        app.modal = Some(Modal::EditRegex {
            rule_id: rule_id.clone(),
            buffer: buffer.clone(),
            error: Some(error.clone()),
        });
        assert_render_snapshot(
            80,
            24,
            &app,
            move |frame, area, _app| render(frame, area, &rule_id, &buffer, Some(&error)),
            "modal_edit_regex_error",
        );
    }
}
