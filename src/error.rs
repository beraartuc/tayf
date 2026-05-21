//! Single top-level error type for the tayf crate.
//!
//! All public functions surface errors through this enum so callers (including
//! `main`) can map them to user-facing messages and exit codes in one place.
//! See spec §4.

use std::io;

/// All recoverable errors produced by tayf.
#[derive(Debug, thiserror::Error)]
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
}
