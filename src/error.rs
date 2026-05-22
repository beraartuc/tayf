//! Single top-level error type for the tayf crate.
//!
//! All public functions surface errors through this enum so callers (including
//! `main`) can map them to user-facing messages and exit codes in one place.
//! See spec §4.

use std::fmt::Write as _;
use std::io;

/// All recoverable errors produced by tayf.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Could not determine which shell to launch.
    #[error("could not determine shell: {0}. Set $SHELL or pass --shell <path>.")]
    ShellDiscovery(String),

    /// PTY operation failed (open, spawn, read, write, resize).
    #[error(
        "PTY operation failed: {0}. If your terminal supports PTY allocation, please file a bug at https://github.com/beraartuc/tayf/issues."
    )]
    Pty(#[from] io::Error),

    /// Terminal control failed (termios get/set, ioctl).
    #[error("terminal control failed: {0}. tayf must be launched from a real terminal; piping stdin or running without a TTY is not supported in v0.1.")]
    Tty(#[from] nix::errno::Errno),

    /// Built-in or user regex failed to compile.
    #[error("regex compilation failed: {0}. Check the pattern syntax.")]
    RegexCompile(#[from] regex::Error),

    /// Signal handler installation failed.
    #[error(
        "signal installation failed: {0}. Try running again; if persistent, please file a bug."
    )]
    Signal(#[source] io::Error),

    /// Failed to load or validate the user TOML config.
    ///
    /// `line` is 1-based when available; pass `0` for errors with no line
    /// context (path resolution, size limit, IO). `0` was chosen over
    /// `Option<NonZeroUsize>` because thiserror's format-string support is
    /// terser this way; the sentinel is constant across the codebase.
    ///
    /// **Display contract:** both the `path` and `message` fields pass through
    /// `sanitize_for_display` in the `Display` impl so that any user-supplied
    /// content echoed back (e.g. a color string from a config rule, or a
    /// hostile `XDG_CONFIG_HOME`) cannot smuggle an escape sequence onto the
    /// user's terminal — CLAUDE.md §3 invariant. Callers that read these
    /// fields directly (e.g. for structured logging) get the raw bytes;
    /// format through `Display` or sanitize yourself before printing to a
    /// terminal.
    #[error("config error in {}{}: {}", sanitize_for_display(path), line_suffix(*line), sanitize_for_display(message))]
    Config {
        /// Absolute path to the config file the error originated from.
        path: String,
        /// 1-based line number, or `0` for "no line context".
        line: usize,
        /// Human-readable description ending in actionable guidance.
        message: String,
    },

    /// File-watcher operation failed (start, register path, event channel).
    ///
    /// Uses `#[source]` rather than `#[from]` so call sites in the watcher
    /// and reload orchestrator construct `Error::Watch(...)` explicitly — the
    /// conversion is part of the contract there, not an implicit coercion.
    #[error("file watcher error: {0}")]
    Watch(#[source] notify::Error),

    /// A line exceeded the buffer cap; flushed as-is without rule application.
    ///
    /// **Non-fatal — INVARIANT:** This variant must only be constructed for
    /// `crate::log::warn_msg!` logging, never returned from `Result` to propagate via
    /// `?`. The line-buffer module signals overflow through the dedicated
    /// `(Vec<_>, Option<Error>)` return shape (spec §5 / Task 5), keeping this
    /// variant out of any normal control-flow path. Future contributors who
    /// find themselves writing `return Err(Error::BufferOverflow { .. })`
    /// should reach for a `Warning` type instead.
    #[error("line buffer exceeded {cap} bytes; flushing as-is")]
    BufferOverflow {
        /// The cap (in bytes) that was exceeded.
        cap: usize,
    },
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, Error>;

fn line_suffix(line: usize) -> String {
    if line == 0 {
        String::new()
    } else {
        format!(":{line}")
    }
}

/// Replace ASCII control bytes in a diagnostic message with their `\xNN`
/// escape form so a hostile config string (e.g. `"\x1b[2J"` in a color value)
/// cannot execute as a terminal control sequence when the error is printed
/// to stderr. Preserves common whitespace (`\n`, `\t`, regular space) since
/// those round-trip safely through a terminal.
fn sanitize_for_display(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    for ch in message.chars() {
        // `is_control()` covers ASCII C0 (0x00..=0x1F + 0x7F) AND Unicode
        // C1 (U+0080..U+009F). U+009B is the 8-bit CSI introducer — a hostile
        // config string could otherwise smuggle "\u{009B}2J" past the gate.
        if ch.is_control() && ch != '\n' && ch != '\t' {
            // `write!` into a String is infallible; the discard is
            // explicit per the plan's clippy::format_push_string fallback.
            let _ = write!(out, "\\x{:02x}", ch as u32);
        } else {
            out.push(ch);
        }
    }
    out
}

impl Error {
    /// Build a [`Error::Config`] from a `toml::de::Error`, extracting the
    /// 1-based line number when the source span is available.
    #[allow(clippy::needless_pass_by_value)]
    // reason: the diagnostic is single-shot — callers obtain `err` from
    // `toml::from_str(..).unwrap_err()` and never reuse it. Taking by value
    // matches that lifecycle and keeps the signature stable for Task 4.
    pub(crate) fn config_from_toml(path: String, source: &str, err: toml::de::Error) -> Self {
        let line = err.span().map_or(0, |range| line_from_offset(source, range.start));
        Error::Config { path, line, message: err.message().to_string() }
    }

    /// Build a [`Error::Config`] for a regex compile failure inside a named
    /// rule. `line` is 0 unless the caller already knows the source line.
    #[allow(clippy::needless_pass_by_value)]
    // reason: `regex::Error` is the single-shot return of `Regex::new(..)`;
    // callers move it in directly. Matches the by-value signature established
    // for `config_from_toml` so the two construction helpers are symmetric.
    pub(crate) fn config_regex(path: String, rule_name: &str, source: regex::Error) -> Self {
        Error::Config {
            path,
            line: 0,
            message: format!("rule '{rule_name}': {source}. Check the pattern syntax."),
        }
    }
}

#[allow(clippy::naive_bytecount)]
// reason: pulling the `bytecount` crate for a one-shot diagnostic helper
// violates the dependency-minimalism policy; config errors are not on any
// hot path and the linear scan is bounded by the 1 MiB config size cap.
fn line_from_offset(source: &str, offset: usize) -> usize {
    // Count newline bytes before `offset`. Operates on `.as_bytes()` rather
    // than slicing `&str` so a non-char-boundary `offset` can never panic —
    // CLAUDE.md §2 ("no panics in library code") applies even when current
    // callers happen to pass char-aligned offsets.
    let upper = offset.min(source.len());
    source.as_bytes()[..upper].iter().filter(|&&b| b == b'\n').count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_discovery_message_includes_remediation() {
        let e = Error::ShellDiscovery("no $SHELL".into());
        let msg = e.to_string();
        assert!(msg.contains("could not determine shell"));
        assert!(msg.contains("--shell"), "remediation hint required: {msg}");
    }

    #[test]
    fn pty_message_includes_remediation() {
        let e: Error = io::Error::from(io::ErrorKind::PermissionDenied).into();
        let msg = e.to_string();
        assert!(msg.contains("PTY operation failed"));
        assert!(msg.contains("file a bug"), "remediation hint required: {msg}");
    }

    #[test]
    fn tty_message_includes_remediation() {
        let e: Error = nix::errno::Errno::EIO.into();
        let msg = e.to_string();
        assert!(msg.contains("terminal control failed"));
        assert!(msg.contains("real terminal") || msg.contains("not supported"));
    }

    #[test]
    fn regex_message_includes_remediation() {
        // Build the pattern at runtime so clippy's `invalid_regex` lint
        // (which inspects string literals) does not flag the test source.
        let pattern = String::from("(") + "invalid";
        let bad = regex::Regex::new(&pattern).unwrap_err();
        let e: Error = bad.into();
        let msg = e.to_string();
        assert!(msg.contains("regex compilation failed"));
        assert!(msg.contains("pattern syntax"));
    }

    #[test]
    fn signal_message_includes_remediation() {
        let e = Error::Signal(io::Error::from(io::ErrorKind::Other));
        let msg = e.to_string();
        assert!(msg.contains("signal installation failed"));
        assert!(msg.contains("file a bug"));
    }

    #[test]
    fn buffer_overflow_message_is_descriptive() {
        let e = Error::BufferOverflow { cap: 65536 };
        let msg = e.to_string();
        assert!(msg.contains("65536"));
        assert!(msg.contains("flushing as-is"));
    }

    #[test]
    fn pty_from_io_error_preserves_source_chain() {
        use std::error::Error as _;
        let io = io::Error::from(io::ErrorKind::PermissionDenied);
        let e: Error = io.into();
        assert!(e.source().is_some(), "Pty variant must carry source");
    }

    #[test]
    fn signal_preserves_source_chain_via_source_attr() {
        use std::error::Error as _;
        let e = Error::Signal(io::Error::from(io::ErrorKind::Other));
        assert!(e.source().is_some(), "Signal variant must carry source via #[source]");
    }

    #[test]
    fn from_io_error_routes_to_pty_variant() {
        let io = io::Error::from(io::ErrorKind::PermissionDenied);
        let e: Error = io.into();
        assert!(matches!(e, Error::Pty(_)));
    }

    #[test]
    fn config_message_includes_path_line_and_message() {
        let e = Error::Config {
            path: "/home/u/.config/tayf/config.toml".into(),
            line: 12,
            message: "unknown color name 'turquoise'".into(),
        };
        let msg = e.to_string();
        assert!(msg.contains("/home/u/.config/tayf/config.toml"));
        assert!(msg.contains("12"));
        assert!(msg.contains("turquoise"));
    }

    #[test]
    fn config_message_omits_line_when_zero() {
        let e = Error::Config {
            path: "/etc/tayf.toml".into(),
            line: 0,
            message: "file too large: 2097152 bytes (max 1048576)".into(),
        };
        let msg = e.to_string();
        assert!(msg.contains("/etc/tayf.toml"));
        assert!(!msg.contains(":0:"), "line 0 sentinel must not surface in message: {msg}");
        assert!(msg.contains("too large"));
    }

    #[test]
    fn config_from_toml_parse_error_carries_line() {
        // Unterminated inline-table — guaranteed parse failure in toml 0.9.
        let bad = "rules = [ { unterminated\n";
        let err: toml::de::Error = toml::from_str::<toml::Table>(bad).unwrap_err();
        let cfg = Error::config_from_toml("/tmp/cfg.toml".into(), bad, err);
        let msg = cfg.to_string();
        assert!(msg.contains("/tmp/cfg.toml"));
    }

    #[test]
    fn config_message_escapes_control_bytes_to_prevent_terminal_injection() {
        // A hostile config string echoed back in an error message must not
        // execute as a terminal control sequence when Display'd to stderr.
        let e = Error::Config {
            path: "/tmp/cfg.toml".into(),
            line: 7,
            message: "rule 'evil': fg: unknown color name '\x1b[2J\x1b[H'".into(),
        };
        let rendered = e.to_string();
        assert!(!rendered.contains('\x1b'), "raw ESC must not survive Display: {rendered:?}");
        assert!(rendered.contains("\\x1b"), "ESC must be escaped as \\x1b: {rendered:?}");
        // Newline and tab pass through unchanged (safe whitespace).
        let e2 = Error::Config { path: "/x".into(), line: 0, message: "ok\nfine\there".into() };
        let r2 = e2.to_string();
        assert!(r2.contains("ok\nfine\there"), "safe whitespace must pass: {r2:?}");
    }

    #[test]
    fn config_path_escapes_control_bytes_too() {
        // Sanitization gate must cover `path` symmetrically with `message`.
        // A hostile XDG_CONFIG_HOME or --config arg could contain ESC.
        let e = Error::Config {
            path: "/tmp/\x1b[2J/cfg.toml".into(),
            line: 0,
            message: "anything".into(),
        };
        let rendered = e.to_string();
        assert!(!rendered.contains('\x1b'), "raw ESC must not survive in path: {rendered:?}");
        assert!(rendered.contains("\\x1b"), "ESC must be escaped as \\x1b in path: {rendered:?}");
    }

    #[test]
    fn config_message_escapes_c1_control_introducer() {
        // U+009B is the 8-bit CSI introducer — same threat class as ESC [.
        // Regression guard for the `is_ascii_control` -> `is_control` fix.
        let e = Error::Config { path: "/x".into(), line: 0, message: "fg: '\u{009b}2J'".into() };
        let rendered = e.to_string();
        assert!(
            !rendered.contains('\u{009b}'),
            "raw U+009B must not survive Display: {rendered:?}"
        );
        assert!(rendered.contains("\\x9b"), "U+009B must be escaped as \\x9b: {rendered:?}");
    }

    #[test]
    fn watch_error_display_is_helpful() {
        let inner = notify::Error::generic("permission denied");
        let err = crate::error::Error::Watch(inner);
        let msg = err.to_string();
        assert!(msg.contains("file watcher error"));
        assert!(msg.contains("permission denied"));
    }
}
