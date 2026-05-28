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

/// Tracks which boolean style axis (if any) currently owns keyboard
/// focus inside the color picker modal. Parallel to [`PickerSection`]
/// which only governs the three color sub-sections. When
/// `axis_focus != AxisFocus::None`, the `c` keystroke clears the focused
/// axis (writes `Some(None)` into the staged `Option<Option<bool>>`)
/// instead of falling through to the `TrueHex` hex-digit branch. Spec §3.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AxisFocus {
    None,
    Bold,
    Italic,
    Underline,
}

#[derive(Debug)]
pub(crate) struct ColorPickerState {
    pub(crate) section: PickerSection,
    pub(crate) ansi16_idx: u8,
    pub(crate) palette_idx: u16,
    pub(crate) hex_buf: String,
    pub(crate) goto_buf: Option<String>,
    /// Which bool axis (if any) currently has focus. `AxisFocus::None`
    /// means color-section focus (one of `section`'s three values).
    pub(crate) axis_focus: AxisFocus,
    /// Staged tri-state edits per bool axis. Outer `None` = unedited
    /// (no write into `app.edits` on commit), `Some(None)` = explicit
    /// clear (`c` keystroke), `Some(Some(b))` = explicit set
    /// (Space-toggle). Spec §3.1.
    //
    // reason: `Option<Option<bool>>` is the load-bearing tri-state shape
    // mirrored from `NewStyle::{bold,italic,underline}`. Replacing with a
    // custom enum would only rename the same three states.
    #[allow(clippy::option_option)]
    pub(crate) staged_bold: Option<Option<bool>>,
    #[allow(clippy::option_option)]
    pub(crate) staged_italic: Option<Option<bool>>,
    #[allow(clippy::option_option)]
    pub(crate) staged_underline: Option<Option<bool>>,
}

impl Default for ColorPickerState {
    fn default() -> Self {
        Self {
            section: PickerSection::Ansi16,
            ansi16_idx: 0,
            palette_idx: 0,
            hex_buf: String::new(),
            goto_buf: None,
            axis_focus: AxisFocus::None,
            staged_bold: None,
            staged_italic: None,
            staged_underline: None,
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
        Constraint::Length(1),
        Constraint::Length(2),
    ])
    .split(inner);

    render_ansi16(frame, chunks[0], state);
    render_palette256(frame, chunks[1], state);
    render_truecolor_hex(frame, chunks[2], state);
    render_axis_row(frame, chunks[3], state);
    render_status(frame, chunks[4], state);
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
    let active_section =
        state.section == PickerSection::TrueHex && state.axis_focus == AxisFocus::None;
    let prefix = if active_section { "▶ #" } else { "  #" };
    let display = format!("{prefix}{:<6}", state.hex_buf);
    frame.render_widget(Paragraph::new(display), area);
}

