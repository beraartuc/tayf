//! Hex-primary color picker (truecolor + palette-index / ANSI16 / bool axes).
//!
//! Two color sections in a single pane; `Hex` is the default focus.
//! Tab advances section and bool axes, ←→ move values within each section,
//! Enter accepts, Esc cancels. See spec §6.
//!
//! Public API:
//! - [`ColorPickerState`] — editor state (color + bool-axis staged values).
//! - [`PickerSection`] — which color-input section currently has focus.
//! - [`AxisFocus`] — which bool axis (if any) currently has focus.
//! - [`ColorPickerOutcome`] — what the dispatcher signals to the caller.
//! - [`dispatch_key`] — handle a key event, return an outcome.
//! - [`render`] — draw the picker into an area.
//!
//! Accept-commit contract (preserved from the old three-section picker):
//! `selected_color() -> Option<crate::style::Color>` and the three
//! `staged_*: Option<Option<bool>>` fields carry the same meaning so
//! `events.rs` and `new_pattern.rs` need no logic change on commit.
//!
//! Invariant: `section == PickerSection::Hex` implies `axis_focus == AxisFocus::None`
//! (equivalently: `axis_focus != None` implies `section == PickerSection::Ansi16`).
//! `PickerSection::Ansi16` is the gateway to the bool-axis segment of the Tab cycle.

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color as RaColor, Style as RaStyle};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

/// Which color-input section currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickerSection {
    /// Truecolor hex `#rrggbb`, or `@0-255` palette index. Primary focus.
    Hex,
    /// 16 ANSI swatches.
    Ansi16,
}

/// Tracks which boolean style axis (if any) currently owns keyboard focus
/// inside the color picker modal. Parallel to [`PickerSection`] which governs
/// the two color sub-sections. When `axis_focus != AxisFocus::None`, the `c`
/// keystroke clears the focused axis (writes `Some(None)` into the staged
/// `Option<Option<bool>>`) instead of shadowing a hex-digit branch. Spec §3.1.
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
    /// Hex entry buffer: either `#rrggbb` (up to 6 hex digits) or `@NNN`
    /// (up to 3 decimal digits after `@`, representing palette index 0-255).
    pub(crate) hex_buf: String,
    /// Which bool axis (if any) currently has focus. `AxisFocus::None`
    /// means color-section focus (one of `section`'s two values).
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
    // reason: same tri-state shape as `staged_bold`; see the doc-comment above.
    #[allow(clippy::option_option)]
    pub(crate) staged_italic: Option<Option<bool>>,
    // reason: same tri-state shape as `staged_bold`; see the doc-comment above.
    #[allow(clippy::option_option)]
    pub(crate) staged_underline: Option<Option<bool>>,
}

impl Default for ColorPickerState {
    fn default() -> Self {
        Self {
            section: PickerSection::Hex,
            ansi16_idx: 0,
            hex_buf: String::new(),
            axis_focus: AxisFocus::None,
            staged_bold: None,
            staged_italic: None,
            staged_underline: None,
        }
    }
}

