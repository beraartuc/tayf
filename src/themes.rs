//! Embedded preset color themes.
//!
//! Theme files live in `assets/themes/` and are baked into the binary at
//! compile time via `include_str!`. They are NOT loaded from disk; users
//! who want a custom theme should use the user-config layer (which is
//! applied AFTER the theme layer and so wins on conflicts).
//!
//! A theme is a subset of the user-config schema: a sequence of `[[rules]]`
//! blocks with `name` and `style` only. Validation at load time rejects
//! `pattern`, `enabled = false`, and unknown rule names — see
//! [`validate_theme_rules`] for the precise rules.
//!
//! Public to crate:
//! - [`load`] — resolve a theme name to its embedded TOML source.
//! - [`names`] — alphabetically-sorted list of available theme names.
//! - [`validate_theme_rules`] — schema-shape check applied after parsing.
//! - [`synthetic_path`] — embedded source label used when feeding a theme through the user-config merge.

use crate::error::Error;
use crate::Result;

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
}
