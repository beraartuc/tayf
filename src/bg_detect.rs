//! Background color detection for automatic light/dark theme resolution.
//!
//! Startup-only resolver: COLORFGBG env var → OSC 11 query → dark fallback.
//! See spec §3 for the full algorithm and timing budget.
//!
//! Reference: xterm Operating System Commands ("OSC") — sequence 11 reports
//! the terminal's default background color via the response
//! `\e]11;rgb:RRRR/GGGG/BBBB\e\\` (or BEL-terminated, or 8-bit C1 ST
//! terminated). Rec. 601 weighted luminance (Y = 0.299·R + 0.587·G +
//! 0.114·B) decides light vs dark with threshold 0.5 (inclusive → Light).
//!
//! Termios + panic safety: the OSC 11 path opens `/dev/tty`, snapshots
//! termios, installs a process-wide panic hook that restores on panic
//! (necessary because release builds use `panic = "abort"` and Drop does
//! NOT run on panic), switches to raw mode, queries, reads with a
//! `nix::poll::poll` timeout, then restores termios via Drop. The panic
//! hook clears its slot on Drop so subsequent panics don't re-apply a
//! stale termios.

/// Resolved background theme. Maps directly to v0.2.3 preset theme names:
/// `BgTheme::Light` → `"light"`, `BgTheme::Dark` → `"dark"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BgTheme {
    Light,
    Dark,
}

impl BgTheme {
    /// String identifier matching `themes::load` registry. Stable.
    #[allow(dead_code)]
    // reason: subsequent v0.3.1 tasks wire this into the theme resolution
    // path in `lib.rs`; the skeleton commit lands the API surface first.
    pub(crate) fn as_theme_name(self) -> &'static str {
        match self {
            BgTheme::Light => "light",
            BgTheme::Dark => "dark",
        }
    }
}

/// Resolve the effective background theme by trying detection paths in
/// order. Never panics. Falls back to `BgTheme::Dark` on any failure.
///
/// Time budget: ≤ 100 ms wall clock for the OSC 11 path; COLORFGBG path
/// is synchronous and zero-I/O. See spec §3.4.
///
/// Side effects: may briefly toggle termios on `/dev/tty` if it reaches
/// the OSC 11 path. All paths restore termios on return (including panic).
#[allow(dead_code)]
// reason: wired into `Tayf::run` by a subsequent v0.3.1 task; this commit
// lands only the module skeleton so the API surface is reviewable in
// isolation.
pub(crate) fn resolve() -> BgTheme {
    if let Some(t) = detect_from_colorfgbg() {
        debug_log(format_args!("bg_detect: colorfgbg -> {t:?}"));
        return t;
    }
    if let Some(t) = detect_from_osc11() {
        debug_log(format_args!("bg_detect: osc11 -> {t:?}"));
        return t;
    }
    debug_log(format_args!("bg_detect: fallback -> Dark"));
    BgTheme::Dark
}

/// Emit a debug-level trace via the in-crate `log` module. The crate's
/// `log` module exposes `warn_msg!` and `info_msg!` macros but no
/// `debug_msg!`; we route through the lower-level [`crate::log::emit`]
/// entry point directly, gated on the latched log level so the path is
/// zero-cost when `TAYF_LOG` is unset (the default).
fn debug_log(args: std::fmt::Arguments<'_>) {
    if crate::log::enabled(crate::log::LogLevel::Debug) {
        crate::log::emit(crate::log::LogLevel::Debug, args);
    }
}

// Subsequent tasks fill in `detect_from_osc11` and its helpers. Stub it
// with `None` so this module compiles cleanly until that task lands.
fn detect_from_colorfgbg() -> Option<BgTheme> {
    let raw = std::env::var("COLORFGBG").ok()?;
    parse_colorfgbg(&raw)
}

/// Parse the COLORFGBG environment variable.
///
/// rxvt / urxvt format: `fg;bg` where bg is an xterm color number 0..15.
/// Some implementations include a third field (`fg;bd;bg`) for default-bd
/// status; we accept both by consulting only the last `;`-separated field.
/// Value `default` (any case) is rejected — no useful signal.
fn parse_colorfgbg(s: &str) -> Option<BgTheme> {
    let bg = s.split(';').next_back()?;
    let bg = bg.trim();
    if bg.is_empty() || bg.eq_ignore_ascii_case("default") {
        return None;
    }
    let n: u8 = bg.parse().ok()?;
    if n > 15 {
        return None;
    }
    if n < 8 {
        Some(BgTheme::Dark)
    } else {
        Some(BgTheme::Light)
    }
}

fn detect_from_osc11() -> Option<BgTheme> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colorfgbg_two_field_dark() {
        assert_eq!(parse_colorfgbg("15;0"), Some(BgTheme::Dark));
    }

    #[test]
    fn colorfgbg_two_field_light() {
        assert_eq!(parse_colorfgbg("0;15"), Some(BgTheme::Light));
    }

    #[test]
    fn colorfgbg_three_field_with_non_numeric_middle() {
        // Parser must use only the last field; middle is ignored.
        assert_eq!(parse_colorfgbg("0;garbage;15"), Some(BgTheme::Light));
    }

    #[test]
    fn colorfgbg_default_keyword_returns_none() {
        assert_eq!(parse_colorfgbg("0;default"), None);
        assert_eq!(parse_colorfgbg("0;DEFAULT"), None);
    }

    #[test]
    fn colorfgbg_malformed_returns_none() {
        assert_eq!(parse_colorfgbg(""), None);
        assert_eq!(parse_colorfgbg("abc"), None);
        assert_eq!(parse_colorfgbg("0;99"), None);
        assert_eq!(parse_colorfgbg("0;-1"), None);
    }
}
