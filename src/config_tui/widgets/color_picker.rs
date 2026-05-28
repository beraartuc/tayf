//! Y hybrid color picker (ANSI16 / 256-palette / truecolor hex).
//!
//! Three sections in a single pane; Tab advances section, ←→ moves
//! within section, Enter accepts, Esc cancels. See spec §12.4.

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color as RaColor, Style as RaStyle};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickerSection {
    Ansi16,
    Palette256,
    TrueHex,
}

#[derive(Debug)]
pub(crate) struct ColorPickerState {
    pub(crate) section: PickerSection,
    pub(crate) ansi16_idx: u8,
    pub(crate) palette_idx: u16,
    pub(crate) hex_buf: String,
    pub(crate) goto_buf: Option<String>,
}

impl Default for ColorPickerState {
    fn default() -> Self {
        Self {
            section: PickerSection::Ansi16,
            ansi16_idx: 0,
            palette_idx: 0,
            hex_buf: String::new(),
            goto_buf: None,
        }
    }
}

impl ColorPickerState {
    /// Returns the color currently highlighted by the active section.
    ///
    /// - `Ansi16` always yields `Some(_)` (one of `Color::Black..BrightWhite`).
    /// - `Palette256` always yields `Some(Color::Indexed(_))`.
    /// - `TrueHex` yields `Some(Color::Rgb(_,_,_))` only when `hex_buf`
    ///   is a complete six-digit hex value; partial input yields `None`
    ///   so the Accept caller can show a toast instead of binding to a
    ///   spurious color.
    pub(crate) fn selected_color(&self) -> Option<crate::style::Color> {
        use crate::style::Color;
        match self.section {
            PickerSection::Ansi16 => Some(match self.ansi16_idx {
                0 => Color::Black,
                1 => Color::Red,
                2 => Color::Green,
                3 => Color::Yellow,
                4 => Color::Blue,
                5 => Color::Magenta,
                6 => Color::Cyan,
                7 => Color::White,
                8 => Color::BrightBlack,
                9 => Color::BrightRed,
                10 => Color::BrightGreen,
                11 => Color::BrightYellow,
                12 => Color::BrightBlue,
                13 => Color::BrightMagenta,
                14 => Color::BrightCyan,
                _ => Color::BrightWhite,
            }),
            PickerSection::Palette256 => {
                // reason: palette_idx is clamped to 0..=255 in dispatch_key,
                // so the cast is in-range. Mirrors the same cast in render_palette256.
                #[allow(clippy::cast_possible_truncation)]
                let idx_u8 = self.palette_idx as u8;
                Some(Color::Indexed(idx_u8))
            }
            PickerSection::TrueHex => {
                if self.hex_buf.len() != 6 {
                    return None;
                }
                let r = u8::from_str_radix(&self.hex_buf[0..2], 16).ok()?;
                let g = u8::from_str_radix(&self.hex_buf[2..4], 16).ok()?;
                let b = u8::from_str_radix(&self.hex_buf[4..6], 16).ok()?;
                Some(Color::Rgb(r, g, b))
            }
        }
    }
}

pub(crate) fn render(frame: &mut Frame, area: Rect, state: &ColorPickerState) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Color picker — Tab section, ←→ value, Enter accept, Esc cancel");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(3),
        Constraint::Length(2),
    ])
    .split(inner);

    render_ansi16(frame, chunks[0], state);
    render_palette256(frame, chunks[1], state);
    render_truecolor_hex(frame, chunks[2], state);
    render_status(frame, chunks[3], state);
}

