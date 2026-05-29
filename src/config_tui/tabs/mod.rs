//! Tab dispatch facade. Each tab module exposes `render` + `dispatch_key`.
//!
//! Render fn signature: `pub(crate) fn render(frame: &mut Frame, area: Rect, app: &App)`.
//! Dispatch fn signature: `pub(crate) fn dispatch_key(app: &mut App, k: KeyEvent)`.

use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::config_tui::app::{App, Tab};

pub(crate) mod patterns;
pub(crate) mod profiles;
pub(crate) mod status;
pub(crate) mod themes;

/// Render the currently-focused tab into `area`.
pub(crate) fn render(frame: &mut Frame, area: Rect, app: &App) {
    match app.tab {
        Tab::Patterns => patterns::render(frame, area, app),
        Tab::Themes => themes::render(frame, area, app),
        Tab::Profiles => profiles::render(frame, area, app),
        Tab::Status => status::render(frame, area, app),
    }
}

/// Forward a key to the currently-focused tab's dispatcher.
pub(crate) fn dispatch_key(app: &mut App, k: KeyEvent) {
    match app.tab {
        Tab::Patterns => patterns::dispatch_key(app, k),
        Tab::Themes => themes::dispatch_key(app, k),
        Tab::Profiles => profiles::dispatch_key(app, k),
        Tab::Status => status::dispatch_key(app, k),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_tui::app::App;
    use crate::config_tui::test_support::assert_render_snapshot;

    #[test]
    fn render_themes_tab_init_matches_snapshot() {
        let mut app = App::default_for_test();
        app.tab = Tab::Themes;
        assert_render_snapshot(80, 24, &app, render, "tabs_themes_init");
    }

    #[test]
    fn render_rules_tab_overlay_matches_snapshot() {
        let mut app = App::default_for_test();
        app.tab = Tab::Patterns;
        assert_render_snapshot(80, 24, &app, render, "tabs_rules_overlay");
    }

    #[test]
    fn render_profiles_tab_init_matches_snapshot() {
        let mut app = App::default_for_test();
        app.tab = Tab::Profiles;
        assert_render_snapshot(80, 24, &app, render, "tabs_profiles_init");
    }
}
