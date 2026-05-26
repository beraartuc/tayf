//! Profile loading, validation, and merging.
//!
//! A *profile* is a named bundle of rule configuration. v0.5.2 shipped
//! the mechanism; v0.5.3 ships the curated built-in library (`aws`,
//! `k8s`, `docker`, `gcp`, `network`) embedded via `include_str!` —
//! see [`EMBEDDED_PROFILES`].
//!
//! Profiles are loaded from disk
//! (`~/.config/tayf/profiles/<name>.toml`) or from the embedded
//! library. Disk discovery wins over embedded (user customization
//! shadows shipped defaults). Discovery
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
// Phase 5 (lib.rs orchestration + reload.rs hot-reload) lights up
// `profiles::load`; Phase 3 (`RuleSource::EmbeddedProfile` dispatch)
// lights up `validate_profile` via the call from `load`. The
// module-level dead_code allow is no longer required.

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

/// Profiles shipped with the binary, embedded at compile time via
/// `include_str!`. Discovery order in `load_with` is: disk first
/// (user customization wins) → embedded fallback → `NotFound`. To
/// add a profile, drop a TOML file under `assets/profiles/` and add
/// an entry here; a unit test added in Task 6 (`network` profile)
/// will pin the table count + name set.
const EMBEDDED_PROFILES: &[(&str, &str)] = &[
    ("aws", include_str!("../assets/profiles/aws.toml")),
    ("k8s", include_str!("../assets/profiles/k8s.toml")),
];

