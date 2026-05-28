//! `compile_pending` — synthesize `Config` from `ConfigSnapshot` +
//! `PendingEdits` and invoke `rules::compile_from_config` for live
//! preview recompile.
//!
//! `ReDoS` guard: each [`crate::config_tui::edit::NewRule`] pattern is
//! validated via `RegexBuilder::size_limit(REGEX_SIZE_LIMIT_BYTES)`
//! before the synth `Config` is built. Mirrors the
//! `size_limit`/`dfa_size_limit` invocation in `src/rules.rs`
//! `Compiled::load_with_theme` so adversarial patterns cannot `DoS` the
//! preview thread by reaching the heavier compile path with an
//! oversized NFA. CLAUDE.md §3 (security threat model).
//!
//! Edit composition order (mirrors spec §6.x):
//! 1. Clone `snapshot.parsed.rules` as the base.
//! 2. Apply pattern + default-style overlays from
//!    [`PendingEdits::rules`] (`UserConfig` variant only in v0.6;
//!    `Builtin` / `Embedded` / `DiskProfile` override paths land in
//!    v0.6.1+).
//! 3. Filter out [`PendingEdits::deleted`] entries.
//! 4. Validate + push [`PendingEdits::added`] entries.
//! 5. Synthesize a [`crate::config::Config`] and invoke
//!    [`crate::rules::compile_from_config`].
//!
//! All four overlay paths delegate downstream validation to
//! `compile_from_config`. The only validation done locally is the
//! `ReDoS`-guard pre-flight on new patterns; everything else (style
//! grammar, capture-group bounds, theme/profile resolution) flows
//! through the canonical entry-point so there is a single source of
//! truth for merge semantics (memory
//! `feedback_parallel_call_site_invariant_audit`).

use crate::config_tui::edit::{NewRule, NewStyle, PendingEdits, RuleId, StyleKey};
use crate::config_tui::snapshot::ConfigSnapshot;
use crate::rules::{compile_from_config, Compiled};

// reason: mirrors `src/rules.rs` `REGEX_SIZE_LIMIT_BYTES` constant
// (module-private there); duplicating the literal here is preferable
// to widening visibility of an internal NFA-size cap. Keep in sync by
// inspection — both values gate the same ReDoS guard. See
// `src/rules.rs:15` and CLAUDE.md §3.
const REGEX_SIZE_LIMIT_BYTES: usize = 1 << 20;

