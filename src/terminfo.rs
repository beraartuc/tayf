//! Terminal capability detection: TTY status and color depth.
//!
//! Provides two queries used by the colorization pipeline to decide whether
//! and how aggressively to emit SGR sequences:
//!
//! - [`stdout_is_tty`] — is our standard output attached to a real terminal,
//!   or has it been redirected to a file/pipe (in which case we should pass
//!   bytes through untouched)?
//! - [`detect_depth`] — the richest [`ColorDepth`] supported by the terminal,
//!   inferred from `$COLORTERM` and `$TERM`.
//!
//! The pure decision function [`depth_from_env`] is split out so that depth
//! inference is testable without mutating the process environment.

use std::env;

/// Maximum color depth supported by the terminal we are connected to.
// reason: v0.1 always emits the basic 16-color SGR palette (see `style.rs`)
// so the facade has no consumer for depth yet. Detection is implemented and
// tested so v0.2 can branch on it without churn. Same rationale applies to
// `detect_depth` and `depth_from_env` below.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorDepth {
    /// `TERM=dumb` or no color allowed.
    None,
    /// 16-color ANSI.
    Basic16,
    /// 256-color indexed palette.
    Indexed256,
    /// 24-bit truecolor.
    Truecolor,
}

/// Check whether stdout is connected to a terminal.
pub(crate) fn stdout_is_tty() -> bool {
    use nix::libc::STDOUT_FILENO;
    use nix::unistd::isatty;
    isatty(STDOUT_FILENO).unwrap_or(false)
}

/// Detect the maximum supported color depth from `$COLORTERM` and `$TERM`.
#[allow(dead_code)] // reason: see `ColorDepth` above.
pub(crate) fn detect_depth() -> ColorDepth {
    let colorterm = env::var("COLORTERM").ok();
    let term = env::var("TERM").ok();
    depth_from_env(colorterm.as_deref(), term.as_deref())
}

#[allow(dead_code)] // reason: see `ColorDepth` above; exercised by unit tests.
fn depth_from_env(colorterm: Option<&str>, term: Option<&str>) -> ColorDepth {
    if matches!(term, Some("dumb")) {
        return ColorDepth::None;
    }
    if let Some(ct) = colorterm {
        if ct.eq_ignore_ascii_case("truecolor") || ct == "24bit" {
            return ColorDepth::Truecolor;
        }
    }
    if let Some(t) = term {
        if t.contains("256color") {
            return ColorDepth::Indexed256;
        }
    }
    ColorDepth::Basic16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_from_colorterm_truecolor() {
        assert_eq!(
            depth_from_env(Some("truecolor"), Some("xterm-256color")),
            ColorDepth::Truecolor
        );
        assert_eq!(depth_from_env(Some("24bit"), None), ColorDepth::Truecolor);
    }

    #[test]
    fn depth_from_term_256() {
        assert_eq!(depth_from_env(None, Some("xterm-256color")), ColorDepth::Indexed256);
        assert_eq!(depth_from_env(None, Some("screen-256color")), ColorDepth::Indexed256);
    }

    #[test]
    fn depth_falls_back_to_basic() {
        assert_eq!(depth_from_env(None, Some("xterm")), ColorDepth::Basic16);
        assert_eq!(depth_from_env(None, None), ColorDepth::Basic16);
    }

    #[test]
    fn depth_handles_dumb_term() {
        assert_eq!(depth_from_env(None, Some("dumb")), ColorDepth::None);
    }
}
