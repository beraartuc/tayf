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

use crate::config::UserRule;
use crate::config_tui::app::{App, ConfirmAction, Modal};

/// Render-row layout for the Patterns tab list (union of built-in and
/// user-config rules under section headers).
///
/// Render rows = `[Builtin header, builtin_1, ..., builtin_N, User header,
/// user_1, ..., user_M]`. Section header rows are non-selectable.
///
/// Selectable indices map to `[0..builtin_count)` (built-ins) followed by
/// `[builtin_count..builtin_count + user_count)` (user rules).
///
/// Built by [`patterns_list_layout`]; navigated via
/// [`selectable_to_render_idx`].
pub(crate) struct PatternsListLayout {
    pub(crate) builtin_count: usize,
    pub(crate) builtin_names: Vec<&'static str>,
    pub(crate) user_count: usize,
    pub(crate) user_names: Vec<String>,
    pub(crate) builtin_header_render_idx: usize,
    pub(crate) user_header_render_idx: usize,
}

/// Build the render-row layout from the active catalog + user-config rules.
///
/// `builtin_names` is `app.catalog.builtin_rule_names`; `user_rules` is
/// `app.snapshot.parsed.rules`. `filter` is the active search filter
/// (empty = no filter), applied case-insensitively as a substring match
/// against both built-in and user names.
///
/// User rules whose `name` shadows a built-in (regardless of whether the
/// built-in passes the filter) are suppressed from the User section —
/// they are overrides, not separate entries. The Built-in section
/// continues to display the shadowed built-in subject to the filter.
pub(crate) fn patterns_list_layout(
    builtin_names: &[&'static str],
    user_rules: &[UserRule],
    filter: &str,
) -> PatternsListLayout {
    let filter_lc = filter.to_lowercase();
    let matches = |name: &str| filter.is_empty() || name.to_lowercase().contains(&filter_lc);

    // Shadowing test uses the unfiltered built-in set so that a user-rule
    // named `ipv4` is treated as an override even when the filter would
    // hide `ipv4` from the rendered Built-in section.
    let unfiltered_builtin_set: std::collections::HashSet<&str> =
        builtin_names.iter().copied().collect();

    let filtered_builtin: Vec<&'static str> =
        builtin_names.iter().copied().filter(|n| matches(n)).collect();
    let user_filtered: Vec<String> = user_rules
        .iter()
        .filter(|r| !unfiltered_builtin_set.contains(r.name.as_str()))
        .map(|r| r.name.clone())
        .filter(|n| matches(n))
        .collect();

    let builtin_count = filtered_builtin.len();
    let user_count = user_filtered.len();
    PatternsListLayout {
        builtin_count,
        builtin_names: filtered_builtin,
        user_count,
        user_names: user_filtered,
        builtin_header_render_idx: 0,
        user_header_render_idx: 1 + builtin_count,
    }
}

/// Map a selectable index (`0..builtin_count + user_count`) to a render-row
/// index in the rendered `Vec<ListItem>`. Returns `None` when out of range.
pub(crate) fn selectable_to_render_idx(
    selectable_idx: usize,
    layout: &PatternsListLayout,
) -> Option<usize> {
    if selectable_idx < layout.builtin_count {
        Some(layout.builtin_header_render_idx + 1 + selectable_idx)
    } else if selectable_idx < layout.builtin_count + layout.user_count {
        let user_pos = selectable_idx - layout.builtin_count;
        Some(layout.user_header_render_idx + 1 + user_pos)
    } else {
        None
    }
}

pub(crate) fn render(frame: &mut Frame, area: Rect, app: &App) {
    let chunks =
        Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).split(area);
    render_list(frame, chunks[0], app);
    render_detail(frame, chunks[1], app);
}

