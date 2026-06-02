//! Profile loading, validation, and active-rule resolution.
//!
//! A *profile* is a named, switchable preset: a `[[rules]]` list (built-in
//! overrides / enable-disable / recolor / new patterns) plus an optional
//! theme — the same schema as `config.toml`'s rule section. `config.toml`
//! is the implicit "default" profile; named profiles live at
//! `~/.config/tayf/profiles/<name>.toml` and REPLACE `config.toml`'s
//! `[[rules]]` when active (the built-ins remain the substrate).
//!
//! Disk discovery + canonicalisation mirror [`crate::themes`] exactly —
//! same file layout (no `[profile.X]` header; the file name is the profile
//! name), same predicate for valid names ([`crate::themes::name_is_valid`]),
//! same path-traversal guards.
//!
//! ## Public API (crate-internal)
//!
//! - [`Profile`] — parsed TOML body (`rules` + optional `theme`).
//! - [`load`] — discover + parse + validate a named disk profile.
//! - [`resolve_active`] — pick config-default vs named-profile rules for a
//!   compile, with the diagnostics path + [`crate::rules::RuleSource`].
//! - [`validate_profile`] — Phase 1 fail-collected shape check.
//! - [`name_is_valid`] — re-export of [`crate::themes::name_is_valid`].

use serde::Deserialize;

use crate::error::{Error, ProfileErrorKind, ProfileRuleError, ProfileRuleErrorKind, Result};

/// A named, switchable preset: a `[[rules]]` list (built-in overrides and/or
/// new patterns) plus an optional theme. Same schema as `config.toml`'s rule
/// section. When active, this profile's `rules` REPLACE `config.toml`'s.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Profile {
    /// Rules applied over the built-in substrate (override / enable-disable /
    /// recolor / new patterns). Each is a [`crate::config::UserRule`].
    #[serde(default)]
    pub(crate) rules: Vec<crate::config::UserRule>,
    /// Optional theme override, slotting into the CLI > config > profile > bg
    /// precedence chain.
    #[serde(default)]
    pub(crate) theme: Option<String>,
}

/// Result of [`load`]. Carries the parsed profile + the canonical disk path
/// it was loaded from (surfaced in profile-rule diagnostics).
#[derive(Debug)]
pub(crate) struct LoadedProfile {
    /// The parsed and Phase-1-validated profile body.
    pub profile: Profile,
    /// Canonical disk path of the loaded profile file.
    pub path_label: String,
}

/// Re-export of [`crate::themes::name_is_valid`] — profile names share
/// the same predicate as theme names (v0.5.1 spec §11.1 I-3 mandate).
pub(crate) fn name_is_valid(name: &str) -> bool {
    crate::themes::name_is_valid(name)
}

/// Resolve the active rule set + diagnostics path + source + profile theme
/// for a compile. `None` profile name → the caller's `config` rules as
/// [`crate::rules::RuleSource::UserConfig`] with no profile theme. A named
/// profile → its `rules` as [`crate::rules::RuleSource::DiskProfile`] with the
/// profile path and its optional `theme`. Returns an owned synthetic
/// [`crate::config::Config`] carrying the caller's `[general]` plus the active
/// rules, the path label, the source, and the named profile's theme (the 4th
/// element) to feed [`crate::rules::Compiled::load_with_theme`] and the
/// caller's CLI > config > profile > bg-detect theme-precedence chain.
///
/// # Errors
/// Propagates [`load`] failures (`NotFound` / parse / validation / IO).
pub(crate) fn resolve_active(
    config: &crate::config::Config,
    profile_name: Option<&str>,
) -> Result<(crate::config::Config, Option<String>, crate::rules::RuleSource, Option<String>)> {
    match profile_name {
        Some(name) => {
            let lp = load(name)?;
            let profile_theme = lp.profile.theme.clone();
            let effective =
                crate::config::Config { general: config.general.clone(), rules: lp.profile.rules };
            Ok((
                effective,
                Some(lp.path_label),
                crate::rules::RuleSource::DiskProfile,
                profile_theme,
            ))
        }
        None => Ok((config.clone(), None, crate::rules::RuleSource::UserConfig, None)),
    }
}

