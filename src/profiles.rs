//! Profile loading, validation, and merging.
//!
//! A *profile* is a named bundle of rule configuration. v0.5.2 ships
//! the **mechanism only**; the curated profile library lands in
//! v0.5.3 (see umbrella vision §3.4).
//!
//! Profiles are loaded from disk
//! (`~/.config/tayf/profiles/<name>.toml`) or from embedded sources
//! (`assets/profiles/<name>.toml`, none shipped in v0.5.2). Discovery
//! and canonicalisation mirror [`crate::themes`] exactly — same file
//! layout (no `[profile.X]` header in either location; file name is
//! the profile name), same predicate for valid names
//! ([`crate::themes::name_is_valid`]), same path-traversal guards.
//!
//! ## Public API (crate-internal)
//!
//! - [`Profile`] / [`ProfileRule`] — parsed TOML body.
//! - [`load`] — discover + parse + validate a named profile.
//! - [`validate_profile`] — Phase 1 fail-collected shape check.
//! - [`name_is_valid`] — re-export of [`crate::themes::name_is_valid`].
//! - [`synthetic_path`] — `<embedded:profile/{name}>` label.

// v0.5.2 Phase 2 lands the foundation: types + load + validate_profile.
// The orchestration call sites (`lib.rs`, `reload.rs`) + the
// `RuleSource::EmbeddedProfile` dispatch wiring land in subsequent
// Phase 3+ tasks. Until those callers exist, the module-level
// `allow(dead_code)` suppresses dead-code warnings on intentionally
// public-to-the-crate symbols. The attribute is REMOVED in Phase 3
// once the dispatch + orchestration call sites land.
#![allow(dead_code)]

use serde::Deserialize;
use std::collections::BTreeMap;

use crate::error::{Error, ProfileErrorKind, Result};

/// Parsed profile TOML body.
///
/// All fields are optional with sensible defaults. `Profile::default()`
/// is the "no-op" profile (no whitelist filter, no `append_rules`, no
/// theme override) — semantically equivalent to not using `--profile`
/// at all.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Profile {
    /// Whitelist of built-in rule names. `None` = "all built-ins kept"
    /// (current default behaviour). `Some(vec![])` = "filter all
    /// built-ins out" (only `append_rules` apply).
    #[serde(default)]
    pub(crate) rules: Option<Vec<String>>,

    /// New rules added by the profile. Each entry MUST have a pattern
    /// (unlike `UserRule.pattern: Option<String>`) — a profile rule
    /// without a pattern has no behaviour to append.
    #[serde(default)]
    pub(crate) append_rules: Vec<ProfileRule>,

    /// Optional theme override. Resolved per the precedence chain in
    /// `crate::lib` / `crate::reload`: CLI `--theme` > config
    /// `[general] theme` > `profile.theme` > bg-detect default.
    #[serde(default)]
    pub(crate) theme: Option<String>,
}

/// A rule defined in a profile's `[[append_rules]]` block.
///
/// Mandatory `pattern` (unlike `UserRule.pattern: Option<String>`) — a
/// profile rule without a pattern has no behaviour to append.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileRule {
    /// Profile-local rule name. Must pass [`name_is_valid`] and must
    /// not collide with a built-in rule.
    pub(crate) name: String,
    /// Regex pattern. Compiled at Phase 2 ([`crate::rules`] dispatch).
    pub(crate) pattern: String,
    /// Optional whole-match style.
    #[serde(default)]
    pub(crate) style: Option<crate::config::UserStyle>,
    /// Optional capture-group style map. Keys validated at Phase 2.
    #[serde(default)]
    pub(crate) styles: Option<BTreeMap<String, crate::config::UserStyle>>,
}

/// Result of [`load`]. Carries the parsed profile + a label for the
/// source path (canonical disk path or `<embedded:profile/{name}>`).
#[derive(Debug)]
pub(crate) struct LoadedProfile {
    /// The parsed and Phase-1-validated profile body.
    pub profile: Profile,
    /// `<embedded:profile/{name}>` for embedded; canonical disk path
    /// for disk-loaded profiles.
    pub path_label: String,
}

/// Re-export of [`crate::themes::name_is_valid`] — profile names share
/// the same predicate as theme names (v0.5.1 spec §11.1 I-3 mandate).
pub(crate) fn name_is_valid(name: &str) -> bool {
    crate::themes::name_is_valid(name)
}

/// Synthetic path label used when feeding an embedded profile (or
/// reporting a load failure that happened before disk discovery
/// completed) through the load pipeline. Pure formatting — kept here
/// so call sites need not duplicate the convention.
pub(crate) fn synthetic_path(name: &str) -> String {
    format!("<embedded:profile/{name}>")
}

/// Load a profile by name. Reads `$XDG_CONFIG_HOME` and `$HOME` from
/// the environment for disk discovery; embedded profiles are not
/// shipped in v0.5.2.
///
/// On success returns [`LoadedProfile`]. On failure returns
/// [`Error::Profile`] (single-error: `NotFound`, `ParseError`,
/// `PathCanonicalization`) or [`Error::ProfileValidation`]
/// (fail-collected Phase 1 violations).
///
/// # Errors
/// See variants of [`ProfileErrorKind`] + [`ProfileRuleErrorKind`].
pub(crate) fn load(name: &str) -> Result<LoadedProfile> {
    load_with(
        name,
        || std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from),
        || std::env::var_os("HOME").map(std::path::PathBuf::from),
    )
}

/// Testable variant of [`load`]; accepts env-var closures so unit
/// tests can scope `$XDG_CONFIG_HOME` and `$HOME` to a `tempdir`
/// without mutating the process environment.
#[cfg_attr(not(test), allow(dead_code))]
// reason: production callers go through `load`; the env-closure
// variant exists for unit tests that need to scope discovery to a
// tempdir without mutating the process environment.
pub(crate) fn load_with(
    name: &str,
    xdg: impl FnOnce() -> Option<std::path::PathBuf>,
    home: impl FnOnce() -> Option<std::path::PathBuf>,
) -> Result<LoadedProfile> {
    // Stub body — full discovery + canonicalisation + parse +
    // validate flow lands in Task 3. The skeleton returns
    // ProfileErrorKind::NotFound unconditionally so lib.rs wiring +
    // type-checks succeed before Task 3 fills in the body.
    let _ = (xdg, home);
    Err(Error::Profile {
        name: name.to_owned(),
        source_path: synthetic_path(name),
        kind: ProfileErrorKind::NotFound { searched: Vec::new() },
    })
}

/// Validate the shape of a parsed profile (Phase 1). Fail-collected —
/// every violation across the whole profile is gathered into a single
/// [`Error::ProfileValidation`].
///
/// Phase 2 (capture-group key dispatch on `append_rules.styles` maps)
/// happens later in `Compiled::load_with_theme` — covered by
/// `RuleSource::EmbeddedProfile` dispatch arms (v0.5.2 Phase 3 work).
///
/// # Errors
/// Returns [`Error::ProfileValidation`] with at least one
/// [`ProfileRuleError`] when any Phase 1 violation is found. Returns
/// `Ok(())` when the parsed profile passes Phase 1.
#[allow(clippy::unnecessary_wraps)]
// reason: stub body for Task 2 scaffold — Task 4 fills in the
// fail-collected validation pass, which returns Err on any violation.
pub(crate) fn validate_profile(name: &str, source_path: &str, profile: &Profile) -> Result<()> {
    // Stub body — full Phase 1 fail-collected validation lands in
    // Task 4.
    let _ = (name, source_path, profile);
    Ok(())
}

#[cfg(test)]
mod tests {
    // Unit tests added in Task 6.
}
