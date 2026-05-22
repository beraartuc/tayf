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
#[allow(dead_code)] // reason: first non-test caller lands in Task 7 (apply_user_rules reads general).
pub(crate) struct GeneralSection {
    /// Accepted but ignored in v0.2.0 — v0.3 will use it once full ANSI
    /// awareness lands.
    #[serde(default = "default_true")]
    pub(crate) respect_existing_colors: bool,
}

impl Default for GeneralSection {
    fn default() -> Self {
        Self { respect_existing_colors: true }
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
#[allow(dead_code)] // reason: first non-test caller lands in Task 7 (apply_user_rules iterates rules).
pub(crate) struct UserRule {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) pattern: Option<String>,
    #[serde(default)]
    pub(crate) style: Option<UserStyle>,
    #[serde(default = "default_true")]
    pub(crate) enabled: bool,
}

/// Inline `style = { ... }` table. Field types are strings/bools so user
/// input goes through [`crate::style::Color::parse_str`] and bool literals.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // reason: first non-test caller lands in Task 7 (apply_user_rules reads style fields).
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
    #[allow(dead_code)] // reason: first non-test caller lands in Task 7 (apply_user_rules).
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

#[allow(dead_code)] // reason: first non-test caller lands in Task 7 (apply_user_rules via UserStyle::to_style).
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
#[allow(dead_code)] // reason: first non-test caller lands in Task 9 (Tayf::run wires config::load).
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

    if let Some(base) = xdg() {
        if let Some(p) = check_default_path(&base.join("tayf"))? {
            return Ok(Some(p));
        }
    }

    if let Some(home) = home() {
        if let Some(p) = check_default_path(&home.join(".config").join("tayf"))? {
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
}