/// `<tayf_root>/profiles/<name>.toml` — canonical on-disk location for a
/// user-editable profile file. `tayf_root` is the resolved
/// `~/.config/tayf/` directory (see
/// [`crate::config_tui::save::tayf_config_root`]).
// reason: wired by the Profiles-tab create/delete handlers in the next task of
// this group; exercised meanwhile by the profiles unit tests.
#[allow(dead_code)]
pub(crate) fn disk_path_with_root(tayf_root: &std::path::Path, name: &str) -> std::path::PathBuf {
    profiles_dir_with_root(tayf_root).join(format!("{name}.toml"))
}

/// `<tayf_root>/profiles/` — sibling of the env-resolved directory. Used by
/// the Profiles-tab listing + create/delete handlers and by integration
/// tests that prefer a deterministic root over mutating `XDG_CONFIG_HOME`.
pub(crate) fn profiles_dir_with_root(tayf_root: &std::path::Path) -> std::path::PathBuf {
    tayf_root.join("profiles")
}

/// List the `.toml` profile stems under `<tayf_root>/profiles/`, sorted
/// case-insensitively. A missing directory yields an empty list (no disk
/// profiles). Non-`.toml` entries, directories, and names that fail
/// [`name_is_valid`] are skipped. Drives the Profiles-tab list (the caller
/// prepends the synthetic `default` entry for `config.toml`).
pub(crate) fn list_names_with_root(tayf_root: &std::path::Path) -> Vec<String> {
    let dir = profiles_dir_with_root(tayf_root);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(std::result::Result::ok)
        .filter_map(|e| {
            let path = e.path();
            if !path.is_file() {
                return None;
            }
            if path.extension().and_then(|x| x.to_str()) != Some("toml") {
                return None;
            }
            let stem = path.file_stem()?.to_str()?.to_owned();
            name_is_valid(&stem).then_some(stem)
        })
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// Load a profile by name. Reads `$XDG_CONFIG_HOME` and `$HOME` from
/// the environment for disk discovery (`~/.config/tayf/profiles/<name>.toml`).
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
/// Mirrors `themes::load_with`. Disk-only: `<config_base>/profiles/<name>.toml`.
/// A miss yields [`ProfileErrorKind::NotFound`] (no embedded fallback —
/// retired names like `aws` are now built-ins, see CHANGELOG).
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
            source_path: String::new(),
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

    // 4. NotFound — the disk candidate (if any) is the searched diagnostic.
    //    There is no embedded fallback: retired names (`aws`/`docker`/…) are
    //    now built-ins, so a missing disk file is a clean NotFound.
    let source_path = searched.first().map(|p| p.display().to_string()).unwrap_or_default();
    Err(Error::Profile {
        name: name.to_owned(),
        source_path,
        kind: ProfileErrorKind::NotFound { searched },
    })
}

