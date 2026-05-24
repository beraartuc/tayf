//! TOML configuration loading and validation.
//!
//! Public entry: [`load`] resolves the config file (CLI `--config`, then
//! `$XDG_CONFIG_HOME/tayf/config.toml`, then `~/.config/tayf/config.toml`),
//! reads it under a 1 MB cap, parses with `serde` + `toml`, validates rule
//! shape, and returns `Ok(None)` when no file is present (preserving v0.1
//! behavior). See `docs/superpowers/specs/2026-05-21-tayf-v0.2.0-design.md`.

use std::path::Path;

use serde::Deserialize;

use crate::error::{Error, Result};

/// Top-level config shape.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    #[serde(default)]
    pub(crate) general: GeneralSection,
    #[serde(default)]
    pub(crate) rules: Vec<UserRule>,
}

/// `[general]` table. All fields optional; defaults preserve v0.1 behavior.
///
/// `Default` is implemented manually (not derived) so that an entirely
/// missing `[general]` table — handled by `#[serde(default)]` on the parent
/// field — still yields `respect_existing_colors = true`. Deriving `Default`
/// would route through `bool::default()` (= `false`) in that path, while
/// `#[serde(default = "default_true")]` only fires when the table is present
/// but the field itself is omitted.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
// reason: `respect_existing_colors` is parsed and stored but not yet
// consumed in v0.2.0 — full ANSI awareness lands in v0.3, at which
// point this field gates the new behavior. Documented as "accepted
// but ignored" in the field doc-comment below.
#[allow(dead_code)]
pub(crate) struct GeneralSection {
    /// Accepted but ignored in v0.2.0 — v0.3 will use it once full ANSI
    /// awareness lands.
    #[serde(default = "default_true")]
    pub(crate) respect_existing_colors: bool,

    /// Preset theme name applied before user-config rules. `None` means
    /// "no theme" (built-in defaults verbatim). CLI `--theme` overrides
    /// this value. Validated by [`crate::themes::load`]; an unknown name
    /// surfaces as [`crate::Error::Theme`].
    #[serde(default)]
    pub(crate) theme: Option<String>,

    /// v0.3.3: when `true`, the reload orchestrator writes a one-line dim
    /// banner (`tayf: config reloaded`) to `/dev/tty` after each
    /// successful hot reload (file change or `SIGHUP`). Default `false`
    /// (opt-in). Failures during reload do NOT write the banner; they
    /// continue to surface via the existing `warn_msg!` log path on
    /// stderr. The banner is naive about TUI / alt-screen state —
    /// opt-in users accept the trade-off. Alt-screen-aware queuing is
    /// deferred to v0.4.
    #[serde(default)]
    pub(crate) show_reload_banner: bool,
}

impl Default for GeneralSection {
    fn default() -> Self {
        Self { respect_existing_colors: true, theme: None, show_reload_banner: false }
    }
}

fn default_true() -> bool {
    true
}

/// A single `[[rules]]` entry. `pattern` and `style` are optional at the
/// schema level so built-in overrides (which omit them) parse cleanly;
/// validation in `apply_user_rules` enforces that *new* rules supply both.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct UserRule {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) pattern: Option<String>,
    #[serde(default)]
    pub(crate) style: Option<UserStyle>,
    #[serde(default = "default_true")]
    pub(crate) enabled: bool,
    /// Per-capture-group style overlay map. Keys are positive-decimal
    /// strings (capture-group index, 1-based; grammar `^[1-9][0-9]*$`).
    /// Both TOML inline-table (`styles = { "1" = {...} }`) and dotted-table
    /// (`[rules.styles."1"]`) forms parse here via serde. Validation
    /// against the rule's regex `captures_len()` happens at compile time.
    /// See spec §1.3 / §3.3.
    #[serde(default)]
    pub(crate) styles: Option<std::collections::BTreeMap<String, UserStyle>>,
}

/// Inline `style = { ... }` table. Field types are strings/bools so user
/// input goes through [`crate::style::Color::parse_str`] and bool literals.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)] // reason: mirrors the SGR style attribute set (bold/italic/underline/dim); each maps 1:1 to a TOML key and a distinct ANSI code. Collapsing into an enum or bitflags would obscure the user-facing schema.
pub(crate) struct UserStyle {
    #[serde(default)]
    pub(crate) fg: Option<String>,
    #[serde(default)]
    pub(crate) bg: Option<String>,
    #[serde(default)]
    pub(crate) bold: bool,
    #[serde(default)]
    pub(crate) italic: bool,
    #[serde(default)]
    pub(crate) underline: bool,
    #[serde(default)]
    pub(crate) dim: bool,
}

impl UserStyle {
    /// Convert into a [`crate::style::Style`], or produce an actionable
    /// [`Error::Config`].
    ///
    /// `path` and `rule_name` are folded into the error message so users
    /// know exactly which `[[rules]]` entry is wrong.
    pub(crate) fn to_style(&self, path: &str, rule_name: &str) -> Result<crate::style::Style> {
        let fg =
            self.fg.as_deref().map(|s| parse_color_field(path, rule_name, "fg", s)).transpose()?;
        let bg =
            self.bg.as_deref().map(|s| parse_color_field(path, rule_name, "bg", s)).transpose()?;

        let style = crate::style::Style {
            fg,
            bg,
            bold: self.bold,
            italic: self.italic,
            underline: self.underline,
            dim: self.dim,
        };

        if style == crate::style::Style::DEFAULT {
            return Err(Error::Config {
                path: path.into(),
                line: 0,
                message: format!(
                    "rule '{rule_name}': style has no visible effect; use `enabled = false` to disable this rule instead"
                ),
            });
        }

        Ok(style)
    }
}

fn parse_color_field(
    path: &str,
    rule_name: &str,
    field: &str,
    value: &str,
) -> Result<crate::style::Color> {
    crate::style::Color::parse_str(value).map_err(|msg| Error::Config {
        path: path.into(),
        line: 0,
        message: format!("rule '{rule_name}': {field}: {msg}"),
    })
}