/// Load a profile by name. Reads `$XDG_CONFIG_HOME` and `$HOME` from
/// the environment for disk discovery; falls back to the embedded
/// library ([`EMBEDDED_PROFILES`]) on miss.
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
/// 1. Disk: `<config_base>/profiles/<name>.toml` (user customization).
/// 2. Embedded: [`EMBEDDED_PROFILES`] (v0.5.3 library — aws, k8s,
///    docker, gcp, network).
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

    // 4. Embedded discovery (v0.5.3 +). Library content shipped via
    //    `include_str!`; tried AFTER disk so user disk profiles take
    //    precedence (allows users to override an embedded profile by
    //    writing ~/.config/tayf/profiles/<name>.toml).
    if let Some((_, source)) = EMBEDDED_PROFILES.iter().find(|(n, _)| *n == name) {
        let path_label = synthetic_path(name);
        let profile: Profile = toml::from_str(source).map_err(|e| Error::Profile {
            name: name.to_owned(),
            source_path: path_label.clone(),
            kind: ProfileErrorKind::ParseError { message: e.to_string() },
        })?;
        validate_profile(name, &path_label, &profile)?;
        return Ok(LoadedProfile { profile, path_label });
    }

    // 5. NotFound — disk attempt + embedded namespace listed for diagnostics.
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
    use super::*;

    fn profile_with_rules(rules: Option<Vec<String>>) -> Profile {
        Profile { rules, append_rules: Vec::new(), theme: None }
    }

    fn profile_with_append(append_rules: Vec<ProfileRule>) -> Profile {
        Profile { rules: None, append_rules, theme: None }
    }

    fn make_rule(name: &str) -> ProfileRule {
        ProfileRule {
            name: name.to_owned(),
            pattern: r"\b[a-z]+\b".to_owned(),
            style: None,
            styles: None,
        }
    }

    // 1. validate_profile_phase1_accepts_minimal_shape
    #[test]
    fn validate_profile_phase1_accepts_minimal_shape() {
        let p = Profile::default();
        validate_profile("test", "<test>", &p).expect("default profile is valid");
    }

    // 2. validate_profile_phase1_rules_whitelist_unknown_built_in
    #[test]
    fn validate_profile_phase1_rules_whitelist_unknown_built_in() {
        let p = profile_with_rules(Some(vec!["not_a_builtin".to_owned()]));
        let err = validate_profile("test", "<test>", &p).expect_err("must reject unknown built-in");
        match err {
            Error::ProfileValidation { errors, .. } => {
                assert_eq!(errors.len(), 1);
                assert_eq!(errors[0].rule_name, "<rules>");
                match &errors[0].kind {
                    ProfileRuleErrorKind::RuleUnknown { name, known } => {
                        assert_eq!(name, "not_a_builtin");
                        assert!(
                            known.contains(&"ipv4".to_owned()),
                            "known list should contain ipv4; got: {known:?}"
                        );
                        // Alphabetical order regression guard.
                        let mut sorted = known.clone();
                        sorted.sort_unstable();
                        assert_eq!(known, &sorted, "known names must be alphabetically sorted");
                    }
                    other => panic!("expected RuleUnknown, got {other:?}"),
                }
            }
            other => panic!("expected ProfileValidation, got {other:?}"),
        }
    }

    // 3. validate_profile_phase1_append_rules_collides_with_builtin
    #[test]
    fn validate_profile_phase1_append_rules_collides_with_builtin() {
        let p = profile_with_append(vec![make_rule("ipv4")]);
        let err =
            validate_profile("test", "<test>", &p).expect_err("must reject builtin collision");
        let s = err.to_string();
        assert!(
            s.contains("append_rules entry \"ipv4\": collides with built-in rule"),
            "byte-pinned collision wording; got: {s}"
        );
        // Negative regression guard: must not also surface as
        // RuleNameInvalid (the name is valid; collision is the
        // only problem).
        assert!(
            !s.contains("name must be ASCII alphanumeric"),
            "valid-name path must not surface RuleNameInvalid; got: {s}"
        );
    }

    // 4. validate_profile_phase1_append_rules_duplicate_name
    #[test]
    fn validate_profile_phase1_append_rules_duplicate_name() {
        let p = profile_with_append(vec![make_rule("foo"), make_rule("foo")]);
        let err = validate_profile("test", "<test>", &p).expect_err("must reject duplicates");
        let s = err.to_string();
        assert!(
            s.contains("append_rules: duplicate entry \"foo\""),
            "byte-pinned duplicate wording; got: {s}"
        );
    }

    // 5. validate_profile_phase1_append_rules_invalid_name
    #[test]
    fn validate_profile_phase1_append_rules_invalid_name() {
        let p = profile_with_append(vec![make_rule("bad name")]);
        let err = validate_profile("test", "<test>", &p).expect_err("must reject invalid name");
        let s = err.to_string();
        assert!(
            s.contains(
                "append_rules entry \"bad name\": name must be ASCII alphanumeric with '-' or '_'"
            ),
            "byte-pinned invalid-name wording; got: {s}"
        );
        // Negative regression: invalid name short-circuits — must
        // NOT also surface as collision.
        assert!(!s.contains("collides with built-in"), "must short-circuit; got: {s}");
    }

    // 6. validate_profile_phase1_theme_name_invalid
    #[test]
    fn validate_profile_phase1_theme_name_invalid() {
        let p =
            Profile { rules: None, append_rules: Vec::new(), theme: Some("bad name".to_owned()) };
        let err =
            validate_profile("test", "<test>", &p).expect_err("must reject invalid theme name");
        let s = err.to_string();
        assert!(
            s.contains("theme \"bad name\": name must be ASCII alphanumeric with '-' or '_'"),
            "byte-pinned theme-name wording; got: {s}"
        );
    }

    // 7. validate_profile_phase1_collects_multiple_errors_in_one_pass
    #[test]
    fn validate_profile_phase1_collects_multiple_errors_in_one_pass() {
        let p = Profile {
            rules: Some(vec!["not_a_builtin".to_owned()]),
            append_rules: vec![make_rule("ipv4"), make_rule("foo"), make_rule("foo")],
            theme: Some("bad name".to_owned()),
        };
        let err = validate_profile("test", "<test>", &p).expect_err("must collect multiple errors");
        match err {
            Error::ProfileValidation { errors, .. } => {
                // 4 errors expected:
                //   RuleUnknown(not_a_builtin),
                //   AppendRuleConflictsWithBuiltin(ipv4),
                //   AppendRuleConflictsWithOther(foo),
                //   ThemeNameInvalid(bad name).
                assert_eq!(errors.len(), 4, "must collect all 4; got: {errors:?}");
                let kinds: Vec<_> =
                    errors.iter().map(|e| std::mem::discriminant(&e.kind)).collect();
                assert!(kinds.contains(&std::mem::discriminant(
                    &ProfileRuleErrorKind::RuleUnknown { name: String::new(), known: Vec::new() }
                )));
                assert!(kinds.contains(&std::mem::discriminant(
                    &ProfileRuleErrorKind::AppendRuleConflictsWithBuiltin { name: String::new() }
                )));
                assert!(kinds.contains(&std::mem::discriminant(
                    &ProfileRuleErrorKind::AppendRuleConflictsWithOther { name: String::new() }
                )));
                assert!(kinds.contains(&std::mem::discriminant(
                    &ProfileRuleErrorKind::ThemeNameInvalid { name: String::new() }
                )));
            }
            other => panic!("expected ProfileValidation, got {other:?}"),
        }
    }

    // v0.5.3 — embedded discovery (§7.7).

    #[test]
    fn load_embedded_aws_returns_loadedprofile_with_synthetic_path() {
        let xdg = tempfile::tempdir().expect("tmpdir");
        let lp = load_with("aws", || Some(xdg.path().to_path_buf()), || None)
            .expect("embedded aws must load");
        assert_eq!(lp.path_label, "<embedded:profile/aws>");
        let names: Vec<&str> = lp.profile.append_rules.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["instance_id", "region", "arn"],
            "aws append_rules must ship in this order and with these names"
        );
    }

    #[test]
    fn load_embedded_k8s_returns_loadedprofile_with_synthetic_path() {
        let xdg = tempfile::tempdir().expect("tmpdir");
        let lp = load_with("k8s", || Some(xdg.path().to_path_buf()), || None)
            .expect("embedded k8s must load");
        assert_eq!(lp.path_label, "<embedded:profile/k8s>");
        assert_eq!(lp.profile.append_rules.len(), 1, "k8s ships pod_name only");
    }
}
