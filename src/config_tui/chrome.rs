//! Chrome accent palette ("Brass & Slate") for the config TUI.
//!
//! Single source of truth for every *chrome* color — tab strip, borders,
//! section headers, selected-row highlight, modal titles, key hints. Chrome is
//! the TUI's own furniture; it is never applied to colorized *content* (rule
//! samples and the live-preview strip keep the user's real colors).
//!
//! Public API: [`AccentPalette`] + [`AccentPalette::from_bg`] and the ratatui
//! `Style` role helpers (`tab_active`, `tab_inactive`, `border`, `header`,
//! `selection`, `modal_title`, `hint`). Renderers ask for a role, never an RGB.
//!
//! Invariant: warm brass focus (`tab_active_bg` == `header` == `modal_title`)
//! over cool slate neutrals (`tab_inactive_fg` == `hint_dim`), retuned per
//! background for AA contrast. Token values: spec §7.2.

use ratatui::style::{Color, Modifier, Style};

use crate::bg_detect::BgTheme;

/// The "Brass & Slate" chrome palette for one background polarity.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AccentPalette {
    pub(crate) tab_active_bg: Color,
    pub(crate) tab_active_fg: Color,
    pub(crate) tab_inactive_fg: Color,
    pub(crate) border: Color,
    pub(crate) header: Color,
    pub(crate) selection_bg: Color,
    pub(crate) selection_fg: Color,
    pub(crate) modal_title: Color,
    pub(crate) hint_dim: Color,
}

impl AccentPalette {
    /// Build the per-background "Brass & Slate" palette (spec §7.2).
    pub(crate) fn from_bg(bg: BgTheme) -> Self {
        match bg {
            BgTheme::Dark => Self {
                tab_active_bg: Color::Rgb(0xd8, 0xa6, 0x57),
                tab_active_fg: Color::Rgb(0x0e, 0x0e, 0x12),
                tab_inactive_fg: Color::Rgb(0x6b, 0x72, 0x80),
                border: Color::Rgb(0x56, 0x5f, 0x73),
                header: Color::Rgb(0xd8, 0xa6, 0x57),
                selection_bg: Color::Rgb(0x3a, 0x2f, 0x1c),
                selection_fg: Color::Rgb(0xf0, 0xed, 0xe6),
                modal_title: Color::Rgb(0xd8, 0xa6, 0x57),
                hint_dim: Color::Rgb(0x6b, 0x72, 0x80),
            },
            BgTheme::Light => Self {
                tab_active_bg: Color::Rgb(0x9a, 0x6b, 0x1e),
                tab_active_fg: Color::Rgb(0xf7, 0xf7, 0xf5),
                tab_inactive_fg: Color::Rgb(0x8a, 0x8f, 0x99),
                border: Color::Rgb(0x8a, 0x93, 0xa3),
                header: Color::Rgb(0x9a, 0x6b, 0x1e),
                selection_bg: Color::Rgb(0xf0, 0xe4, 0xcf),
                selection_fg: Color::Rgb(0x2a, 0x21, 0x18),
                modal_title: Color::Rgb(0x9a, 0x6b, 0x1e),
                hint_dim: Color::Rgb(0x8a, 0x8f, 0x99),
            },
        }
    }

    /// Active-tab chip: brass fill, base-bg text, bold.
    pub(crate) fn tab_active(self) -> Style {
        Style::default().fg(self.tab_active_fg).bg(self.tab_active_bg).add_modifier(Modifier::BOLD)
    }

    /// Inactive tab: muted slate, recedes.
    pub(crate) fn tab_inactive(self) -> Style {
        Style::default().fg(self.tab_inactive_fg)
    }

    /// Pane / modal / divider border.
    pub(crate) fn border(self) -> Style {
        Style::default().fg(self.border)
    }

    /// Section title: brass, bold.
    pub(crate) fn header(self) -> Style {
        Style::default().fg(self.header).add_modifier(Modifier::BOLD)
    }

    /// Selected list row: brass-tint fill + high-contrast text.
    pub(crate) fn selection(self) -> Style {
        Style::default().fg(self.selection_fg).bg(self.selection_bg)
    }

    /// Modal title: brass, bold.
    pub(crate) fn modal_title(self) -> Style {
        Style::default().fg(self.modal_title).add_modifier(Modifier::BOLD)
    }

    /// Dim contextual key-hint line.
    pub(crate) fn hint(self) -> Style {
        Style::default().fg(self.hint_dim).add_modifier(Modifier::DIM)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_bg_dark_and_light_differ_on_focus() {
        let d = AccentPalette::from_bg(BgTheme::Dark);
        let l = AccentPalette::from_bg(BgTheme::Light);
        assert_ne!(d.tab_active_bg, l.tab_active_bg, "brass focus retuned per bg");
        assert_eq!(d.tab_active_bg, Color::Rgb(0xd8, 0xa6, 0x57), "dark brass token");
        assert_eq!(l.tab_active_bg, Color::Rgb(0x9a, 0x6b, 0x1e), "light brass token");
    }

    #[test]
    fn focus_roles_share_one_hue() {
        let d = AccentPalette::from_bg(BgTheme::Dark);
        assert_eq!(d.tab_active_bg, d.header, "header reuses brass focus");
        assert_eq!(d.tab_active_bg, d.modal_title, "modal title reuses brass focus");
        assert_eq!(d.tab_inactive_fg, d.hint_dim, "hint reuses muted slate");
    }

    #[test]
    fn tab_active_style_is_brass_chip_bold() {
        let s = AccentPalette::from_bg(BgTheme::Dark).tab_active();
        assert_eq!(s.fg, Some(Color::Rgb(0x0e, 0x0e, 0x12)));
        assert_eq!(s.bg, Some(Color::Rgb(0xd8, 0xa6, 0x57)));
        assert!(s.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn hint_style_is_dim_slate() {
        let s = AccentPalette::from_bg(BgTheme::Dark).hint();
        assert_eq!(s.fg, Some(Color::Rgb(0x6b, 0x72, 0x80)));
        assert!(s.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn tab_active_style_light_uses_brass_bg() {
        let s = AccentPalette::from_bg(BgTheme::Light).tab_active();
        assert_eq!(s.bg, Some(Color::Rgb(0x9a, 0x6b, 0x1e)), "light brass chip bg");
        assert_eq!(s.fg, Some(Color::Rgb(0xf7, 0xf7, 0xf5)), "light chip text");
    }
}
