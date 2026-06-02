//! Profiles tab — embedded + disk list, active marker, Space-to-activate.
//! Uniform Enter semantic (spec §12.3): Enter = focus detail; Space = activate.

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
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
        .profile
        .as_ref()
        .and_then(|x| x.as_deref())
        .or(app.snapshot.parsed.profile.as_deref())
        .unwrap_or("");
    let filter = app.search_filter.as_deref().unwrap_or("");
    let filtered = crate::config_tui::search::filter_names_lowercase(
        app.catalog.embedded_profile_names.iter().copied(),
        filter,
    );
    let items: Vec<ListItem> = filtered
        .iter()
        .map(|name| {
            let marker = if *name == active { "● " } else { "  " };
            ListItem::new(format!("{marker}{name}"))
        })
        .collect();
    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(app.focus.profiles.selected_idx.min(items.len() - 1)));
    }
    let accent = app.tui_env.accent;
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(accent.border())
                .title("Profiles (embedded)")
                .title_style(accent.header()),
        )
        .highlight_style(accent.selection());
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_detail(frame: &mut Frame, area: Rect, app: &App) {
    let filter = app.search_filter.as_deref().unwrap_or("");
    let filtered = crate::config_tui::search::filter_names_lowercase(
        app.catalog.embedded_profile_names.iter().copied(),
        filter,
    );
    let selected = filtered.get(app.focus.profiles.selected_idx).copied().unwrap_or("");
    let body = if selected.is_empty() {
        "(no profile selected)".to_owned()
    } else {
        format!(
            "Profile: {selected}\n\nSource: embedded\n\nPress Space to set as active\nPress 'o' to copy to disk for editing"
        )
    };
    let accent = app.tui_env.accent;
    frame.render_widget(
        Paragraph::new(body).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(accent.border())
                .title("Detail")
                .title_style(accent.header()),
        ),
        area,
    );
}

pub(crate) fn dispatch_key(app: &mut App, k: KeyEvent) {
    let filter = app.search_filter.as_deref().unwrap_or("");
    let filtered = crate::config_tui::search::filter_names_lowercase(
        app.catalog.embedded_profile_names.iter().copied(),
        filter,
    );
    let len = filtered.len();
    match k.code {
        KeyCode::Char('j') | KeyCode::Down => {
            app.focus.profiles.selected_idx =
                (app.focus.profiles.selected_idx + 1).min(len.saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.focus.profiles.selected_idx = app.focus.profiles.selected_idx.saturating_sub(1);
        }
        KeyCode::Char('g') => app.focus.profiles.selected_idx = 0,
        KeyCode::Char('G') => app.focus.profiles.selected_idx = len.saturating_sub(1),
        KeyCode::Enter => app.focus.profiles.detail_focused = true,
        KeyCode::Char(' ') => {
            if let Some(name) = filtered.get(app.focus.profiles.selected_idx) {
                app.edits.general.profile = Some(Some((*name).to_owned()));
                app.toast = Some(crate::config_tui::app::Toast::ok(format!(
                    "staged profile = {name}; Ctrl+S to save"
                )));
            }
        }
        KeyCode::Char('o') => {
            // The embedded profile library is retired (v0.12.0) — there is
            // nothing to copy to disk. Profile management (create/delete) is
            // reworked in the Profiles-tab rework; until then this is a no-op
            // with an explanatory toast.
            app.toast = Some(crate::config_tui::app::Toast::warn(
                "Embedded profiles are retired; the six domain rules are now built-in".to_owned(),
            ));
        }
        _ => {}
    }
}
