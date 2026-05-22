//! Visual styling primitive and its safe rendering to ANSI SGR.
//!
//! `Style` describes a foreground/background/attribute combination.
//! `Style::to_sgr` is the *only* function in the crate that emits ANSI escape
//! bytes that surround pattern matches; it is restricted to SGR sequences
//! (`\x1b[…m` and `\x1b[0m`) and audited by a unit test. See spec §3.7.

/// 16-color ANSI base palette plus 256-indexed and 24-bit RGB.
// reason: `Color` is a complete model of the SGR color space — kept whole so
// `fg_params`/`bg_params` stay symmetric and v0.2 TOML config parsing has a
// target to land on. The v0.1 built-in rule set references only a subset of
// variants; the rest become reachable when Task 7 wires user-configurable
// rules to `Color::parse_str`. Tests in this file exercise the unused variants.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    /// Standard ANSI black (SGR 30 fg / 40 bg).
    Black,
    /// Standard ANSI red (SGR 31 fg / 41 bg).
    Red,
    /// Standard ANSI green (SGR 32 fg / 42 bg).
    Green,
    /// Standard ANSI yellow (SGR 33 fg / 43 bg).
    Yellow,
    /// Standard ANSI blue (SGR 34 fg / 44 bg).
    Blue,
    /// Standard ANSI magenta (SGR 35 fg / 45 bg).
    Magenta,
    /// Standard ANSI cyan (SGR 36 fg / 46 bg).
    Cyan,
    /// Standard ANSI white (SGR 37 fg / 47 bg).
    White,
    /// Bright ANSI black (SGR 90 fg / 100 bg).
    BrightBlack,
    /// Bright ANSI red (SGR 91 fg / 101 bg).
    BrightRed,
    /// Bright ANSI green (SGR 92 fg / 102 bg).
    BrightGreen,
    /// Bright ANSI yellow (SGR 93 fg / 103 bg).
    BrightYellow,
    /// Bright ANSI blue (SGR 94 fg / 104 bg).
    BrightBlue,
    /// Bright ANSI magenta (SGR 95 fg / 105 bg).
    BrightMagenta,
    /// Bright ANSI cyan (SGR 96 fg / 106 bg).
    BrightCyan,
    /// Bright ANSI white (SGR 97 fg / 107 bg).
    BrightWhite,
    /// 256-color palette index.
    Indexed(u8),
    /// 24-bit truecolor (r, g, b).
    Rgb(u8, u8, u8),
}

impl Color {
    /// Foreground SGR parameter list (e.g. `"31"`, `"38;5;178"`, `"38;2;255;136;0"`).
    fn fg_params(self) -> String {
        match self {
            Color::Black => "30".into(),
            Color::Red => "31".into(),
            Color::Green => "32".into(),
            Color::Yellow => "33".into(),
            Color::Blue => "34".into(),
            Color::Magenta => "35".into(),
            Color::Cyan => "36".into(),
            Color::White => "37".into(),
            Color::BrightBlack => "90".into(),
            Color::BrightRed => "91".into(),
            Color::BrightGreen => "92".into(),
            Color::BrightYellow => "93".into(),
            Color::BrightBlue => "94".into(),
            Color::BrightMagenta => "95".into(),
            Color::BrightCyan => "96".into(),
            Color::BrightWhite => "97".into(),
            Color::Indexed(n) => format!("38;5;{n}"),
            Color::Rgb(r, g, b) => format!("38;2;{r};{g};{b}"),
        }
    }

    /// Background SGR parameter list.
    fn bg_params(self) -> String {
        match self {
            Color::Black => "40".into(),
            Color::Red => "41".into(),
            Color::Green => "42".into(),
            Color::Yellow => "43".into(),
            Color::Blue => "44".into(),
            Color::Magenta => "45".into(),
            Color::Cyan => "46".into(),
            Color::White => "47".into(),
            Color::BrightBlack => "100".into(),
            Color::BrightRed => "101".into(),
            Color::BrightGreen => "102".into(),
            Color::BrightYellow => "103".into(),
            Color::BrightBlue => "104".into(),
            Color::BrightMagenta => "105".into(),
            Color::BrightCyan => "106".into(),
            Color::BrightWhite => "107".into(),
            Color::Indexed(n) => format!("48;5;{n}"),
            Color::Rgb(r, g, b) => format!("48;2;{r};{g};{b}"),
        }
    }
}

