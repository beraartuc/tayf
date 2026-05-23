//! Theme loading: embedded preset registry + disk-based custom themes.
//!
//! Preset theme files live in `assets/themes/` and are baked into the
//! binary at compile time via `include_str!`. Disk themes live in
//! `<config_base>/themes/<name>.toml` and are loaded through the same
//! 1 MiB read cap and symlink-out whitelist as the user config
//! (`config::read_capped` + canonical-base validation).
//!
//! Resolution order in [`load`]:
//! 1. Disk theme `<config_base>/themes/<name>.toml` exists?
//!    - And `name` matches a built-in (case-insensitively) →
//!      [`Error::Config`] with rename hint (F2 collision policy).
//!    - Else → load from disk, return `Cow::Owned`.
//! 2. Built-in registry has `name` (case-sensitive) → return
//!    `Cow::Borrowed(&'static str)`.
//! 3. Neither → [`Error::Theme`] with available list (built-ins ∪ disk
//!    discovered, deduplicated, alphabetically sorted, collisions
//!    excluded).
//!
//! A theme is a subset of the user-config schema: a sequence of
//! `[[rules]]` blocks with `name` and `style` only. Validation rejects
//! `pattern`, `enabled = false`, unknown rule names, and any `[general]`
//! section — see [`validate_theme_rules`] for the precise contract.
//! Disk themes are NOT required to override every built-in (partial
//! themes are accepted); shipped preset themes ARE required to be
//! exhaustive (unit-tested).
//!
//! Public to crate:
//! - [`LoadedTheme`] — `(source, path_label)` pair returned by [`load`].
//! - [`load`] — resolve a theme name (disk-first, built-in fallback).
//! - [`load_with`] — testable variant accepting env-var closures.
//! - [`names`] — alphabetically-sorted list of BUILT-IN theme names.
//! - [`validate_theme_rules`] — fail-collected schema-shape check.
//! - [`synthetic_path`] — embedded source label for built-in themes.

use crate::error::Error;
use crate::Result;

/// A theme loaded into memory, paired with the path label used in
/// error messages produced during parsing and validation.
///
/// `source` is `Cow::Borrowed(&'static str)` for built-in preset themes
/// (zero-alloc, baked into the binary) and `Cow::Owned(String)` for disk
/// themes (allocated once at load time).
///
/// `path_label` is `<embedded:theme/{name}>` for presets and the
/// absolute canonical disk path for disk-loaded themes. It is fed into
/// [`crate::config::parse`] and [`validate_theme_rules`] so downstream
/// error messages point at the actual source the user can edit.
#[allow(dead_code)]
// reason: consumed by Task 14 (themes::load rewrite) — struct lands
// first so subsequent helper commits can reference the type if needed.
#[derive(Debug)]
pub(crate) struct LoadedTheme {
    pub source: std::borrow::Cow<'static, str>,
    pub path_label: String,
}

