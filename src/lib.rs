//! tayf — terminal-agnostic, PTY-based, regex-driven output colorizer.
//!
//! Public entry: [`Tayf::run`]. See `docs/superpowers/specs/2026-05-21-tayf-v0.1-design.md`
//! for full design.

// Crate-wide policy: unsafe is permitted only with a SAFETY comment;
// reviewer enforces. See src/pty.rs::current_term_size for the sole use
// in v0.1 (TIOCGWINSZ ioctl).
#![warn(unsafe_code)]
#![warn(clippy::pedantic, clippy::cargo)]
#![allow(clippy::module_name_repetitions)]
// reason: transitive dependency duplicates (bitflags 1/2, nix 0.25/0.27) come from
// portable-pty and signal-hook; we cannot dedupe without forking upstream crates.
#![allow(clippy::multiple_crate_versions)]

pub mod cli;
pub mod error;
pub mod style;
pub mod version;

pub(crate) mod line_buffer;
pub(crate) mod logging;
pub(crate) mod pipeline;
pub(crate) mod pty;
pub(crate) mod rules;
pub(crate) mod shell;
pub(crate) mod terminfo;
pub(crate) mod tty_guard;

pub use error::{Error, Result};

/// Placeholder facade. Implemented in Task 16.
pub struct Tayf;

impl Tayf {
    /// Placeholder; real implementation in Task 16.
    #[must_use]
    pub fn placeholder() -> &'static str {
        "tayf"
    }
}
