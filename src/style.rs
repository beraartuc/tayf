//! Visual styling primitive and its safe rendering to ANSI SGR.
//!
//! `Style` describes a foreground/background/attribute combination.
//! `Style::to_sgr` is the *only* function in the crate that emits ANSI escape
//! bytes that surround pattern matches; it is restricted to SGR sequences
//! (`\x1b[…m` and `\x1b[0m`) and audited by a unit test. See spec §3.7.

/// 16-color ANSI base palette plus 256-indexed and 24-bit RGB.
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

fn parse_hex(rest: &str) -> Option<Color> {
    if rest.len() != 6 || !rest.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&rest[0..2], 16).ok()?;
    let g = u8::from_str_radix(&rest[2..4], 16).ok()?;
    let b = u8::from_str_radix(&rest[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

impl Color {
    /// Return the closest representable color for `depth`, or `None` if the
    /// terminal cannot display color at all (`ColorDepth::None`).
    ///
    /// Approximation strategy:
    /// - Truecolor: identity.
    /// - Indexed256: ANSI / Indexed → unchanged; Rgb → 6×6×6 xterm cube
    ///   quantization (`16 + 36*r + 6*g + b` with each channel in `0..=5`).
    /// - Basic16: ANSI → unchanged; Indexed `0..=15` → matching ANSI;
    ///   Indexed `>=16` and Rgb → nearest ANSI by RGB Euclidean distance.
    /// - None: always `None`.
    ///
    /// `pub(crate)` rather than `pub` — only `Compiled::load` calls it.
    #[must_use]
    pub(crate) fn downgrade(self, depth: crate::terminfo::ColorDepth) -> Option<Color> {
        use crate::terminfo::ColorDepth as D;
        // reason: arms intentionally grouped by `ColorDepth` rather than by
        // RHS to keep the depth-major dispatch table readable; merging the
        // three `Some(c)` arms across depths would obscure intent.
        #[allow(clippy::match_same_arms)]
        match (depth, self) {
            (D::None, _) => None,
            (D::Truecolor, c) => Some(c),
            (D::Indexed256, Color::Rgb(r, g, b)) => Some(Color::Indexed(rgb_to_xterm_256(r, g, b))),
            (D::Indexed256, c) => Some(c),
            (
                D::Basic16,
                c @ (Color::Black
                | Color::Red
                | Color::Green
                | Color::Yellow
                | Color::Blue
                | Color::Magenta
                | Color::Cyan
                | Color::White
                | Color::BrightBlack
                | Color::BrightRed
                | Color::BrightGreen
                | Color::BrightYellow
                | Color::BrightBlue
                | Color::BrightMagenta
                | Color::BrightCyan
                | Color::BrightWhite),
            ) => Some(c),
            (D::Basic16, Color::Indexed(n)) if n < 16 => Some(ansi_from_low_index(n)),
            (D::Basic16, Color::Indexed(n)) => {
                let (r, g, b) = xterm_256_to_rgb(n);
                Some(nearest_ansi_basic(r, g, b))
            }
            (D::Basic16, Color::Rgb(r, g, b)) => Some(nearest_ansi_basic(r, g, b)),
        }
    }
}

impl Style {
    /// Apply [`Color::downgrade`] to both `fg` and `bg`. Attribute bits
    /// (`bold`, `italic`, ...) are preserved as-is — they render on every
    /// terminal that supports SGR at all.
    ///
    /// `pub(crate)` rather than `pub` — only `Compiled::load` calls it.
    #[must_use]
    pub(crate) fn downgrade(self, depth: crate::terminfo::ColorDepth) -> Self {
        Style {
            fg: self.fg.and_then(|c| c.downgrade(depth)),
            bg: self.bg.and_then(|c| c.downgrade(depth)),
            ..self
        }
    }
}

/// Map an 8-bit RGB triple into xterm's 6×6×6 color cube (indices 16..=231).
fn rgb_to_xterm_256(r: u8, g: u8, b: u8) -> u8 {
    // Pure grayscale gets the 232..=255 ramp (24 levels).
    if r == g && g == b {
        if r < 8 {
            return 16;
        }
        if r > 246 {
            return 231;
        }
        // u16 conversion avoids any overflow.
        let level = (u16::from(r) - 8) / 10;
        // level ∈ 0..=23 by `if r > 246` guard above; 232 + level ∈ 232..=255 fits u8.
        #[allow(clippy::cast_possible_truncation)] // reason: bounded 0..=23
        return 232 + level as u8;
    }
    16 + 36 * quantize(r) + 6 * quantize(g) + quantize(b)
}

/// Quantize a single 8-bit channel to the xterm cube level `0..=5`.
fn quantize(v: u8) -> u8 {
    // Cube levels: 0, 95, 135, 175, 215, 255.
    match v {
        0..=47 => 0,
        48..=114 => 1,
        115..=154 => 2,
        155..=194 => 3,
        195..=234 => 4,
        _ => 5,
    }
}

/// Inverse of `rgb_to_xterm_256` for indices `16..=255` (RGB approximation).
fn xterm_256_to_rgb(n: u8) -> (u8, u8, u8) {
    if n < 16 {
        // Low palette — return canonical ANSI rgb approximations.
        let (r, g, b) = ansi_rgb_approx(n);
        return (r, g, b);
    }
    if n >= 232 {
        let level = u16::from(n - 232) * 10 + 8;
        #[allow(clippy::cast_possible_truncation)] // reason: level fits u8
        let v = level as u8;
        return (v, v, v);
    }
    let idx = n - 16;
    let r = idx / 36;
    let g = (idx % 36) / 6;
    let b = idx % 6;
    (cube_level(r), cube_level(g), cube_level(b))
}

fn cube_level(q: u8) -> u8 {
    match q {
        0 => 0,
        1 => 95,
        2 => 135,
        3 => 175,
        4 => 215,
        _ => 255,
    }
}

fn ansi_from_low_index(n: u8) -> Color {
    match n {
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

fn ansi_rgb_approx(n: u8) -> (u8, u8, u8) {
    // Standard VGA approximations of the 16-color ANSI palette.
    match n {
        0 => (0, 0, 0),
        1 => (170, 0, 0),
        2 => (0, 170, 0),
        3 => (170, 85, 0),
        4 => (0, 0, 170),
        5 => (170, 0, 170),
        6 => (0, 170, 170),
        7 => (170, 170, 170),
        8 => (85, 85, 85),
        9 => (255, 85, 85),
        10 => (85, 255, 85),
        11 => (255, 255, 85),
        12 => (85, 85, 255),
        13 => (255, 85, 255),
        14 => (85, 255, 255),
        _ => (255, 255, 255),
    }
}

fn nearest_ansi_basic(r: u8, g: u8, b: u8) -> Color {
    let mut best = (u32::MAX, 0u8);
    for n in 0u8..=15u8 {
        let (ar, ag, ab) = ansi_rgb_approx(n);
        let dr = i32::from(r) - i32::from(ar);
        let dg = i32::from(g) - i32::from(ag);
        let db = i32::from(b) - i32::from(ab);
        // u32 cast safe — squares of i32 differences in [-255, 255] sum to <= 3*255^2.
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        // reason: positive sum bounded
        let dist = (dr * dr + dg * dg + db * db) as u32;
        if dist < best.0 {
            best = (dist, n);
        }
    }
    ansi_from_low_index(best.1)
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

    use crate::terminfo::ColorDepth;

    #[test]
    fn downgrade_truecolor_keeps_rgb() {
        let c = Color::Rgb(255, 136, 0);
        assert_eq!(c.downgrade(ColorDepth::Truecolor), Some(c));
        assert_eq!(Color::Indexed(178).downgrade(ColorDepth::Truecolor), Some(Color::Indexed(178)));
        assert_eq!(Color::Red.downgrade(ColorDepth::Truecolor), Some(Color::Red));
    }

    #[test]
    fn downgrade_rgb_to_indexed256_uses_6x6x6_cube() {
        // Pure orange (255, 136, 0) with the quantize() table below:
        //   r=255 → q=5  (in `_=>5`)
        //   g=136 → q=2  (in `115..=154 => 2`)
        //   b=0   → q=0
        //   index = 16 + 36*5 + 6*2 + 0 = 208
        assert_eq!(
            Color::Rgb(255, 136, 0).downgrade(ColorDepth::Indexed256),
            Some(Color::Indexed(208))
        );
        // Pure black takes the grayscale fast path (r==g==b<8 → 16).
        assert_eq!(Color::Rgb(0, 0, 0).downgrade(ColorDepth::Indexed256), Some(Color::Indexed(16)));
        // Pure white takes the grayscale fast path (r==g==b>248 → 231).
        assert_eq!(
            Color::Rgb(255, 255, 255).downgrade(ColorDepth::Indexed256),
            Some(Color::Indexed(231))
        );
    }

    #[test]
    fn downgrade_grayscale_ramp_boundaries() {
        // The xterm grayscale ramp covers indices 232..=255 with formula
        // RGB = (i - 232) * 10 + 8; cube white at 231 covers (255,255,255).
        // The nearest-neighbor cutoff between ramp top (238,238,238) and cube
        // white (255,255,255) sits at r=246.5, so r<=246 routes to ramp, r>=247
        // to cube white. Regression guard for the r=248 overflow bug (commit
        // c195d59 pre-fix would panic in debug / miscolor in release).

        // Black side of the ramp.
        assert_eq!(Color::Rgb(7, 7, 7).downgrade(ColorDepth::Indexed256), Some(Color::Indexed(16)));
        assert_eq!(
            Color::Rgb(8, 8, 8).downgrade(ColorDepth::Indexed256),
            Some(Color::Indexed(232))
        );

        // Top of the ramp — boundary that triggered the overflow.
        assert_eq!(
            Color::Rgb(238, 238, 238).downgrade(ColorDepth::Indexed256),
            Some(Color::Indexed(255))
        );
        assert_eq!(
            Color::Rgb(246, 246, 246).downgrade(ColorDepth::Indexed256),
            Some(Color::Indexed(255))
        );

        // Crossing the cutoff — cube white wins from here on, with no panic.
        assert_eq!(
            Color::Rgb(247, 247, 247).downgrade(ColorDepth::Indexed256),
            Some(Color::Indexed(231))
        );
        assert_eq!(
            Color::Rgb(248, 248, 248).downgrade(ColorDepth::Indexed256),
            Some(Color::Indexed(231))
        );
        assert_eq!(
            Color::Rgb(255, 255, 255).downgrade(ColorDepth::Indexed256),
            Some(Color::Indexed(231))
        );
    }

    #[test]
    fn downgrade_rgb_to_basic16_uses_nearest_ansi() {
        // Pure red (255,0,0) vs ansi_rgb_approx table:
        //   Red (170,0,0):       d² = 85² = 7225
        //   BrightRed (255,85,85): d² = 0 + 85² + 85² = 14450
        // Red wins.
        assert_eq!(Color::Rgb(255, 0, 0).downgrade(ColorDepth::Basic16), Some(Color::Red));
        // Pure black → Black (d² = 0).
        assert_eq!(Color::Rgb(0, 0, 0).downgrade(ColorDepth::Basic16), Some(Color::Black));
        // Pure white (255,255,255) vs:
        //   White (170,170,170): d² = 3*85² = 21675
        //   BrightWhite (255,255,255): d² = 0
        // BrightWhite wins.
        assert_eq!(
            Color::Rgb(255, 255, 255).downgrade(ColorDepth::Basic16),
            Some(Color::BrightWhite)
        );
    }

    #[test]
    fn downgrade_indexed_to_basic16() {
        // Standard 0..=15 indexed map directly to ANSI 16.
        assert_eq!(Color::Indexed(0).downgrade(ColorDepth::Basic16), Some(Color::Black));
        assert_eq!(Color::Indexed(1).downgrade(ColorDepth::Basic16), Some(Color::Red));
        assert_eq!(Color::Indexed(9).downgrade(ColorDepth::Basic16), Some(Color::BrightRed));
        assert_eq!(Color::Indexed(15).downgrade(ColorDepth::Basic16), Some(Color::BrightWhite));
    }

    #[test]
    fn downgrade_indexed_keeps_self_at_256() {
        assert_eq!(
            Color::Indexed(178).downgrade(ColorDepth::Indexed256),
            Some(Color::Indexed(178))
        );
    }

    #[test]
    fn downgrade_indexed_above_16_to_basic16_via_cube_roundtrip() {
        // Indexed(208) is xterm cube index: idx=208-16=192, r=192/36=5, g=(192%36)/6=2, b=192%6=0
        // → cube_level(5,2,0) = (255, 135, 0).
        // Distance² against ansi_rgb_approx:
        //   Yellow   (170,85,0):   (255-170)² + (135-85)² + 0 = 85² + 50² = 7225+2500 = 9725
        //   BrightRed (255,85,85): 0 + (135-85)² + (0-85)²    = 50² + 85² = 2500+7225 = 9725
        // Tie at 9725. `nearest_ansi_basic` iterates n ascending and uses strict `<`,
        // so the first-seen (Yellow at n=3) wins over BrightRed (n=9).
        assert_eq!(Color::Indexed(208).downgrade(ColorDepth::Basic16), Some(Color::Yellow));
    }

    #[test]
    fn downgrade_ansi_basic_passthrough() {
        for depth in [ColorDepth::Basic16, ColorDepth::Indexed256, ColorDepth::Truecolor] {
            assert_eq!(Color::Red.downgrade(depth), Some(Color::Red));
            assert_eq!(Color::BrightYellow.downgrade(depth), Some(Color::BrightYellow));
        }
    }

    #[test]
    fn downgrade_to_none_yields_no_color() {
        assert_eq!(Color::Red.downgrade(ColorDepth::None), None);
        assert_eq!(Color::Rgb(1, 2, 3).downgrade(ColorDepth::None), None);
        assert_eq!(Color::Indexed(178).downgrade(ColorDepth::None), None);
    }

    #[test]
    fn style_downgrade_preserves_attributes() {
        let s = Style {
            fg: Some(Color::Rgb(255, 136, 0)),
            bg: None,
            bold: true,
            italic: true,
            underline: false,
            dim: false,
        };
        let d = s.downgrade(ColorDepth::None);
        assert_eq!(d.fg, None);
        assert!(d.bold);
        assert!(d.italic);
    }

    #[test]
    fn style_downgrade_maps_both_fg_and_bg() {
        let s = Style {
            fg: Some(Color::Rgb(255, 0, 0)),
            bg: Some(Color::Rgb(0, 0, 0)),
            ..Style::DEFAULT
        };
        let d = s.downgrade(ColorDepth::Basic16);
        assert_eq!(d.fg, Some(Color::Red));
        assert_eq!(d.bg, Some(Color::Black));
    }
}
