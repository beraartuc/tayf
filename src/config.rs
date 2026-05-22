//! TOML configuration loading and validation.
//!
//! Public entry: [`load`] resolves the config file (CLI `--config`, then
//! `$XDG_CONFIG_HOME/tayf/config.toml`, then `~/.config/tayf/config.toml`),
//! reads it under a 1 MB cap, parses with `serde` + `toml`, validates rule
//! shape, and returns `Ok(None)` when no file is present (preserving v0.1
//! behavior). See `docs/superpowers/specs/2026-05-21-tayf-v0.2.0-design.md`.

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
#[allow(dead_code)] // reason: first non-test caller lands in Task 6 (config::load reads the file and calls parse).
pub(crate) fn parse(path: &str, source: &str) -> Result<Config> {
    toml::from_str::<Config>(source).map_err(|e| Error::config_from_toml(path.into(), source, e))
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
}