impl Color {
    /// Parse a color from a TOML configuration string.
    ///
    /// Accepts:
    /// - ANSI names: `"red"`, `"green"`, ..., `"white"`, `"bright_red"`, ..., `"bright_white"` (case-insensitive).
    /// - Indexed palette: `"color(N)"` where `0 <= N <= 255`.
    /// - 24-bit hex: `"#rrggbb"` (six hex digits, case-insensitive).
    /// - 24-bit functional: `"rgb(R, G, B)"` where each channel is `0..=255`.
    ///
    /// `pub(crate)` rather than `pub` — the exact error-message wording is
    /// not a public contract; consumers go through `config::parse_color_field`
    /// which wraps results into `Error::Config`. See CLAUDE.md §4 on
    /// public-API stability.
    ///
    /// # Errors
    /// Returns a human-readable error string on any unrecognised or
    /// out-of-range input. Callers wrap this into `Error::Config` with file
    /// path + line context.
    // reason: first caller (`config::parse_color_field`) lands in v0.2.0 Task 4;
    // until then the function is only exercised by unit tests in this file.
    #[allow(dead_code)]
    pub(crate) fn parse_str(input: &str) -> Result<Self, String> {
        let trimmed = input.trim();
        // Lowercase once for branch dispatch so `COLOR(178)`, `RGB(...)`,
        // and `#FF8800` are all accepted symmetrically with the bare ANSI
        // names. Keep `input` intact for error echo.
        let lower = trimmed.to_ascii_lowercase();

        if let Some(rest) = lower.strip_prefix('#') {
            return parse_hex(rest)
                .ok_or_else(|| format!("invalid hex color '{input}': expected '#rrggbb'"));
        }

        if let Some(inner) = lower.strip_prefix("color(").and_then(|s| s.strip_suffix(')')) {
            let n: u16 = inner.trim().parse().map_err(|_| {
                format!("invalid indexed color '{input}': expected 'color(N)' with 0 <= N <= 255")
            })?;
            if n > 255 {
                return Err(format!("invalid indexed color '{input}': index {n} > 255"));
            }
            #[allow(clippy::cast_possible_truncation)] // reason: bounded by check above
            return Ok(Color::Indexed(n as u8));
        }

        if let Some(inner) = lower.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
            let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
            if parts.len() != 3 {
                return Err(format!("invalid rgb color '{input}': expected 'rgb(R, G, B)'"));
            }
            let parse_channel = |s: &str| -> Result<u8, String> {
                s.parse::<u8>()
                    .map_err(|_| format!("invalid rgb channel '{s}' in '{input}': must be 0..=255"))
            };
            return Ok(Color::Rgb(
                parse_channel(parts[0])?,
                parse_channel(parts[1])?,
                parse_channel(parts[2])?,
            ));
        }

        // Bare ANSI name — already lower-cased above.
        match lower.as_str() {
            "black" => Ok(Color::Black),
            "red" => Ok(Color::Red),
            "green" => Ok(Color::Green),
            "yellow" => Ok(Color::Yellow),
            "blue" => Ok(Color::Blue),
            "magenta" => Ok(Color::Magenta),
            "cyan" => Ok(Color::Cyan),
            "white" => Ok(Color::White),
            "bright_black" => Ok(Color::BrightBlack),
            "bright_red" => Ok(Color::BrightRed),
            "bright_green" => Ok(Color::BrightGreen),
            "bright_yellow" => Ok(Color::BrightYellow),
            "bright_blue" => Ok(Color::BrightBlue),
            "bright_magenta" => Ok(Color::BrightMagenta),
            "bright_cyan" => Ok(Color::BrightCyan),
            "bright_white" => Ok(Color::BrightWhite),
            other => Err(format!(
                "unknown color name '{other}'; expected an ANSI name (e.g. 'red', 'bright_cyan'), 'color(N)', '#rrggbb', or 'rgb(R, G, B)'"
            )),
        }
    }
}