fn render_list(frame: &mut Frame, area: Rect, app: &App) {
    let filter = app.search_filter.as_deref().unwrap_or("");
    let layout =
        patterns_list_layout(&app.catalog.builtin_rule_names, &app.snapshot.parsed.rules, filter);

    let header_style = Style::default().add_modifier(Modifier::DIM);
    let mut items: Vec<ListItem> = Vec::with_capacity(2 + layout.builtin_count + layout.user_count);
    items.push(ListItem::new("── Builtin ──").style(header_style));
    for name in &layout.builtin_names {
        items.push(ListItem::new(format!("  {name}")));
    }
    items.push(ListItem::new("── User ──").style(header_style));
    for name in &layout.user_names {
        items.push(ListItem::new(format!("  {name}")));
    }

    let mut state = ListState::default();
    let total_selectable = layout.builtin_count + layout.user_count;
    if total_selectable > 0 {
        let clamped = app.focus.patterns.selected_idx.min(total_selectable - 1);
        state.select(selectable_to_render_idx(clamped, &layout));
    }

    let accent = app.tui_env.accent;
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(accent.border())
                .title("Patterns")
                .title_style(accent.header()),
        )
        .highlight_style(accent.selection());
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_detail(frame: &mut Frame, area: Rect, app: &App) {
    let filter = app.search_filter.as_deref().unwrap_or("");
    let layout =
        patterns_list_layout(&app.catalog.builtin_rule_names, &app.snapshot.parsed.rules, filter);
    let total = layout.builtin_count + layout.user_count;
    let body = if total == 0 {
        "(no pattern selected)".to_owned()
    } else {
        let sel = app.focus.patterns.selected_idx.min(total - 1);
        if sel < layout.builtin_count {
            let name = layout.builtin_names[sel];
            let builtin = crate::rules::builtin_rules().into_iter().find(|r| r.name == name);
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
        } else {
            let user_pos = sel - layout.builtin_count;
            let name = &layout.user_names[user_pos];
            let user_rule = app.snapshot.parsed.rules.iter().find(|r| &r.name == name);
            match user_rule {
                Some(r) => format!(
                    "Pattern: {name}\n\nSource: user config\nRegex: {regex}\n\n\
                     Press 'e' to edit the regex source (inline editor)\n\
                     Press 'c' to open color picker (C4)\n\
                     Press 'd' to delete this user rule",
                    name = r.name,
                    regex = r.pattern.as_deref().unwrap_or("(none)"),
                ),
                None => "(detail not found)".to_owned(),
            }
        }
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
    let layout =
        patterns_list_layout(&app.catalog.builtin_rule_names, &app.snapshot.parsed.rules, filter);
    let total = layout.builtin_count + layout.user_count;
    match k.code {
        KeyCode::Char('j') | KeyCode::Down if total > 0 => {
            app.focus.patterns.selected_idx = (app.focus.patterns.selected_idx + 1) % total;
        }
        KeyCode::Char('k') | KeyCode::Up if total > 0 => {
            let cur = app.focus.patterns.selected_idx.min(total - 1);
            app.focus.patterns.selected_idx = (cur + total - 1) % total;
        }
        KeyCode::Char('g') => app.focus.patterns.selected_idx = 0,
        KeyCode::Char('G') => app.focus.patterns.selected_idx = total.saturating_sub(1),
        KeyCode::Char('h') => app.focus.patterns.detail_focused = false,
        KeyCode::Char('l') | KeyCode::Enter => app.focus.patterns.detail_focused = true,
        KeyCode::Char(' ') => {
            app.toast = Some(crate::config_tui::app::Toast::ok(
                "(activate semantic n/a for patterns — use 'c' to edit style)",
            ));
        }
        KeyCode::Char('o') => {
            // `o` overrides the currently-selected built-in by staging a
            // UserConfig RuleId — semantic only valid in the Built-in
            // section. In the User section the rule is already user-config;
            // suppress with a toast hint.
            let sel = app.focus.patterns.selected_idx;
            if sel < layout.builtin_count {
                let name = layout.builtin_names[sel];
                app.edits.rules.insert(
                    crate::config_tui::edit::RuleId::UserConfig(name.to_owned()),
                    crate::config_tui::edit::RuleEdit::default(),
                );
                app.toast = Some(crate::config_tui::app::Toast::ok(format!(
                    "staged override of '{name}' — edit then Ctrl+S to save"
                )));
            } else if sel < total {
                app.toast = Some(crate::config_tui::app::Toast::ok(
                    "(already a user rule — 'o' has no effect)",
                ));
            }
        }
        KeyCode::Char('d') | KeyCode::Delete => {
            if let Some(rule_id) = crate::config_tui::events::resolve_selected_rule_id(app) {
                let display_name = crate::config_tui::events::rule_id_display_name(&rule_id);
                app.modal = Some(Modal::Confirm {
                    msg: format!("Delete rule '{display_name}'? (staged for removal on save)"),
                    action: ConfirmAction::DeleteRule(rule_id),
                });
            }
        }
        KeyCode::Char('r') => {
            if let Some(rule_id) = crate::config_tui::events::resolve_selected_rule_id(app) {
                let display_name = crate::config_tui::events::rule_id_display_name(&rule_id);
                app.modal = Some(Modal::Confirm {
                    msg: format!("Reset all staged overrides of '{display_name}'?"),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn user_rule(name: &str) -> UserRule {
        UserRule {
            name: name.to_owned(),
            pattern: Some("FOO".to_owned()),
            style: None,
            enabled: true,
            styles: None,
            priority: None,
        }
    }

    #[test]
    fn patterns_list_layout_includes_user_rules_under_user_section() {
        let builtins: &[&'static str] = &["ipv4", "ipv6"];
        let users = vec![user_rule("my-foo"), user_rule("my-bar")];
        let layout = patterns_list_layout(builtins, &users, "");

        assert_eq!(layout.builtin_count, 2, "two builtins survive empty filter");
        assert_eq!(layout.builtin_names, vec!["ipv4", "ipv6"]);
        assert_eq!(layout.user_count, 2, "two user rules");
        assert_eq!(
            layout.user_names,
            vec!["my-foo".to_owned(), "my-bar".to_owned()],
            "user rules appear under User section in source order"
        );
        assert_eq!(layout.builtin_header_render_idx, 0);
        assert_eq!(layout.user_header_render_idx, 3, "User header sits at row 1 + builtin_count");
    }

    #[test]
    fn patterns_list_layout_user_rule_with_builtin_name_only_in_builtin_section() {
        let builtins: &[&'static str] = &["ipv4"];
        // user rule shadows the ipv4 builtin — it is an override, not a
        // new entry, so it must NOT duplicate in the User section.
        let users = vec![user_rule("ipv4")];
        let layout = patterns_list_layout(builtins, &users, "");

        assert!(
            layout.builtin_names.contains(&"ipv4"),
            "shadowed builtin still rendered in Builtin section"
        );
        assert_eq!(
            layout.user_count, 0,
            "user override of a builtin name does not duplicate as a User row"
        );
    }

    #[test]
    fn patterns_list_layout_filter_covers_user_section_case_insensitively() {
        let builtins: &[&'static str] = &["ipv4", "ipv6", "url"];
        let users = vec![user_rule("unique-user-FOO"), user_rule("other")];
        let layout = patterns_list_layout(builtins, &users, "unique");

        assert!(layout.builtin_names.is_empty(), "no builtin contains 'unique'");
        assert_eq!(layout.user_names, vec!["unique-user-FOO".to_owned()]);
        // case-insensitive: filter "FOO" matches "unique-user-FOO".
        let layout_lc = patterns_list_layout(builtins, &users, "foo");
        assert_eq!(layout_lc.user_names, vec!["unique-user-FOO".to_owned()]);
    }

    #[test]
    fn selectable_to_render_idx_skips_section_header_rows() {
        // Layout: header(0) | b0(1) b1(2) | header(3) | u0(4) u1(5)
        let builtins: &[&'static str] = &["b0", "b1"];
        let users = vec![user_rule("u0"), user_rule("u1")];
        let layout = patterns_list_layout(builtins, &users, "");

        assert_eq!(
            selectable_to_render_idx(0, &layout),
            Some(1),
            "first builtin sits at render row 1 (after Builtin header at row 0)"
        );
        assert_eq!(
            selectable_to_render_idx(1, &layout),
            Some(2),
            "second builtin at row 2 — no header gap inside the section"
        );
        assert_eq!(
            selectable_to_render_idx(2, &layout),
            Some(4),
            "first user rule at row 4 (User header at row 3 is skipped)"
        );
        assert_eq!(selectable_to_render_idx(3, &layout), Some(5), "second user rule at row 5");
        assert_eq!(
            selectable_to_render_idx(4, &layout),
            None,
            "out-of-range selectable returns None"
        );
    }
}