/// Returns `Some(n)` if `key` is a valid v0.3.5 positive-decimal capture-
/// group index (grammar: `^[1-9][0-9]*$`); `None` otherwise. The caller
/// decides how to route the rejection (theme vs config error variant).
///
/// `"0"` is NOT accepted here — group 0 has dedicated semantics (the
/// entire match, covered by the rule's `style` field). Callers
/// special-case `key == "0"` BEFORE invoking this helper to emit
/// `ThemeRuleErrorKind::CaptureGroupIndexZeroForbidden`.
#[allow(dead_code)] // reason: Task 9 + Task 10 consume this helper.
pub(crate) fn validate_styles_map_key(key: &str) -> Option<usize> {
    if key.is_empty() {
        return None;
    }
    let bytes = key.as_bytes();
    if bytes[0] == b'0' {
        // Leading-zero (including bare "0") rejected; "0" callers intercept
        // earlier for a dedicated diagnostic.
        return None;
    }
    if !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    key.parse::<usize>().ok()
}

/// Parse the TOML body. Caller supplies `path` for error context.
pub(crate) fn parse(path: &str, source: &str) -> Result<Config> {
    toml::from_str::<Config>(source).map_err(|e| Error::config_from_toml(path.into(), source, e))
}

/// Maximum config size in bytes. Files larger than this are rejected to
/// bound the regex compile and TOML parse work the user can trigger.
pub(crate) const MAX_CONFIG_BYTES: usize = 1024 * 1024;

/// Public entry point — wires real `$XDG_CONFIG_HOME` and `$HOME` env vars
/// into [`load_with`]. Tests use `load_with` directly to avoid mutating env.
///
/// Returns `Some((config, path))` when a config file was loaded, so callers
/// can surface the path in downstream error messages without re-resolving
/// (avoids env-race and file-deletion-between-calls regressions). Returns
/// `Ok(None)` when no config file is present, preserving v0.1 behavior.
pub(crate) fn load(explicit: Option<&Path>) -> Result<Option<(Config, std::path::PathBuf)>> {
    load_with(
        explicit,
        || std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from),
        || std::env::var_os("HOME").map(std::path::PathBuf::from),
    )
}

pub(crate) fn load_with(
    explicit: Option<&Path>,
    xdg: impl FnOnce() -> Option<std::path::PathBuf>,
    home: impl FnOnce() -> Option<std::path::PathBuf>,
) -> Result<Option<(Config, std::path::PathBuf)>> {
    let Some(path) = resolve_path(explicit, xdg, home)? else {
        return Ok(None);
    };
    let path_str = path.to_string_lossy().into_owned();
    let body = read_capped(&path)?;
    let cfg = parse(&path_str, &body)?;
    Ok(Some((cfg, path)))
}

/// Resolve the tayf config base directory using XDG Base Directory rules.
/// Returns `<xdg_config_home>/tayf` if `$XDG_CONFIG_HOME` is set and
/// non-empty, otherwise `<home>/.config/tayf` if `$HOME` is set and
/// non-empty, otherwise `None`.
///
/// Shared by [`resolve_path`] (which appends `config.toml`) and
/// [`crate::themes::load_with`] (which appends `themes/<name>.toml`).
/// Empty-OS-string handling follows the XDG spec: empty
/// `$XDG_CONFIG_HOME` is treated as unset and falls through to `$HOME`.
pub(crate) fn config_base(
    xdg: impl FnOnce() -> Option<std::path::PathBuf>,
    home: impl FnOnce() -> Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    if let Some(b) = xdg() {
        if !b.as_os_str().is_empty() {
            return Some(b.join("tayf"));
        }
    }
    if let Some(h) = home() {
        if !h.as_os_str().is_empty() {
            return Some(h.join(".config").join("tayf"));
        }
    }
    None
}

pub(crate) fn resolve_path(
    explicit: Option<&Path>,
    xdg: impl FnOnce() -> Option<std::path::PathBuf>,
    home: impl FnOnce() -> Option<std::path::PathBuf>,
) -> Result<Option<std::path::PathBuf>> {
    if let Some(p) = explicit {
        let meta = std::fs::metadata(p).map_err(|e| Error::Config {
            path: p.display().to_string(),
            line: 0,
            message: format!("cannot read --config path: {e}"),
        })?;
        if !meta.is_file() {
            return Err(Error::Config {
                path: p.display().to_string(),
                line: 0,
                message: "--config path is not a regular file".into(),
            });
        }
        return Ok(Some(p.to_path_buf()));
    }

    // XDG Base Directory spec: "If $XDG_CONFIG_HOME is either not set or
    // empty, a default equal to $HOME/.config should be used." The empty-OS-
    // string check covers shells that imperatively clear the var via
    // `export XDG_CONFIG_HOME=""` rather than `unset`; without this guard,
    // an empty base would join to a CWD-relative `tayf/config.toml`. The
    // same rule is applied to `$HOME` for defense in depth. Both branches
    // live inside [`config_base`], which is also shared with the themes
    // resolver so the two stay in lockstep.
    if let Some(base) = config_base(xdg, home) {
        if let Some(p) = check_default_path(&base)? {
            return Ok(Some(p));
        }
    }

    Ok(None)
}

