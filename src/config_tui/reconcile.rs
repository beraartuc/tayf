//! [`PendingEdits`] → [`DocumentMut`] walk + serialize.
//!
//! Pure data-transform module: takes a frozen [`DocumentMut`] + staged
//! edits, returns the new TOML source bytes. No I/O, no side effects.
//!
//! Public-to-crate API:
//!   `pub(crate) fn apply_edits(doc, edits) -> Result<String, ReconcileError>`
//!
//! Spec §5 — handler-by-handler walk. Spec §6 — [`ReconcileError`] variants.

use crate::config_tui::edit::{GeneralEdits, PendingEdits, RuleEdit, RuleId};
use toml_edit::{ArrayOfTables, DocumentMut, Table};

// reason: UnsupportedDeletionTarget consumed by Task B5 (apply_deletion);
// v0.5.5 Phase A2 skeleton, used by B1 (TypeMismatch path reachable via
// [[general]] user-written shape in ensure_general_table).
#[allow(dead_code)]
#[derive(Debug, thiserror::Error)]
pub(crate) enum ReconcileError {
    #[error(
        "unsupported deletion target: {rule_id} \
         (currently only `RuleId::UserConfig` deletion is supported; \
         other variants are reserved for future work)"
    )]
    UnsupportedDeletionTarget { rule_id: String },

    #[error(
        "type mismatch at {path}: expected {expected}, found {actual} \
         (DocumentMut shape diverged from validated parse — config may be corrupt; \
         try reloading the file)"
    )]
    TypeMismatch { path: String, expected: &'static str, actual: &'static str },
}

/// Apply `[general]` staged edits (theme + profile tri-state) to `doc`.
///
/// Tri-state semantics per [`GeneralEdits`]:
/// - `None`         — field untouched.
/// - `Some(None)`   — remove the key from `[general]`.
/// - `Some(Some(s))`— set (or overwrite) the key to `s`.
///
/// Creates the `[general]` table if it does not yet exist.
fn apply_general(doc: &mut DocumentMut, ge: &GeneralEdits) -> Result<(), ReconcileError> {
    // Both fields None = no general edits; skip to avoid ensure_general_table
    // side effect of creating an empty [general] section when one didn't exist.
    if ge.theme.is_none() && ge.profile.is_none() {
        return Ok(());
    }
    let general = ensure_general_table(doc)?;
    match &ge.theme {
        None => {}
        Some(None) => {
            general.remove("theme");
        }
        Some(Some(name)) => {
            general["theme"] = toml_edit::value(name.as_str());
        }
    }
    match &ge.profile {
        None => {}
        Some(None) => {
            general.remove("profile");
        }
        Some(Some(name)) => {
            general["profile"] = toml_edit::value(name.as_str());
        }
    }
    Ok(())
}

/// Return a mutable reference to the `[general]` table, creating it if absent.
///
/// Returns [`ReconcileError::TypeMismatch`] if `doc["general"]` exists but is
/// not a `Table` (e.g. user hand-wrote `[[general]]`). Spec §6 B5 fold.
fn ensure_general_table(doc: &mut DocumentMut) -> Result<&mut Table, ReconcileError> {
    if !doc.contains_key("general") {
        let mut t = Table::new();
        // N4 NIT: defensive — Table::new() default is non-implicit but explicit
        // pin guards against toml_edit default-change in future versions.
        t.set_implicit(false);
        doc["general"] = toml_edit::Item::Table(t);
        return Ok(doc["general"].as_table_mut().unwrap_or_else(|| {
            unreachable!(
                "doc[\"general\"] was just set to Item::Table; toml_edit invariant violation if not Table now"
            )
        }));
    }
    // B5 fold: existing-key path — emit TypeMismatch on non-Table Item.
    let item = &mut doc["general"];
    let actual_ty = item.type_name();
    item.as_table_mut().ok_or(ReconcileError::TypeMismatch {
        path: "general".into(),
        expected: "table",
        actual: actual_ty,
    })
}

