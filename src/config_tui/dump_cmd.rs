//! `tayf config dump` implementation (no ratatui). Iterates
//! `rules::builtin_rules()` + `themes::REGISTRY` + `profiles::EMBEDDED_PROFILES`
//! and serializes to stdout as TOML via `toml::ser::to_string_pretty`.
//!
//! `--kind` restricts the output to one section; default emits all
//! three concatenated with one blank line between sections.

use std::fmt::Write as _;
use std::io::Write;
use std::process::ExitCode;

use crate::cli::DumpKind;

/// Entry point invoked by `crate::config_tui::dump` dispatcher.
///
/// Writes the catalog to stdout. Returns `ExitCode::SUCCESS` on
/// success; `ExitCode::from(70)` (`EX_SOFTWARE`) on a stdout write
/// error (practically unreachable in a normal process context).
pub(crate) fn run(kind: Option<DumpKind>) -> ExitCode {
    let body = render(kind);
    let mut out = std::io::stdout().lock();
    if out.write_all(body.as_bytes()).is_err() {
        return ExitCode::from(70);
    }
    ExitCode::SUCCESS
}

/// Pure render fn — returns the catalog string. Separated from `run`
/// so unit tests can assert byte-pinned output without intercepting
/// stdout.
#[must_use]
pub(crate) fn render(kind: Option<DumpKind>) -> String {
    let mut out = String::new();
    let want_patterns = matches!(kind, None | Some(DumpKind::Patterns));
    let want_themes = matches!(kind, None | Some(DumpKind::Themes));
    let want_profiles = matches!(kind, None | Some(DumpKind::Profiles));

    if want_patterns {
        out.push_str("# Built-in pattern CATALOG (reference, not paste-ready).\n");
        out.push_str("# The [[patterns]] table below is documentation only — your\n");
        out.push_str("# user config uses [[rules]] (not [[patterns]]). To override a\n");
        out.push_str("# built-in style, add a [[rules]] entry in ~/.config/tayf/config.toml\n");
        out.push_str("# with the same `name` and your `pattern` + `style` fields.\n\n");
        for rule in crate::rules::builtin_rules() {
            out.push_str("[[patterns]]\n");
            // reason: writeln! on String avoids the temporary allocation
            // that push_str(&format!(...)) would create per call.
            let _ = writeln!(out, "name = {:?}", rule.name);
            let _ = writeln!(out, "pattern = {:?}", rule.pattern);
            out.push('\n');
        }
    }

    if want_themes {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("# Built-in themes. Override a built-in name by writing\n");
        out.push_str("# `<config_base>/themes/<name>.toml` (disk theme wins).\n\n");
        for name in crate::themes::names() {
            let _ = writeln!(out, "[themes.{name}]");
            out.push_str("# (body omitted — see assets/themes/*.toml in the tayf source tree)\n\n");
        }
    }

    if want_profiles {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("# Embedded profile library shipped with tayf. Override an\n");
        out.push_str("# embedded profile by writing `<config_base>/profiles/<name>.toml`.\n\n");
        for name in crate::profiles::embedded_profile_names() {
            let _ = writeln!(out, "[profiles.{name}]");
            out.push_str(
                "# (body omitted — see assets/profiles/*.toml in the tayf source tree)\n\n",
            );
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use crate::cli::DumpKind;

    #[test]
    fn dump_patterns_only_emits_patterns_section() {
        let out = super::render(Some(DumpKind::Patterns));
        assert!(
            out.contains("[[patterns]]"),
            "dump --kind patterns must emit at least one [[patterns]] table; got: {out}"
        );
        assert!(
            !out.contains("[themes."),
            "dump --kind patterns must NOT emit [themes.*]; got: {out}"
        );
        assert!(
            !out.contains("[profiles."),
            "dump --kind patterns must NOT emit [profiles.*]; got: {out}"
        );
    }

    #[test]
    fn dump_default_kind_emits_all_three_sections() {
        let out = super::render(None);
        assert!(out.contains("[[patterns]]"), "default dump must emit [[patterns]]");
        assert!(out.contains("[themes.dark]"), "default dump must emit [themes.dark]");
        assert!(out.contains("[themes.light]"), "default dump must emit [themes.light]");
        assert!(out.contains("[profiles.aws]"), "default dump must emit [profiles.aws]");
        assert!(out.contains("[profiles.k8s]"), "default dump must emit [profiles.k8s]");
        assert!(out.contains("[profiles.docker]"), "default dump must emit [profiles.docker]");
        assert!(out.contains("[profiles.gcp]"), "default dump must emit [profiles.gcp]");
        assert!(out.contains("[profiles.network]"), "default dump must emit [profiles.network]");
    }

    #[test]
    fn dump_themes_only_excludes_patterns_and_profiles() {
        let out = super::render(Some(DumpKind::Themes));
        assert!(
            out.contains("[themes."),
            "dump --kind themes must emit at least one [themes.*] table; got: {out}"
        );
        assert!(
            !out.contains("[[patterns]]"),
            "dump --kind themes must NOT emit [[patterns]]; got: {out}"
        );
        assert!(
            !out.contains("[profiles."),
            "dump --kind themes must NOT emit [profiles.*]; got: {out}"
        );
    }

    #[test]
    fn dump_profiles_only_excludes_patterns_and_themes() {
        let out = super::render(Some(DumpKind::Profiles));
        assert!(
            out.contains("[profiles."),
            "dump --kind profiles must emit at least one [profiles.*] table; got: {out}"
        );
        assert!(
            !out.contains("[[patterns]]"),
            "dump --kind profiles must NOT emit [[patterns]]; got: {out}"
        );
        assert!(
            !out.contains("[themes."),
            "dump --kind profiles must NOT emit [themes.*]; got: {out}"
        );
    }

    #[test]
    fn dump_patterns_only_round_trips_as_valid_toml() {
        let out = super::render(Some(DumpKind::Patterns));
        let parsed: toml::Value = toml::de::from_str(&out)
            .unwrap_or_else(|e| panic!("dump must be valid TOML; err={e}; body=\n{out}"));
        let patterns = parsed
            .get("patterns")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("expected top-level [[patterns]] array; got: {parsed:?}"));
        assert_eq!(
            patterns.len(),
            crate::rules::builtin_rules().len(),
            "patterns count must match builtin_rules"
        );
    }
}
