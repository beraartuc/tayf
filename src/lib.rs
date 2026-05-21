//! tayf — terminal-agnostic, PTY-based, regex-driven output colorizer.
//!
//! Public entry: [`Tayf::run`]. See `docs/superpowers/specs/2026-05-21-tayf-v0.1-design.md`
//! for full design.

#![deny(unsafe_code)]
#![warn(clippy::pedantic, clippy::cargo)]
#![allow(clippy::module_name_repetitions)]
// reason: transitive dependency duplicates (bitflags 1/2, nix 0.25/0.27) come from
// portable-pty and signal-hook; we cannot dedupe without forking upstream crates.
#![allow(clippy::multiple_crate_versions)]

// Modules are added in subsequent tasks.

/// Placeholder facade. Implemented in Task 16.
pub struct Tayf;

impl Tayf {
    /// Placeholder; real implementation in Task 16.
    #[must_use]
    pub fn placeholder() -> &'static str {
        "tayf"
    }
}