impl ColorPickerState {
    /// The color the active section currently designates, or `None` when the
    /// hex field holds an incomplete/out-of-range value (Accept shows a toast).
    ///
    /// - `Ansi16` always yields `Some(_)`.
    /// - `Hex` with `@NNN` (0..=255) yields `Some(Color::Indexed(N))`.
    /// - `Hex` with 6 hex digits yields `Some(Color::Rgb(_,_,_))`.
    /// - Partial or out-of-range `Hex` yields `None`.
    pub(crate) fn selected_color(&self) -> Option<crate::style::Color> {
        use crate::style::Color;
        match self.section {
            PickerSection::Ansi16 => Some(ansi16_color(self.ansi16_idx)),
            PickerSection::Hex => {
                if let Some(rest) = self.hex_buf.strip_prefix('@') {
                    let n: u16 = rest.parse().ok()?;
                    if n >= 256 {
                        return None;
                    }
                    // reason: bounded < 256 above, fits u8.
                    #[allow(clippy::cast_possible_truncation)]
                    return Some(Color::Indexed(n as u8));
                }
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

/// Map ANSI index (0-15) to a `crate::style::Color` variant.
fn ansi16_color(idx: u8) -> crate::style::Color {
    use crate::style::Color;
    match idx {
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
    }
}

// reason: flat keystroke dispatch table; splitting would obscure the trace.
#[allow(clippy::too_many_lines)]
pub(crate) fn dispatch_key(state: &mut ColorPickerState, k: KeyEvent) -> ColorPickerOutcome {
    if k.code == KeyCode::Esc {
        return ColorPickerOutcome::Cancel;
    }
    match k.code {
        // Tab cycle: Hex → Ansi16 → Bold → Italic → Underline → wrap.
        KeyCode::Tab => {
            match (state.section, state.axis_focus) {
                (PickerSection::Hex, _) => {
                    state.section = PickerSection::Ansi16;
                    state.axis_focus = AxisFocus::None;
                }
                (PickerSection::Ansi16, AxisFocus::None) => state.axis_focus = AxisFocus::Bold,
                (_, AxisFocus::Bold) => state.axis_focus = AxisFocus::Italic,
                (_, AxisFocus::Italic) => state.axis_focus = AxisFocus::Underline,
                (_, AxisFocus::Underline) => {
                    state.section = PickerSection::Hex;
                    state.axis_focus = AxisFocus::None;
                }
            }
            ColorPickerOutcome::StayOpen
        }
        // BackTab: exact reverse of the Tab cycle.
        KeyCode::BackTab => {
            match (state.section, state.axis_focus) {
                (_, AxisFocus::Bold) => {
                    state.axis_focus = AxisFocus::None;
                    state.section = PickerSection::Ansi16;
                }
                (_, AxisFocus::Italic) => state.axis_focus = AxisFocus::Bold,
                (_, AxisFocus::Underline) => state.axis_focus = AxisFocus::Italic,
                (PickerSection::Hex, AxisFocus::None) => {
                    state.section = PickerSection::Ansi16;
                    state.axis_focus = AxisFocus::Underline;
                }
                (PickerSection::Ansi16, AxisFocus::None) => state.section = PickerSection::Hex,
            }
            ColorPickerOutcome::StayOpen
        }
        KeyCode::Left if state.section == PickerSection::Ansi16 => {
            state.ansi16_idx = state.ansi16_idx.saturating_sub(1);
            ColorPickerOutcome::StayOpen
        }
        KeyCode::Right if state.section == PickerSection::Ansi16 => {
            state.ansi16_idx = (state.ansi16_idx + 1).min(15);
            ColorPickerOutcome::StayOpen
        }
        // Backspace (the intuitive delete key) OR `←` removes the last char of
        // the hex/`@NNN` buffer. Both are accepted; `Backspace` is what users
        // reach for. `pop()` on an empty buffer is a safe no-op.
        KeyCode::Backspace | KeyCode::Left if state.section == PickerSection::Hex => {
            state.hex_buf.pop();
            ColorPickerOutcome::StayOpen
        }
        // `c` IS a valid hex digit (0xC), so this arm MUST be gated on
        // `axis_focus != None`: without the gate, pressing `c` while an axis is
        // focused would fall through to the hex-digit arm and write into hex_buf
        // instead of clearing the axis (T-B2 regression pin).
        KeyCode::Char('c') if state.axis_focus != AxisFocus::None => {
            match state.axis_focus {
                AxisFocus::Bold => state.staged_bold = Some(None),
                AxisFocus::Italic => state.staged_italic = Some(None),
                AxisFocus::Underline => state.staged_underline = Some(None),
                AxisFocus::None => unreachable!("guarded by `if` clause above"),
            }
            ColorPickerOutcome::StayOpen
        }
        // Space toggles the focused bool axis: unedited/cleared/false → true;
        // true → false. Spec §3.1.
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
        // `@` switches the hex field to palette-index mode (only as first char).
        KeyCode::Char('@') if state.section == PickerSection::Hex && state.hex_buf.is_empty() => {
            state.hex_buf.push('@');
            ColorPickerOutcome::StayOpen
        }
        // Palette-index digits after `@` (0-9, up to 3 digits → max "@255").
        KeyCode::Char(c @ '0'..='9')
            if state.section == PickerSection::Hex && state.hex_buf.starts_with('@') =>
        {
            if state.hex_buf.len() < 4 {
                state.hex_buf.push(c);
            }
            ColorPickerOutcome::StayOpen
        }
        // Hex digits (0-9a-f, up to 6) in non-`@` mode.
        KeyCode::Char(c @ ('0'..='9' | 'a'..='f'))
            if state.section == PickerSection::Hex && !state.hex_buf.starts_with('@') =>
        {
            if state.hex_buf.len() < 6 {
                state.hex_buf.push(c);
            }
            ColorPickerOutcome::StayOpen
        }
        // `N` clears the active section's value.
        KeyCode::Char('N') => {
            match state.section {
                PickerSection::Hex => state.hex_buf.clear(),
                PickerSection::Ansi16 => state.ansi16_idx = 0,
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

pub(crate) fn render(frame: &mut Frame, area: Rect, state: &ColorPickerState) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Color picker — Tab field, ←→ value, Enter accept, Esc cancel");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(1), // Current indicator
        Constraint::Length(1), // spacer
        Constraint::Length(1), // Hex field
        Constraint::Length(2), // ANSI swatch row
        Constraint::Length(1), // Style axes
        Constraint::Length(1), // status/help
    ])
    .split(inner);

    render_current(frame, chunks[0], state);
    render_hex(frame, chunks[2], state);
    render_ansi16(frame, chunks[3], state);
    render_axis_row(frame, chunks[4], state);
    render_status(frame, chunks[5], state);
}

/// Top "Current:" indicator — live swatch + textual value + kind tag.
fn render_current(frame: &mut Frame, area: Rect, state: &ColorPickerState) {
    use crate::config_tui::style_ratatui::to_ratatui;
    use crate::style::Style as TayfStyle;
    let (swatch_style, value, kind) = match state.selected_color() {
        Some(c) => {
            let st = to_ratatui(TayfStyle { bg: Some(c), ..TayfStyle::DEFAULT });
            let (value, kind) = match c {
                crate::style::Color::Rgb(r, g, b) => {
                    (format!("#{r:02x}{g:02x}{b:02x}"), "truecolor")
                }
                crate::style::Color::Indexed(n) => (format!("@{n}"), "palette"),
                _ => (color_name(c).to_owned(), "ansi"),
            };
            (st, value, kind)
        }
        None => (RaStyle::default(), "—".to_owned(), "none"),
    };
    let line = Line::from(vec![
        Span::raw("Current:  "),
        Span::styled("  ", swatch_style),
        Span::raw(format!("  {value}  ({kind})")),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_hex(frame: &mut Frame, area: Rect, state: &ColorPickerState) {
    let active = state.section == PickerSection::Hex;
    let marker = if active { "▶ " } else { "  " };
    let shown = if state.hex_buf.starts_with('@') {
        state.hex_buf.clone()
    } else {
        format!("#{:<6}", state.hex_buf)
    };
    let hint = if state.hex_buf.starts_with('@') {
        "palette index 0-255"
    } else {
        "6 hex digits · @0-255 = palette"
    };
    let line = format!("{marker}Hex   {shown}      ({hint})");
    frame.render_widget(Paragraph::new(line), area);
}

fn render_ansi16(frame: &mut Frame, area: Rect, state: &ColorPickerState) {
    let active = state.section == PickerSection::Ansi16;
    let mut spans: Vec<Span> = vec![Span::raw(if active { "▶ ANSI  " } else { "  ANSI  " })];
    for i in 0..16u8 {
        let style = if i == state.ansi16_idx && active {
            RaStyle::default().bg(RaColor::Indexed(i)).fg(RaColor::White)
        } else {
            RaStyle::default().bg(RaColor::Indexed(i))
        };
        spans.push(Span::styled(format!(" {i:2} "), style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// One-line bool-axis row; the focused axis is underlined. Spec §3.1.
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
        Span::raw("Style  "),
        format_axis("bold", state.staged_bold, state.axis_focus == AxisFocus::Bold),
        Span::raw(" "),
        format_axis("italic", state.staged_italic, state.axis_focus == AxisFocus::Italic),
        Span::raw(" "),
        format_axis("underline", state.staged_underline, state.axis_focus == AxisFocus::Underline),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_status(frame: &mut Frame, area: Rect, _state: &ColorPickerState) {
    frame.render_widget(
        Paragraph::new("Tab=field  ⌫=delete  Space=toggle  c=clear  Enter=accept  Esc=cancel"),
        area,
    );
}

/// ANSI index → display name (for the Current indicator's ansi kind).
fn color_name(c: crate::style::Color) -> &'static str {
    use crate::style::Color;
    match c {
        Color::Black => "black",
        Color::Red => "red",
        Color::Green => "green",
        Color::Yellow => "yellow",
        Color::Blue => "blue",
        Color::Magenta => "magenta",
        Color::Cyan => "cyan",
        Color::White => "white",
        Color::BrightBlack => "bright-black",
        Color::BrightRed => "bright-red",
        Color::BrightGreen => "bright-green",
        Color::BrightYellow => "bright-yellow",
        Color::BrightBlue => "bright-blue",
        Color::BrightMagenta => "bright-magenta",
        Color::BrightCyan => "bright-cyan",
        Color::BrightWhite => "bright-white",
        _ => "color",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_tui::app::{App, Modal};
    use crate::config_tui::test_support::assert_render_snapshot;
    use ratatui::crossterm::event::KeyModifiers;

    fn mk(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    #[test]
    fn render_modal_color_picker_default_matches_snapshot() {
        let mut app = App::default_for_test();
        let state = ColorPickerState::default();
        app.modal = Some(Modal::ColorPicker(ColorPickerState::default()));
        assert_render_snapshot(
            80,
            24,
            &app,
            move |frame, area, _app| render(frame, area, &state),
            "modal_color_picker",
        );
    }

    #[test]
    fn hex_section_six_digits_yields_rgb() {
        // Arbitrary hex parse — deliberately NOT a built-in palette value, so
        // this test is independent of any default-color re-tone.
        let mut s = ColorPickerState { section: PickerSection::Hex, ..Default::default() };
        for c in ['a', 'b', 'c', 'd', 'e', 'f'] {
            dispatch_key(&mut s, mk(KeyCode::Char(c)));
        }
        assert_eq!(s.selected_color(), Some(crate::style::Color::Rgb(0xab, 0xcd, 0xef)));
    }

    #[test]
    fn hex_section_at_index_yields_palette() {
        let mut s = ColorPickerState { section: PickerSection::Hex, ..Default::default() };
        for c in ['@', '1', '3', '7'] {
            dispatch_key(&mut s, mk(KeyCode::Char(c)));
        }
        assert_eq!(s.hex_buf, "@137");
        assert_eq!(s.selected_color(), Some(crate::style::Color::Indexed(137)));
    }

    #[test]
    fn hex_section_at_index_out_of_range_is_none() {
        let mut s = ColorPickerState { section: PickerSection::Hex, ..Default::default() };
        for c in ['@', '9', '9', '9'] {
            dispatch_key(&mut s, mk(KeyCode::Char(c)));
        }
        assert_eq!(s.selected_color(), None, "@999 exceeds palette range");
    }

    #[test]
    fn partial_hex_is_none() {
        let mut s = ColorPickerState { section: PickerSection::Hex, ..Default::default() };
        for c in ['1', 'f', '9'] {
            dispatch_key(&mut s, mk(KeyCode::Char(c)));
        }
        assert_eq!(s.selected_color(), None, "partial hex binds nothing");
    }

    #[test]
    fn backspace_and_left_delete_hex_chars() {
        let mut s = ColorPickerState { section: PickerSection::Hex, ..Default::default() };
        for c in ['1', 'f', '9'] {
            dispatch_key(&mut s, mk(KeyCode::Char(c)));
        }
        assert_eq!(s.hex_buf, "1f9");
        dispatch_key(&mut s, mk(KeyCode::Backspace));
        assert_eq!(s.hex_buf, "1f", "Backspace removes the last hex digit");
        dispatch_key(&mut s, mk(KeyCode::Left));
        assert_eq!(s.hex_buf, "1", "Left also removes the last hex digit");
        dispatch_key(&mut s, mk(KeyCode::Backspace));
        dispatch_key(&mut s, mk(KeyCode::Backspace));
        assert_eq!(s.hex_buf, "", "Backspace on an empty buffer is a safe no-op");
    }

    #[test]
    fn backspace_deletes_the_at_palette_prefix() {
        let mut s = ColorPickerState { section: PickerSection::Hex, ..Default::default() };
        for c in ['@', '1', '3'] {
            dispatch_key(&mut s, mk(KeyCode::Char(c)));
        }
        assert_eq!(s.hex_buf, "@13");
        dispatch_key(&mut s, mk(KeyCode::Backspace));
        dispatch_key(&mut s, mk(KeyCode::Backspace));
        assert_eq!(s.hex_buf, "@");
        dispatch_key(&mut s, mk(KeyCode::Backspace));
        assert_eq!(s.hex_buf, "", "Backspace clears the @ prefix, returning to hex mode");
    }

    #[test]
    fn tab_cycle_is_hex_ansi_then_axes() {
        let mut s = ColorPickerState::default();
        assert_eq!(s.section, PickerSection::Hex, "hex is the default focus");
        dispatch_key(&mut s, mk(KeyCode::Tab));
        assert_eq!(s.section, PickerSection::Ansi16);
        assert_eq!(s.axis_focus, AxisFocus::None);
        dispatch_key(&mut s, mk(KeyCode::Tab));
        assert_eq!(s.axis_focus, AxisFocus::Bold);
        dispatch_key(&mut s, mk(KeyCode::Tab));
        assert_eq!(s.axis_focus, AxisFocus::Italic);
        dispatch_key(&mut s, mk(KeyCode::Tab));
        assert_eq!(s.axis_focus, AxisFocus::Underline);
        dispatch_key(&mut s, mk(KeyCode::Tab));
        assert_eq!(s.section, PickerSection::Hex, "wraps to hex");
        assert_eq!(s.axis_focus, AxisFocus::None);
    }

    #[test]
    fn invalid_hex_char_in_hex_section_is_ignored() {
        let mut s = ColorPickerState { section: PickerSection::Hex, ..Default::default() };
        dispatch_key(&mut s, mk(KeyCode::Char('z')));
        assert!(s.hex_buf.is_empty());
    }

    #[test]
    fn arrow_within_ansi16_moves_cursor() {
        // Navigate to Ansi16 section first (Tab from default Hex).
        let mut s = ColorPickerState::default();
        dispatch_key(&mut s, mk(KeyCode::Tab));
        assert_eq!(s.section, PickerSection::Ansi16);
        dispatch_key(&mut s, mk(KeyCode::Right));
        assert_eq!(s.ansi16_idx, 1);
        dispatch_key(&mut s, mk(KeyCode::Right));
        assert_eq!(s.ansi16_idx, 2);
        dispatch_key(&mut s, mk(KeyCode::Left));
        assert_eq!(s.ansi16_idx, 1);
    }

    #[test]
    fn hex_input_records_six_digits() {
        // In default Hex section, type 6 hex digits.
        let mut s = ColorPickerState::default();
        for c in ['f', 'f', '8', '8', '0', '0'] {
            dispatch_key(&mut s, mk(KeyCode::Char(c)));
        }
        assert_eq!(s.hex_buf, "ff8800");
    }

    #[test]
    fn n_clears_ansi16_section() {
        // Tab to Ansi16, move the cursor, then N clears it.
        let mut s = ColorPickerState::default();
        dispatch_key(&mut s, mk(KeyCode::Tab));
        s.ansi16_idx = 7;
        dispatch_key(&mut s, mk(KeyCode::Char('N')));
        assert_eq!(s.ansi16_idx, 0);
    }

    #[test]
    fn n_clears_hex_buf() {
        let mut s = ColorPickerState { section: PickerSection::Hex, ..Default::default() };
        s.hex_buf = "ff8800".to_owned();
        dispatch_key(&mut s, mk(KeyCode::Char('N')));
        assert!(s.hex_buf.is_empty());
    }

    #[test]
    fn c_in_hex_section_without_axis_focus_is_recorded_as_hex_digit() {
        let mut s = ColorPickerState { section: PickerSection::Hex, ..Default::default() };
        dispatch_key(&mut s, mk(KeyCode::Char('c')));
        assert_eq!(s.hex_buf, "c", "`c` is a hex digit when no axis is focused");
        assert_eq!(s.staged_bold, None, "no axis was cleared");
    }

    #[test]
    fn at_index_boundaries() {
        let mk_hex = |buf: &str| ColorPickerState {
            section: PickerSection::Hex,
            hex_buf: buf.to_owned(),
            ..Default::default()
        };
        assert_eq!(mk_hex("@0").selected_color(), Some(crate::style::Color::Indexed(0)));
        assert_eq!(mk_hex("@255").selected_color(), Some(crate::style::Color::Indexed(255)));
        assert_eq!(mk_hex("@256").selected_color(), None, "256 is out of palette range");
    }

    #[test]
    fn backtab_cycles_in_reverse() {
        // BackTab: exact reverse of the Tab cycle.
        // Tab order: Hex → Ansi16(None) → Bold → Italic → Underline → Hex(None)
        // BackTab from Hex: → Ansi16+Underline → Italic → Bold → Ansi16(None) → Hex
        let mut s = ColorPickerState::default(); // Hex
        dispatch_key(&mut s, mk(KeyCode::BackTab));
        assert_eq!(s.axis_focus, AxisFocus::Underline);
        assert_eq!(s.section, PickerSection::Ansi16);
        dispatch_key(&mut s, mk(KeyCode::BackTab));
        assert_eq!(s.axis_focus, AxisFocus::Italic);
        dispatch_key(&mut s, mk(KeyCode::BackTab));
        assert_eq!(s.axis_focus, AxisFocus::Bold);
        dispatch_key(&mut s, mk(KeyCode::BackTab));
        assert_eq!(s.axis_focus, AxisFocus::None);
        assert_eq!(s.section, PickerSection::Ansi16);
        dispatch_key(&mut s, mk(KeyCode::BackTab));
        assert_eq!(s.section, PickerSection::Hex);
    }
}
