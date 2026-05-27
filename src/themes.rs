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
/// absolute disk path (as the user typed/configured it, not the
/// canonicalized form — symlinks are preserved so the label matches
/// what the user can `ls` or edit; this mirrors the
/// `config::check_default_path` precedent). It is fed into
/// [`crate::config::parse`] and [`validate_theme_rules`] so downstream
/// error messages point at the actual source the user can edit.
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
pub(crate) fn name_is_valid(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Resolve `<base>/themes/<name>.toml` if it exists, validating the
/// symlink chain and file type. Returns `Ok(None)` cleanly if the file
/// doesn't exist (themes dir or specific theme not present is non-fatal
/// — caller falls back to the built-in registry).
///
/// `base` is the already-resolved config base directory (e.g.
/// `$XDG_CONFIG_HOME/tayf`); caller obtains it via
/// [`crate::config::config_base`]. This function constructs
/// `<base>/themes/` internally.
///
/// **TOCTOU rationale (Rev2 I-12, mirrors `config::check_default_path`):**
/// The gap between `canonicalize` and the subsequent `read_capped` open
/// is intentionally not closed. tayf operates under a single-user threat
/// model (CLAUDE.md §3): an attacker who can swap files inside
/// `<base>/themes/` already has write access there and can put
/// adversarial content directly; symlink games gain them nothing.
/// Cross-user attacks would require shared `HOME`/`XDG`, which is
/// outside the threat model.
///
/// Only the CANONICAL target of `<name>.toml` is constrained to live
/// under the canonical themes base; if `<base>/themes` itself is a
/// symlink to `/elsewhere/themes`, both sides resolve into `/elsewhere/`
/// and the file is accepted (user intentionally redirected the entire
/// directory).
fn resolve_disk_path_in_base(
    name: &str,
    base: &std::path::Path,
) -> Result<Option<std::path::PathBuf>> {
    let themes_dir = base.join("themes");
    let candidate = themes_dir.join(format!("{name}.toml"));
    if !candidate.exists() {
        return Ok(None);
    }
    let canonical_file = std::fs::canonicalize(&candidate).map_err(|e| Error::Config {
        path: candidate.display().to_string(),
        line: 0,
        message: format!("cannot canonicalize theme path: {e}"),
    })?;
    let canonical_base = std::fs::canonicalize(&themes_dir).map_err(|e| Error::Config {
        path: themes_dir.display().to_string(),
        line: 0,
        message: format!("cannot canonicalize themes directory: {e}"),
    })?;
    if !canonical_file.starts_with(&canonical_base) {
        return Err(Error::Config {
            path: candidate.display().to_string(),
            line: 0,
            message: format!(
                "theme file must live under {base}; symlinks pointing outside are rejected. \
                 Move the file under {base} or remove the symlink.",
                base = canonical_base.display()
            ),
        });
    }
    let meta = std::fs::metadata(&canonical_file).map_err(|e| Error::Config {
        path: candidate.display().to_string(),
        line: 0,
        message: format!("cannot stat theme target: {e}"),
    })?;
    if !meta.is_file() {
        return Err(Error::Config {
            path: candidate.display().to_string(),
            line: 0,
            message: "theme path is not a regular file".into(),
        });
    }
    Ok(Some(candidate))
}

/// List `*.toml` filenames in `<base>/themes/` (alphabetical, basename
/// only, `.toml` extension stripped). Returns an empty Vec when the
/// themes dir doesn't exist or cannot be read — cold path, errors are
/// swallowed because this only feeds the diagnostic-list inside
/// `Error::Theme`.
///
/// `read_dir` failures are logged via `info_msg!` (`TAYF_LOG=info`
/// gated) so a user debugging "why don't my themes show up in the
/// not-found list?" gets a signal without polluting stderr by default
/// (Rev2 N-3). The crate-local logger does not yet expose a
/// `debug_msg!` macro, so `info` is the lowest gate available.
fn discover_disk_themes(base: &std::path::Path) -> Vec<String> {
    let themes_dir = base.join("themes");
    let entries = match std::fs::read_dir(&themes_dir) {
        Ok(it) => it,
        Err(e) => {
            crate::log::info_msg!(
                "themes dir discovery failed for {dir}: {e}",
                dir = themes_dir.display()
            );
            return Vec::new();
        }
    };
    let mut names: Vec<String> = entries
        .filter_map(std::result::Result::ok)
        .filter_map(|e| {
            let p = e.path();
            if !p.is_file() {
                return None;
            }
            p.file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_owned)
                .filter(|_| p.extension().and_then(|s| s.to_str()) == Some("toml"))
        })
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// Combined available list: built-ins ∪ disk-discovered, deduplicated,
/// alphabetically sorted. Disk themes that collide with built-in names
/// (case-insensitively, Rev2 I-1) are EXCLUDED from this list — they
/// would error on use before being usable.
///
/// `base` is the already-resolved config base (`<config_base>`). `None`
/// means no `$XDG_CONFIG_HOME` and no `$HOME`; only built-ins are listed.
fn available_theme_names_from_base(base: Option<&std::path::Path>) -> Vec<String> {
    let mut names: Vec<String> = REGISTRY.iter().map(|(n, _)| (*n).to_owned()).collect();
    if let Some(base) = base {
        let disk = discover_disk_themes(base);
        let builtins: std::collections::HashSet<String> =
            REGISTRY.iter().map(|(n, _)| (*n).to_ascii_lowercase()).collect();
        for d in disk {
            if !builtins.contains(&d.to_ascii_lowercase()) {
                names.push(d);
            }
        }
    }
    names.sort_unstable();
    names.dedup();
    names
}

/// Build the `Error::Config` for the F2 collision case — a disk theme
/// file shares a built-in name. Message carries the disk path so the
/// user knows exactly which file to rename, plus a concrete hint for a
/// safe replacement name.
fn collision_error(name: &str, disk_path: &std::path::Path) -> Error {
    Error::Config {
        path: disk_path.display().to_string(),
        line: 0,
        message: format!(
            "theme '{name}' shadows the built-in theme with the same name; \
             rename the file (e.g. to '{name}-custom.toml') or remove it"
        ),
    }
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

/// Resolve a theme name to its TOML source, preferring disk themes over
/// built-in presets when both exist for the same `name`. Production
/// entry — reads `$XDG_CONFIG_HOME` and `$HOME` from the environment.
///
/// # Errors
/// - [`Error::Config`] when a disk theme exists for a built-in name
///   (F2 collision policy; case-insensitive — `--theme DARK` with disk
///   `dark.toml` errors on case-insensitive filesystems too).
/// - [`Error::Config`] when the disk theme cannot be read, is too large,
///   resolves outside the canonical themes base (symlink-out reject),
///   or is not a regular file.
/// - [`Error::Theme`] when neither a disk theme nor a built-in by this
///   name exists; the `available` list contains built-ins ∪
///   disk-discovered names, deduplicated and alphabetically sorted
///   (collisions excluded).
pub(crate) fn load(name: &str) -> Result<LoadedTheme> {
    load_with(
        name,
        || std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from),
        || std::env::var_os("HOME").map(std::path::PathBuf::from),
    )
}

/// Testable variant of [`load`]; accepts env-var closures so unit tests
/// can scope `$XDG_CONFIG_HOME` and `$HOME` to a `tempdir` without
/// mutating the process environment.
pub(crate) fn load_with(
    name: &str,
    xdg: impl FnOnce() -> Option<std::path::PathBuf>,
    home: impl FnOnce() -> Option<std::path::PathBuf>,
) -> Result<LoadedTheme> {
    // Resolve base ONCE — closures consumed here, never re-used downstream.
    // `base` may be None (neither XDG nor HOME set, or both empty) — that
    // short-circuits disk lookup but built-in registry still works.
    let base = crate::config::config_base(xdg, home);

    // Name shape — fail-fast on path separators / traversal / empty.
    // Defense-in-depth: name reaches us from CLI args or config TOML, both
    // of which could carry adversarial values. Available list reuses
    // `base` (None → built-ins only).
    if !name_is_valid(name) {
        return Err(Error::Theme {
            name: name.to_owned(),
            available: available_theme_names_from_base(base.as_deref()),
        });
    }

    // Disk lookup — `resolve_disk_path_in_base` runs the symlink-out +
    // regular-file gates. Returns `Ok(None)` cleanly when the themes/
    // subdir or the specific theme file doesn't exist.
    let disk = match base.as_deref() {
        Some(b) => resolve_disk_path_in_base(name, b)?,
        None => None,
    };

    // F2 collision check (Rev2 I-1 — case-insensitive).
    // macOS APFS / HFS+ default is case-insensitive; `dark.toml` matches
    // `--theme DARK` at the filesystem layer. Without
    // `eq_ignore_ascii_case` the registry compare misses, and the user
    // accidentally bypasses the F2 protection by typo'ing case.
    let is_builtin = REGISTRY.iter().any(|(n, _)| name.eq_ignore_ascii_case(n));
    if let Some(ref disk_path) = disk {
        if is_builtin {
            return Err(collision_error(name, disk_path));
        }
    }

    // Disk theme — read with cap, wrap in Cow::Owned.
    if let Some(disk_path) = disk {
        let body = crate::config::read_capped(&disk_path)?;
        let path_label = disk_path.display().to_string();
        return Ok(LoadedTheme { source: std::borrow::Cow::Owned(body), path_label });
    }

    // Built-in registry (case-sensitive — only the lowercase canonical
    // names match; user typing `--theme DARK` without a disk file still
    // errors with `Error::Theme` so the casing rule stays predictable).
    if let Some((_, src)) = REGISTRY.iter().find(|(n, _)| *n == name) {
        return Ok(LoadedTheme {
            source: std::borrow::Cow::Borrowed(*src),
            path_label: synthetic_path(name),
        });
    }

    // Unknown — Error::Theme with full available list reusing cached base.
    Err(Error::Theme {
        name: name.to_owned(),
        available: available_theme_names_from_base(base.as_deref()),
    })
}

/// Alphabetically-sorted slice of BUILT-IN theme names. Used by
/// [`crate::config_tui::dump_cmd`] (v0.5.4) and by the in-module
/// test suite. Production-path resolution otherwise goes through
/// [`available_theme_names_from_base`].
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
        if let Some(styles_map) = &r.styles {
            for key in styles_map.keys() {
                // Special-case "0" BEFORE grammar validation: group 0 has
                // a dedicated diagnostic that points at the `style` field.
                if key == "0" {
                    errors.push(crate::error::ThemeRuleError {
                        rule_name: r.name.clone(),
                        kind: crate::error::ThemeRuleErrorKind::CaptureGroupIndexZeroForbidden,
                    });
                    continue;
                }
                // v0.5.1 — Phase-1 only gates digit-shape keys; non-digit
                // keys (e.g. `"scheme"`, `"date"`) defer to Phase-2 named
                // resolution in `Compiled::load_with_theme`. Previously
                // this gate rejected all non-digit keys as
                // CaptureGroupKeyMalformed, making RuleSource::Theme
                // named-key support effectively dead-code (v0.5.0 final
                // cross-cutting review §I-1).
                if key.bytes().all(|b| b.is_ascii_digit())
                    && crate::config::validate_styles_map_key(key).is_none()
                {
                    errors.push(crate::error::ThemeRuleError {
                        rule_name: r.name.clone(),
                        kind: crate::error::ThemeRuleErrorKind::CaptureGroupKeyMalformed {
                            key: key.clone(),
                        },
                    });
                }
            }
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
            let loaded = load(name).expect("known theme should load");
            assert!(
                !loaded.source.trim().is_empty(),
                "theme {name:?} embedded source must not be empty"
            );
        }
    }

    #[test]
    fn load_unknown_theme_returns_error_theme_with_available() {
        // Use `load_with` with no disk base so the available list is
        // deterministic across dev machines (a real `~/.config/tayf/themes`
        // on the dev's box would otherwise leak into this assertion).
        let err = load_with("nope", || None, || None).expect_err("unknown theme must error");
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
            styles: None,
            priority: None,
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
        for &name in names() {
            let loaded = load(name).unwrap();
            let src: &str = &loaded.source;
            let cfg = crate::config::parse(&synthetic_path(name), src)
                .unwrap_or_else(|e| panic!("theme {name:?} did not parse: {e}"));
            validate_theme_rules(name, &synthetic_path(name), &cfg)
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

    use std::fs;
    use std::path::Path;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().expect("create tmpdir")
    }

    fn write_theme(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
        let themes = dir.join("themes");
        fs::create_dir_all(&themes).expect("create themes dir");
        let p = themes.join(format!("{name}.toml"));
        fs::write(&p, body).expect("write theme");
        p
    }

    #[test]
    fn resolve_disk_path_returns_none_when_file_missing() {
        let dir = tmp();
        let resolved = resolve_disk_path_in_base("foo", dir.path()).unwrap();
        assert!(resolved.is_none(), "no file => Ok(None)");
    }

    #[test]
    fn resolve_disk_path_returns_some_for_regular_file() {
        let dir = tmp();
        let path = write_theme(dir.path(), "mine", "");
        let resolved = resolve_disk_path_in_base("mine", dir.path()).unwrap();
        assert_eq!(resolved.as_deref(), Some(path.as_path()));
    }

    #[test]
    #[cfg(unix)]
    fn resolve_disk_path_rejects_symlink_outside_base() {
        use std::os::unix::fs::symlink;
        let dir = tmp();
        let evil_dir = dir.path().join("evil");
        fs::create_dir(&evil_dir).unwrap();
        let evil_target = evil_dir.join("mine.toml");
        fs::write(&evil_target, "# attacker payload\n").unwrap();
        let themes_dir = dir.path().join("themes");
        fs::create_dir(&themes_dir).unwrap();
        let link = themes_dir.join("mine.toml");
        symlink(&evil_target, &link).unwrap();

        let err =
            resolve_disk_path_in_base("mine", dir.path()).expect_err("symlink out must error");
        let msg = err.to_string();
        assert!(
            msg.contains("symlink") || msg.contains("outside") || msg.contains("must live under"),
            "expected symlink-out diagnostic: {msg}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn resolve_disk_path_accepts_symlink_inside_base() {
        use std::os::unix::fs::symlink;
        let dir = tmp();
        let themes_dir = dir.path().join("themes");
        fs::create_dir(&themes_dir).unwrap();
        let real = themes_dir.join("shared.toml");
        fs::write(&real, "").unwrap();
        let link = themes_dir.join("mine.toml");
        symlink(&real, &link).unwrap();

        let resolved = resolve_disk_path_in_base("mine", dir.path()).unwrap();
        assert!(resolved.is_some());
    }

    #[test]
    #[cfg(unix)]
    fn resolve_disk_path_rejects_path_pointing_to_directory() {
        use std::os::unix::fs::symlink;
        let dir = tmp();
        let themes_dir = dir.path().join("themes");
        fs::create_dir(&themes_dir).unwrap();
        let target_dir = themes_dir.join("real_dir");
        fs::create_dir(&target_dir).unwrap();
        let link = themes_dir.join("mine.toml");
        symlink(&target_dir, &link).unwrap();

        let err = resolve_disk_path_in_base("mine", dir.path())
            .expect_err("symlink-to-directory must error");
        let msg = err.to_string();
        assert!(
            msg.contains("not a regular file") || msg.contains("regular file"),
            "expected regular-file diagnostic: {msg}"
        );
    }

    #[test]
    fn resolve_disk_path_returns_none_when_themes_dir_missing() {
        let dir = tmp();
        let resolved = resolve_disk_path_in_base("any", dir.path()).unwrap();
        assert!(resolved.is_none());
    }

    #[test]
    fn discover_disk_themes_ignores_non_toml_files() {
        let dir = tmp();
        let themes_dir = dir.path().join("themes");
        fs::create_dir(&themes_dir).unwrap();
        fs::write(themes_dir.join("good.toml"), "").unwrap();
        fs::write(themes_dir.join("bad.bak"), "").unwrap();
        fs::write(themes_dir.join("noext"), "").unwrap();

        let names = discover_disk_themes(dir.path());
        assert_eq!(names, vec!["good".to_string()]);
    }

    #[test]
    fn discover_disk_themes_returns_empty_when_dir_missing() {
        let dir = tmp();
        let names = discover_disk_themes(dir.path());
        assert!(names.is_empty());
    }

    #[test]
    fn discover_disk_themes_sorts_alphabetically() {
        let dir = tmp();
        let themes_dir = dir.path().join("themes");
        fs::create_dir(&themes_dir).unwrap();
        for n in &["foo", "bar", "baz", "alpha"] {
            fs::write(themes_dir.join(format!("{n}.toml")), "").unwrap();
        }
        let names = discover_disk_themes(dir.path());
        assert_eq!(names, vec!["alpha", "bar", "baz", "foo"]);
    }

    #[test]
    fn available_theme_names_from_base_returns_builtins_when_base_none() {
        let names = available_theme_names_from_base(None);
        assert_eq!(names, vec!["dark".to_string(), "light".to_string()]);
    }

    #[test]
    fn available_theme_names_from_base_merges_builtin_and_disk() {
        let dir = tmp();
        let themes_dir = dir.path().join("themes");
        fs::create_dir(&themes_dir).unwrap();
        fs::write(themes_dir.join("foo.toml"), "").unwrap();
        fs::write(themes_dir.join("bar.toml"), "").unwrap();

        let names = available_theme_names_from_base(Some(dir.path()));
        assert_eq!(
            names,
            vec!["bar".to_string(), "dark".to_string(), "foo".to_string(), "light".to_string()]
        );
    }

    #[test]
    fn available_theme_names_from_base_excludes_collisions_case_insensitively() {
        let dir = tmp();
        let themes_dir = dir.path().join("themes");
        fs::create_dir(&themes_dir).unwrap();
        fs::write(themes_dir.join("dark.toml"), "").unwrap();
        fs::write(themes_dir.join("LIGHT.toml"), "").unwrap();
        fs::write(themes_dir.join("custom.toml"), "").unwrap();

        let names = available_theme_names_from_base(Some(dir.path()));
        assert_eq!(names, vec!["custom".to_string(), "dark".to_string(), "light".to_string()]);
    }

    #[test]
    fn collision_error_carries_path_and_rename_hint() {
        let p = std::path::PathBuf::from("/tmp/themes/dark.toml");
        let e = collision_error("dark", &p);
        let s = e.to_string();
        assert!(s.contains("'dark'"), "should quote theme name: {s}");
        assert!(s.contains("shadows the built-in"), "rationale: {s}");
        assert!(s.contains("rename"), "actionable hint: {s}");
        assert!(s.contains("/tmp/themes/dark.toml"), "should include disk path: {s}");
    }

    #[test]
    fn load_built_in_returns_borrowed_cow() {
        let loaded = load("dark").expect("built-in load");
        assert!(matches!(loaded.source, std::borrow::Cow::Borrowed(_)));
        assert_eq!(loaded.path_label, "<embedded:theme/dark>");
        assert!(!loaded.source.is_empty());
    }

    #[test]
    fn load_with_disk_overrides_returns_owned_cow() {
        let dir = tmp();
        // `config_base` appends `tayf` to the XDG value, so the themes
        // dir we write into is `<xdg>/tayf/themes/`.
        let xdg_tayf = dir.path().join("tayf");
        fs::create_dir_all(&xdg_tayf).unwrap();
        let path = write_theme(&xdg_tayf, "mine", "# disk theme\n");
        let loaded =
            load_with("mine", || Some(dir.path().to_path_buf()), || None).expect("disk load");
        assert!(matches!(loaded.source, std::borrow::Cow::Owned(_)));
        assert!(loaded.source.contains("# disk theme"));
        assert_eq!(loaded.path_label, path.display().to_string());
    }

    #[test]
    fn load_with_xdg_unset_falls_back_to_home() {
        let dir = tmp();
        let home_themes = dir.path().join(".config").join("tayf");
        fs::create_dir_all(&home_themes).unwrap();
        write_theme(&home_themes, "via_home", "# via home\n");
        let loaded =
            load_with("via_home", || None, || Some(dir.path().to_path_buf())).expect("home load");
        assert!(loaded.source.contains("# via home"));
    }

    #[test]
    fn load_with_xdg_empty_is_treated_as_unset() {
        let dir = tmp();
        let home_themes = dir.path().join(".config").join("tayf");
        fs::create_dir_all(&home_themes).unwrap();
        write_theme(&home_themes, "via_home", "# via home\n");
        let loaded = load_with(
            "via_home",
            || Some(std::path::PathBuf::new()), // empty == unset per XDG spec
            || Some(dir.path().to_path_buf()),
        )
        .expect("home load");
        assert!(loaded.source.contains("# via home"));
    }

    #[test]
    fn load_collision_with_builtin_name_errors() {
        let dir = tmp();
        let xdg_tayf = dir.path().join("tayf");
        fs::create_dir_all(&xdg_tayf).unwrap();
        write_theme(&xdg_tayf, "dark", "# user dark theme\n");
        let err = load_with("dark", || Some(dir.path().to_path_buf()), || None)
            .expect_err("disk dark.toml + built-in dark must collide");
        let msg = err.to_string();
        assert!(msg.contains("shadows the built-in"), "got: {msg}");
        assert!(msg.contains("'dark'"), "got: {msg}");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn load_collision_with_mixed_case_built_in_name_errors() {
        // Rev2 I-1 — case-insensitive collision protects macOS APFS users.
        // Guarded to macOS because case-sensitive filesystems (Linux ext4 /
        // tmpfs) cannot trigger the collision arm in the first place: when
        // only `dark.toml` exists on disk and `--theme DARK` is requested,
        // `resolve_disk_path_in_base` returns `Ok(None)` and the collision
        // check is dead. The production guard exists for APFS / HFS+ users.
        let dir = tmp();
        let xdg_tayf = dir.path().join("tayf");
        fs::create_dir_all(&xdg_tayf).unwrap();
        write_theme(&xdg_tayf, "dark", "# user dark theme\n");
        let err = load_with("DARK", || Some(dir.path().to_path_buf()), || None)
            .expect_err("DARK requested with dark.toml on disk must collide");
        assert!(err.to_string().contains("shadows the built-in"));
    }

    #[test]
    fn load_unknown_name_lists_built_ins_and_disk() {
        let dir = tmp();
        let xdg_tayf = dir.path().join("tayf");
        fs::create_dir_all(&xdg_tayf).unwrap();
        write_theme(&xdg_tayf, "foo", "");
        write_theme(&xdg_tayf, "bar", "");
        let err = load_with("baz", || Some(dir.path().to_path_buf()), || None)
            .expect_err("baz not in built-ins, not on disk");
        let crate::error::Error::Theme { available, .. } = err else {
            panic!("expected Error::Theme");
        };
        assert_eq!(available, vec!["bar", "dark", "foo", "light"]);
    }

    #[test]
    fn load_invalid_name_with_separator_fails_fast() {
        let dir = tmp();
        let err = load_with("../etc/passwd", || Some(dir.path().to_path_buf()), || None)
            .expect_err("path separator name must error");
        assert!(matches!(err, crate::error::Error::Theme { .. }));
    }

    #[test]
    fn load_invalid_name_with_dot_fails_fast() {
        let dir = tmp();
        let err = load_with("bad.name", || Some(dir.path().to_path_buf()), || None)
            .expect_err("dot in name must error");
        assert!(matches!(err, crate::error::Error::Theme { .. }));
    }

    #[test]
    fn load_invalid_name_error_lists_disk_themes_too() {
        // Rev2 C-2 — fast-fail branch still consumes closures via config_base
        // and reuses base for the available list.
        let dir = tmp();
        let xdg_tayf = dir.path().join("tayf");
        fs::create_dir_all(&xdg_tayf).unwrap();
        write_theme(&xdg_tayf, "foo", "");
        let err = load_with("../bad", || Some(dir.path().to_path_buf()), || None)
            .expect_err("invalid name");
        let crate::error::Error::Theme { available, .. } = err else {
            panic!("expected Error::Theme");
        };
        assert!(available.contains(&"foo".to_string()), "disk theme listed: {available:?}");
        assert!(available.contains(&"dark".to_string()));
        assert!(available.contains(&"light".to_string()));
    }

    #[test]
    fn validate_theme_rules_collects_capture_group_zero_forbidden() {
        let src = r#"
[[rules]]
name = "ipv4"
style = { fg = "yellow" }
styles = { "0" = { fg = "red" } }
"#;
        let cfg: crate::config::Config = crate::config::parse("<test>", src).unwrap();
        let err =
            validate_theme_rules("dark", "<embedded:theme/dark>", &cfg).expect_err("should fail");
        if let crate::error::Error::ThemeValidation { errors, .. } = err {
            assert_eq!(errors.len(), 1);
            assert!(matches!(
                errors[0].kind,
                crate::error::ThemeRuleErrorKind::CaptureGroupIndexZeroForbidden
            ));
            assert_eq!(errors[0].rule_name, "ipv4");
        } else {
            panic!("expected ThemeValidation");
        }
    }

    #[test]
    fn validate_theme_rules_collects_capture_group_key_malformed() {
        let src = r#"
[[rules]]
name = "ipv4"
styles = { "01" = { fg = "red" } }
"#;
        let cfg: crate::config::Config = crate::config::parse("<test>", src).unwrap();
        let err =
            validate_theme_rules("dark", "<embedded:theme/dark>", &cfg).expect_err("should fail");
        if let crate::error::Error::ThemeValidation { errors, .. } = err {
            assert_eq!(errors.len(), 1);
            match &errors[0].kind {
                crate::error::ThemeRuleErrorKind::CaptureGroupKeyMalformed { key } => {
                    assert_eq!(key, "01");
                }
                other => panic!("unexpected kind: {other:?}"),
            }
        } else {
            panic!("expected ThemeValidation");
        }
    }

    #[test]
    fn validate_theme_rules_collects_multiple_styles_key_errors_in_one_pass() {
        // v0.5.1: Phase-1 only gates digit-shape styles keys; non-digit
        // keys defer to dispatch-time named resolution. Use two digit-shape
        // failure modes to keep the fail-collection invariant exercisable
        // at Phase-1: "0" (IndexZeroForbidden) + "01" (KeyMalformed).
        let src = r#"
[[rules]]
name = "ipv4"
styles = { "0" = { fg = "red" }, "01" = { fg = "blue" } }
"#;
        let cfg: crate::config::Config = crate::config::parse("<test>", src).unwrap();
        let err =
            validate_theme_rules("dark", "<embedded:theme/dark>", &cfg).expect_err("should fail");
        if let crate::error::Error::ThemeValidation { errors, .. } = err {
            assert_eq!(errors.len(), 2, "fail-collected; got: {errors:?}");
        } else {
            panic!("expected ThemeValidation");
        }
    }

    #[test]
    fn validate_theme_rules_accepts_valid_styles_keys_defers_range_check() {
        let src = r#"
[[rules]]
name = "ipv4"
styles = { "1" = { fg = "red" }, "10" = { fg = "blue" } }
"#;
        let cfg: crate::config::Config = crate::config::parse("<test>", src).unwrap();
        // Grammar passes; out-of-range check is deferred to Compiled::load_with_theme.
        validate_theme_rules("dark", "<embedded:theme/dark>", &cfg).expect("grammar ok");
    }

    #[test]
    fn validate_theme_rules_phase1_accepts_non_digit_styles_keys() {
        // v0.5.1 §I-1: Phase-1 must accept non-digit styles-map keys
        // (e.g. `"date"`) and defer name resolution to
        // `Compiled::load_with_theme`. Pre-v0.5.1 these keys were
        // rejected as CaptureGroupKeyMalformed before reaching dispatch.
        use std::collections::BTreeMap;
        let mut styles = BTreeMap::new();
        styles.insert("date".to_owned(), UserStyle::default());
        let cfg = crate::config::Config {
            general: crate::config::GeneralSection::default(),
            rules: vec![UserRule {
                name: "timestamp".into(),
                pattern: None,
                style: None,
                enabled: true,
                styles: Some(styles),
                priority: None,
            }],
        };
        validate_theme_rules("dark", "<embedded:theme/dark>", &cfg)
            .expect("non-digit keys must pass Phase-1 (defer to dispatch)");
    }

    #[test]
    fn load_disk_theme_too_large_rejected() {
        let dir = tmp();
        let xdg_tayf = dir.path().join("tayf");
        fs::create_dir_all(&xdg_tayf).unwrap();
        let big = "a".repeat(crate::config::MAX_CONFIG_BYTES + 1);
        write_theme(&xdg_tayf, "huge", &big);
        let err = load_with("huge", || Some(dir.path().to_path_buf()), || None)
            .expect_err("oversized theme must error");
        assert!(err.to_string().contains("too large"), "got: {err}");
    }
}
