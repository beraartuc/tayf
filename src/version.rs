//! Build-time info surfaced by `--version`.
//!
//! Constants are populated by `build.rs` via `cargo:rustc-env=...`.
//! `PKG_VERSION` comes from `CARGO_PKG_VERSION`, which Cargo always sets, so
//! it does not need a build-script intermediary. The git SHA, dirty flag, and
//! rustc version are all emitted by `build.rs` and fall back to `"unknown"` /
//! `"0"` when the source tree is not a git checkout.

pub(crate) const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) const GIT_SHA: &str = env!("TAYF_GIT_SHA");
pub(crate) const GIT_DIRTY: &str = env!("TAYF_GIT_DIRTY");
pub(crate) const RUSTC_VERSION: &str = env!("TAYF_RUSTC");

/// Format the version banner shown by `--version`.
///
/// clap prepends the binary name (`tayf `) to this string, so the result is
/// the metadata block only: `<version> (sha <hash>[-dirty], rustc <rustc>)`.
#[must_use]
pub fn version_string() -> String {
    let suffix = if GIT_DIRTY == "1" { "-dirty" } else { "" };
    format!("{PKG_VERSION} (sha {GIT_SHA}{suffix}, rustc {RUSTC_VERSION})")
}