// reason: helper for `Color::parse_str`; same dead-code window — first
// non-test caller arrives with v0.2.0 Task 4.
#[allow(dead_code)]
fn parse_hex(rest: &str) -> Option<Color> {
    if rest.len() != 6 || !rest.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&rest[0..2], 16).ok()?;
    let g = u8::from_str_radix(&rest[2..4], 16).ok()?;
    let b = u8::from_str_radix(&rest[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

/// Visual styling for a pattern match.
// reason: v0.1 models only four SGR attributes (bold, dim, italic, underline).
// SGR has more (reverse, strikethrough, blink, hidden, etc.); v0.2 will migrate
// to a `bitflags!` set if/when the additional attributes are scoped in.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    /// Foreground color, if any.
    pub fg: Option<Color>,
    /// Background color, if any.
    pub bg: Option<Color>,
    /// Bold attribute (SGR 1).
    pub bold: bool,
    /// Italic attribute (SGR 3).
    pub italic: bool,
    /// Underline attribute (SGR 4).
    pub underline: bool,
    /// Dim attribute (SGR 2).
    pub dim: bool,
}

impl Style {
    /// Convenient const for builder-style construction in `rules.rs`.
    pub const DEFAULT: Style =
        Style { fg: None, bg: None, bold: false, italic: false, underline: false, dim: false };

    /// Render this style as an opening SGR escape sequence.
    ///
    /// Returns an empty string if the style would have no visible effect.
    /// Guaranteed to emit only `\x1b[…m` — see `tests::to_sgr_emits_only_sgr_sequences`,
    /// the audit gate for escape injection (CLAUDE.md §3, spec §3.7).
    #[must_use]
    pub fn to_sgr(self) -> String {
        let mut params: Vec<String> = Vec::new();

        if self.bold {
            params.push("1".into());
        }
        if self.dim {
            params.push("2".into());
        }
        if self.italic {
            params.push("3".into());
        }
        if self.underline {
            params.push("4".into());
        }
        if let Some(fg) = self.fg {
            params.push(fg.fg_params());
        }
        if let Some(bg) = self.bg {
            params.push(bg.bg_params());
        }

        if params.is_empty() {
            String::new()
        } else {
            format!("\x1b[{}m", params.join(";"))
        }
    }

    /// The single canonical SGR reset sequence.
    #[must_use]
    pub fn reset_sgr() -> &'static str {
        "\x1b[0m"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sgr_pattern() -> regex::Regex {
        // Permits CSI parameter list (digits and semicolons) followed by 'm',
        // and the reset sequence \x1b[0m.
        regex::Regex::new(r"^(?:\x1b\[(?:\d+;)*\d+m|\x1b\[0m)$").unwrap()
    }

    #[test]
    fn to_sgr_emits_only_sgr_sequences() {
        let cases: Vec<Style> = vec![
            Style {
                fg: Some(Color::Red),
                bg: None,
                bold: false,
                italic: false,
                underline: false,
                dim: false,
            },
            Style {
                fg: Some(Color::BrightYellow),
                bg: None,
                bold: true,
                italic: false,
                underline: false,
                dim: false,
            },
            Style {
                fg: Some(Color::Indexed(178)),
                bg: Some(Color::Indexed(0)),
                bold: false,
                italic: false,
                underline: false,
                dim: false,
            },
            Style {
                fg: Some(Color::Rgb(255, 136, 0)),
                bg: None,
                bold: false,
                italic: true,
                underline: true,
                dim: false,
            },
            Style::default(),
        ];

        let re = sgr_pattern();
        for style in &cases {
            let sgr = style.to_sgr();
            if sgr.is_empty() {
                continue; // Default style may render to empty string.
            }
            assert!(re.is_match(&sgr), "style {style:?} emitted non-SGR: {sgr:?}");
        }
    }

    #[test]
    fn reset_emits_zero_m() {
        assert_eq!(Style::reset_sgr(), "\x1b[0m");
    }

    #[test]
    fn default_style_renders_empty_or_zero_m() {
        let s = Style::default().to_sgr();
        assert!(s == "\x1b[0m" || s.is_empty(), "default produced: {s:?}");
    }

    #[test]
    fn empty_sgr_iff_no_visible_effect() {
        // Audit gate: the only legitimate way to_sgr produces an empty string is
        // a Style with no fg, no bg, and every attribute false. If a future edit
        // ever lets to_sgr return "" while *some* effect was requested, this
        // test catches it before the audit gate can be silently bypassed.
        let visible_cases: Vec<Style> = vec![
            Style { fg: Some(Color::Red), ..Style::DEFAULT },
            Style { bg: Some(Color::Blue), ..Style::DEFAULT },
            Style { bold: true, ..Style::DEFAULT },
            Style { dim: true, ..Style::DEFAULT },
            Style { italic: true, ..Style::DEFAULT },
            Style { underline: true, ..Style::DEFAULT },
        ];
        for s in &visible_cases {
            assert!(!s.to_sgr().is_empty(), "visible style {s:?} unexpectedly produced empty SGR");
        }
        // Inverse direction: only Style::DEFAULT (all None / all false) produces "".
        assert!(Style::DEFAULT.to_sgr().is_empty());
        assert!(Style::default().to_sgr().is_empty());
    }

    #[test]
    fn parse_ansi_basic_names() {
        assert_eq!(Color::parse_str("red"), Ok(Color::Red));
        assert_eq!(Color::parse_str("black"), Ok(Color::Black));
        assert_eq!(Color::parse_str("white"), Ok(Color::White));
    }

    #[test]
    fn parse_ansi_bright_names() {
        assert_eq!(Color::parse_str("bright_red"), Ok(Color::BrightRed));
        assert_eq!(Color::parse_str("bright_cyan"), Ok(Color::BrightCyan));
    }

    #[test]
    fn parse_indexed_form() {
        assert_eq!(Color::parse_str("color(0)"), Ok(Color::Indexed(0)));
        assert_eq!(Color::parse_str("color(178)"), Ok(Color::Indexed(178)));
        assert_eq!(Color::parse_str("color(255)"), Ok(Color::Indexed(255)));
    }

    #[test]
    fn parse_indexed_overflow_rejected() {
        assert!(Color::parse_str("color(256)").is_err());
        assert!(Color::parse_str("color(-1)").is_err());
        assert!(Color::parse_str("color()").is_err());
    }

    #[test]
    fn parse_hex_six_digit() {
        assert_eq!(Color::parse_str("#ff8800"), Ok(Color::Rgb(0xff, 0x88, 0x00)));
        assert_eq!(Color::parse_str("#FFFFFF"), Ok(Color::Rgb(0xff, 0xff, 0xff)));
        assert_eq!(Color::parse_str("#000000"), Ok(Color::Rgb(0, 0, 0)));
    }

    #[test]
    fn parse_hex_invalid_lengths_rejected() {
        // Three-digit short form not supported in v0.2.0 — keep parser strict.
        assert!(Color::parse_str("#fff").is_err());
        assert!(Color::parse_str("#ff").is_err());
        assert!(Color::parse_str("#gggggg").is_err());
    }

    #[test]
    fn parse_rgb_function_form() {
        assert_eq!(Color::parse_str("rgb(255, 136, 0)"), Ok(Color::Rgb(255, 136, 0)));
        assert_eq!(Color::parse_str("rgb(0,0,0)"), Ok(Color::Rgb(0, 0, 0)));
    }

    #[test]
    fn parse_rgb_overflow_rejected() {
        assert!(Color::parse_str("rgb(256, 0, 0)").is_err());
        assert!(Color::parse_str("rgb(1, 2)").is_err());
        assert!(Color::parse_str("rgb()").is_err());
    }

    #[test]
    fn parse_unknown_name_returns_friendly_error() {
        let err = Color::parse_str("turquoise").unwrap_err();
        assert!(err.contains("turquoise"), "error must echo input: {err}");
        assert!(
            err.contains("color name") || err.contains("recognised") || err.contains("color("),
            "error must hint at accepted formats: {err}"
        );
    }

    #[test]
    fn parse_case_insensitive_names() {
        assert_eq!(Color::parse_str("RED"), Ok(Color::Red));
        assert_eq!(Color::parse_str("Bright_Blue"), Ok(Color::BrightBlue));
    }

    #[test]
    fn parse_case_insensitive_functional_forms() {
        // `COLOR(178)`, `RGB(...)`, and `#FF8800` are accepted symmetrically
        // with ANSI names — no surprising case asymmetry.
        assert_eq!(Color::parse_str("COLOR(178)"), Ok(Color::Indexed(178)));
        assert_eq!(Color::parse_str("RGB(255, 136, 0)"), Ok(Color::Rgb(255, 136, 0)));
        assert_eq!(Color::parse_str("#FF8800"), Ok(Color::Rgb(0xff, 0x88, 0x00)));
        // Mixed case also fine.
        assert_eq!(Color::parse_str("Rgb(0, 0, 0)"), Ok(Color::Rgb(0, 0, 0)));
    }
}
