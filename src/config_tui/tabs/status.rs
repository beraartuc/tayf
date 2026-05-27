//! Status tab — read-only resolved config + hot-reload event tail.
//! Renders the same information `tayf config status` prints, but
//! formatted for in-TUI consumption (per-line + scrollable).

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::config_tui::app::App;

pub(crate) fn render(frame: &mut Frame, area: Rect, app: &App) {
    let config_str = app.snapshot.source_path.as_deref().map_or_else(
        || {
            "(no user config file at ~/.config/tayf/config.toml — will be created on first save)"
                .to_owned()
        },
        |p| p.display().to_string(),
    );
    let mut lines: Vec<Line> = vec![
        Line::from(vec![Span::raw("config: "), Span::raw(config_str)]),
        Line::from(vec![
            Span::raw("theme: "),
            Span::raw(
                app.snapshot.parsed.theme.as_deref().unwrap_or("(unresolved: none set)").to_owned(),
            ),
        ]),
        Line::from(vec![
            Span::raw("profile: "),
            Span::raw(
                app.snapshot
                    .parsed
                    .profile
                    .as_deref()
                    .unwrap_or("(unresolved: none set)")
                    .to_owned(),
            ),
        ]),
        Line::from(""),
        Line::from("recent reload events:"),
    ];
    if let Some(cfg_dir) = app.snapshot.source_path.as_deref().and_then(|p| p.parent()) {
        // reload.rs writes to <cfg_dir>/runtime/reload.log per ReloadLogger::create.
        let state_dir = cfg_dir.join("runtime");
        let events = crate::reload::read_recent_events(&state_dir, 50);
        if events.is_empty() {
            lines.push(Line::from("  (no events recorded — no wrapper active)"));
        } else {
            for ev in events.iter().take(20) {
                let outcome = match &ev.outcome {
                    crate::reload::ReloadOutcome::Ok => "ok".to_owned(),
                    crate::reload::ReloadOutcome::Err(e) => format!("err: {e}"),
                };
                lines.push(Line::from(format!("  reload #{}: {}", ev.reload_count, outcome)));
            }
        }
    } else {
        lines.push(Line::from("  (no config dir; nothing to tail)"));
    }
    let block = Block::default().borders(Borders::ALL).title("Status");
    let p = Paragraph::new(lines)
        .block(block)
        .scroll((u16::try_from(app.focus.status.scroll).unwrap_or(0), 0));
    frame.render_widget(p, area);
}

pub(crate) fn dispatch_key(app: &mut App, k: KeyEvent) {
    match k.code {
        KeyCode::Char('j') | KeyCode::Down => {
            app.focus.status.scroll = app.focus.status.scroll.saturating_add(1);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.focus.status.scroll = app.focus.status.scroll.saturating_sub(1);
        }
        _ => {}
    }
}