/// Check `<base>/config.toml` for existence; enforce the symlink whitelist
/// (canonical file MUST live under canonical base). Returns `Ok(None)` if
/// the candidate doesn't exist, `Ok(Some(path))` if it's safe, or
/// `Err(Error::Config)` if it resolves outside the base.
///
/// Returns the **original** `candidate` (not the canonicalized form) so
/// downstream display doesn't leak platform-specific routing — on macOS,
/// `/tmp/foo/...` would otherwise surface as `/private/tmp/foo/...` and
/// confuse users who never typed `/private`. Canonical form is used solely
/// for the `starts_with` symlink check.
fn check_default_path(base: &Path) -> Result<Option<std::path::PathBuf>> {
    let candidate = base.join("config.toml");
    if !candidate.exists() {
        return Ok(None);
    }
    let canonical_file = std::fs::canonicalize(&candidate).map_err(|e| Error::Config {
        path: candidate.display().to_string(),
        line: 0,
        message: format!("cannot canonicalize config path: {e}"),
    })?;
    // The check is deliberately one-sided: only the CANONICAL target of
    // config.toml is constrained to live under the canonical base.
    //
    // - If `~/.config/tayf` itself is a symlink to `/elsewhere`, both
    //   sides resolve into `/elsewhere/` and the file is accepted. The
    //   user redirected the entire base, which is unambiguous opt-in
    //   (matches the --config escape-hatch philosophy).
    // - If `~/.config/tayf/config.toml` is a symlink to `/elsewhere/x`,
    //   the file's canonical target is outside the base and is rejected.
    //
    // TOCTOU note: the gap between canonicalize and File::open is
    // intentionally not closed. An attacker who can swap files inside
    // `~/.config/tayf/` already has write access there and can put
    // adversarial content directly; symlink games gain them nothing.
    // Cross-user attacks would require shared HOME/XDG, which is outside
    // the single-user threat model (CLAUDE.md §3).
    let canonical_base = std::fs::canonicalize(base).map_err(|e| Error::Config {
        path: base.display().to_string(),
        line: 0,
        message: format!("cannot canonicalize config base directory: {e}"),
    })?;
    if !canonical_file.starts_with(&canonical_base) {
        return Err(Error::Config {
            path: candidate.display().to_string(),
            line: 0,
            message: format!(
                "config file must live under {base}; symlinks pointing outside are rejected. Use --config <PATH> if you intentionally want a file outside this directory.",
                base = canonical_base.display()
            ),
        });
    }
    // Mirror the explicit-path branch: verify the target is a regular file.
    // Without this check, a symlink-to-directory under the base would pass
    // the whitelist, then `File::open` + `read_to_end` would surface a less
    // helpful EISDIR diagnostic than the upfront `is_file` check used by
    // `--config <PATH>`.
    let meta = std::fs::metadata(&canonical_file).map_err(|e| Error::Config {
        path: candidate.display().to_string(),
        line: 0,
        message: format!("cannot stat config target: {e}"),
    })?;
    if !meta.is_file() {
        return Err(Error::Config {
            path: candidate.display().to_string(),
            line: 0,
            message: "config path is not a regular file".into(),
        });
    }
    Ok(Some(candidate))
}

pub(crate) fn read_capped(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| Error::Config {
        path: path.display().to_string(),
        line: 0,
        message: format!("cannot open config: {e}"),
    })?;
    let mut buf = Vec::with_capacity(4096);
    // `Read::take(limit + 1)` so we can distinguish "exactly at cap" from "over cap".
    let _read =
        (&mut file).take((MAX_CONFIG_BYTES as u64) + 1).read_to_end(&mut buf).map_err(|e| {
            Error::Config {
                path: path.display().to_string(),
                line: 0,
                message: format!("cannot read config: {e}"),
            }
        })?;
    if buf.len() > MAX_CONFIG_BYTES {
        return Err(Error::Config {
            path: path.display().to_string(),
            line: 0,
            message: format!(
                "config file too large: {actual} bytes (max {MAX_CONFIG_BYTES})",
                actual = buf.len()
            ),
        });
    }
    String::from_utf8(buf).map_err(|e| Error::Config {
        path: path.display().to_string(),
        line: 0,
        message: format!("config is not valid UTF-8: {e}"),
    })
}

/// Merge user-defined rules into the built-in vector following spec §4.2.
///
/// Semantics (strict):
/// - `name` matching a built-in → override-in-place. `pattern`, `style`,
///   and `enabled = false` all apply individually.
/// - `name` NOT matching a built-in → new custom rule, appended at the end.
///   Both `pattern` and `style` are required; `style` must yield a visible
///   effect (via [`UserStyle::to_style`]).
/// - `enabled = false` removes the matching rule (built-in or already-appended
///   custom) without further inspection of `pattern`/`style`.
///
/// `path` flows into error messages so users see file/rule context.
#[cfg(test)]
// reason: production code now calls `apply_user_rules_with_source` directly
// (Task 10), but the test suite in this module exercises the two-arg shape
// extensively. Wrapping cfg(test) keeps the surface honest in release builds.
pub(crate) fn apply_user_rules(
    path: &str,
    builtins: &mut Vec<crate::rules::BuiltinRule>,
    user: &[UserRule],
) -> Result<()> {
    apply_user_rules_with_source(path, builtins, user, false)
}

