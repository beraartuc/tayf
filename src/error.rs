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
    #[error("could not determine shell: {0}")]
    ShellDiscovery(String),

    /// PTY operation failed (open, spawn, read, write, resize).
    #[error("PTY operation failed: {0}")]
    Pty(#[from] io::Error),

    /// Terminal control failed (termios get/set, ioctl).
    #[error("terminal control failed: {0}")]
    Tty(#[from] nix::errno::Errno),

    /// Built-in or user regex failed to compile.
    #[error("regex compilation failed: {0}")]
    RegexCompile(#[from] regex::Error),

    /// Signal handler installation failed.
    #[error("signal installation failed: {0}")]
    Signal(io::Error),

    /// A line exceeded the buffer cap; flushed as-is without rule application.
    /// Non-fatal — logged via tracing, not returned to caller.
    #[error("line buffer exceeded {cap} bytes; flushing as-is")]
    BufferOverflow {
        /// The cap (in bytes) that was exceeded.
        cap: usize,
    },
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, Error>;
