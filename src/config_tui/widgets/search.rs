//! Search input modal (`/` global key). Spec §12.1 sticky filter.
//!
//! Single-line text input; on Enter commits to `App.search_filter`
//! which the tabs read to filter their list rendering.

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

#[derive(Debug, Default)]
pub(crate) struct SearchState {
    pub(crate) buf: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SearchOutcome {
    StayOpen,
    Commit(String),
    Cancel,
}

pub(crate) fn render(frame: &mut Frame, area: Rect, state: &SearchState) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Search — type to filter, Enter to commit, Esc to cancel");
    let body = format!("/{}_", state.buf);
    frame.render_widget(Paragraph::new(body).block(block), area);
}

pub(crate) fn dispatch_key(state: &mut SearchState, k: KeyEvent) -> SearchOutcome {
    match k.code {
        KeyCode::Esc => SearchOutcome::Cancel,
        KeyCode::Enter => SearchOutcome::Commit(state.buf.clone()),
        KeyCode::Backspace => {
            state.buf.pop();
            SearchOutcome::StayOpen
        }
        KeyCode::Char(c) => {
            // Cap at a reasonable length to keep the input single-line.
            if state.buf.len() < 128 {
                state.buf.push(c);
            }
            SearchOutcome::StayOpen
        }
        _ => SearchOutcome::StayOpen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    fn mk(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    #[test]
    fn typing_appends_to_buf() {
        let mut s = SearchState::default();
        for c in ['f', 'o', 'o'] {
            dispatch_key(&mut s, mk(KeyCode::Char(c)));
        }
        assert_eq!(s.buf, "foo");
    }

    #[test]
    fn backspace_pops() {
        let mut s = SearchState { buf: "foo".to_owned() };
        dispatch_key(&mut s, mk(KeyCode::Backspace));
        assert_eq!(s.buf, "fo");
    }

    #[test]
    fn enter_returns_commit_with_buf_contents() {
        let mut s = SearchState { buf: "bar".to_owned() };
        let out = dispatch_key(&mut s, mk(KeyCode::Enter));
        assert_eq!(out, SearchOutcome::Commit("bar".to_owned()));
    }

    #[test]
    fn esc_returns_cancel() {
        let mut s = SearchState { buf: "x".to_owned() };
        let out = dispatch_key(&mut s, mk(KeyCode::Esc));
        assert_eq!(out, SearchOutcome::Cancel);
    }
}
