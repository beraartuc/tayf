//! `Style` → `ratatui::style::Style` mapping for TUI preview rendering.
//!
//! Kept out of `src/style.rs` to avoid ratatui dep in the core style
//! module (spec §14.1 #11 decision).

use ratatui::style::{Color as RatColor, Modifier, Style as RatStyle};

use crate::style::{Color, Style};

/// Convert tayf's `Style` to ratatui's `Style`. Maps fg/bg colors and
/// the 4 bool axes (bold/italic/underline/dim — see `src/style.rs:418-435`
/// for v0.1 axis scope; strikethrough/reverse not modeled in tayf).
pub(crate) fn to_ratatui(style: Style) -> RatStyle {
    let mut s = RatStyle::default();
    if let Some(fg) = style.fg {
        s = s.fg(color_to_ratatui(fg));
    }
    if let Some(bg) = style.bg {
        s = s.bg(color_to_ratatui(bg));
    }
    let mut m = Modifier::empty();
    if style.bold {
        m |= Modifier::BOLD;
    }
    if style.italic {
        m |= Modifier::ITALIC;
    }
    if style.underline {
        m |= Modifier::UNDERLINED;
    }
    if style.dim {
        m |= Modifier::DIM;
    }
    s.add_modifier(m)
}

fn color_to_ratatui(c: Color) -> RatColor {
    match c {
        Color::Black => RatColor::Black,
        Color::Red => RatColor::Red,
        Color::Green => RatColor::Green,
        Color::Yellow => RatColor::Yellow,
        Color::Blue => RatColor::Blue,
        Color::Magenta => RatColor::Magenta,
        Color::Cyan => RatColor::Cyan,
        Color::White => RatColor::White,
        Color::BrightBlack => RatColor::DarkGray,
        Color::BrightRed => RatColor::LightRed,
        Color::BrightGreen => RatColor::LightGreen,
        Color::BrightYellow => RatColor::LightYellow,
        Color::BrightBlue => RatColor::LightBlue,
        Color::BrightMagenta => RatColor::LightMagenta,
        Color::BrightCyan => RatColor::LightCyan,
        Color::BrightWhite => RatColor::Gray,
        Color::Indexed(n) => RatColor::Indexed(n),
        Color::Rgb(r, g, b) => RatColor::Rgb(r, g, b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_to_ratatui_named_colors_map_one_to_one() {
        let s = Style { fg: Some(Color::Red), ..Default::default() };
        let r = to_ratatui(s);
        assert_eq!(r.fg, Some(RatColor::Red));
    }

    #[test]
    fn style_to_ratatui_indexed_color_maps_to_ratatui_color_indexed() {
        let s = Style { fg: Some(Color::Indexed(178)), ..Default::default() };
        let r = to_ratatui(s);
        assert_eq!(r.fg, Some(RatColor::Indexed(178)));
    }

    #[test]
    fn style_to_ratatui_rgb_color_maps_to_ratatui_color_rgb() {
        let s = Style { fg: Some(Color::Rgb(255, 136, 0)), ..Default::default() };
        let r = to_ratatui(s);
        assert_eq!(r.fg, Some(RatColor::Rgb(255, 136, 0)));
    }

    #[test]
    fn style_to_ratatui_bool_axes_combine_modifiers() {
        let s =
            Style { bold: true, italic: true, underline: true, dim: true, ..Default::default() };
        let r = to_ratatui(s);
        let want = Modifier::BOLD | Modifier::ITALIC | Modifier::UNDERLINED | Modifier::DIM;
        assert_eq!(r.add_modifier, want);
    }

    #[test]
    fn style_to_ratatui_default_style_yields_empty_ratatui_style() {
        let r = to_ratatui(Style::default());
        assert_eq!(r.fg, None);
        assert_eq!(r.bg, None);
        assert_eq!(r.add_modifier, Modifier::empty());
    }

    #[test]
    fn style_to_ratatui_semantics_match_to_sgr_for_color_and_axes() {
        let s = Style { fg: Some(Color::Cyan), bold: true, ..Default::default() };
        let sgr = s.to_sgr();
        assert!(sgr.contains('1'), "bold present in SGR");
        assert!(sgr.contains("36"), "cyan param present in SGR");
        let r = to_ratatui(s);
        assert_eq!(r.fg, Some(RatColor::Cyan));
        assert!(r.add_modifier.contains(Modifier::BOLD));
    }
}
