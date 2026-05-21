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

/// Query the controlling terminal's window size via `TIOCGWINSZ`.
///
/// Returns `Some((rows, cols))` on success and `None` when the ioctl fails —
/// the common cases being stdout not being a TTY (e.g. redirected to a file
/// or pipe) or the kernel rejecting the request. Pixel dimensions reported
/// by the kernel are discarded; v0.1 has no consumer for them, and the
/// `PtySize` callers already pass `pixel_{width,height}: 0` on the fallback
/// path.
///
/// This is the sole TIOCGWINSZ call site in v0.1.1; the PTY-spawn and
/// `SIGWINCH` paths both route through here.
// reason: crate-wide policy is `warn(unsafe_code)` with SAFETY comments; the
// `-D warnings` gate would otherwise reject the ioctl call.
#[allow(unsafe_code)]
pub(crate) fn winsize() -> Option<(u16, u16)> {
    use nix::libc::{ioctl, winsize as LibcWinsize, STDOUT_FILENO, TIOCGWINSZ};
    // SAFETY: `LibcWinsize` is a `#[repr(C)]` plain-old-data struct of
    // integer fields, so the all-zero bit pattern is a valid inhabitant
    // (`std::mem::zeroed()` is sound).
    let mut ws: LibcWinsize = unsafe { std::mem::zeroed() };
    // SAFETY: `STDOUT_FILENO` is a fd we own (the process's standard output;
    // never closed by tayf itself). `TIOCGWINSZ` is a read-only ioctl: the
    // kernel writes into the `LibcWinsize` we pass and reads nothing else
    // from our address space. The pointer comes from `addr_of_mut!` on a
    // local that no other reference observes for the duration of the call,
    // so exclusivity holds. On any failure (non-TTY, EINVAL, ...) we return
    // `None` and `ws` is dropped untouched.
    #[allow(clippy::useless_conversion)] // reason: TIOCGWINSZ type differs per-target
    let rc = unsafe { ioctl(STDOUT_FILENO, TIOCGWINSZ as _, std::ptr::addr_of_mut!(ws)) };
    if rc == 0 {
        Some((ws.ws_row, ws.ws_col))
    } else {
        None
    }
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
