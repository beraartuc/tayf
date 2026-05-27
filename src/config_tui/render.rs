//! Frame composition (Layout split, narrow-term gate). v0.5.4 C2b target.
//!
//! C2a minimal stub: provides `DEFAULT_PREVIEW_SAMPLE` and a placeholder
//! `frame` fn so that `app.rs` and `events.rs` compile. C2b replaces this
//! file entirely with the full layout implementation.

use ratatui::widgets::Paragraph;

use crate::config_tui::app::App;

/// Default sample text shown in the live-preview strip.
/// C2b wires the real multi-line sample (spec §9.3).
pub(crate) const DEFAULT_PREVIEW_SAMPLE: &str =
    "2026-01-15T14:32:01Z INFO  user@host.example.com permission denied";

/// Render one TUI frame. C2a placeholder — C2b replaces with full layout.
pub(crate) fn frame(f: &mut ratatui::Frame, _app: &App) {
    let area = f.area();
    f.render_widget(Paragraph::new("tayf config TUI — loading (C2b wires full layout)"), area);
}