/// Render the one-line bool-axis row (chunks[3]). Each axis is shown as
/// `[label: value]` where `value` is `—` (unedited), `✗` (explicit clear),
/// `yes` or `no` (explicit set). The currently focused axis is underlined.
/// Spec §3.1.
fn render_axis_row(frame: &mut Frame, area: Rect, state: &ColorPickerState) {
    use ratatui::style::Modifier;
    let format_axis = |label: &str, staged: Option<Option<bool>>, focused: bool| -> Span<'_> {
        let val = match staged {
            None => "—",
            Some(None) => "✗",
            Some(Some(true)) => "yes",
            Some(Some(false)) => "no",
        };
        let style = if focused {
            RaStyle::default().add_modifier(Modifier::UNDERLINED)
        } else {
            RaStyle::default()
        };
        Span::styled(format!("[{label}: {val}]"), style)
    };

    let line = Line::from(vec![
        format_axis("bold", state.staged_bold, state.axis_focus == AxisFocus::Bold),
        Span::raw(" "),
        format_axis("italic", state.staged_italic, state.axis_focus == AxisFocus::Italic),
        Span::raw(" "),
        format_axis("underline", state.staged_underline, state.axis_focus == AxisFocus::Underline),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_status(frame: &mut Frame, area: Rect, state: &ColorPickerState) {
    let s = state.goto_buf.as_ref().map_or_else(
        || "Tab=section ←→=val Space=toggle c=clear Enter=accept Esc=cancel".to_owned(),
        |b| format!("goto idx: {b}_"),
    );
    frame.render_widget(Paragraph::new(s), area);
}

// reason: G3 expanded dispatch with the 6-step Tab cycle + `c`/Space axis
// arms + their `BackTab` mirrors. Splitting the match into helpers would
// duplicate the `state` `&mut` plumbing and obscure the keystroke trace;
// the function remains a single flat dispatch table consistent with the
// other TUI widget dispatchers.
#[allow(clippy::too_many_lines)]
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
        // 6-step Tab cycle: Ansi16 → Palette256 → TrueHex → Bold → Italic →
        // Underline → wrap. While an `AxisFocus` is non-None the `section`
        // stays at TrueHex (its last color value); resetting to Ansi16 on
        // wrap restores the spec §3.1 invariant that color-section focus
        // implies `axis_focus == None`.
        KeyCode::Tab => {
            match (state.section, state.axis_focus) {
                (PickerSection::Ansi16, _) => {
                    state.section = PickerSection::Palette256;
                    state.axis_focus = AxisFocus::None;
                }
                (PickerSection::Palette256, _) => {
                    state.section = PickerSection::TrueHex;
                    state.axis_focus = AxisFocus::None;
                }
                (PickerSection::TrueHex, AxisFocus::None) => {
                    state.axis_focus = AxisFocus::Bold;
                }
                (_, AxisFocus::Bold) => state.axis_focus = AxisFocus::Italic,
                (_, AxisFocus::Italic) => state.axis_focus = AxisFocus::Underline,
                (_, AxisFocus::Underline) => {
                    state.section = PickerSection::Ansi16;
                    state.axis_focus = AxisFocus::None;
                }
            }
            ColorPickerOutcome::StayOpen
        }
        KeyCode::BackTab => {
            // Mirror of the 6-step Tab cycle in reverse.
            match (state.section, state.axis_focus) {
                (_, AxisFocus::Bold) => {
                    state.axis_focus = AxisFocus::None;
                    state.section = PickerSection::TrueHex;
                }
                (_, AxisFocus::Italic) => state.axis_focus = AxisFocus::Bold,
                (_, AxisFocus::Underline) => state.axis_focus = AxisFocus::Italic,
                (PickerSection::Ansi16, AxisFocus::None) => {
                    state.section = PickerSection::TrueHex;
                    state.axis_focus = AxisFocus::Underline;
                }
                (PickerSection::Palette256, AxisFocus::None) => {
                    state.section = PickerSection::Ansi16;
                }
                (PickerSection::TrueHex, AxisFocus::None) => {
                    state.section = PickerSection::Palette256;
                }
            }
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
        // `c` — clear the focused bool axis to `Some(None)`. GATED on
        // `axis_focus != None` so it does NOT shadow the TrueHex
        // hex-digit branch below (T-B2 regression pin). Spec §3.1.
        KeyCode::Char('c') if state.axis_focus != AxisFocus::None => {
            match state.axis_focus {
                AxisFocus::Bold => state.staged_bold = Some(None),
                AxisFocus::Italic => state.staged_italic = Some(None),
                AxisFocus::Underline => state.staged_underline = Some(None),
                AxisFocus::None => unreachable!("guarded by `if` clause above"),
            }
            ColorPickerOutcome::StayOpen
        }
        // Space — toggle the focused bool axis. Unedited/cleared/false all
        // advance to `Some(Some(true))`; `Some(Some(true))` flips to
        // `Some(Some(false))`. Spec §3.1.
        KeyCode::Char(' ') if state.axis_focus != AxisFocus::None => {
            let toggle = |staged: &mut Option<Option<bool>>| {
                *staged = match staged {
                    Some(Some(true)) => Some(Some(false)),
                    None | Some(None | Some(false)) => Some(Some(true)),
                };
            };
            match state.axis_focus {
                AxisFocus::Bold => toggle(&mut state.staged_bold),
                AxisFocus::Italic => toggle(&mut state.staged_italic),
                AxisFocus::Underline => toggle(&mut state.staged_underline),
                AxisFocus::None => unreachable!("guarded by `if` clause above"),
            }
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
    fn tab_advances_section_then_axes_then_wraps() {
        // G3 — 6-step Tab cycle:
        //   Ansi16 → Palette256 → TrueHex → Bold → Italic → Underline → wrap.
        let mut s = ColorPickerState::default();
        assert_eq!(s.section, PickerSection::Ansi16);
        assert_eq!(s.axis_focus, AxisFocus::None);
        dispatch_key(&mut s, mk(KeyCode::Tab));
        assert_eq!(s.section, PickerSection::Palette256);
        assert_eq!(s.axis_focus, AxisFocus::None);
        dispatch_key(&mut s, mk(KeyCode::Tab));
        assert_eq!(s.section, PickerSection::TrueHex);
        assert_eq!(s.axis_focus, AxisFocus::None);
        dispatch_key(&mut s, mk(KeyCode::Tab));
        assert_eq!(s.axis_focus, AxisFocus::Bold);
        dispatch_key(&mut s, mk(KeyCode::Tab));
        assert_eq!(s.axis_focus, AxisFocus::Italic);
        dispatch_key(&mut s, mk(KeyCode::Tab));
        assert_eq!(s.axis_focus, AxisFocus::Underline);
        dispatch_key(&mut s, mk(KeyCode::Tab));
        assert_eq!(s.section, PickerSection::Ansi16, "wrap back to first color section");
        assert_eq!(s.axis_focus, AxisFocus::None);
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