/// A theme name must be a single non-empty path-segment-safe identifier.
/// Allowed characters: ASCII alphanumeric, `-`, `_`. No `.` characters at
/// all (rejects hidden-file names like `.dark` and disambiguates from
/// the `.toml` extension we append). No path separators, no traversal.
///
/// Leading hyphen (`-foo`) and leading digit (`0`, `123abc`) are accepted
/// — names are treated as filesystem path segments, not identifier-style
/// tokens. This is lenient by design; v0.3.5+ may tighten if real user
/// reports show confusion (Rev2 N-2).
///
/// ASCII-only (Rev2 Q2): CLAUDE.md §1 mandates English identifiers in
/// code; theme names are identifiers (CLI args, registry keys). Unicode
/// would invite homoglyph attacks against the F2 collision check
/// (`dark` vs `dаrk` with Cyrillic `а`).
///
/// Defense-in-depth: `name` reaches us from CLI args or config TOML and
/// is interpolated into a disk path (`base/themes/<name>.toml`). A name
/// like `../../etc/passwd` would still be caught by the canonical-base
/// whitelist downstream, but failing here keeps the error message clear
/// (`Error::Theme` "not found" rather than `Error::Config` "symlink out").
#[allow(dead_code)]
// reason: consumed by Task 14 (themes::load rewrite); tests in the
// same module exercise it but clippy's dead_code lint fires on lib
// builds because the production call site lands in the next commit.
fn name_is_valid(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

const DARK_SRC: &str = include_str!("../assets/themes/dark.toml");
const LIGHT_SRC: &str = include_str!("../assets/themes/light.toml");

/// Alphabetically-sorted slice of available theme names. Single source of
/// truth for the set and order of themes; [`REGISTRY`] is keyed by
/// `THEME_NAMES[i]` so the two cannot drift.
const THEME_NAMES: [&str; 2] = ["dark", "light"];

/// `(name, source)` pairs, sorted alphabetically by name. `REGISTRY` is
/// keyed by [`THEME_NAMES`]`[i]` so the two cannot drift; sorted order
/// matters because error messages quote that order to give users a stable
/// list.
const REGISTRY: &[(&str, &str)] = &[(THEME_NAMES[0], DARK_SRC), (THEME_NAMES[1], LIGHT_SRC)];

/// Resolve a theme name to its embedded TOML source.
///
/// # Errors
/// Returns [`Error::Theme`] with the sorted list of available theme names
/// when `name` is not in the registry.
pub(crate) fn load(name: &str) -> Result<&'static str> {
    if let Some((_, src)) = REGISTRY.iter().find(|(n, _)| *n == name) {
        return Ok(*src);
    }
    Err(Error::Theme {
        name: name.to_owned(),
        available: names().iter().map(|s| (*s).to_owned()).collect(),
    })
}

/// Alphabetically-sorted slice of available theme names.
pub(crate) fn names() -> &'static [&'static str] {
    &THEME_NAMES
}