/// Failure modes for [`compile_pending`]. `InvalidPattern` originates
/// from the local `ReDoS` pre-flight on [`NewRule`] patterns;
/// `CompileFailed` wraps any error surfaced by
/// [`crate::rules::compile_from_config`] after the synth `Config` has
/// been built.
#[derive(Debug)]
pub(crate) enum CompileError {
    /// A user-supplied [`NewRule`] pattern failed to compile via the
    /// `RegexBuilder::size_limit` guard.
    InvalidPattern {
        /// The [`NewRule::name`] of the offending added rule.
        rule_name: String,
        /// The underlying `regex` crate error message.
        source: String,
    },
    /// [`crate::rules::compile_from_config`] itself returned an error
    /// after [`PendingEdits`] were applied (e.g., merge validation
    /// failed, theme/profile resolution failed).
    CompileFailed(Box<crate::error::Error>),
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPattern { rule_name, source } => {
                write!(f, "invalid regex for rule '{rule_name}': {source}")
            }
            Self::CompileFailed(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CompileError {}

/// Recompile a [`Compiled`] from a frozen [`ConfigSnapshot`] plus a
/// staged [`PendingEdits`] delta, with optional `theme_name` and
/// `profile_name` overrides.
///
/// Validates added patterns through the same `size_limit` guard used
/// by [`crate::rules::Compiled::load_with_theme`] (`ReDoS` defence),
/// then delegates merge semantics to
/// [`crate::rules::compile_from_config`] — the single canonical entry
/// point for config-driven `Compiled` construction. See module-level
/// docs for the overlay composition order.
///
/// # Errors
/// - [`CompileError::InvalidPattern`] if any [`NewRule`] pattern
///   fails the local `RegexBuilder::size_limit` guard.
/// - [`CompileError::CompileFailed`] for any error surfaced by
///   [`crate::rules::compile_from_config`].
pub(crate) fn compile_pending(
    snapshot: &ConfigSnapshot,
    edits: &PendingEdits,
    theme_name: Option<&str>,
    profile_name: Option<&str>,
) -> Result<Compiled, CompileError> {
    // 1. Clone the snapshot's user_rules as the base.
    let mut user_rules = snapshot.parsed.rules.clone();

    // 2. Apply rules overlay (pattern + default-style edits) for
    //    UserConfig RuleIds. Builtin / Embedded / DiskProfile overlays
    //    are deferred to v0.6.1+; until then those variants are
    //    silently ignored here (the TUI does not yet expose entry
    //    points that construct them).
    for (rule_id, rule_edit) in &edits.rules {
        if let RuleId::UserConfig(name) = rule_id {
            if let Some(existing) = user_rules.iter_mut().find(|r| &r.name == name) {
                if let Some(new_pat) = &rule_edit.pattern {
                    existing.pattern = Some(new_pat.clone());
                }
                if let Some(new_style) = rule_edit.styles.get(&StyleKey::Default) {
                    apply_new_style_to_user_rule(existing, new_style);
                }
            }
        }
    }

    // 3. Apply deletions (UserConfig variant only — non-UserConfig
    //    variants name builtins/embedded/disk-profile rules that don't
    //    appear in `user_rules` and so naturally no-op here).
    user_rules.retain(|r| !edits.deleted.contains(&RuleId::UserConfig(r.name.clone())));

    // 4. ReDoS-guard each added pattern, then push as a UserRule.
    for new_rule in &edits.added {
        regex::bytes::RegexBuilder::new(&new_rule.pattern)
            .size_limit(REGEX_SIZE_LIMIT_BYTES)
            .dfa_size_limit(REGEX_SIZE_LIMIT_BYTES)
            .build()
            .map_err(|e| CompileError::InvalidPattern {
                rule_name: new_rule.name.clone(),
                source: e.to_string(),
            })?;
        user_rules.push(new_rule_to_user_rule(new_rule));
    }

    // 5. Synthesize Config + delegate compilation.
    let synth =
        crate::config::Config { general: snapshot.parsed.general.clone(), rules: user_rules };
    compile_from_config(&synth, theme_name, profile_name)
        .map_err(|e| CompileError::CompileFailed(Box::new(e)))
}

/// Convert a TUI-side [`NewRule`] into the on-disk [`crate::config::UserRule`]
/// shape expected by `compile_from_config`. Style axes that were never
/// edited (outer `None` on [`NewStyle`] fields) map to the
/// [`crate::config::UserStyle`] default for that axis.
fn new_rule_to_user_rule(new_rule: &NewRule) -> crate::config::UserRule {
    let style = new_style_to_user_style(&new_rule.style);
    crate::config::UserRule {
        name: new_rule.name.clone(),
        pattern: Some(new_rule.pattern.clone()),
        style: Some(style),
        enabled: true,
        styles: None,
        priority: None,
    }
}

/// Project [`NewStyle`] (tri-state per axis) onto the on-disk
/// [`crate::config::UserStyle`] (single-state per axis with `String`
/// colour fields). Unedited axes collapse to the `UserStyle` default
/// (no colour / attribute off).
fn new_style_to_user_style(ns: &NewStyle) -> crate::config::UserStyle {
    crate::config::UserStyle {
        fg: ns.fg.unwrap_or(None).map(crate::style::Color::to_toml_str),
        bg: ns.bg.unwrap_or(None).map(crate::style::Color::to_toml_str),
        bold: ns.bold.unwrap_or(false),
        italic: ns.italic.unwrap_or(false),
        underline: ns.underline.unwrap_or(false),
        dim: ns.dim.unwrap_or(false),
    }
}

/// Overlay a staged [`NewStyle`] onto an existing on-disk
/// [`crate::config::UserStyle`]. Axes left unedited
/// (outer `None`) preserve the existing value; edited axes overwrite.
/// If the existing rule has `style: None`, a fresh `UserStyle::default()`
/// receives the overlay.
fn apply_new_style_to_user_rule(rule: &mut crate::config::UserRule, ns: &NewStyle) {
    let mut us = rule.style.clone().unwrap_or_default();
    if let Some(fg) = ns.fg {
        us.fg = fg.map(crate::style::Color::to_toml_str);
    }
    if let Some(bg) = ns.bg {
        us.bg = bg.map(crate::style::Color::to_toml_str);
    }
    if let Some(b) = ns.bold {
        us.bold = b;
    }
    if let Some(i) = ns.italic {
        us.italic = i;
    }
    if let Some(u) = ns.underline {
        us.underline = u;
    }
    if let Some(d) = ns.dim {
        us.dim = d;
    }
    rule.style = Some(us);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{UserRule, UserStyle};
    use crate::config_tui::edit::{NewRule, NewStyle, RuleEdit};
    use crate::style::Color;
    use std::collections::HashMap;

    /// A `UserStyle` with a single visible attribute — enough to pass
    /// `to_style`'s "no visible effect" guard.
    fn visible_user_style() -> UserStyle {
        UserStyle { fg: Some("red".into()), ..UserStyle::default() }
    }

    /// A `NewStyle` with a single visible attribute — enough to pass
    /// `to_style`'s "no visible effect" guard once collapsed via
    /// `new_style_to_user_style`.
    fn visible_new_style() -> NewStyle {
        NewStyle { fg: Some(Some(Color::Magenta)), ..Default::default() }
    }

    fn snapshot_with_one_user_rule(name: &str, pattern: &str) -> ConfigSnapshot {
        let mut snap = ConfigSnapshot::empty();
        snap.parsed.rules.push(UserRule {
            name: name.to_owned(),
            pattern: Some(pattern.to_owned()),
            style: Some(visible_user_style()),
            enabled: true,
            styles: None,
            priority: None,
        });
        snap
    }

    #[test]
    fn compile_error_display_invalid_pattern_includes_rule_name_and_source() {
        let err = CompileError::InvalidPattern {
            rule_name: "test_rule".to_owned(),
            source: "regex syntax error".to_owned(),
        };
        assert_eq!(err.to_string(), "invalid regex for rule 'test_rule': regex syntax error");
    }

    #[test]
    fn compile_pending_empty_edits_returns_compiled_with_builtins() {
        let snapshot = ConfigSnapshot::empty();
        let edits = PendingEdits::default();
        let compiled = compile_pending(&snapshot, &edits, None, None).expect("compile");
        assert!(compiled.individuals.len() >= 12, "at least 12 builtins");
    }

    #[test]
    fn compile_pending_added_rule_appears_in_compiled() {
        let snapshot = ConfigSnapshot::empty();
        let mut edits = PendingEdits::default();
        edits.added.push(NewRule {
            name: "test_pattern".to_owned(),
            pattern: r"\bTEST\b".to_owned(),
            style: visible_new_style(),
        });
        let compiled = compile_pending(&snapshot, &edits, None, None).expect("compile");
        let found = compiled.individuals.iter().any(|re| re.as_str() == r"\bTEST\b");
        assert!(found, "added pattern present in compiled individuals");
    }

    #[test]
    fn compile_pending_added_rule_invalid_pattern_errs_with_rule_name() {
        let snapshot = ConfigSnapshot::empty();
        let mut edits = PendingEdits::default();
        edits.added.push(NewRule {
            name: "bad_pattern".to_owned(),
            pattern: r"[invalid".to_owned(),
            style: NewStyle::default(),
        });
        let err = compile_pending(&snapshot, &edits, None, None).expect_err("must err");
        match err {
            CompileError::InvalidPattern { rule_name, .. } => {
                assert_eq!(rule_name, "bad_pattern");
            }
            CompileError::CompileFailed(_) => panic!("expected InvalidPattern variant"),
        }
    }

    #[test]
    fn compile_pending_rejects_oversized_pattern_with_size_limit_error() {
        // `[01]{4,1000000}` is the canonical size_limit-buster proven in
        // `src/rules.rs::tests::load_rejects_pattern_exceeding_size_limit`
        // — the bounded-counted-repetition compiler expands the upper
        // bound into the NFA representation past 1 MiB.
        let snapshot = ConfigSnapshot::empty();
        let mut edits = PendingEdits::default();
        edits.added.push(NewRule {
            name: "redos_attempt".to_owned(),
            pattern: "[01]{4,1000000}".to_owned(),
            style: NewStyle::default(),
        });
        let err = compile_pending(&snapshot, &edits, None, None).expect_err("must err");
        match err {
            CompileError::InvalidPattern { rule_name, source } => {
                assert_eq!(rule_name, "redos_attempt");
                let lc = source.to_lowercase();
                assert!(
                    lc.contains("size") || lc.contains("limit"),
                    "size limit error mentions size or limit, got: {source}"
                );
            }
            CompileError::CompileFailed(_) => panic!("expected InvalidPattern from size_limit"),
        }
    }

    #[test]
    fn compile_pending_user_rule_pattern_override_updates_existing() {
        let snap = snapshot_with_one_user_rule("foo", r"\bfoo\b");
        let mut edits = PendingEdits::default();
        edits.rules.insert(
            RuleId::UserConfig("foo".to_owned()),
            RuleEdit { pattern: Some(r"\bFOO\b".to_owned()), styles: HashMap::new() },
        );
        let compiled = compile_pending(&snap, &edits, None, None).expect("compile");
        let found = compiled.individuals.iter().any(|re| re.as_str() == r"\bFOO\b");
        assert!(found, "override pattern \\bFOO\\b present");
        let still_old = compiled.individuals.iter().any(|re| re.as_str() == r"\bfoo\b");
        assert!(!still_old, "old pattern \\bfoo\\b replaced");
    }

    #[test]
    fn compile_pending_deleted_user_rule_dropped_from_compiled() {
        let snap = snapshot_with_one_user_rule("to_delete", r"\bdelete_me\b");
        let mut edits = PendingEdits::default();
        edits.deleted.insert(RuleId::UserConfig("to_delete".to_owned()));
        let compiled = compile_pending(&snap, &edits, None, None).expect("compile");
        let found = compiled.individuals.iter().any(|re| re.as_str() == r"\bdelete_me\b");
        assert!(!found, "deleted rule pattern absent from compiled");
    }

    #[test]
    fn compile_pending_compose_added_modified_deleted_in_single_call() {
        let snap = snapshot_with_one_user_rule("existing", r"\bold\b");
        let mut edits = PendingEdits::default();
        // Modify existing pattern (keeps the snapshot's visible style).
        edits.rules.insert(
            RuleId::UserConfig("existing".to_owned()),
            RuleEdit { pattern: Some(r"\bnew\b".to_owned()), styles: HashMap::new() },
        );
        // Add a wholly new rule with a visible style so `to_style`
        // accepts it.
        edits.added.push(NewRule {
            name: "added".to_owned(),
            pattern: r"\bADDED\b".to_owned(),
            style: visible_new_style(),
        });
        let compiled = compile_pending(&snap, &edits, None, None).expect("compile");
        let has_new = compiled.individuals.iter().any(|re| re.as_str() == r"\bnew\b");
        let has_added = compiled.individuals.iter().any(|re| re.as_str() == r"\bADDED\b");
        let has_old = compiled.individuals.iter().any(|re| re.as_str() == r"\bold\b");
        assert!(has_new, "modified pattern present");
        assert!(has_added, "added pattern present");
        assert!(!has_old, "old pattern replaced");
    }
}
