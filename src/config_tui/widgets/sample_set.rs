//! Sample input set modal (`s` global key). Spec §9.3.
//!
//! Single-line text-input contract for v0.5.4: Enter commits; for
//! multi-line samples users must paste via terminal (paste support
//! lands in v0.7+).

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

#[derive(Debug, Default)]
pub(crate) struct SampleSetState {
    pub(crate) buf: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SampleSetOutcome {
    StayOpen,
    Commit(String),
    Cancel,
}

pub(crate) fn render(frame: &mut Frame, area: Rect, state: &SampleSetState) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Sample input — Enter to commit, Esc to cancel");
    let body = format!("{}_", state.buf);
    frame.render_widget(Paragraph::new(body).block(block), area);
}

pub(crate) fn dispatch_key(state: &mut SampleSetState, k: KeyEvent) -> SampleSetOutcome {
    match k.code {
        KeyCode::Esc => SampleSetOutcome::Cancel,
        KeyCode::Enter => SampleSetOutcome::Commit(state.buf.clone()),
        KeyCode::Backspace => {
            state.buf.pop();
            SampleSetOutcome::StayOpen
        }
        KeyCode::Char(c) => {
            if state.buf.len() < 4096 {
                state.buf.push(c);
            }
            SampleSetOutcome::StayOpen
        }
        _ => SampleSetOutcome::StayOpen,
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
    fn typing_appends() {
        let mut s = SampleSetState::default();
        for c in ['a', 'b'] {
            dispatch_key(&mut s, mk(KeyCode::Char(c)));
        }
        assert_eq!(s.buf, "ab");
    }

    #[test]
    fn enter_commits_buf() {
        let mut s = SampleSetState { buf: "log line".to_owned() };
        let out = dispatch_key(&mut s, mk(KeyCode::Enter));
        assert_eq!(out, SampleSetOutcome::Commit("log line".to_owned()));
    }

    #[test]
    fn esc_cancels() {
        let mut s = SampleSetState { buf: "x".to_owned() };
        let out = dispatch_key(&mut s, mk(KeyCode::Esc));
        assert_eq!(out, SampleSetOutcome::Cancel);
    }
}
