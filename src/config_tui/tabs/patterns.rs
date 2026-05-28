//! Patterns tab — built-in + user list, detail/edit. v0.5.4 C3.
//!
//! Vim navigation (§12.2). `o` override built-in into user-config;
//! `d` delete user-config rule (confirm modal); `r` reset user
//! override (confirm modal); `n` opens the 3-phase `Modal::NewPattern`
//! wizard (v0.6 D2).

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::config_tui::app::{App, ConfirmAction, Modal};

pub(crate) fn render(frame: &mut Frame, area: Rect, app: &App) {
    let chunks =
        Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).split(area);
    render_list(frame, chunks[0], app);
    render_detail(frame, chunks[1], app);
}

fn render_list(frame: &mut Frame, area: Rect, app: &App) {
    let filter = app.search_filter.as_deref().unwrap_or("");
    let filtered = crate::config_tui::search::filter_names_lowercase(
        app.catalog.builtin_rule_names.iter().copied(),
        filter,
    );
    let items: Vec<ListItem> =
        filtered.iter().map(|name| ListItem::new(format!("  {name}"))).collect();
    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(app.focus.patterns.selected_idx.min(items.len() - 1)));
    }
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Patterns"))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_detail(frame: &mut Frame, area: Rect, app: &App) {
    let filter = app.search_filter.as_deref().unwrap_or("");
    let filtered = crate::config_tui::search::filter_names_lowercase(
        app.catalog.builtin_rule_names.iter().copied(),
        filter,
    );
    let selected = filtered.get(app.focus.patterns.selected_idx).copied().unwrap_or("");
    let body = if selected.is_empty() {
        "(no pattern selected)".to_owned()
    } else {
        let builtin = crate::rules::builtin_rules().into_iter().find(|r| r.name == selected);
        match builtin {
            Some(r) => format!(
                "Pattern: {}\n\nSource: built-in\nRegex: {}\nStyle: (default — Edit with 'e' or 'c' for color picker)\n\n\
                 Press 'o' to override (copy into user-config so you can edit)\n\
                 Press 'e' to edit the regex source (inline editor)\n\
                 Press 'c' to open color picker (C4)",
                r.name, r.pattern,
            ),
            None => "(detail not found)".to_owned(),
        }
    };
    frame.render_widget(
        Paragraph::new(body).block(Block::default().borders(Borders::ALL).title("Detail")),
        area,
    );
}

pub(crate) fn dispatch_key(app: &mut App, k: KeyEvent) {
    let filter = app.search_filter.as_deref().unwrap_or("");
    let filtered = crate::config_tui::search::filter_names_lowercase(
        app.catalog.builtin_rule_names.iter().copied(),
        filter,
    );
    let len = filtered.len();
    match k.code {
        KeyCode::Char('j') | KeyCode::Down => {
            app.focus.patterns.selected_idx =
                (app.focus.patterns.selected_idx + 1).min(len.saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.focus.patterns.selected_idx = app.focus.patterns.selected_idx.saturating_sub(1);
        }
        KeyCode::Char('g') => app.focus.patterns.selected_idx = 0,
        KeyCode::Char('G') => app.focus.patterns.selected_idx = len.saturating_sub(1),
        KeyCode::Char('h') => app.focus.patterns.detail_focused = false,
        KeyCode::Char('l') | KeyCode::Enter => app.focus.patterns.detail_focused = true,
        KeyCode::Char(' ') => {
            app.toast = Some(crate::config_tui::app::Toast::ok(
                "(activate semantic n/a for patterns — use 'c' to edit style)",
            ));
        }
        KeyCode::Char('o') => {
            if let Some(name) = filtered.get(app.focus.patterns.selected_idx) {
                app.edits.rules.insert(
                    crate::config_tui::edit::RuleId::UserConfig((*name).to_owned()),
                    crate::config_tui::edit::RuleEdit::default(),
                );
                app.toast = Some(crate::config_tui::app::Toast::ok(format!(
                    "staged override of '{name}' — edit then Ctrl+S to save"
                )));
            }
        }
        KeyCode::Char('d') | KeyCode::Delete => {
            // Patterns tab currently renders builtins-only; every entry in
            // `filtered` is a &'static str from BUILTIN_NAMES, so
            // RuleId::Builtin is the correct payload. v0.6.2 §3.3.
            if let Some(name) = filtered.get(app.focus.patterns.selected_idx) {
                let rule_id = crate::config_tui::edit::RuleId::Builtin(name);
                app.modal = Some(Modal::Confirm {
                    msg: format!("Delete rule '{name}'? (staged for removal on save)"),
                    action: ConfirmAction::DeleteRule(rule_id),
                });
            }
        }
        KeyCode::Char('r') => {
            // Symmetric with 'd': reset all staged overrides for the
            // selected builtin. v0.6.2 §3.3.
            if let Some(name) = filtered.get(app.focus.patterns.selected_idx) {
                let rule_id = crate::config_tui::edit::RuleId::Builtin(name);
                app.modal = Some(Modal::Confirm {
                    msg: format!("Reset all staged overrides of '{name}'?"),
                    action: ConfirmAction::ResetOverride(rule_id),
                });
            }
        }
        KeyCode::Char('n') if app.modal.is_none() => {
            app.modal = Some(crate::config_tui::app::Modal::NewPattern {
                phase: crate::config_tui::app::NewPatternPhase::Name,
                draft: crate::config_tui::app::PatternDraft::new(),
            });
        }
        KeyCode::Char('c') if app.modal.is_none() => {
            app.modal = Some(Modal::ColorPicker(
                crate::config_tui::widgets::color_picker::ColorPickerState::default(),
            ));
        }
        KeyCode::Char('e') if app.modal.is_none() => {
            if let Some(rule_id) = crate::config_tui::events::resolve_selected_rule_id(app) {
                let current_pattern = crate::config_tui::events::pattern_for_rule_id(&rule_id, app);
                app.modal = Some(crate::config_tui::app::Modal::EditRegex {
                    rule_id,
                    buffer: current_pattern,
                    error: None,
                });
            }
        }
        _ => {}
    }
}