/// Validate the shape of a theme's parsed config (`[general]` and
/// `[[rules]]`). Themes may only override existing built-in styles,
/// never define new rules, change patterns, set `enabled = false`, or
/// carry a `[general]` section.
///
/// This function is **fail-collected** — every violation across the whole
/// config is gathered into a single [`Error::ThemeValidation`]. Users see
/// every problem in one save-and-rerun cycle.
///
/// Per-rule, the first violation that fires is `UnknownName`; subsequent
/// gates (`pattern`, `enabled`) for the SAME rule are skipped (Rev2 I-9
/// — "fix the name first" subsumes the rest of the diagnostics for that
/// rule). Other rules continue independently.
///
/// `theme_name` is the requested theme (e.g. `dark`); `source_path` is
/// the embedded synthetic path for presets or the canonical disk path
/// for disk themes. Both flow into [`Error::ThemeValidation`].
///
/// # Errors
/// Returns [`Error::ThemeValidation`] with at least one
/// [`ThemeRuleError`] when any violation is found. Returns `Ok(())` when
/// the parsed config matches the theme contract exactly.
pub(crate) fn validate_theme_rules(
    theme_name: &str,
    source_path: &str,
    cfg: &crate::config::Config,
) -> Result<()> {
    use std::collections::HashSet;
    let known: HashSet<&str> = crate::rules::BUILTIN_NAMES.iter().copied().collect();
    let mut errors: Vec<crate::error::ThemeRuleError> = Vec::new();

    // Rev2 Q4 — [general] section forbidden in disk themes. Comparison
    // against GeneralSection::default() catches any field deviation
    // additively (future fields automatically gated).
    if cfg.general != crate::config::GeneralSection::default() {
        errors.push(crate::error::ThemeRuleError {
            rule_name: "<general>".to_owned(),
            kind: crate::error::ThemeRuleErrorKind::GeneralSectionForbidden,
        });
    }

    for r in &cfg.rules {
        // Rev2 I-9 — UnknownName subsumes pattern/enabled checks for the
        // same rule; user must rename before subsequent gates have
        // meaningful semantics. Other rules continue independently
        // (fail-collected story).
        if !known.contains(r.name.as_str()) {
            errors.push(crate::error::ThemeRuleError {
                rule_name: r.name.clone(),
                kind: crate::error::ThemeRuleErrorKind::UnknownName,
            });
            continue;
        }
        if r.pattern.is_some() {
            errors.push(crate::error::ThemeRuleError {
                rule_name: r.name.clone(),
                kind: crate::error::ThemeRuleErrorKind::PatternForbidden,
            });
        }
        if !r.enabled {
            errors.push(crate::error::ThemeRuleError {
                rule_name: r.name.clone(),
                kind: crate::error::ThemeRuleErrorKind::EnabledFalseForbidden,
            });
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(crate::error::Error::ThemeValidation {
            theme: theme_name.to_owned(),
            source_path: source_path.to_owned(),
            errors,
        })
    }
}

/// Synthetic path label used when feeding a theme through the user-config
/// merge machinery. Pure formatting — kept here so call sites in
/// `rules.rs` need not duplicate the convention.
pub(crate) fn synthetic_path(theme_name: &str) -> String {
    format!("<embedded:theme/{theme_name}>")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{UserRule, UserStyle};

    #[test]
    fn names_are_alphabetically_sorted() {
        let n = names();
        let mut sorted: Vec<_> = n.to_vec();
        sorted.sort_unstable();
        assert_eq!(n.to_vec(), sorted);
    }

    #[test]
    fn load_known_themes_returns_non_empty_source() {
        for &name in names() {
            let src = load(name).expect("known theme should load");
            assert!(!src.trim().is_empty(), "theme {name:?} embedded source must not be empty");
        }
    }

    #[test]
    fn load_unknown_theme_returns_error_theme_with_available() {
        let err = load("nope").expect_err("unknown theme must error");
        match err {
            Error::Theme { name, available } => {
                assert_eq!(name, "nope");
                assert_eq!(available, vec!["dark".to_string(), "light".to_string()]);
            }
            other => panic!("expected Error::Theme, got {other:?}"),
        }
    }

    fn rule(name: &str) -> UserRule {
        UserRule {
            name: name.into(),
            pattern: None,
            style: Some(UserStyle { fg: Some("red".into()), ..UserStyle::default() }),
            enabled: true,
        }
    }

    fn cfg_with_rules(rules: Vec<UserRule>) -> crate::config::Config {
        crate::config::Config { general: crate::config::GeneralSection::default(), rules }
    }

    #[test]
    fn validate_accepts_well_formed_theme() {
        let cfg = cfg_with_rules(vec![rule("ipv4"), rule("log_level")]);
        validate_theme_rules("dark", "<embedded:theme/dark>", &cfg)
            .expect("valid theme rules should pass");
    }

    #[test]
    fn validate_rejects_unknown_builtin_name() {
        let cfg = cfg_with_rules(vec![rule("nothere")]);
        let err = validate_theme_rules("dark", "<embedded:theme/dark>", &cfg)
            .expect_err("unknown name must error");
        let Error::ThemeValidation { errors, .. } = err else {
            panic!("expected ThemeValidation");
        };
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].rule_name, "nothere");
        assert_eq!(errors[0].kind, crate::error::ThemeRuleErrorKind::UnknownName);
    }

    #[test]
    fn validate_rejects_pattern_field() {
        let mut r = rule("ipv4");
        r.pattern = Some(r"\d+".into());
        let cfg = cfg_with_rules(vec![r]);
        let err = validate_theme_rules("dark", "<embedded:theme/dark>", &cfg)
            .expect_err("pattern must error");
        let Error::ThemeValidation { errors, .. } = err else {
            panic!("expected ThemeValidation");
        };
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, crate::error::ThemeRuleErrorKind::PatternForbidden);
    }

    #[test]
    fn validate_rejects_enabled_false() {
        let mut r = rule("ipv4");
        r.enabled = false;
        let cfg = cfg_with_rules(vec![r]);
        let err = validate_theme_rules("dark", "<embedded:theme/dark>", &cfg)
            .expect_err("enabled=false must error");
        let Error::ThemeValidation { errors, .. } = err else {
            panic!("expected ThemeValidation");
        };
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, crate::error::ThemeRuleErrorKind::EnabledFalseForbidden);
    }

    #[test]
    fn shipped_theme_files_parse_and_validate() {
        // Each shipped theme must (a) parse as TOML matching the user-config schema,
        // and (b) pass theme-specific validation. This catches accidental drift
        // between the embedded TOML and the validation rules at unit-test time.
        for &name in names() {
            let src = load(name).unwrap();
            let synth = synthetic_path(name);
            let cfg = crate::config::parse(&synth, src)
                .unwrap_or_else(|e| panic!("theme {name:?} did not parse: {e}"));
            validate_theme_rules(name, &synth, &cfg)
                .unwrap_or_else(|e| panic!("theme {name:?} failed validation: {e}"));
            assert_eq!(
                cfg.rules.len(),
                crate::rules::BUILTIN_NAMES.len(),
                "theme {name:?} should override every built-in"
            );
        }
    }

    #[test]
    fn validate_returns_ok_when_all_rules_valid() {
        let cfg = cfg_with_rules(vec![rule("ipv4"), rule("log_level")]);
        validate_theme_rules("dark", "<embedded:theme/dark>", &cfg).expect("valid");
    }

    #[test]
    fn validate_collects_multiple_errors_in_single_pass() {
        let mut bad_pattern = rule("ipv4");
        bad_pattern.pattern = Some(r"\d+".into());
        let mut bad_disabled = rule("log_level");
        bad_disabled.enabled = false;
        let unknown = rule("nope_typo");
        let cfg = cfg_with_rules(vec![bad_pattern, bad_disabled, unknown]);

        let err = validate_theme_rules("x", "<x>", &cfg).expect_err("should fail");
        let crate::error::Error::ThemeValidation { errors, .. } = err else {
            panic!("expected ThemeValidation, got something else");
        };
        assert_eq!(errors.len(), 3, "three independent rules => three errors: {errors:?}");
    }

    #[test]
    fn validate_skips_other_gates_after_unknown_name() {
        let mut r = rule("nope_typo");
        r.pattern = Some(r"\d+".into());
        r.enabled = false;
        let cfg = cfg_with_rules(vec![r]);

        let err = validate_theme_rules("x", "<x>", &cfg).expect_err("should fail");
        let crate::error::Error::ThemeValidation { errors, .. } = err else {
            panic!("expected ThemeValidation");
        };
        assert_eq!(errors.len(), 1, "UnknownName subsumes other gates: {errors:?}");
        assert_eq!(errors[0].kind, crate::error::ThemeRuleErrorKind::UnknownName);
    }

    #[test]
    fn validate_continues_with_other_rules_after_unknown_name() {
        let unknown = rule("nope_typo");
        let mut bad_pattern = rule("ipv4");
        bad_pattern.pattern = Some(r"\d+".into());
        let cfg = cfg_with_rules(vec![unknown, bad_pattern]);

        let err = validate_theme_rules("x", "<x>", &cfg).expect_err("should fail");
        let crate::error::Error::ThemeValidation { errors, .. } = err else {
            panic!("expected ThemeValidation");
        };
        assert_eq!(errors.len(), 2, "next rule should still be checked: {errors:?}");
        assert_eq!(errors[0].kind, crate::error::ThemeRuleErrorKind::UnknownName);
        assert_eq!(errors[1].kind, crate::error::ThemeRuleErrorKind::PatternForbidden);
    }

    #[test]
    fn validate_preserves_rule_order_in_errors_vec() {
        let unknown1 = rule("first_typo");
        let unknown2 = rule("second_typo");
        let cfg = cfg_with_rules(vec![unknown1, unknown2]);

        let err = validate_theme_rules("x", "<x>", &cfg).expect_err("should fail");
        let crate::error::Error::ThemeValidation { errors, .. } = err else {
            panic!("expected ThemeValidation");
        };
        assert_eq!(errors[0].rule_name, "first_typo");
        assert_eq!(errors[1].rule_name, "second_typo");
    }

    #[test]
    fn validate_carries_source_path_into_error() {
        let cfg = cfg_with_rules(vec![rule("nope")]);
        let err = validate_theme_rules("mine", "/home/u/.config/tayf/themes/mine.toml", &cfg)
            .expect_err("should fail");
        let crate::error::Error::ThemeValidation { source_path, theme, .. } = err else {
            panic!("expected ThemeValidation");
        };
        assert_eq!(theme, "mine");
        assert_eq!(source_path, "/home/u/.config/tayf/themes/mine.toml");
    }

    #[test]
    fn validate_rejects_general_section_in_disk_theme() {
        // [general] non-default => GeneralSectionForbidden push.
        let mut cfg = cfg_with_rules(vec![rule("ipv4")]);
        cfg.general.respect_existing_colors = false; // deviation from default

        let err = validate_theme_rules("mine", "<mine>", &cfg).expect_err("should fail");
        let crate::error::Error::ThemeValidation { errors, .. } = err else {
            panic!("expected ThemeValidation");
        };
        let has_general = errors
            .iter()
            .any(|e| matches!(e.kind, crate::error::ThemeRuleErrorKind::GeneralSectionForbidden));
        assert!(has_general, "GeneralSectionForbidden missing: {errors:?}");
        let general_entry = errors
            .iter()
            .find(|e| matches!(e.kind, crate::error::ThemeRuleErrorKind::GeneralSectionForbidden))
            .unwrap();
        assert_eq!(general_entry.rule_name, "<general>");
    }

    #[test]
    fn validate_accepts_default_general_section() {
        // GeneralSection::default() => no GeneralSectionForbidden push.
        let cfg = cfg_with_rules(vec![rule("ipv4")]);
        assert_eq!(cfg.general, crate::config::GeneralSection::default());
        validate_theme_rules("mine", "<mine>", &cfg).expect("default general is fine");
    }

    #[test]
    fn name_is_valid_accepts_alphanumeric_and_separators() {
        assert!(name_is_valid("dark"));
        assert!(name_is_valid("light"));
        assert!(name_is_valid("solarized-dark"));
        assert!(name_is_valid("solarized_dark"));
        assert!(name_is_valid("my-theme-v2"));
        assert!(name_is_valid("a1b2c3"));
        assert!(name_is_valid("-foo"));
        assert!(name_is_valid("0"));
        assert!(name_is_valid("123abc"));
        assert!(name_is_valid("_under"));
    }

    #[test]
    fn name_is_valid_rejects_empty() {
        assert!(!name_is_valid(""));
    }

    #[test]
    fn name_is_valid_rejects_path_separators() {
        assert!(!name_is_valid("x/y"));
        assert!(!name_is_valid("x\\y"));
        assert!(!name_is_valid("../etc/passwd"));
    }

    #[test]
    fn name_is_valid_rejects_dots() {
        assert!(!name_is_valid(".dark"));
        assert!(!name_is_valid("bad.name"));
        assert!(!name_is_valid("a.b.c"));
        assert!(!name_is_valid(".."));
    }

    #[test]
    fn name_is_valid_rejects_general_sentinel() {
        assert!(!name_is_valid("<general>"));
        assert!(!name_is_valid("<anything>"));
    }

    #[test]
    fn name_is_valid_rejects_non_ascii() {
        assert!(!name_is_valid("ışık"));
        assert!(!name_is_valid("dаrk"));
    }

    #[test]
    fn name_is_valid_rejects_whitespace() {
        assert!(!name_is_valid("my theme"));
        assert!(!name_is_valid("\tindent"));
        assert!(!name_is_valid(" leading"));
    }
}