fn render_ansi16(frame: &mut Frame, area: Rect, state: &ColorPickerState) {
    let active_section = state.section == PickerSection::Ansi16;
    let mut spans: Vec<Span> =
        vec![Span::raw(if active_section { "▶ ANSI16: " } else { "  ANSI16: " })];
    for i in 0..16u8 {
        let s = if i == state.ansi16_idx && active_section {
            RaStyle::default().bg(RaColor::Indexed(i)).fg(RaColor::White)
        } else {
            RaStyle::default().bg(RaColor::Indexed(i))
        };
        spans.push(Span::styled(format!(" {i:2} "), s));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_palette256(frame: &mut Frame, area: Rect, state: &ColorPickerState) {
    let active_section = state.section == PickerSection::Palette256;
    let header =
        if active_section { "▶ 256-palette (g<idx>Enter jump)" } else { "  256-palette" };
    let mut lines: Vec<Line> = vec![Line::from(header.to_owned())];
    for row in 0..16u16 {
        let mut spans: Vec<Span> = Vec::new();
        for col in 0..16u16 {
            let idx = row * 16 + col;
            // reason: row × 16 + col is bounded by 15 × 16 + 15 = 255, fits u8.
            #[allow(clippy::cast_possible_truncation)]
            let idx_u8 = idx as u8;
            let style = if idx == state.palette_idx && active_section {
                RaStyle::default().bg(RaColor::Indexed(idx_u8)).fg(RaColor::White)
            } else {
                RaStyle::default().bg(RaColor::Indexed(idx_u8))
            };
            spans.push(Span::styled("  ", style));
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_truecolor_hex(frame: &mut Frame, area: Rect, state: &ColorPickerState) {
    let active_section = state.section == PickerSection::TrueHex;
    let prefix = if active_section { "▶ #" } else { "  #" };
    let display = format!("{prefix}{:<6}", state.hex_buf);
    frame.render_widget(Paragraph::new(display), area);
}

fn render_status(frame: &mut Frame, area: Rect, state: &ColorPickerState) {
    let s = state.goto_buf.as_ref().map_or_else(
        || "Tab=section ←→=value N=none Enter=accept Esc=cancel".to_owned(),
        |b| format!("goto idx: {b}_"),
    );
    frame.render_widget(Paragraph::new(s), area);
}

pub(crate) fn dispatch_key(state: &mut ColorPickerState, k: KeyEvent) -> ColorPickerOutcome {
    if k.code == KeyCode::Esc {
        if state.goto_buf.take().is_some() {
            return ColorPickerOutcome::StayOpen;
        }
        return ColorPickerOutcome::Cancel;
    }
    if let Some(buf) = state.goto_buf.as_mut() {
        if let KeyCode::Char(c @ '0'..='9') = k.code {
            if buf.len() < 3 {
                buf.push(c);
            }
            return ColorPickerOutcome::StayOpen;
        }
        if k.code == KeyCode::Enter {
            if let Ok(idx) = buf.parse::<u16>() {
                if idx < 256 {
                    state.palette_idx = idx;
                }
            }
            state.goto_buf = None;
            return ColorPickerOutcome::StayOpen;
        }
        return ColorPickerOutcome::StayOpen;
    }
    match k.code {
        KeyCode::Tab => {
            state.section = match state.section {
                PickerSection::Ansi16 => PickerSection::Palette256,
                PickerSection::Palette256 => PickerSection::TrueHex,
                PickerSection::TrueHex => PickerSection::Ansi16,
            };
            ColorPickerOutcome::StayOpen
        }
        KeyCode::BackTab => {
            state.section = match state.section {
                PickerSection::Ansi16 => PickerSection::TrueHex,
                PickerSection::Palette256 => PickerSection::Ansi16,
                PickerSection::TrueHex => PickerSection::Palette256,
            };
            ColorPickerOutcome::StayOpen
        }
        KeyCode::Left => {
            match state.section {
                PickerSection::Ansi16 => state.ansi16_idx = state.ansi16_idx.saturating_sub(1),
                PickerSection::Palette256 => {
                    state.palette_idx = state.palette_idx.saturating_sub(1);
                }
                PickerSection::TrueHex => {
                    state.hex_buf.pop();
                }
            }
            ColorPickerOutcome::StayOpen
        }
        KeyCode::Right => {
            match state.section {
                PickerSection::Ansi16 => state.ansi16_idx = (state.ansi16_idx + 1).min(15),
                PickerSection::Palette256 => {
                    state.palette_idx = (state.palette_idx + 1).min(255);
                }
                PickerSection::TrueHex => {}
            }
            ColorPickerOutcome::StayOpen
        }
        KeyCode::Up if state.section == PickerSection::Palette256 => {
            state.palette_idx = state.palette_idx.saturating_sub(16);
            ColorPickerOutcome::StayOpen
        }
        KeyCode::Down if state.section == PickerSection::Palette256 => {
            state.palette_idx = (state.palette_idx + 16).min(255);
            ColorPickerOutcome::StayOpen
        }
        KeyCode::Char('g') if state.section == PickerSection::Palette256 => {
            state.goto_buf = Some(String::new());
            ColorPickerOutcome::StayOpen
        }
        KeyCode::Char(c @ ('0'..='9' | 'a'..='f')) if state.section == PickerSection::TrueHex => {
            if state.hex_buf.len() < 6 {
                state.hex_buf.push(c);
            }
            ColorPickerOutcome::StayOpen
        }
        KeyCode::Char('N') => {
            match state.section {
                PickerSection::Ansi16 => state.ansi16_idx = 0,
                PickerSection::Palette256 => state.palette_idx = 0,
                PickerSection::TrueHex => state.hex_buf.clear(),
            }
            ColorPickerOutcome::StayOpen
        }
        KeyCode::Enter => ColorPickerOutcome::Accept,
        _ => ColorPickerOutcome::StayOpen,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorPickerOutcome {
    StayOpen,
    Accept,
    Cancel,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    fn mk(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    #[test]
    fn tab_advances_section_circular() {
        let mut s = ColorPickerState::default();
        assert_eq!(s.section, PickerSection::Ansi16);
        dispatch_key(&mut s, mk(KeyCode::Tab));
        assert_eq!(s.section, PickerSection::Palette256);
        dispatch_key(&mut s, mk(KeyCode::Tab));
        assert_eq!(s.section, PickerSection::TrueHex);
        dispatch_key(&mut s, mk(KeyCode::Tab));
        assert_eq!(s.section, PickerSection::Ansi16);
    }

    #[test]
    fn arrow_within_ansi16_moves_cursor() {
        let mut s = ColorPickerState::default();
        dispatch_key(&mut s, mk(KeyCode::Right));
        assert_eq!(s.ansi16_idx, 1);
        dispatch_key(&mut s, mk(KeyCode::Right));
        assert_eq!(s.ansi16_idx, 2);
        dispatch_key(&mut s, mk(KeyCode::Left));
        assert_eq!(s.ansi16_idx, 1);
    }

    #[test]
    fn truecolor_hex_input_parses_6_digit() {
        let mut s = ColorPickerState::default();
        dispatch_key(&mut s, mk(KeyCode::Tab));
        dispatch_key(&mut s, mk(KeyCode::Tab));
        for c in ['f', 'f', '8', '8', '0', '0'] {
            dispatch_key(&mut s, mk(KeyCode::Char(c)));
        }
        assert_eq!(s.hex_buf, "ff8800");
    }

    #[test]
    fn invalid_hex_char_in_truecolor_section_stays() {
        let mut s = ColorPickerState { section: PickerSection::TrueHex, ..Default::default() };
        dispatch_key(&mut s, mk(KeyCode::Char('z')));
        assert!(s.hex_buf.is_empty());
    }

    #[test]
    fn n_clears_current_section() {
        let mut s = ColorPickerState { ansi16_idx: 7, ..Default::default() };
        dispatch_key(&mut s, mk(KeyCode::Char('N')));
        assert_eq!(s.ansi16_idx, 0);
    }

    #[test]
    fn goto_then_three_digits_then_enter_jumps_palette() {
        let mut s = ColorPickerState { section: PickerSection::Palette256, ..Default::default() };
        dispatch_key(&mut s, mk(KeyCode::Char('g')));
        assert!(s.goto_buf.is_some());
        dispatch_key(&mut s, mk(KeyCode::Char('1')));
        dispatch_key(&mut s, mk(KeyCode::Char('3')));
        dispatch_key(&mut s, mk(KeyCode::Char('7')));
        dispatch_key(&mut s, mk(KeyCode::Enter));
        assert_eq!(s.palette_idx, 137);
        assert!(s.goto_buf.is_none());
    }

    #[test]
    fn esc_clears_goto_input_first_then_cancels_on_second_press() {
        let mut s = ColorPickerState { section: PickerSection::Palette256, ..Default::default() };
        dispatch_key(&mut s, mk(KeyCode::Char('g')));
        let out = dispatch_key(&mut s, mk(KeyCode::Esc));
        assert_eq!(out, ColorPickerOutcome::StayOpen, "first Esc clears goto input only");
        assert!(s.goto_buf.is_none());
        let out = dispatch_key(&mut s, mk(KeyCode::Esc));
        assert_eq!(out, ColorPickerOutcome::Cancel, "second Esc cancels modal");
    }
}