/// Variant of [`apply_user_rules`] that tags any `styles` map writes with
/// the supplied `from_theme` flag, so `Compiled::load_with_theme` can route
/// later range/key errors to either [`Error::ThemeValidation`] (theme) or
/// [`Error::Config`] (user config). The base [`apply_user_rules`] calls
/// through with `from_theme = false`; the theme layer in
/// `Compiled::load_with_theme` calls through with `from_theme = true`.
///
/// Spec ref: §3.6, Rev2 I-1 (fail-collected theme routing).
pub(crate) fn apply_user_rules_with_source(
    path: &str,
    builtins: &mut Vec<crate::rules::BuiltinRule>,
    user: &[UserRule],
    from_theme: bool,
) -> Result<()> {
    let known: std::collections::HashSet<&str> =
        crate::rules::BUILTIN_NAMES.iter().copied().collect();
    let mut seen: std::collections::HashSet<&str> =
        std::collections::HashSet::with_capacity(user.len());

    for ur in user {
        // Validate rule name shape before anything else — silent acceptance
        // of empty/whitespace names is the worst footgun in this loop.
        if ur.name.trim().is_empty() {
            return Err(Error::Config {
                path: path.into(),
                line: 0,
                message: "rule name must not be empty".into(),
            });
        }
        if ur.name != ur.name.trim() {
            return Err(Error::Config {
                path: path.into(),
                line: 0,
                message: format!(
                    "rule '{n}': name has leading/trailing whitespace; did you mean '{t}'?",
                    n = ur.name,
                    t = ur.name.trim()
                ),
            });
        }

        if !seen.insert(ur.name.as_str()) {
            return Err(Error::Config {
                path: path.into(),
                line: 0,
                message: format!(
                    "rule '{n}': defined more than once; merge the entries into a single `[[rules]]` block",
                    n = ur.name
                ),
            });
        }

        let is_builtin = known.contains(ur.name.as_str());

        if !ur.enabled {
            builtins.retain(|b| b.name != ur.name);
            continue;
        }

        if is_builtin {
            // Override in place.
            let Some(existing) = builtins.iter_mut().find(|b| b.name == ur.name) else {
                // `enabled = true` re-introduction of a previously disabled
                // built-in within the same TOML — not a documented case in
                // v0.2.0; treat as a friendly error rather than silently
                // ignoring it.
                return Err(Error::Config {
                    path: path.into(),
                    line: 0,
                    message: format!(
                        "rule '{name}': appears twice with conflicting `enabled` values",
                        name = ur.name
                    ),
                });
            };
            if let Some(p) = &ur.pattern {
                existing.pattern.clone_from(p);
                existing.is_user_supplied = true;
            }
            if let Some(s) = &ur.style {
                existing.style = s.to_style(path, &ur.name)?;
            }
            // v0.3.5: REPLACE semantics (Rev2 Karar 27). When a user/theme
            // entry supplies a `styles = { ... }` map for a built-in, the
            // built-in's pre-populated `group_styles` is REPLACED in full —
            // not merged — at `Compiled::load_with_theme` build time. We
            // plumb the raw map here; range/key validation against the
            // compiled regex's `captures_len()` happens in
            // `rules::resolve_group_styles_for_rule`.
            if ur.styles.is_some() {
                existing.styles_override.clone_from(&ur.styles);
                existing.styles_override_from_theme = from_theme;
            }
        } else {
            // New custom rule — both pattern and style required.
            let Some(pattern) = ur.pattern.clone() else {
                return Err(Error::Config {
                    path: path.into(),
                    line: 0,
                    message: format!(
                        "rule '{name}': missing `pattern` (no built-in by this name; new rules must define one)",
                        name = ur.name
                    ),
                });
            };
            let Some(user_style) = &ur.style else {
                return Err(Error::Config {
                    path: path.into(),
                    line: 0,
                    message: format!(
                        "rule '{name}': missing `style` (new rules must define one)",
                        name = ur.name
                    ),
                });
            };
            let style = user_style.to_style(path, &ur.name)?;
            builtins.push(crate::rules::BuiltinRule {
                name: ur.name.clone(),
                pattern,
                style,
                group_styles: Vec::new(),
                is_user_supplied: true,
                styles_override: ur.styles.clone(),
                // New custom rule from theme TOML is currently unreachable —
                // `themes::validate_theme_rules` rejects names not in
                // `BUILTIN_NAMES` before this point. We still propagate
                // `from_theme` for forward-compat (defensive).
                styles_override_from_theme: from_theme,
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_yields_defaults() {
        let cfg = parse("/x", "").unwrap();
        assert!(cfg.general.respect_existing_colors);
        assert!(cfg.rules.is_empty());
    }

    #[test]
    fn parse_general_only() {
        let cfg = parse("/x", "[general]\nrespect_existing_colors = false\n").unwrap();
        assert!(!cfg.general.respect_existing_colors);
    }

    #[test]
    fn parse_full_example() {
        let src = r##"
[general]
respect_existing_colors = true

[[rules]]
name = "log_level"
style = { fg = "yellow", bold = true }

[[rules]]
name = "fqdn"
enabled = false

[[rules]]
name = "uuid"
pattern = '\b[0-9a-fA-F]{8}\b'
style = { fg = "#888888" }
"##;
        let cfg = parse("/x", src).unwrap();
        assert_eq!(cfg.rules.len(), 3);
        assert_eq!(cfg.rules[0].name, "log_level");
        assert!(cfg.rules[0].pattern.is_none());
        assert!(cfg.rules[0].enabled);
        assert_eq!(cfg.rules[0].style.as_ref().unwrap().fg.as_deref(), Some("yellow"));
        assert!(cfg.rules[0].style.as_ref().unwrap().bold);
        assert_eq!(cfg.rules[1].name, "fqdn");
        assert!(!cfg.rules[1].enabled);
        assert_eq!(cfg.rules[2].pattern.as_deref(), Some(r"\b[0-9a-fA-F]{8}\b"));
        assert_eq!(cfg.rules[2].style.as_ref().unwrap().fg.as_deref(), Some("#888888"));
    }

    #[test]
    fn user_rule_parses_inline_styles_map() {
        let src = r#"
[[rules]]
name = "ipv4"
style = { fg = "yellow" }
styles = { "1" = { fg = "red" }, "2" = { fg = "blue" } }
"#;
        let cfg: Config = parse("<test>", src).expect("parse");
        assert_eq!(cfg.rules.len(), 1);
        let r = &cfg.rules[0];
        let styles = r.styles.as_ref().expect("styles set");
        assert_eq!(styles.len(), 2);
        assert!(styles.contains_key("1"));
        assert!(styles.contains_key("2"));
    }

    #[test]
    fn user_rule_parses_dotted_table_styles_form() {
        let src = r#"
[[rules]]
name = "ipv4"
style = { fg = "yellow" }
[rules.styles."1"]
fg = "red"
[rules.styles."2"]
fg = "blue"
"#;
        let cfg: Config = parse("<test>", src).expect("parse");
        assert_eq!(cfg.rules.len(), 1);
        let r = &cfg.rules[0];
        let styles = r.styles.as_ref().expect("styles set");
        assert_eq!(styles.len(), 2);
    }

    #[test]
    fn user_rule_parses_empty_styles_map_as_some_empty() {
        let src = r#"
[[rules]]
name = "ipv4"
styles = {}
"#;
        let cfg: Config = parse("<test>", src).expect("parse");
        let r = &cfg.rules[0];
        assert_eq!(r.styles.as_ref().map(std::collections::BTreeMap::len), Some(0));
    }

    #[test]
    fn user_rule_styles_absent_defaults_to_none() {
        let src = r#"
[[rules]]
name = "ipv4"
"#;
        let cfg: Config = parse("<test>", src).expect("parse");
        let r = &cfg.rules[0];
        assert_eq!(r.styles, None);
    }

    #[test]
    fn unknown_top_level_field_rejected() {
        let src = "rulez = []\n";
        let err = parse("/x", src).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("/x"));
        assert!(msg.to_lowercase().contains("unknown") || msg.contains("rulez"));
    }

    #[test]
    fn unknown_style_field_rejected() {
        let src = r#"
[[rules]]
name = "x"
pattern = "foo"
style = { colour = "red" }
"#;
        let err = parse("/x", src).unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("colour")
                || err.to_string().to_lowercase().contains("unknown")
        );
    }

    #[test]
    fn rule_missing_name_rejected() {
        let src = r#"
[[rules]]
pattern = "foo"
style = { fg = "red" }
"#;
        let err = parse("/x", src).unwrap_err();
        assert!(err.to_string().to_lowercase().contains("name"));
    }

    #[test]
    fn malformed_toml_carries_line_number() {
        // `notabool` is an unquoted identifier where a bool/string is expected;
        // toml 0.9 reliably errors at the offending token with a span.
        let src = "[general]\nrespect_existing_colors = notabool\n";
        let err = parse("/cfg.toml", src).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("/cfg.toml"));
        // The offending token sits on line 2; toml::Error::span points there.
        assert!(msg.contains(":2") || msg.contains("line 2"), "expected line 2 in: {msg}");
    }

    use crate::style::{Color, Style};

    #[test]
    fn user_style_with_fg_only() {
        let us = UserStyle { fg: Some("yellow".into()), ..UserStyle::default() };
        let s = us.to_style("/x", "log_level").unwrap();
        assert_eq!(s.fg, Some(Color::Yellow));
        assert_eq!(s.bg, None);
        assert!(!s.bold);
    }

    #[test]
    fn user_style_full_round_trip() {
        let us = UserStyle {
            fg: Some("#ff8800".into()),
            bg: Some("color(0)".into()),
            bold: true,
            italic: true,
            underline: true,
            dim: false,
        };
        let s = us.to_style("/x", "kubernetes-pod").unwrap();
        assert_eq!(s.fg, Some(Color::Rgb(0xff, 0x88, 0x00)));
        assert_eq!(s.bg, Some(Color::Indexed(0)));
        assert!(s.bold && s.italic && s.underline && !s.dim);
    }

    #[test]
    fn user_style_bad_color_carries_rule_name() {
        let us = UserStyle { fg: Some("turquoise".into()), ..UserStyle::default() };
        let err = us.to_style("/x/cfg.toml", "log_level").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("log_level"), "rule name missing: {msg}");
        assert!(msg.contains("turquoise"));
        assert!(msg.contains("/x/cfg.toml"));
    }

    #[test]
    fn user_style_bad_bg_color_carries_field_label() {
        // Mirrors user_style_bad_color_carries_rule_name but exercises the bg
        // branch of parse_color_field. The "bg:" substring is load-bearing —
        // a user with both fg and bg set needs to know which field to fix.
        let us = UserStyle { bg: Some("turquoise".into()), ..UserStyle::default() };
        let err = us.to_style("/x/cfg.toml", "log_level").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("log_level"), "rule name missing: {msg}");
        assert!(msg.contains("bg:"), "field label must distinguish bg from fg: {msg}");
        assert!(msg.contains("turquoise"));
    }

    #[test]
    fn user_style_attribute_only_is_accepted() {
        let us = UserStyle { bold: true, ..UserStyle::default() };
        let s = us.to_style("/x", "any").unwrap();
        assert_eq!(s, Style { bold: true, ..Style::DEFAULT });
    }

    #[test]
    fn user_style_dim_only_is_accepted() {
        // `dim` is the SGR attribute most likely to be filtered by minimal
        // terminals, so a rule with only `dim = true` is the legitimate-but-
        // suspect case. The design intent: a single attribute, even a weak
        // one, counts as a visible effect — Style::DEFAULT equality is the
        // sole rejection trigger.
        let us = UserStyle { dim: true, ..UserStyle::default() };
        let s = us.to_style("/x", "any").unwrap();
        assert_eq!(s, Style { dim: true, ..Style::DEFAULT });
    }

    #[test]
    fn user_style_empty_is_rejected_with_actionable_message() {
        let us = UserStyle::default();
        let err = us.to_style("/x", "uuid").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("uuid"));
        assert!(
            msg.contains("no visible effect") || msg.contains("enabled = false"),
            "must hint at the fix: {msg}"
        );
    }

    use std::fs;
    use std::path::PathBuf;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().expect("create tmpdir")
    }

    fn write_config(dir: &Path, body: &str) -> PathBuf {
        let p = dir.join("config.toml");
        fs::write(&p, body).expect("write config");
        p
    }

    #[test]
    fn resolve_returns_explicit_path_when_present() {
        let dir = tmp();
        let path = write_config(dir.path(), "");
        let resolved = resolve_path(Some(&path), || None, || None).unwrap();
        assert_eq!(resolved.as_deref(), Some(path.as_path()));
    }

    #[test]
    fn resolve_errors_when_explicit_path_missing() {
        let err =
            resolve_path(Some(Path::new("/nonexistent/cfg.toml")), || None, || None).unwrap_err();
        assert!(err.to_string().contains("/nonexistent/cfg.toml"));
    }

    #[test]
    fn resolve_falls_back_to_xdg() {
        let dir = tmp();
        let tayf_dir = dir.path().join("tayf");
        fs::create_dir(&tayf_dir).unwrap();
        let path = write_config(&tayf_dir, "");
        let resolved = resolve_path(None, || Some(dir.path().to_path_buf()), || None).unwrap();
        assert_eq!(resolved.as_deref(), Some(path.as_path()));
    }

    #[test]
    fn resolve_falls_back_to_home_when_xdg_unset() {
        let dir = tmp();
        let tayf_dir = dir.path().join(".config").join("tayf");
        fs::create_dir_all(&tayf_dir).unwrap();
        let path = write_config(&tayf_dir, "");
        let resolved = resolve_path(None, || None, || Some(dir.path().to_path_buf())).unwrap();
        assert_eq!(resolved.as_deref(), Some(path.as_path()));
    }

    #[test]
    fn resolve_returns_none_when_nothing_exists() {
        let dir = tmp();
        let resolved = resolve_path(
            None,
            || Some(dir.path().join("nowhere")),
            || Some(dir.path().join("nowhere2")),
        )
        .unwrap();
        assert!(resolved.is_none());
    }

    #[test]
    fn resolve_treats_empty_xdg_as_unset_per_xdg_spec() {
        // XDG Base Directory spec: "If $XDG_CONFIG_HOME is either not set or
        // empty, a default equal to $HOME/.config should be used." Tayf's own
        // design doc (tayf-tasarim.md §3) inherits this rule. Regression guard
        // for a foot-gun where `export XDG_CONFIG_HOME=""` would otherwise
        // route to a CWD-relative `tayf/config.toml` lookup.
        let dir = tmp();
        let tayf_dir = dir.path().join(".config").join("tayf");
        fs::create_dir_all(&tayf_dir).unwrap();
        let path = write_config(&tayf_dir, "");
        let resolved = resolve_path(
            None,
            || Some(std::path::PathBuf::new()), // empty path simulates empty XDG_CONFIG_HOME
            || Some(dir.path().to_path_buf()),
        )
        .unwrap();
        assert_eq!(resolved.as_deref(), Some(path.as_path()));
    }

    #[test]
    #[cfg(unix)]
    fn resolve_rejects_default_path_symlinked_outside_base() {
        use std::os::unix::fs::symlink;
        // Create the legitimate base dir AND a sibling "evil" dir; symlink the
        // config.toml from inside the base to a file in the sibling.
        let dir = tmp();
        let tayf_dir = dir.path().join("tayf");
        let evil_dir = dir.path().join("evil");
        std::fs::create_dir(&tayf_dir).unwrap();
        std::fs::create_dir(&evil_dir).unwrap();
        let evil_target = evil_dir.join("config.toml");
        std::fs::write(&evil_target, "# attacker payload\n").unwrap();
        let link_path = tayf_dir.join("config.toml");
        symlink(&evil_target, &link_path).unwrap();

        // The symlink lives under `dir.path()/tayf` (the base), but after
        // canonicalization it resolves to `dir.path()/evil/config.toml`,
        // which is outside the base. Must be rejected.
        let err = resolve_path(None, || Some(dir.path().to_path_buf()), || None).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("symlink") || msg.contains("outside") || msg.contains("must live under"),
            "expected symlink-out diagnostic in: {msg}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn resolve_accepts_default_path_when_symlink_stays_inside_base() {
        use std::os::unix::fs::symlink;
        // A symlink to a regular file INSIDE the same base is fine — covers the
        // common "config.toml -> shared.toml" case in the user's own dir.
        let dir = tmp();
        let tayf_dir = dir.path().join("tayf");
        std::fs::create_dir(&tayf_dir).unwrap();
        let real = tayf_dir.join("shared.toml");
        std::fs::write(&real, "").unwrap();
        let link = tayf_dir.join("config.toml");
        symlink(&real, &link).unwrap();

        let resolved = resolve_path(None, || Some(dir.path().to_path_buf()), || None).unwrap();
        assert!(resolved.is_some());
    }

    #[test]
    #[cfg(unix)]
    fn resolve_rejects_default_path_symlinked_to_directory() {
        use std::os::unix::fs::symlink;
        // Mirror of the explicit-path "not a regular file" guard for the
        // default-path branch. Without it the user would see EISDIR via the
        // subsequent read; this fails upfront with the same diagnostic shape.
        let dir = tmp();
        let tayf_dir = dir.path().join("tayf");
        fs::create_dir(&tayf_dir).unwrap();
        let target_dir = tayf_dir.join("real_dir");
        fs::create_dir(&target_dir).unwrap();
        let link = tayf_dir.join("config.toml");
        symlink(&target_dir, &link).unwrap();

        let err = resolve_path(None, || Some(dir.path().to_path_buf()), || None).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("not a regular file") || msg.contains("regular file"),
            "expected regular-file diagnostic: {msg}"
        );
    }

    #[test]
    fn resolve_explicit_config_bypasses_whitelist() {
        // --config <path> is the explicit escape hatch: it goes through even
        // when the file is nowhere near $HOME or $XDG_CONFIG_HOME.
        let dir = tmp();
        let outside = dir.path().join("anywhere.toml");
        std::fs::write(&outside, "").unwrap();
        let resolved = resolve_path(Some(&outside), || None, || None).unwrap();
        assert_eq!(resolved.as_deref(), Some(outside.as_path()));
    }

    #[test]
    fn read_under_cap_returns_body() {
        let dir = tmp();
        let path = write_config(dir.path(), "[general]\n");
        let body = read_capped(&path).unwrap();
        assert_eq!(body, "[general]\n");
    }

    #[test]
    fn read_over_cap_errors() {
        let dir = tmp();
        let big = "a".repeat(MAX_CONFIG_BYTES + 1);
        let path = write_config(dir.path(), &big);
        let err = read_capped(&path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("too large"));
        assert!(msg.contains(&MAX_CONFIG_BYTES.to_string()));
    }

    #[test]
    fn load_returns_none_when_no_config() {
        let dir = tmp();
        let cfg = load_with(
            None,
            || Some(dir.path().join("nowhere")),
            || Some(dir.path().join("nowhere2")),
        )
        .unwrap();
        assert!(cfg.is_none());
    }

    #[test]
    fn load_returns_some_for_valid_explicit_path() {
        let dir = tmp();
        let path = write_config(dir.path(), "[general]\n");
        let (cfg, loaded_path) = load_with(Some(&path), || None, || None).unwrap().unwrap();
        assert!(cfg.general.respect_existing_colors);
        assert_eq!(loaded_path, path, "loaded path must round-trip exactly");
    }

    use crate::rules::{builtin_rules, BUILTIN_NAMES};

    fn user_rule(name: &str) -> UserRule {
        UserRule { name: name.into(), pattern: None, style: None, enabled: true, styles: None }
    }

    #[test]
    fn apply_with_no_user_rules_is_identity() {
        let mut rules = builtin_rules();
        let before_len = rules.len();
        apply_user_rules("/x", &mut rules, &[]).unwrap();
        assert_eq!(rules.len(), before_len);
    }

    #[test]
    fn override_builtin_style_replaces_wholesale() {
        let mut rules = builtin_rules();
        // log_level built-in is bold + BrightRed. Override with just yellow.
        let user = vec![UserRule {
            name: "log_level".into(),
            pattern: None,
            style: Some(UserStyle { fg: Some("yellow".into()), ..UserStyle::default() }),
            enabled: true,
            styles: None,
        }];
        apply_user_rules("/x", &mut rules, &user).unwrap();
        let log = rules.iter().find(|r| r.name == "log_level").expect("present");
        assert_eq!(log.style.fg, Some(crate::style::Color::Yellow));
        assert!(!log.style.bold, "REPLACE semantics: built-in bold must NOT carry over");
    }

    #[test]
    fn disable_removes_builtin() {
        let mut rules = builtin_rules();
        let user = vec![UserRule { enabled: false, ..user_rule("fqdn") }];
        apply_user_rules("/x", &mut rules, &user).unwrap();
        assert!(rules.iter().all(|r| r.name != "fqdn"));
    }

    #[test]
    fn append_new_custom_rule() {
        let mut rules = builtin_rules();
        let user = vec![UserRule {
            name: "custom_id".into(),
            pattern: Some(r"\b[0-9a-fA-F]{8}\b".into()),
            style: Some(UserStyle { fg: Some("#888888".into()), ..UserStyle::default() }),
            enabled: true,
            styles: None,
        }];
        apply_user_rules("/x", &mut rules, &user).unwrap();
        let appended = rules.last().expect("appended");
        assert_eq!(appended.name, "custom_id");
        assert_eq!(appended.style.fg, Some(crate::style::Color::Rgb(0x88, 0x88, 0x88)));
    }

    #[test]
    fn appended_rules_preserve_declaration_order() {
        let mut rules = builtin_rules();
        let user = vec![
            UserRule {
                name: "a".into(),
                pattern: Some("a".into()),
                style: Some(UserStyle { fg: Some("red".into()), ..UserStyle::default() }),
                enabled: true,
                styles: None,
            },
            UserRule {
                name: "b".into(),
                pattern: Some("b".into()),
                style: Some(UserStyle { fg: Some("blue".into()), ..UserStyle::default() }),
                enabled: true,
                styles: None,
            },
        ];
        apply_user_rules("/x", &mut rules, &user).unwrap();
        let last_two: Vec<&str> = rules.iter().rev().take(2).map(|r| r.name.as_str()).collect();
        assert_eq!(last_two, vec!["b", "a"], "user rules append in TOML order");
    }

    #[test]
    fn override_replaces_pattern_when_provided() {
        let mut rules = builtin_rules();
        let user = vec![UserRule {
            name: "ipv4".into(),
            pattern: Some(r"\bX\.X\.X\.X\b".into()),
            style: Some(UserStyle { fg: Some("red".into()), ..UserStyle::default() }),
            enabled: true,
            styles: None,
        }];
        apply_user_rules("/x", &mut rules, &user).unwrap();
        let ipv4 = rules.iter().find(|r| r.name == "ipv4").unwrap();
        assert_eq!(ipv4.pattern, r"\bX\.X\.X\.X\b");
    }

    #[test]
    fn new_rule_without_pattern_is_rejected() {
        let mut rules = builtin_rules();
        let user = vec![UserRule {
            name: "custom_id".into(),
            pattern: None,
            style: Some(UserStyle { fg: Some("red".into()), ..UserStyle::default() }),
            enabled: true,
            styles: None,
        }];
        let err = apply_user_rules("/x", &mut rules, &user).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("custom_id"));
        assert!(msg.to_lowercase().contains("pattern"));
    }

    #[test]
    fn new_rule_without_style_is_rejected() {
        let mut rules = builtin_rules();
        let user = vec![UserRule {
            name: "custom_id".into(),
            pattern: Some(r"\b[0-9a-f]{8}\b".into()),
            style: None,
            enabled: true,
            styles: None,
        }];
        let err = apply_user_rules("/x", &mut rules, &user).unwrap_err();
        assert!(err.to_string().contains("custom_id"));
        assert!(err.to_string().to_lowercase().contains("style"));
    }

    #[test]
    fn disable_user_appended_rule_is_noop() {
        // A disabled user-only rule must simply not be added.
        let mut rules = builtin_rules();
        let before_len = rules.len();
        let user = vec![UserRule {
            name: "custom_id".into(),
            pattern: Some("X".into()),
            style: Some(UserStyle { fg: Some("red".into()), ..UserStyle::default() }),
            enabled: false,
            styles: None,
        }];
        apply_user_rules("/x", &mut rules, &user).unwrap();
        assert_eq!(rules.len(), before_len);
    }

    #[test]
    fn builtin_names_constant_is_exhaustive() {
        let rules = builtin_rules();
        let from_rules: std::collections::HashSet<&str> =
            rules.iter().map(|r| r.name.as_str()).collect();
        let from_const: std::collections::HashSet<&str> = BUILTIN_NAMES.iter().copied().collect();
        assert_eq!(from_rules, from_const);
    }

    #[test]
    fn override_with_invalid_color_propagates_error_with_rule_name() {
        let mut rules = builtin_rules();
        let user = vec![UserRule {
            name: "log_level".into(),
            pattern: None,
            style: Some(UserStyle { fg: Some("turquoise".into()), ..UserStyle::default() }),
            enabled: true,
            styles: None,
        }];
        let err = apply_user_rules("/x/cfg.toml", &mut rules, &user).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("log_level"));
        assert!(msg.contains("turquoise"));
    }

    #[test]
    fn empty_rule_name_is_rejected() {
        let mut rules = builtin_rules();
        let user = vec![UserRule {
            name: String::new(),
            pattern: Some("X".into()),
            style: Some(UserStyle { fg: Some("red".into()), ..UserStyle::default() }),
            enabled: true,
            styles: None,
        }];
        let err = apply_user_rules("/x", &mut rules, &user).unwrap_err();
        let msg = err.to_string();
        assert!(msg.to_lowercase().contains("empty"), "expected empty-name diagnostic: {msg}");
    }

    #[test]
    fn whitespace_rule_name_is_rejected_with_hint() {
        let mut rules = builtin_rules();
        let user = vec![UserRule {
            name: " ipv4 ".into(),
            pattern: None,
            style: Some(UserStyle { fg: Some("red".into()), ..UserStyle::default() }),
            enabled: true,
            styles: None,
        }];
        let err = apply_user_rules("/x", &mut rules, &user).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("' ipv4 '"), "must echo the bad name: {msg}");
        assert!(msg.contains("'ipv4'"), "must suggest the trimmed form: {msg}");
    }

    #[test]
    fn duplicate_user_rule_name_is_rejected() {
        let mut rules = builtin_rules();
        let user = vec![
            UserRule {
                name: "uuid".into(),
                pattern: Some("X".into()),
                style: Some(UserStyle { fg: Some("red".into()), ..UserStyle::default() }),
                enabled: true,
                styles: None,
            },
            UserRule {
                name: "uuid".into(),
                pattern: Some("Y".into()),
                style: Some(UserStyle { fg: Some("blue".into()), ..UserStyle::default() }),
                enabled: true,
                styles: None,
            },
        ];
        let err = apply_user_rules("/x", &mut rules, &user).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("uuid"), "must name the duplicate: {msg}");
        assert!(
            msg.to_lowercase().contains("more than once")
                || msg.to_lowercase().contains("duplicate"),
            "must say duplicate: {msg}"
        );
    }

    #[test]
    fn duplicate_builtin_override_is_rejected() {
        // Same protection applies to overriding a built-in twice.
        let mut rules = builtin_rules();
        let user = vec![
            UserRule {
                name: "log_level".into(),
                pattern: None,
                style: Some(UserStyle { fg: Some("yellow".into()), ..UserStyle::default() }),
                enabled: true,
                styles: None,
            },
            UserRule {
                name: "log_level".into(),
                pattern: None,
                style: Some(UserStyle { fg: Some("cyan".into()), ..UserStyle::default() }),
                enabled: true,
                styles: None,
            },
        ];
        let err = apply_user_rules("/x", &mut rules, &user).unwrap_err();
        assert!(err.to_string().to_lowercase().contains("more than once"));
    }

    #[test]
    fn parse_picks_up_general_theme() {
        let src = r#"
[general]
theme = "light"
"#;
        let cfg = parse("/x", src).unwrap();
        assert_eq!(cfg.general.theme.as_deref(), Some("light"));
        assert!(cfg.general.respect_existing_colors, "default preserved");
    }

    #[test]
    fn parse_omits_general_theme_defaults_to_none() {
        let cfg = parse("/x", "").unwrap();
        assert!(cfg.general.theme.is_none());
    }

    #[test]
    fn show_reload_banner_defaults_to_false_when_omitted() {
        let cfg = parse("test", "[general]\n").unwrap();
        assert!(!cfg.general.show_reload_banner);
    }

    #[test]
    fn show_reload_banner_defaults_to_false_when_general_section_missing() {
        let cfg = parse("test", "").unwrap();
        assert!(!cfg.general.show_reload_banner);
    }

    #[test]
    fn show_reload_banner_parses_true() {
        let cfg = parse("test", "[general]\nshow_reload_banner = true\n").unwrap();
        assert!(cfg.general.show_reload_banner);
    }

    #[test]
    fn show_reload_banner_parses_false_explicit() {
        let cfg = parse("test", "[general]\nshow_reload_banner = false\n").unwrap();
        assert!(!cfg.general.show_reload_banner);
    }

    #[test]
    fn show_reload_banner_unknown_typo_rejected() {
        let err = parse("test", "[general]\nreload_banner = true\n").unwrap_err();
        assert!(matches!(err, crate::error::Error::Config { .. }));
    }

    #[test]
    fn config_base_returns_some_when_xdg_set() {
        let base = config_base(|| Some(std::path::PathBuf::from("/tmp/cfg")), || None);
        assert_eq!(base, Some(std::path::PathBuf::from("/tmp/cfg/tayf")));
    }

    #[test]
    fn config_base_falls_back_to_home_when_xdg_unset() {
        let base = config_base(|| None, || Some(std::path::PathBuf::from("/home/u")));
        assert_eq!(base, Some(std::path::PathBuf::from("/home/u/.config/tayf")));
    }

    #[test]
    fn config_base_treats_empty_xdg_as_unset() {
        let base = config_base(
            || Some(std::path::PathBuf::new()), // empty
            || Some(std::path::PathBuf::from("/home/u")),
        );
        assert_eq!(base, Some(std::path::PathBuf::from("/home/u/.config/tayf")));
    }

    #[test]
    fn config_base_returns_none_when_neither_set() {
        let base = config_base(|| None, || None);
        assert!(base.is_none());
    }

    #[test]
    fn config_base_treats_empty_home_as_unset() {
        let base = config_base(|| None, || Some(std::path::PathBuf::new()));
        assert!(base.is_none());
    }

    #[test]
    fn validate_styles_map_key_accepts_positive_decimals() {
        assert_eq!(validate_styles_map_key("1"), Some(1));
        assert_eq!(validate_styles_map_key("2"), Some(2));
        assert_eq!(validate_styles_map_key("99"), Some(99));
        assert_eq!(validate_styles_map_key("100"), Some(100));
    }

    #[test]
    fn validate_styles_map_key_rejects_zero_and_leading_zero() {
        assert_eq!(validate_styles_map_key("0"), None);
        assert_eq!(validate_styles_map_key("01"), None);
        assert_eq!(validate_styles_map_key("00"), None);
        assert_eq!(validate_styles_map_key("099"), None);
    }

    #[test]
    fn validate_styles_map_key_rejects_empty_and_whitespace() {
        assert_eq!(validate_styles_map_key(""), None);
        assert_eq!(validate_styles_map_key(" "), None);
        assert_eq!(validate_styles_map_key(" 1"), None);
        assert_eq!(validate_styles_map_key("1 "), None);
    }

    #[test]
    fn validate_styles_map_key_rejects_signs_and_decimals_and_alpha() {
        assert_eq!(validate_styles_map_key("+1"), None);
        assert_eq!(validate_styles_map_key("-1"), None);
        assert_eq!(validate_styles_map_key("1.0"), None);
        assert_eq!(validate_styles_map_key("abc"), None);
        assert_eq!(validate_styles_map_key("1abc"), None);
        assert_eq!(validate_styles_map_key("a1"), None);
    }
}
