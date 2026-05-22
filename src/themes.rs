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

use crate::config::UserRule;
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

/// Validate the shape of parsed theme rules.
///
/// Themes may only override existing built-in styles, never define new
/// rules or change patterns. Specifically:
/// - every [`UserRule::name`] must match an entry in
///   [`crate::rules::BUILTIN_NAMES`];
/// - [`UserRule::pattern`] must be `None`;
/// - [`UserRule::enabled`] must be `true` (the serde default; explicit
///   `enabled = false` is rejected).
///
/// `theme_name` is interpolated into error messages so the surfaced
/// [`Error::Config`] points at the offending preset.
///
/// # Errors
/// Returns [`Error::Config`] on the first violation. The error's `path`
/// is a synthetic `<embedded:theme/{name}>` label so downstream display
/// makes the source clear without claiming a real filesystem path.
pub(crate) fn validate_theme_rules(theme_name: &str, rules: &[UserRule]) -> Result<()> {
    use std::collections::HashSet;
    let known: HashSet<&str> = crate::rules::BUILTIN_NAMES.iter().copied().collect();
    let path = synthetic_path(theme_name);
    for r in rules {
        if !known.contains(r.name.as_str()) {
            return Err(Error::Config {
                path,
                line: 0,
                message: format!(
                    "rule {n:?}: not a built-in name; themes may only override built-ins",
                    n = r.name
                ),
            });
        }
        if r.pattern.is_some() {
            return Err(Error::Config {
                path,
                line: 0,
                message: format!(
                    "rule {n:?}: must not set 'pattern' (themes only override style)",
                    n = r.name
                ),
            });
        }
        if !r.enabled {
            return Err(Error::Config {
                path,
                line: 0,
                message: format!(
                    "rule {n:?}: must not set 'enabled = false' (themes only override style)",
                    n = r.name
                ),
            });
        }
    }
    Ok(())
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

    #[test]
    fn validate_accepts_well_formed_theme() {
        let rs = vec![rule("ipv4"), rule("log_level")];
        validate_theme_rules("dark", &rs).expect("valid theme rules should pass");
    }

    #[test]
    fn validate_rejects_unknown_builtin_name() {
        let rs = vec![rule("nothere")];
        let err = validate_theme_rules("dark", &rs).expect_err("unknown name must error");
        assert!(
            err.to_string().contains("nothere"),
            "error must mention the offending name; got {err}"
        );
        assert!(
            err.to_string().contains("themes may only override"),
            "error must explain the rule"
        );
    }

    #[test]
    fn validate_rejects_pattern_field() {
        let mut r = rule("ipv4");
        r.pattern = Some(r"\d+".into());
        let err = validate_theme_rules("dark", &[r]).expect_err("pattern must error");
        assert!(err.to_string().contains("must not set 'pattern'"), "got {err}");
    }

    #[test]
    fn validate_rejects_enabled_false() {
        let mut r = rule("ipv4");
        r.enabled = false;
        let err = validate_theme_rules("dark", &[r]).expect_err("enabled=false must error");
        assert!(err.to_string().contains("'enabled = false'"), "got {err}");
    }

    #[test]
    fn shipped_theme_files_parse_and_validate() {
        // Each shipped theme must (a) parse as TOML matching the user-config schema,
        // and (b) pass theme-specific validation. This catches accidental drift
        // between the embedded TOML and the validation rules at unit-test time.
        for &name in names() {
            let src = load(name).unwrap();
            let cfg = crate::config::parse(&synthetic_path(name), src)
                .unwrap_or_else(|e| panic!("theme {name:?} did not parse: {e}"));
            validate_theme_rules(name, &cfg.rules)
                .unwrap_or_else(|e| panic!("theme {name:?} failed validation: {e}"));
            assert_eq!(
                cfg.rules.len(),
                crate::rules::BUILTIN_NAMES.len(),
                "theme {name:?} should override every built-in"
            );
        }
    }
}