/// Return a mutable reference to the `[[rules]]` array-of-tables,
/// creating it if absent.
///
/// Returns [`ReconcileError::TypeMismatch`] if `doc["rules"]` exists but
/// is not an array-of-tables (e.g. user hand-wrote `[rules]` inline).
fn ensure_rules_array(doc: &mut DocumentMut) -> Result<&mut ArrayOfTables, ReconcileError> {
    if !doc.contains_key("rules") {
        doc["rules"] = toml_edit::Item::ArrayOfTables(ArrayOfTables::new());
        return Ok(doc["rules"].as_array_of_tables_mut().unwrap_or_else(|| {
            unreachable!("rules just set to ArrayOfTables; toml_edit invariant violation if not")
        }));
    }
    let item = &mut doc["rules"];
    let actual_ty = item.type_name();
    item.as_array_of_tables_mut().ok_or(ReconcileError::TypeMismatch {
        path: "rules".into(),
        expected: "array-of-tables",
        actual: actual_ty,
    })
}

/// Linear scan for `name = "X"` entry. N-bound: `MAX_CONFIG_BYTES` /
/// min-entry-size ≈ 20k; expected N 5-50 real use. Spec §5.3.
fn find_rule_index_by_name(rules: &ArrayOfTables, name: &str) -> Option<usize> {
    rules.iter().position(|t| t.get("name").and_then(|v| v.as_str()) == Some(name))
}

/// Apply a single [`RuleEdit`] to the named `[[rules]]` entry in `doc`.
///
/// If no entry with `name = <name>` exists, a new stub entry is appended.
/// Pattern edit writes a TOML literal string to avoid backslash escaping.
/// Styles handling defers to Task B3.
fn apply_user_config_rule(
    doc: &mut DocumentMut,
    name: &str,
    edit: &RuleEdit,
) -> Result<(), ReconcileError> {
    let rules = ensure_rules_array(doc)?;
    let idx = find_rule_index_by_name(rules, name);
    let rule_table = if let Some(i) = idx {
        // get_mut returns &mut Table directly (ArrayOfTables invariant).
        rules.get_mut(i).unwrap_or_else(|| {
            unreachable!(
                "find_rule_index_by_name returned valid idx {i}; toml_edit ArrayOfTables index invariant violation"
            )
        })
    } else {
        let mut t = Table::new();
        t["name"] = toml_edit::value(name);
        rules.push(t);
        let last_idx = rules.len() - 1;
        rules
            .get_mut(last_idx)
            .unwrap_or_else(|| unreachable!("just pushed; toml_edit invariant violation"))
    };
    if let Some(pat) = &edit.pattern {
        rule_table["pattern"] = toml_edit::value(pat.as_str());
    }
    // Styles handling lands in Task B3.
    Ok(())
}

