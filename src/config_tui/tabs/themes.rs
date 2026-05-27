//! Themes tab — built-in + disk list, active marker, Space-to-activate.
//! Uniform Enter semantic (spec §12.3): Enter = focus detail; Space = activate.

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::config_tui::app::App;

pub(crate) fn render(frame: &mut Frame, area: Rect, app: &App) {
    let chunks =
        Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).split(area);
    render_list(frame, chunks[0], app);
    render_detail(frame, chunks[1], app);
}

fn render_list(frame: &mut Frame, area: Rect, app: &App) {
    let active = app
        .edits
        .general
        .theme
        .as_ref()
        .and_then(|x| x.as_deref())
        .or(app.snapshot.parsed.theme.as_deref())
        .unwrap_or("");
    let items: Vec<ListItem> = app
        .catalog
        .builtin_theme_names
        .iter()
        .map(|name| {
            let marker = if *name == active { "● " } else { "  " };
            ListItem::new(format!("{marker}{name}"))
        })
        .collect();
    let mut state = ListState::default();
    state.select(Some(app.focus.themes.selected_idx.min(items.len().saturating_sub(1))));
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Themes (built-in)"))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_detail(frame: &mut Frame, area: Rect, app: &App) {
    let selected =
        app.catalog.builtin_theme_names.get(app.focus.themes.selected_idx).copied().unwrap_or("");
    let body = if selected.is_empty() {
        "(no theme selected)".to_owned()
    } else {
        format!("Theme: {selected}\n\nSource: built-in\n\nPress Space to set as active\nPress 'o' to override (copy to ~/.config/tayf/themes/{selected}.toml)")
    };
    frame.render_widget(
        Paragraph::new(body).block(Block::default().borders(Borders::ALL).title("Detail")),
        area,
    );
}

pub(crate) fn dispatch_key(app: &mut App, k: KeyEvent) {
    let len = app.catalog.builtin_theme_names.len();
    match k.code {
        KeyCode::Char('j') | KeyCode::Down => {
            app.focus.themes.selected_idx =
                (app.focus.themes.selected_idx + 1).min(len.saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.focus.themes.selected_idx = app.focus.themes.selected_idx.saturating_sub(1);
        }
        KeyCode::Char('g') => app.focus.themes.selected_idx = 0,
        KeyCode::Char('G') => app.focus.themes.selected_idx = len.saturating_sub(1),
        KeyCode::Enter => app.focus.themes.detail_focused = true,
        KeyCode::Char(' ') => {
            if let Some(name) = app.catalog.builtin_theme_names.get(app.focus.themes.selected_idx) {
                app.edits.general.theme = Some(Some((*name).to_owned()));
                app.toast = Some(crate::config_tui::app::Toast::ok(format!(
                    "staged theme = {name}; Ctrl+S to save"
                )));
            }
        }
        KeyCode::Char('o') => {
            app.toast = Some(crate::config_tui::app::Toast::warn(
                "theme override copy lands in v0.6+ (TUI new-disk-file out of v0.5.4 scope)",
            ));
        }
        _ => {}
    }
}
