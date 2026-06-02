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
//!    [`PendingEdits::rules`] for ALL `RuleId` variants. `UserConfig`
//!    in-place mutates an existing entry; `Builtin` / `Embedded` /
//!    `DiskProfile` use dedupe-then-mutate-or-push (find existing entry
//!    by name → in-place edit; else synth `UserRule` push). See spec
//!    §3.2 + §3.2.5 trace through `apply_user_rules_with_source`.
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
    depth: crate::terminfo::ColorDepth,
) -> Result<Compiled, CompileError> {
    // 1. Clone the snapshot's user_rules as the base.
    let mut user_rules = snapshot.parsed.rules.clone();

    // 2. Apply rules overlay (pattern + default-style edits) for all
    //    RuleId variants. The synth-UserRule push strategy mirrors
    //    `apply_user_rules_with_source` (config.rs) in-place name-
    //    match semantics: if `user_rules` already contains an entry
    //    matching the rule's name, mutate in place; otherwise append a
    //    synth UserRule. This avoids the `seen` HashSet duplicate
    //    rejection inside `apply_user_rules_with_source` while still
    //    routing all non-UserConfig overlays through the canonical
    //    rules.rs mechanism. See spec §3.2 + §3.2.5 trace.
    for (rule_id, rule_edit) in &edits.rules {
        let name_owned: String = match rule_id {
            RuleId::UserConfig(n) => n.clone(),
            RuleId::Builtin(n) => (*n).to_owned(),
            RuleId::DiskProfile { rule, .. } => rule.clone(),
        };
        // Pattern overlay None semantics: Some(_) → set; None → no-op.
        let new_pat = rule_edit.pattern.clone();
        // Style field None semantics: only Some(_) when the Default
        // StyleKey is explicitly edited. `apply_user_rules_with_source`
        // skips `style: None` UserRules — preserves the source style.
        let new_style_opt = rule_edit.styles.get(&StyleKey::Default);

        if let Some(existing) = user_rules.iter_mut().find(|r| r.name == name_owned) {
            // In-place mutate (mirrors UserConfig path; works for any
            // variant because the canonical apply_user_rules_with_source
            // applies the same in-place semantics on name match).
            if let Some(p) = &new_pat {
                existing.pattern = Some(p.clone());
            }
            if let Some(ns) = new_style_opt {
                apply_new_style_to_user_rule(existing, ns);
            }
            // Enabled flip (spec §5): Some(_) overrides the entry's
            // enabled state; None leaves it unchanged.
            if let Some(en) = rule_edit.enabled {
                existing.enabled = en;
            }
        } else if !matches!(rule_id, RuleId::UserConfig(_)) {
            // Non-UserConfig variant with no pre-existing user_rules
            // entry: push a synth UserRule. apply_user_rules_with_source
            // will then name-match it against the canonical rules vec
            // (built-ins + theme + profile rules) and apply the
            // overlay in-place there. UserConfig variant with no match
            // = snapshot drift; silently no-op (matches v0.6 shipped).
            //
            // `enabled` carries the staged toggle (spec §5): an OFF flip
            // synthesizes `enabled = false` (the canonical suppress path);
            // an ON flip of a default-off built-in synthesizes `enabled =
            // true`, which the merge applies in place because all 18
            // built-ins are present in the working set (spec §3.2). No
            // flip → `true` (preserve the default-enabled state).
            user_rules.push(crate::config::UserRule {
                name: name_owned,
                pattern: new_pat,
                style: new_style_opt.map(new_style_to_user_style),
                enabled: rule_edit.enabled.unwrap_or(true),
                styles: None,
                priority: None,
            });
        }
    }

    // 3. Apply deletions for all 4 RuleId variants. Spec v0.6.2 §3.3.
    //
    // `UserConfig` entries live in `user_rules` directly — drop them with
    // `retain`. Builtin / Embedded / DiskProfile rules are injected by
    // `compile_from_config` downstream and are NOT present in `user_rules`;
    // we suppress them by pushing an `enabled = false` UserRule whose name
    // matches the target, which causes `apply_user_rules_with_source` in
    // rules.rs to skip the rule during the canonical merge step.
    //
    // `enabled = false` semantics are established in rules.rs §5 (see
    // `load_with_all_builtins_disabled_yields_passthrough` test) and are
    // guaranteed to work for any variant name present in the canonical
    // rules vec (built-ins + profile rules).
    for rule_id in &edits.deleted {
        match rule_id {
            RuleId::UserConfig(name) => {
                user_rules.retain(|r| r.name != name.as_str());
            }
            RuleId::Builtin(name) => {
                // Suppress a builtin by injecting `enabled = false` so the
                // canonical apply_user_rules_with_source path removes it.
                // Only inject if no existing user_rules entry already covers
                // this name (avoid duplicate-seen rejection inside
                // `apply_user_rules_with_source`).
                let name: &str = name;
                if let Some(r) = user_rules.iter_mut().find(|r| r.name == name) {
                    r.enabled = false;
                } else {
                    user_rules.push(crate::config::UserRule {
                        name: name.to_owned(),
                        pattern: None,
                        style: None,
                        enabled: false,
                        styles: None,
                        priority: None,
                    });
                }
            }
            RuleId::DiskProfile { rule: name, .. } => {
                // `enabled = false` suppression for disk-profile rules.
                let name: &str = name.as_str();
                if let Some(r) = user_rules.iter_mut().find(|r| r.name == name) {
                    r.enabled = false;
                } else {
                    user_rules.push(crate::config::UserRule {
                        name: name.to_owned(),
                        pattern: None,
                        style: None,
                        enabled: false,
                        styles: None,
                        priority: None,
                    });
                }
            }
        }
    }

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
    compile_from_config(&synth, theme_name, profile_name, depth)
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
        // Tri-state flatten: outer None and Some(None) both collapse to false
        // on the on-disk shape (UserStyle has a single bool per axis). Only
        // Some(Some(true)) survives as true. Spec §3.1.
        bold: ns.bold.flatten().unwrap_or(false),
        italic: ns.italic.flatten().unwrap_or(false),
        underline: ns.underline.flatten().unwrap_or(false),
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
    // Per-axis tri-state arms: outer None preserves existing, Some(None)
    // clears to false, Some(Some(b)) sets the inner value. Mirrors the
    // axis row in `reconcile.rs` `write_style_table`. Spec §3.1.
    match ns.bold {
        None => {}
        Some(None) => us.bold = false,
        Some(Some(b)) => us.bold = b,
    }
    match ns.italic {
        None => {}
        Some(None) => us.italic = false,
        Some(Some(b)) => us.italic = b,
    }
    match ns.underline {
        None => {}
        Some(None) => us.underline = false,
        Some(Some(b)) => us.underline = b,
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
    use crate::terminfo::ColorDepth;
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
        let compiled =
            compile_pending(&snapshot, &edits, None, None, crate::terminfo::ColorDepth::Truecolor)
                .expect("compile");
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
        let compiled =
            compile_pending(&snapshot, &edits, None, None, crate::terminfo::ColorDepth::Truecolor)
                .expect("compile");
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
        let err =
            compile_pending(&snapshot, &edits, None, None, crate::terminfo::ColorDepth::Truecolor)
                .expect_err("must err");
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
        let err =
            compile_pending(&snapshot, &edits, None, None, crate::terminfo::ColorDepth::Truecolor)
                .expect_err("must err");
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
            RuleEdit {
                pattern: Some(r"\bFOO\b".to_owned()),
                styles: HashMap::new(),
                enabled: None,
            },
        );
        let compiled =
            compile_pending(&snap, &edits, None, None, crate::terminfo::ColorDepth::Truecolor)
                .expect("compile");
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
        let compiled =
            compile_pending(&snap, &edits, None, None, crate::terminfo::ColorDepth::Truecolor)
                .expect("compile");
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
            RuleEdit {
                pattern: Some(r"\bnew\b".to_owned()),
                styles: HashMap::new(),
                enabled: None,
            },
        );
        // Add a wholly new rule with a visible style so `to_style`
        // accepts it.
        edits.added.push(NewRule {
            name: "added".to_owned(),
            pattern: r"\bADDED\b".to_owned(),
            style: visible_new_style(),
        });
        let compiled =
            compile_pending(&snap, &edits, None, None, crate::terminfo::ColorDepth::Truecolor)
                .expect("compile");
        let has_new = compiled.individuals.iter().any(|re| re.as_str() == r"\bnew\b");
        let has_added = compiled.individuals.iter().any(|re| re.as_str() == r"\bADDED\b");
        let has_old = compiled.individuals.iter().any(|re| re.as_str() == r"\bold\b");
        assert!(has_new, "modified pattern present");
        assert!(has_added, "added pattern present");
        assert!(!has_old, "old pattern replaced");
    }

    #[test]
    fn compile_pending_builtin_pattern_override_replaces_builtin_pattern() {
        // Given a Builtin RuleId overlay with a custom pattern,
        // expect compile_pending to produce a Compiled whose individuals
        // for ipv4 use the custom pattern (not the shipped default).
        let snapshot = ConfigSnapshot::empty();
        let mut edits = PendingEdits::default();
        edits.rules.insert(
            RuleId::Builtin("ipv4"),
            RuleEdit {
                pattern: Some(r"\b1\.2\.3\.4\b".to_owned()),
                styles: HashMap::new(),
                enabled: None,
            },
        );
        let compiled =
            compile_pending(&snapshot, &edits, None, None, crate::terminfo::ColorDepth::Truecolor)
                .expect("compile");
        let has_new = compiled.individuals.iter().any(|re| re.as_str() == r"\b1\.2\.3\.4\b");
        let has_old = compiled
            .individuals
            .iter()
            .any(|re| re.as_str().contains(r"\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}"));
        assert!(has_new, "expected new pattern \\b1\\.2\\.3\\.4\\b in compiled");
        assert!(!has_old, "expected default ipv4 pattern absent (overlay replaces)");
    }

    #[test]
    fn compile_pending_builtin_style_override_replaces_default_style() {
        // Default ipv4 has its shipped color; overlay sets fg=Red;
        // resulting Compiled rule for ipv4 must report fg=Red.
        let snapshot = ConfigSnapshot::empty();
        let mut edits = PendingEdits::default();
        let mut styles = HashMap::new();
        styles.insert(
            StyleKey::Default,
            NewStyle { fg: Some(Some(Color::Red)), ..Default::default() },
        );
        edits
            .rules
            .insert(RuleId::Builtin("ipv4"), RuleEdit { pattern: None, styles, enabled: None });
        let compiled =
            compile_pending(&snapshot, &edits, None, None, crate::terminfo::ColorDepth::Truecolor)
                .expect("compile");
        // Locate the ipv4 entry by name in BUILTIN_NAMES order.
        let ipv4_idx = crate::rules::BUILTIN_NAMES
            .iter()
            .position(|n| *n == "ipv4")
            .expect("ipv4 in BUILTIN_NAMES");
        let style = compiled.styles.get(ipv4_idx).expect("ipv4 style row present");
        assert_eq!(style.fg, Some(crate::style::Color::Red), "fg=Red applied via overlay");
    }

    #[test]
    fn compile_pending_builtin_overlay_pattern_only_preserves_original_style() {
        // Pattern override; no style override. The resulting rule should
        // have the new pattern AND the shipped default style for ipv4.
        let snapshot = ConfigSnapshot::empty();
        let baseline = compile_pending(
            &snapshot,
            &PendingEdits::default(),
            None,
            None,
            crate::terminfo::ColorDepth::Truecolor,
        )
        .expect("baseline compile");
        let ipv4_idx = crate::rules::BUILTIN_NAMES
            .iter()
            .position(|n| *n == "ipv4")
            .expect("ipv4 in BUILTIN_NAMES");
        let original_style = baseline.styles[ipv4_idx];

        let mut edits = PendingEdits::default();
        edits.rules.insert(
            RuleId::Builtin("ipv4"),
            RuleEdit {
                pattern: Some(r"\bxxx\b".to_owned()),
                styles: HashMap::new(),
                enabled: None,
            },
        );
        let compiled =
            compile_pending(&snapshot, &edits, None, None, crate::terminfo::ColorDepth::Truecolor)
                .expect("compile");
        assert!(
            compiled.individuals.iter().any(|re| re.as_str() == r"\bxxx\b"),
            "new pattern present"
        );
        assert_eq!(
            compiled.styles[ipv4_idx], original_style,
            "style unchanged when only pattern overlay set"
        );
    }

    #[test]
    fn compile_pending_builtin_overlay_style_only_preserves_original_pattern() {
        // Style override; no pattern override. The compiled rule must
        // keep the shipped pattern AND apply the new style.
        let snapshot = ConfigSnapshot::empty();
        let baseline = compile_pending(
            &snapshot,
            &PendingEdits::default(),
            None,
            None,
            crate::terminfo::ColorDepth::Truecolor,
        )
        .expect("baseline compile");
        let ipv4_idx = crate::rules::BUILTIN_NAMES
            .iter()
            .position(|n| *n == "ipv4")
            .expect("ipv4 in BUILTIN_NAMES");
        let original_pattern = baseline.individuals[ipv4_idx].as_str().to_owned();

        let mut edits = PendingEdits::default();
        let mut styles = HashMap::new();
        styles.insert(
            StyleKey::Default,
            NewStyle { fg: Some(Some(Color::Green)), ..Default::default() },
        );
        edits
            .rules
            .insert(RuleId::Builtin("ipv4"), RuleEdit { pattern: None, styles, enabled: None });
        let compiled =
            compile_pending(&snapshot, &edits, None, None, crate::terminfo::ColorDepth::Truecolor)
                .expect("compile");
        assert_eq!(
            compiled.individuals[ipv4_idx].as_str(),
            original_pattern,
            "pattern unchanged when only style overlay set"
        );
        assert_eq!(compiled.styles[ipv4_idx].fg, Some(crate::style::Color::Green));
    }

    #[test]
    fn compile_pending_builtin_overlay_dedupes_against_snapshot_userconfig_override() {
        // Snapshot already has a user-config entry overriding `ipv4`'s
        // style (mirror what would land on disk if a user wrote
        // [[rules]] name = "ipv4" style = { fg = "blue" }).
        // ColorPicker then binds fg=Red via RuleId::Builtin("ipv4"). The
        // resulting compile MUST NOT error with `seen` duplicate; the
        // final compiled style for ipv4 must reflect the TUI edit (Red).
        let mut snap = ConfigSnapshot::empty();
        snap.parsed.rules.push(crate::config::UserRule {
            name: "ipv4".to_owned(),
            pattern: None,
            style: Some(crate::config::UserStyle {
                fg: Some("blue".to_owned()),
                ..crate::config::UserStyle::default()
            }),
            enabled: true,
            styles: None,
            priority: None,
        });
        let mut edits = PendingEdits::default();
        let mut styles = HashMap::new();
        styles.insert(
            StyleKey::Default,
            NewStyle { fg: Some(Some(Color::Red)), ..Default::default() },
        );
        edits
            .rules
            .insert(RuleId::Builtin("ipv4"), RuleEdit { pattern: None, styles, enabled: None });
        let compiled =
            compile_pending(&snap, &edits, None, None, crate::terminfo::ColorDepth::Truecolor)
                .expect("dedupe-then-mutate must avoid `seen` duplicate error");
        let ipv4_idx = crate::rules::BUILTIN_NAMES.iter().position(|n| *n == "ipv4").expect("ipv4");
        assert_eq!(
            compiled.styles[ipv4_idx].fg,
            Some(crate::style::Color::Red),
            "TUI edit wins over snapshot user-config override"
        );
        // Verify no duplicate `ipv4` entry leaked into the compiled set
        // (would surface as 2 entries with the same effective name). The
        // compiled default set is the 14 default-on built-ins (12 base + arn +
        // instance_id); the four default-off promoted rules are filtered out,
        // and the ipv4 override mutates in place (no extra appended rule).
        let default_on = crate::rules::builtin_rules().iter().filter(|r| r.enabled).count();
        assert_eq!(default_on, 14, "14 default-on built-ins");
        assert_eq!(compiled.individuals.len(), default_on, "no duplicate rule appended");
    }

    #[test]
    fn compile_pending_disk_profile_rule_style_override_writes_through() {
        // DiskProfile path symmetric to Embedded — exercise the variant's
        // overlay route. The shipped TUI does not construct DiskProfile
        // variants yet, but the overlay loop must handle them.
        let snapshot = ConfigSnapshot::empty();
        let mut edits = PendingEdits::default();
        let mut styles = HashMap::new();
        styles.insert(
            StyleKey::Default,
            NewStyle { fg: Some(Some(Color::Cyan)), ..Default::default() },
        );
        edits.rules.insert(
            RuleId::DiskProfile { profile: "synthetic".to_owned(), rule: "ipv4".to_owned() },
            RuleEdit { pattern: None, styles, enabled: None },
        );
        let compiled =
            compile_pending(&snapshot, &edits, None, None, crate::terminfo::ColorDepth::Truecolor)
                .expect("compile with DiskProfile overlay does not error");
        // Locate ipv4 by BUILTIN_NAMES index.
        let ipv4_idx = crate::rules::BUILTIN_NAMES
            .iter()
            .position(|n| *n == "ipv4")
            .expect("ipv4 in BUILTIN_NAMES");
        assert_eq!(
            compiled.styles[ipv4_idx].fg,
            Some(crate::style::Color::Cyan),
            "DiskProfile overlay applies style on name match"
        );
    }

    #[test]
    fn compile_pending_enabled_flip_on_enables_default_off_builtin() {
        // Toggle-ON of `container_id` (default-off) via a RuleEdit enabled
        // flip must make its pattern appear in the compiled set.
        let snapshot = ConfigSnapshot::empty();
        let baseline =
            compile_pending(&snapshot, &PendingEdits::default(), None, None, ColorDepth::Truecolor)
                .expect("baseline");
        let cid_pattern = crate::rules::builtin_rules()
            .into_iter()
            .find(|r| r.name == "container_id")
            .map(|r| r.pattern)
            .expect("container_id builtin");
        assert!(
            !baseline.individuals.iter().any(|re| re.as_str() == cid_pattern),
            "precondition: container_id absent from default compiled set"
        );

        let mut edits = PendingEdits::default();
        edits.rules.insert(
            RuleId::Builtin("container_id"),
            RuleEdit { pattern: None, styles: HashMap::new(), enabled: Some(true) },
        );
        let compiled =
            compile_pending(&snapshot, &edits, None, None, ColorDepth::Truecolor).expect("compile");
        assert!(
            compiled.individuals.iter().any(|re| re.as_str() == cid_pattern),
            "enabled=true flip brings container_id into the compiled set"
        );
    }

    #[test]
    fn compile_pending_enabled_flip_off_disables_default_on_builtin() {
        // Toggle-OFF of `arn` (default-on) via a RuleEdit enabled flip must
        // drop its pattern from the compiled set.
        let snapshot = ConfigSnapshot::empty();
        let arn_pattern = crate::rules::builtin_rules()
            .into_iter()
            .find(|r| r.name == "arn")
            .map(|r| r.pattern)
            .expect("arn builtin");
        let baseline =
            compile_pending(&snapshot, &PendingEdits::default(), None, None, ColorDepth::Truecolor)
                .expect("baseline");
        assert!(
            baseline.individuals.iter().any(|re| re.as_str() == arn_pattern),
            "precondition: arn present in default compiled set"
        );

        let mut edits = PendingEdits::default();
        edits.rules.insert(
            RuleId::Builtin("arn"),
            RuleEdit { pattern: None, styles: HashMap::new(), enabled: Some(false) },
        );
        let compiled =
            compile_pending(&snapshot, &edits, None, None, ColorDepth::Truecolor).expect("compile");
        assert!(
            !compiled.individuals.iter().any(|re| re.as_str() == arn_pattern),
            "enabled=false flip removes arn from the compiled set"
        );
    }

    // -----------------------------------------------------------------------
    // G3 — NewStyle.{bold,italic,underline} tri-state migration semantics.
    // Pins per-axis arm dispatch at `apply_new_style_to_user_rule` and
    // `new_style_to_user_style` after the type change from `Option<bool>`
    // to `Option<Option<bool>>`. Spec §3.1.
    // -----------------------------------------------------------------------

    /// Outer `None` on every axis must leave the existing `UserStyle` unmodified.
    #[test]
    fn apply_new_style_per_axis_unedited_outer_none_preserves_existing() {
        let mut rule = UserRule {
            name: "x".to_owned(),
            pattern: None,
            style: Some(UserStyle {
                bold: true,
                italic: false,
                underline: true,
                ..UserStyle::default()
            }),
            enabled: true,
            styles: None,
            priority: None,
        };
        let ns = NewStyle { bold: None, italic: None, underline: None, ..Default::default() };
        apply_new_style_to_user_rule(&mut rule, &ns);
        let us = rule.style.expect("style present");
        assert!(us.bold, "outer None preserves existing bold=true");
        assert!(!us.italic, "outer None preserves existing italic=false");
        assert!(us.underline, "outer None preserves existing underline=true");
    }

    /// `Some(None)` on every axis must explicitly clear to `false`.
    #[test]
    fn apply_new_style_per_axis_some_none_explicit_clears_to_false() {
        let mut rule = UserRule {
            name: "x".to_owned(),
            pattern: None,
            style: Some(UserStyle {
                bold: true,
                italic: true,
                underline: true,
                ..UserStyle::default()
            }),
            enabled: true,
            styles: None,
            priority: None,
        };
        let ns = NewStyle {
            bold: Some(None),
            italic: Some(None),
            underline: Some(None),
            ..Default::default()
        };
        apply_new_style_to_user_rule(&mut rule, &ns);
        let us = rule.style.expect("style present");
        assert!(!us.bold);
        assert!(!us.italic);
        assert!(!us.underline);
    }

    /// `Some(Some(b))` on every axis must set to the inner boolean.
    #[test]
    fn apply_new_style_per_axis_some_some_explicit_sets_value() {
        let mut rule = UserRule {
            name: "x".to_owned(),
            pattern: None,
            style: Some(UserStyle::default()),
            enabled: true,
            styles: None,
            priority: None,
        };
        let ns = NewStyle {
            bold: Some(Some(true)),
            italic: Some(Some(false)),
            underline: Some(Some(true)),
            ..Default::default()
        };
        apply_new_style_to_user_rule(&mut rule, &ns);
        let us = rule.style.expect("style present");
        assert!(us.bold);
        assert!(!us.italic);
        assert!(us.underline);
    }

    /// `new_style_to_user_style` flattens both outer-`None` and `Some(None)`
    /// to `false`; only `Some(Some(true))` survives as `true`. Mirrors the
    /// on-disk `UserStyle` shape (single bool per axis).
    #[test]
    fn new_style_to_user_style_flattens_outer_none_and_some_none_to_false() {
        let ns = NewStyle {
            bold: None,
            italic: Some(None),
            underline: Some(Some(true)),
            ..Default::default()
        };
        let us = new_style_to_user_style(&ns);
        assert!(!us.bold);
        assert!(!us.italic);
        assert!(us.underline);
    }
}