/// Walk `edits` into `doc` (cloned internally) and serialize to TOML
/// string. Spec §5 algorithm. **Implementation note:** `doc` is cloned
/// internally (`DocumentMut::clone()` is O(n) tree-walk; acceptable for
/// save-on-Ctrl+S frequency, not a hot path). Caller's snapshot.doc is
/// not mutated.
pub(crate) fn apply_edits(
    doc: &DocumentMut,
    edits: &PendingEdits,
) -> Result<String, ReconcileError> {
    let mut working = doc.clone();
    apply_general(&mut working, &edits.general)?;
    for (rule_id, rule_edit) in &edits.rules {
        match rule_id {
            RuleId::UserConfig(name) => apply_user_config_rule(&mut working, name, rule_edit)?,
            RuleId::Builtin(_) | RuleId::Embedded { .. } | RuleId::DiskProfile { .. } => {
                // v0.5.4 tabs only stage UserConfig; defensive no-op for other variants.
                // Spec §5.1.
            }
        }
    }
    Ok(working.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_theme_set_appends_or_updates() {
        // Spec §7.1 #2.
        let source = "[general]\ntheme = \"dark\"\n";
        let doc: DocumentMut = source.parse().expect("valid TOML");
        let mut edits = PendingEdits::default();
        edits.general.theme = Some(Some("light".to_owned()));
        let out = apply_edits(&doc, &edits).expect("ok");
        assert_eq!(out, "[general]\ntheme = \"light\"\n");
    }

    #[test]
    fn general_theme_clear_removes_key() {
        // Spec §7.1 #3.
        let source = "[general]\ntheme = \"dark\"\nprofile = \"docker\"\n";
        let doc: DocumentMut = source.parse().expect("valid TOML");
        let mut edits = PendingEdits::default();
        edits.general.theme = Some(None);
        let out = apply_edits(&doc, &edits).expect("ok");
        // theme line gone; profile + [general] header preserved.
        assert_eq!(out, "[general]\nprofile = \"docker\"\n");
    }

    #[test]
    fn general_theme_creates_section_when_absent() {
        // Spec §7.1 #4.
        let source = "";
        let doc: DocumentMut = source.parse().expect("valid TOML (empty)");
        let mut edits = PendingEdits::default();
        edits.general.theme = Some(Some("dark".to_owned()));
        let out = apply_edits(&doc, &edits).expect("ok");
        assert_eq!(out, "[general]\ntheme = \"dark\"\n");
    }

    #[test]
    fn general_profile_set_updates_value() {
        // Spec §7.1 — defensive profile-arm coverage (parallel to test #2).
        let source = "[general]\nprofile = \"aws\"\n";
        let doc: DocumentMut = source.parse().expect("valid TOML");
        let mut edits = PendingEdits::default();
        edits.general.profile = Some(Some("docker".to_owned()));
        let out = apply_edits(&doc, &edits).expect("ok");
        assert_eq!(out, "[general]\nprofile = \"docker\"\n");
    }

    #[test]
    fn user_config_rule_pattern_update() {
        // Spec §7.1 #5.
        use crate::config_tui::edit::{RuleEdit, RuleId};
        let source = "[[rules]]\nname = \"uuid\"\npattern = \"old\"\n";
        let doc: DocumentMut = source.parse().expect("valid TOML");
        let mut edits = PendingEdits::default();
        edits.rules.insert(
            RuleId::UserConfig("uuid".to_owned()),
            RuleEdit {
                pattern: Some(r"\bx\b".to_owned()),
                styles: std::collections::HashMap::new(),
            },
        );
        let out = apply_edits(&doc, &edits).expect("ok");
        assert!(out.contains(r"pattern = '\bx\b'"), "literal-string form expected; got: {out:?}");
        assert!(!out.contains("\"old\""), "old pattern must be gone: {out:?}");
    }

    #[test]
    fn user_config_rule_append_when_absent() {
        // Spec §7.1 #6.
        use crate::config_tui::edit::{RuleEdit, RuleId};
        let source = "[general]\ntheme = \"dark\"\n";
        let doc: DocumentMut = source.parse().expect("valid TOML");
        let mut edits = PendingEdits::default();
        edits.rules.insert(RuleId::UserConfig("uuid".to_owned()), RuleEdit::default());
        let out = apply_edits(&doc, &edits).expect("ok");
        assert!(out.contains("[[rules]]"), "must append [[rules]]: {out:?}");
        assert!(out.contains("name = \"uuid\""), "must include name=uuid: {out:?}");
    }

    #[test]
    fn empty_edits_yields_identical_bytes() {
        // Spec §7.1 #1 (N3 concretized) — comment-heavy worst-case
        // fixture (foundational regression guard for the whole walk).
        let source = "# Top-level header comment line one\n\
                      # Top-level header comment line two\n\
                      # Top-level header comment line three\n\
                      \n\
                      [general]\n\
                      theme = \"dark\"\n\
                      profile = \"docker\"\n\
                      \n\
                      [[rules]]\n\
                      name = \"alpha\"\n\
                      pattern = \"a\"\n\
                      style = { fg = \"red\" }\n\
                      \n\
                      # Block-form style on beta\n\
                      [[rules]]\n\
                      name = \"beta\"\n\
                      pattern = \"b\"\n\
                      \n\
                      [rules.style]\n\
                      fg = \"green\"\n\
                      bold = true\n";
        let doc: DocumentMut = source.parse().expect("valid TOML fixture");
        let edits = PendingEdits::default();
        let out = apply_edits(&doc, &edits).expect("empty edits Ok");
        assert_eq!(out, source, "empty edits must yield byte-identical output");
    }
}