/// Phase-1 shape validation for a disk profile: each rule's name must pass
/// [`name_is_valid`] and names must be unique within the profile. New-rule
/// "must have a pattern" + styles-key/range validation happen later in the
/// shared compile path (`apply_user_rules_with_source` →
/// `Compiled::load_with_theme`), routed to `Error::Profile*` with the profile
/// path via `RuleSource::DiskProfile`.
///
/// # Errors
/// Returns [`Error::ProfileValidation`] with at least one [`ProfileRuleError`]
/// when any name is invalid or duplicated.
pub(crate) fn validate_profile(name: &str, source_path: &str, profile: &Profile) -> Result<()> {
    use std::collections::HashSet;
    let mut errors: Vec<ProfileRuleError> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for r in &profile.rules {
        if !name_is_valid(&r.name) {
            errors.push(ProfileRuleError {
                rule_name: r.name.clone(),
                kind: ProfileRuleErrorKind::RuleNameInvalid { name: r.name.clone() },
            });
            continue;
        }
        if !seen.insert(r.name.as_str()) {
            errors.push(ProfileRuleError {
                rule_name: r.name.clone(),
                kind: ProfileRuleErrorKind::AppendRuleConflictsWithOther { name: r.name.clone() },
            });
        }
    }
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
    use crate::config::UserRule;

    // -----------------------------------------------------------------------
    // Schema: a profile is a `[[rules]]` list + optional `theme` (v0.12.0).
    // -----------------------------------------------------------------------

    #[test]
    fn profile_parses_as_a_rules_list_plus_optional_theme() {
        let toml = r##"
theme = "dark"

[[rules]]
name = "ipv4"
enabled = false

[[rules]]
name = "ticket"
pattern = 'JIRA-\d+'
style = { fg = "#f7d17f" }
"##;
        let p: Profile = toml::from_str(toml).expect("profile = rules list + theme");
        assert_eq!(p.theme.as_deref(), Some("dark"));
        assert_eq!(p.rules.len(), 2);
        assert_eq!(p.rules[0].name, "ipv4");
        assert!(!p.rules[0].enabled, "a profile may disable a built-in");
        assert_eq!(p.rules[1].name, "ticket");
        assert_eq!(p.rules[1].pattern.as_deref(), Some(r"JIRA-\d+"));
    }

    #[test]
    fn profile_defaults_to_empty_rules_and_no_theme() {
        let p = Profile::default();
        assert!(p.rules.is_empty());
        assert!(p.theme.is_none());
    }

    #[test]
    fn profile_schema_rejects_unknown_field() {
        // Negative guard — `#[serde(deny_unknown_fields)]` enforcement.
        let toml = r#"
theme = "dark"
unexpected_field = "this must fail"
"#;
        let result: std::result::Result<Profile, _> = toml::from_str(toml);
        assert!(result.is_err(), "deny_unknown_fields must reject typo `unexpected_field`");
    }

    // -----------------------------------------------------------------------
    // validate_profile: name-shape + intra-profile duplicate + theme-name.
    // -----------------------------------------------------------------------

    fn profile_with_rule_names(names: &[&str]) -> Profile {
        Profile {
            rules: names
                .iter()
                .map(|n| UserRule {
                    name: (*n).to_owned(),
                    pattern: None,
                    style: None,
                    enabled: true,
                    styles: None,
                    priority: None,
                })
                .collect(),
            theme: None,
        }
    }

    #[test]
    fn validate_profile_accepts_minimal_shape() {
        validate_profile("test", "<test>", &Profile::default()).expect("default profile is valid");
    }

    #[test]
    fn validate_profile_accepts_builtin_name_as_intentional_override() {
        // A profile rule named like a built-in is an intentional recolor /
        // toggle, NOT a collision error (the whitelist concept is gone).
        let p = profile_with_rule_names(&["ipv4"]);
        validate_profile("test", "<test>", &p).expect("built-in override is valid");
    }

    #[test]
    fn validate_profile_rejects_invalid_rule_name() {
        let p = profile_with_rule_names(&["bad name"]);
        let err = validate_profile("test", "<test>", &p).expect_err("must reject invalid name");
        let s = err.to_string();
        assert!(
            s.contains("name must be ASCII alphanumeric with '-' or '_'"),
            "byte-pinned invalid-name wording; got: {s}"
        );
    }

    #[test]
    fn validate_profile_rejects_duplicate_rule_name_within_profile() {
        let p = profile_with_rule_names(&["foo", "foo"]);
        let err = validate_profile("test", "<test>", &p).expect_err("must reject duplicate");
        let s = err.to_string();
        assert!(s.contains("duplicate entry \"foo\""), "byte-pinned duplicate wording; got: {s}");
    }

    #[test]
    fn validate_profile_rejects_invalid_theme_name() {
        let p = Profile { rules: Vec::new(), theme: Some("bad name".to_owned()) };
        let err = validate_profile("test", "<test>", &p).expect_err("must reject invalid theme");
        let s = err.to_string();
        assert!(
            s.contains("theme \"bad name\": name must be ASCII alphanumeric with '-' or '_'"),
            "byte-pinned theme-name wording; got: {s}"
        );
    }

    // -----------------------------------------------------------------------
    // Disk load + active resolution.
    // -----------------------------------------------------------------------

    #[test]
    fn load_reads_a_disk_profile_with_canonical_path_label() {
        let xdg = tempfile::tempdir().expect("tmpdir");
        let profiles_dir = xdg.path().join("tayf").join("profiles");
        std::fs::create_dir_all(&profiles_dir).expect("create profiles dir");
        std::fs::write(
            profiles_dir.join("work.toml"),
            "theme = \"dark\"\n\n[[rules]]\nname = \"container_id\"\nenabled = true\n",
        )
        .expect("write disk profile");

        let lp = load_with("work", || Some(xdg.path().to_path_buf()), || None)
            .expect("disk profile must load");
        assert!(
            !lp.path_label.is_empty() && lp.path_label.ends_with("work.toml"),
            "path_label must be the canonical disk path; got: {}",
            lp.path_label
        );
        assert_eq!(lp.profile.theme.as_deref(), Some("dark"));
        assert_eq!(lp.profile.rules.len(), 1);
        assert_eq!(lp.profile.rules[0].name, "container_id");
        assert!(lp.profile.rules[0].enabled);
    }

    #[test]
    fn load_missing_profile_returns_notfound_no_embedded_fallback() {
        let xdg = tempfile::tempdir().expect("tmpdir");
        // No profiles/aws.toml on disk → clean NotFound (retired names are
        // now built-ins; there is no embedded shim).
        let err = load_with("aws", || Some(xdg.path().to_path_buf()), || None)
            .expect_err("retired embedded name must be NotFound");
        match err {
            Error::Profile {
                kind: ProfileErrorKind::NotFound { searched }, source_path, ..
            } => {
                assert!(
                    !searched.iter().any(|p| p.to_string_lossy().contains("<embedded:")),
                    "searched must NOT mention an embedded namespace; got: {searched:?}"
                );
                assert!(
                    !source_path.contains("<embedded:"),
                    "source_path must NOT be a synthetic embedded label; got: {source_path}"
                );
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn load_rejects_traversal_name_as_notfound() {
        let xdg = tempfile::tempdir().expect("tmpdir");
        let err = load_with("../etc/passwd", || Some(xdg.path().to_path_buf()), || None)
            .expect_err("path-traversal name must fail");
        assert!(matches!(err, Error::Profile { kind: ProfileErrorKind::NotFound { .. }, .. }));
    }

    #[test]
    fn resolve_active_none_uses_config_rules_as_user_config() {
        let cfg = crate::config::Config {
            general: crate::config::GeneralSection::default(),
            rules: vec![UserRule {
                name: "fqdn".to_owned(),
                pattern: None,
                style: None,
                enabled: false,
                styles: None,
                priority: None,
            }],
        };
        let (eff, path, source, profile_theme) =
            resolve_active(&cfg, None).expect("no-profile resolve");
        assert_eq!(eff.rules.len(), 1, "config rules pass through unchanged");
        assert_eq!(eff.rules[0].name, "fqdn");
        assert!(path.is_none(), "no profile path when no profile is active");
        assert_eq!(source, crate::rules::RuleSource::UserConfig);
        assert!(profile_theme.is_none(), "no profile theme when no profile is active");
    }

    // The named-profile REPLACE path (config rules dropped, profile rules
    // active, DiskProfile source) reads `$XDG_CONFIG_HOME`/`$HOME` from the
    // real environment via `load`, so it is exercised end-to-end by the
    // disk-profile integration suite rather than mutating the process env
    // here.

    // -----------------------------------------------------------------------
    // Disk-path helpers.
    // -----------------------------------------------------------------------

    #[test]
    fn disk_path_with_root_builds_canonical_profiles_layout() {
        let tayf_root = std::path::Path::new("/tmp/example-config/tayf");
        assert_eq!(
            disk_path_with_root(tayf_root, "work"),
            std::path::PathBuf::from("/tmp/example-config/tayf/profiles/work.toml"),
            "disk path = <tayf_root>/profiles/<name>.toml"
        );
    }

    #[test]
    fn profiles_dir_with_root_appends_profiles_segment_to_tayf_root() {
        let tayf_root = std::path::Path::new("/tmp/example-config/tayf");
        assert_eq!(
            profiles_dir_with_root(tayf_root),
            std::path::PathBuf::from("/tmp/example-config/tayf/profiles"),
        );
    }

    #[test]
    fn list_names_with_root_returns_empty_when_dir_missing() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        // profiles/ deliberately absent.
        assert!(list_names_with_root(&tmp.path().join("tayf")).is_empty());
    }

    #[test]
    fn list_names_with_root_lists_toml_stems_sorted_skips_non_toml() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let dir = tmp.path().join("tayf").join("profiles");
        std::fs::create_dir_all(&dir).expect("mkdir profiles");
        std::fs::write(dir.join("work.toml"), "").expect("write work");
        std::fs::write(dir.join("Alpha.toml"), "").expect("write Alpha");
        std::fs::write(dir.join("notes.txt"), "").expect("write notes.txt");
        std::fs::create_dir_all(dir.join("subdir.toml")).expect("mkdir subdir.toml");

        let names = list_names_with_root(&tmp.path().join("tayf"));
        assert_eq!(
            names,
            vec!["Alpha".to_owned(), "work".to_owned()],
            "case-insensitive sort, .toml files only, directories skipped"
        );
    }
}
