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

use crate::error::{Error, ProfileErrorKind, ProfileRuleError, ProfileRuleErrorKind, Result};

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
/// shipped in v0.5.2 (library lands in v0.5.3).
///
/// On success returns [`LoadedProfile`]. On failure returns
/// [`Error::Profile`] (single-error: `NotFound`, `ParseError`,
/// `PathCanonicalization`) or [`Error::ProfileValidation`]
/// (fail-collected Phase 1 violations).
///
/// # Errors
/// See variants of [`ProfileErrorKind`] +
/// [`crate::error::ProfileRuleErrorKind`].
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
///
/// Mirrors `themes::load_with`. Discovery order:
/// 1. Disk: `<config_base>/profiles/<name>.toml`.
/// 2. Embedded: none shipped in v0.5.2 (reserved for v0.5.3).
///
/// # Errors
/// See [`load`].
pub(crate) fn load_with(
    name: &str,
    xdg: impl FnOnce() -> Option<std::path::PathBuf>,
    home: impl FnOnce() -> Option<std::path::PathBuf>,
) -> Result<LoadedProfile> {
    // 1. Name predicate — fail fast on path separators / traversal /
    //    empty. Defense-in-depth: name reaches us from CLI args or
    //    config TOML (both adversarial channels). Surface as
    //    NotFound with empty searched list — the predicate failure
    //    is the diagnostic.
    if !name_is_valid(name) {
        return Err(Error::Profile {
            name: name.to_owned(),
            source_path: synthetic_path(name),
            kind: ProfileErrorKind::NotFound { searched: Vec::new() },
        });
    }

    // 2. Resolve `<config_base>` once via the shared helper so
    //    XDG_CONFIG_HOME / HOME handling stays in lockstep with the
    //    rest of the crate (config.rs, themes.rs).
    let base = crate::config::config_base(xdg, home);
    let mut searched: Vec<std::path::PathBuf> = Vec::new();

    // 3. Disk path candidate. None when neither XDG nor HOME is set.
    if let Some(base) = base.as_deref() {
        let profiles_dir = base.join("profiles");
        let candidate = profiles_dir.join(format!("{name}.toml"));
        searched.push(candidate.clone());

        if candidate.exists() {
            // 3a. Canonicalize the candidate + the profiles dir,
            //     reject anything that resolves outside the dir
            //     (mirror of themes resolve_disk_path_in_base
            //     symlink-out gate, CLAUDE.md §3).
            let canonical_file = std::fs::canonicalize(&candidate).map_err(|e| Error::Profile {
                name: name.to_owned(),
                source_path: candidate.display().to_string(),
                kind: ProfileErrorKind::PathCanonicalization {
                    path: candidate.clone(),
                    message: e.to_string(),
                },
            })?;
            let canonical_base =
                std::fs::canonicalize(&profiles_dir).map_err(|e| Error::Profile {
                    name: name.to_owned(),
                    source_path: profiles_dir.display().to_string(),
                    kind: ProfileErrorKind::PathCanonicalization {
                        path: profiles_dir.clone(),
                        message: e.to_string(),
                    },
                })?;
            if !canonical_file.starts_with(&canonical_base) {
                return Err(Error::Profile {
                    name: name.to_owned(),
                    source_path: candidate.display().to_string(),
                    kind: ProfileErrorKind::PathCanonicalization {
                        path: canonical_file.clone(),
                        message: format!(
                            "profile file must live under {base}; symlinks pointing outside are rejected",
                            base = canonical_base.display(),
                        ),
                    },
                });
            }

            // 3b. Regular-file check.
            let meta = std::fs::metadata(&canonical_file).map_err(|e| Error::Profile {
                name: name.to_owned(),
                source_path: candidate.display().to_string(),
                kind: ProfileErrorKind::PathCanonicalization {
                    path: canonical_file.clone(),
                    message: e.to_string(),
                },
            })?;
            if !meta.is_file() {
                return Err(Error::Profile {
                    name: name.to_owned(),
                    source_path: candidate.display().to_string(),
                    kind: ProfileErrorKind::PathCanonicalization {
                        path: canonical_file.clone(),
                        message: "profile path is not a regular file".to_owned(),
                    },
                });
            }

            // 3c. Read + parse.
            let source = crate::config::read_capped(&candidate)?;
            let path_label = canonical_file.display().to_string();
            let profile: Profile = toml::from_str(&source).map_err(|e| Error::Profile {
                name: name.to_owned(),
                source_path: path_label.clone(),
                kind: ProfileErrorKind::ParseError { message: e.to_string() },
            })?;

            // 3d. Phase 1 fail-collected validation.
            validate_profile(name, &path_label, &profile)?;

            return Ok(LoadedProfile { profile, path_label });
        }
    }

    // 4. Embedded discovery — v0.5.2 ships ZERO profiles. Append the
    //    synthetic path to `searched` so the NotFound diagnostic
    //    surfaces both the disk attempt and the embedded namespace.
    searched.push(std::path::PathBuf::from(synthetic_path(name)));
    Err(Error::Profile {
        name: name.to_owned(),
        source_path: synthetic_path(name),
        kind: ProfileErrorKind::NotFound { searched },
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
pub(crate) fn validate_profile(name: &str, source_path: &str, profile: &Profile) -> Result<()> {
    use std::collections::HashSet;

    let builtin_names: HashSet<&str> = crate::rules::BUILTIN_NAMES.iter().copied().collect();
    let mut errors: Vec<ProfileRuleError> = Vec::new();

    // 1. profile.rules whitelist — each entry must name a built-in.
    if let Some(ref rules) = profile.rules {
        for entry in rules {
            if !builtin_names.contains(entry.as_str()) {
                let mut known: Vec<String> =
                    builtin_names.iter().map(|s| (*s).to_owned()).collect();
                known.sort_unstable();
                errors.push(ProfileRuleError {
                    rule_name: "<rules>".to_owned(),
                    kind: ProfileRuleErrorKind::RuleUnknown { name: entry.clone(), known },
                });
            }
        }
    }

    // 2-4. profile.append_rules — name predicate, no built-in
    //      collision, no duplicate within the array. Per-entry
    //      short-circuit semantics: an invalid name skips the
    //      collision + duplicate checks for that entry (a
    //      malformed name cannot meaningfully collide).
    let mut seen_append: HashSet<&str> = HashSet::new();
    for ar in &profile.append_rules {
        // 2. Name predicate.
        if !name_is_valid(&ar.name) {
            errors.push(ProfileRuleError {
                rule_name: ar.name.clone(),
                kind: ProfileRuleErrorKind::RuleNameInvalid { name: ar.name.clone() },
            });
            continue;
        }

        // 3. Built-in collision.
        if builtin_names.contains(ar.name.as_str()) {
            errors.push(ProfileRuleError {
                rule_name: ar.name.clone(),
                kind: ProfileRuleErrorKind::AppendRuleConflictsWithBuiltin {
                    name: ar.name.clone(),
                },
            });
            continue;
        }

        // 4. Duplicate within append_rules.
        if !seen_append.insert(ar.name.as_str()) {
            errors.push(ProfileRuleError {
                rule_name: ar.name.clone(),
                kind: ProfileRuleErrorKind::AppendRuleConflictsWithOther { name: ar.name.clone() },
            });
        }
    }

    // 5. profile.theme — predicate only. Existence check deferred
    //    to theme-load (surfaces as Error::Theme { NotFound }).
    if let Some(ref theme) = profile.theme {
        if !name_is_valid(theme) {
            errors.push(ProfileRuleError {
                rule_name: "<theme>".to_owned(),
                kind: ProfileRuleErrorKind::ThemeNameInvalid { name: theme.clone() },
            });
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(Error::ProfileValidation {
            profile: name.to_owned(),
            source_path: source_path.to_owned(),
            errors,
        })
    }
}

#[cfg(test)]
mod tests {
    // Unit tests added in Task 6.
}
