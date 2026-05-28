//! Profiles tab — embedded + disk list, active marker, Space-to-activate.
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
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Profiles (embedded)"))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
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
    frame.render_widget(
        Paragraph::new(body).block(Block::default().borders(Borders::ALL).title("Detail")),
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
            let Some(name) = filtered.get(app.focus.profiles.selected_idx).copied() else {
                return;
            };

            // Source check: only embedded profiles need the copy. A disk
            // profile is already editable in place.
            if !crate::profiles::embedded_profile_names().any(|n| n == name) {
                app.toast = Some(crate::config_tui::app::Toast::warn(format!(
                    "Already a disk profile — edit ~/.config/tayf/profiles/{name}.toml"
                )));
                return;
            }

            let Some(tayf_root) = crate::config_tui::save::tayf_config_root() else {
                app.toast = Some(crate::config_tui::app::Toast::warn(
                    "Override failed: cannot resolve ~/.config/tayf/".to_owned(),
                ));
                return;
            };
            let dest = crate::profiles::disk_path_with_root(&tayf_root, name);

            if let Err(reason) =
                crate::config_tui::save::check_safe_write_destination(&dest, &tayf_root)
            {
                app.toast = Some(crate::config_tui::app::Toast::warn(format!(
                    "Override refused: {reason}"
                )));
                return;
            }

            if dest.exists() {
                app.toast = Some(crate::config_tui::app::Toast::warn(format!(
                    "Already on disk — edit ~/.config/tayf/profiles/{name}.toml"
                )));
                return;
            }

            let Some(src) = crate::profiles::embedded_source(name) else {
                // Unreachable: embedded_profile_names() membership was
                // just verified above. Returning silently is correct.
                return;
            };

            if let Err(e) = crate::config_tui::save::write_atomic_to(&dest, src) {
                app.toast =
                    Some(crate::config_tui::app::Toast::warn(format!("Override failed: {e}")));
                return;
            }

            crate::config_tui::events::request_snapshot_reload(app);
            app.toast = Some(crate::config_tui::app::Toast::ok(format!(
                "Copied '{name}' to disk; now editable"
            )));
        }
        _ => {}
    }
}
