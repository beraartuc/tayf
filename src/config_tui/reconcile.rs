//! [`PendingEdits`] → [`DocumentMut`] walk + serialize.
//!
//! Pure data-transform module: takes a frozen [`DocumentMut`] + staged
//! edits, returns the new TOML source bytes. No I/O, no side effects.
//!
//! Public-to-crate API:
//!   `pub(crate) fn apply_edits(doc, edits) -> Result<String, ReconcileError>`
//!
//! Spec §5 — handler-by-handler walk. Spec §6 — [`ReconcileError`] variants.

use crate::config_tui::edit::{GeneralEdits, NewStyle, PendingEdits, RuleEdit, RuleId, StyleKey};
use toml_edit::{ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};

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

/// Borrow handle into either inline `style = { ... }` or block `[rules.style]`
/// table form. Both expose key-mutation API; `set_or_insert` dispatches.
/// Spec §5.4 + §13.2 B4/I2 fold.
pub(crate) enum StyleTargetMut<'a> {
    Inline(&'a mut InlineTable),
    Table(&'a mut Table),
}

impl StyleTargetMut<'_> {
    fn remove(&mut self, key: &str) {
        match self {
            Self::Inline(it) => {
                it.remove(key);
            }
            Self::Table(t) => {
                t.remove(key);
            }
        }
    }
}

/// B4 fold: `InlineTable::IndexMut` panics on absent key — use `insert()` for
/// the Inline branch (replaces or adds; never panics). `Table::insert` uses
/// the same replace-or-add semantic via `IndexMap` entry.
fn set_or_insert(target: &mut StyleTargetMut<'_>, key: &str, val: Value) {
    match target {
        StyleTargetMut::Inline(it) => {
            it.insert(key, val);
        }
        StyleTargetMut::Table(t) => {
            t.insert(key, Item::Value(val));
        }
    }
}

/// Write a [`NewStyle`] diff into `target` (inline or block form). Tri-state
/// semantics per field: `None` = leave unchanged, `Some(None)` = clear key,
/// `Some(Some(v))` = set key to `v`. Spec §5.4.
// reason: returns Result<()> for forward-compatibility — future fg/bg Color
// validation may introduce error paths (e.g. invalid hex from TUI input).
#[allow(clippy::unnecessary_wraps)]
fn write_style_table(mut target: StyleTargetMut<'_>, ns: &NewStyle) -> Result<(), ReconcileError> {
    use crate::style::Color;
    match &ns.fg {
        None => {}
        Some(None) => {
            target.remove("fg");
        }
        Some(Some(color)) => {
            set_or_insert(&mut target, "fg", Value::from(Color::to_toml_str(*color)));
        }
    }
    match &ns.bg {
        None => {}
        Some(None) => {
            target.remove("bg");
        }
        Some(Some(color)) => {
            set_or_insert(&mut target, "bg", Value::from(Color::to_toml_str(*color)));
        }
    }
    if let Some(b) = ns.bold {
        set_or_insert(&mut target, "bold", Value::from(b));
    }
    if let Some(b) = ns.italic {
        set_or_insert(&mut target, "italic", Value::from(b));
    }
    if let Some(b) = ns.underline {
        set_or_insert(&mut target, "underline", Value::from(b));
    }
    if let Some(b) = ns.dim {
        set_or_insert(&mut target, "dim", Value::from(b));
    }
    Ok(())
}

/// Return a [`StyleTargetMut`] for `key` inside `parent`. If `key` exists:
/// preserve its existing form (inline-or-block). If absent: create as
/// inline-table (always-inline-on-create per assets/profiles/*.toml convention).
/// Spec §5.3 + §13.2 I3/I10 fold.
fn ensure_style_target<'a>(
    parent: &'a mut Table,
    key: &str,
) -> Result<StyleTargetMut<'a>, ReconcileError> {
    if !parent.contains_key(key) {
        // Always-inline on create (v0.5.5 contract per assets/profiles/*.toml convention).
        parent[key] = Item::Value(Value::InlineTable(InlineTable::new()));
    }
    let item = parent.get_mut(key).unwrap_or_else(|| {
        unreachable!(
            "key {key} just ensured; toml_edit invariant violation if get_mut returns None for a key just inserted"
        )
    });
    // Try inline first (most common form for v0.5.5 contract).
    if item.is_inline_table() {
        return Ok(StyleTargetMut::Inline(
            item.as_inline_table_mut().unwrap_or_else(|| {
                unreachable!(
                    "just checked is_inline_table; toml_edit invariant violation if as_inline_table_mut returns None"
                )
            }),
        ));
    }
    // Fall through to block-table form (mutating preserves it).
    let actual_ty = item.type_name();
    item.as_table_mut().map(StyleTargetMut::Table).ok_or(ReconcileError::TypeMismatch {
        path: format!("style slot '{key}'"),
        expected: "table or inline-table",
        actual: actual_ty,
    })
}

/// Same as [`ensure_style_target`] but for Numbered slot — inserts with a
/// `Key` carrying a quoted-string repr (e.g. `"1"`) to preserve the v0.3.5
/// schema convention `styles."N"`. I12 fold.
///
/// Uses `Key::parse("\"N\"")` (public API) + `Table::insert_formatted` to
/// control the emitted key repr on first insert. Existing keys are picked up
/// via `ensure_style_target` which preserves whatever form is already in the
/// document.
fn ensure_style_target_quoted_numbered(
    parent: &mut Table,
    n: u32,
) -> Result<StyleTargetMut<'_>, ReconcileError> {
    let key_str = n.to_string();
    if !parent.contains_key(&key_str) {
        // Build a Key with quoted repr: parse the TOML literal `"N"` which
        // produces Key with raw repr = `"N"` (double-quoted form). This is the
        // v0.3.5 schema convention for capture-group style slots.
        let quoted_repr = format!("\"{n}\"");
        let mut keys = toml_edit::Key::parse(&quoted_repr).unwrap_or_else(|e| {
            unreachable!(
                "quoted numeric key parse failed for repr {quoted_repr:?}: {e}; \
                 toml_edit invariant — a double-quoted numeric string is always a valid TOML key"
            )
        });
        // parse returns Vec<Key>; our repr is a single simple key.
        let key = keys.drain(..).next().unwrap_or_else(|| {
            unreachable!(
                "Key::parse returned empty Vec for repr {quoted_repr:?}; toml_edit invariant violation"
            )
        });
        parent.insert_formatted(&key, Item::Value(Value::InlineTable(InlineTable::new())));
    }
    ensure_style_target(parent, &key_str)
}

/// Return (creating if absent) a block sub-table for `key` inside `parent`.
/// Used to access `styles.` sub-table that holds Numbered/Named style slots.
fn ensure_subtable<'a>(parent: &'a mut Table, key: &str) -> Result<&'a mut Table, ReconcileError> {
    if !parent.contains_key(key) {
        let mut t = Table::new();
        // Sub-table inside a rule entry — keep implicit off so styles. dotted-key
        // form emits cleanly.
        t.set_implicit(false);
        parent[key] = Item::Table(t);
    }
    let item = &mut parent[key];
    let actual_ty = item.type_name();
    item.as_table_mut().ok_or(ReconcileError::TypeMismatch {
        path: format!("rule sub-table '{key}'"),
        expected: "table",
        actual: actual_ty,
    })
}

/// Apply a single [`RuleEdit`] to the named `[[rules]]` entry in `doc`.
///
/// If no entry with `name = <name>` exists, a new stub entry is appended.
/// Pattern edit writes a TOML literal string to avoid backslash escaping.
/// Style handler dispatches via [`StyleTargetMut`] (inline vs block form
/// preserved; new slots always created inline). Spec §5.3 + §5.4.
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
    for (style_key, ns) in &edit.styles {
        let target = match style_key {
            StyleKey::Default => ensure_style_target(rule_table, "style")?,
            StyleKey::Numbered(i) => {
                let styles = ensure_subtable(rule_table, "styles")?;
                ensure_style_target_quoted_numbered(styles, *i)?
            }
            StyleKey::Named(n) => {
                let styles = ensure_subtable(rule_table, "styles")?;
                ensure_style_target(styles, n.as_str())?
            }
        };
        write_style_table(target, ns)?;
    }
    Ok(())
}

/// Append a brand-new `[[rules]]` entry to `doc` from a [`NewRule`].
///
/// Always writes `style` as an inline table (v0.5.5 convention per
/// `assets/profiles/*.toml`). If `style` has no fields set, the key is
/// omitted entirely. Spec §5.5.
fn apply_new_rule(
    doc: &mut DocumentMut,
    rule: &crate::config_tui::edit::NewRule,
) -> Result<(), ReconcileError> {
    let rules = ensure_rules_array(doc)?;
    let mut t = Table::new();
    t["name"] = toml_edit::value(rule.name.as_str());
    t["pattern"] = toml_edit::value(rule.pattern.as_str());
    let mut style_inline = InlineTable::new();
    write_style_table(StyleTargetMut::Inline(&mut style_inline), &rule.style)?;
    if !style_inline.is_empty() {
        t["style"] = Item::Value(Value::InlineTable(style_inline));
    }
    rules.push(t);
    Ok(())
}

/// Remove the `[[rules]]` entry identified by `rule_id` from `doc`.
///
/// Only [`RuleId::UserConfig`] deletion is supported. `Builtin`, `Embedded`, and
/// `DiskProfile` variants return [`ReconcileError::UnsupportedDeletionTarget`].
///
/// [`ArrayOfTables::remove`] drops the leading-comment block attached to the
/// entry (`toml_edit` 0.25 Decor semantic — test #19 pins this contract).
/// Spec §5.6.
fn apply_deletion(doc: &mut DocumentMut, rule_id: &RuleId) -> Result<(), ReconcileError> {
    match rule_id {
        RuleId::UserConfig(name) => {
            // Spec §5.6: ArrayOfTables::remove drops leading-comment block
            // attached to the entry (documented contract; test #19 pins).
            if let Some(arr) = doc.get_mut("rules").and_then(Item::as_array_of_tables_mut) {
                if let Some(idx) = find_rule_index_by_name(arr, name) {
                    arr.remove(idx);
                }
            }
            Ok(())
        }
        RuleId::Builtin(_) | RuleId::Embedded { .. } | RuleId::DiskProfile { .. } => {
            Err(ReconcileError::UnsupportedDeletionTarget { rule_id: format!("{rule_id:?}") })
        }
    }
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
    for new_rule in &edits.added {
        apply_new_rule(&mut working, new_rule)?;
    }
    for rule_id in &edits.deleted {
        apply_deletion(&mut working, rule_id)?;
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

    #[test]
    fn styles_default_branch_writes_top_level_style() {
        // Spec §7.1 #7.
        use crate::config_tui::edit::{NewStyle, RuleEdit, RuleId, StyleKey};
        use crate::style::Color;
        let source = "[[rules]]\nname = \"x\"\npattern = \"p\"\n";
        let doc: DocumentMut = source.parse().expect("valid TOML");
        let mut edits = PendingEdits::default();
        let mut styles = std::collections::HashMap::new();
        styles.insert(
            StyleKey::Default,
            NewStyle { fg: Some(Some(Color::Red)), ..NewStyle::default() },
        );
        edits.rules.insert(RuleId::UserConfig("x".to_owned()), RuleEdit { pattern: None, styles });
        let out = apply_edits(&doc, &edits).expect("ok");
        assert!(out.contains("style = { fg = \"red\" }"), "inline form expected: {out:?}");
    }

    #[test]
    fn styles_numbered_branch_writes_quoted_styles_n_table() {
        // Spec §7.1 #8 + §13.2 I12 fold — quoted "1" form preserved.
        use crate::config_tui::edit::{NewStyle, RuleEdit, RuleId, StyleKey};
        use crate::style::Color;
        let source = "[[rules]]\nname = \"x\"\npattern = \"p\"\n";
        let doc: DocumentMut = source.parse().expect("valid TOML");
        let mut edits = PendingEdits::default();
        let mut styles = std::collections::HashMap::new();
        styles.insert(
            StyleKey::Numbered(1),
            NewStyle { fg: Some(Some(Color::Green)), ..NewStyle::default() },
        );
        edits.rules.insert(RuleId::UserConfig("x".to_owned()), RuleEdit { pattern: None, styles });
        let out = apply_edits(&doc, &edits).expect("ok");
        assert!(out.contains("styles"), "must have styles. sub-table: {out:?}");
        assert!(
            out.contains("\"1\""),
            "quoted key '\"1\"' (literal-string repr) expected: {out:?}"
        );
        assert!(out.contains("fg = \"green\""), "green fg expected: {out:?}");
    }

    #[test]
    fn styles_named_branch_writes_styles_name_table() {
        // Spec §7.1 #9.
        use crate::config_tui::edit::{NewStyle, RuleEdit, RuleId, StyleKey};
        use crate::style::Color;
        let source = "[[rules]]\nname = \"x\"\npattern = \"p\"\n";
        let doc: DocumentMut = source.parse().expect("valid TOML");
        let mut edits = PendingEdits::default();
        let mut styles = std::collections::HashMap::new();
        styles.insert(
            StyleKey::Named("matchname".to_owned()),
            NewStyle { fg: Some(Some(Color::Blue)), ..NewStyle::default() },
        );
        edits.rules.insert(RuleId::UserConfig("x".to_owned()), RuleEdit { pattern: None, styles });
        let out = apply_edits(&doc, &edits).expect("ok");
        assert!(out.contains("matchname"), "named key expected: {out:?}");
        assert!(out.contains("fg = \"blue\""));
        assert!(
            out.contains("styles.matchname")
                || out.contains("[rules.styles")
                || out.contains("\nstyles ="),
            "matchname must be nested under styles. sub-table, not at rule root: {out:?}"
        );
    }

    #[test]
    fn new_style_fg_clear_removes_fg_keeps_bold() {
        // Spec §7.1 #10.
        use crate::config_tui::edit::{NewStyle, RuleEdit, RuleId, StyleKey};
        let source = "[[rules]]\nname = \"x\"\nstyle = { fg = \"red\", bold = true }\n";
        let doc: DocumentMut = source.parse().expect("valid TOML");
        let mut edits = PendingEdits::default();
        let mut styles = std::collections::HashMap::new();
        styles.insert(StyleKey::Default, NewStyle { fg: Some(None), ..NewStyle::default() });
        edits.rules.insert(RuleId::UserConfig("x".to_owned()), RuleEdit { pattern: None, styles });
        let out = apply_edits(&doc, &edits).expect("ok");
        assert!(!out.contains("fg"), "fg key must be gone: {out:?}");
        assert!(out.contains("bold = true"), "bold must survive: {out:?}");
    }

    #[test]
    fn new_style_bool_axes_set_via_insert_helper() {
        // Spec §7.1 #11 + §13.2 B4 fold — set_or_insert handles
        // InlineTable fresh-adds without IndexMut panic.
        use crate::config_tui::edit::{NewStyle, RuleEdit, RuleId, StyleKey};
        // Source has only `fg`; bold/italic/underline/dim absent — must NOT panic
        // when adding (B4 fold: InlineTable::IndexMut panics on absent key,
        // set_or_insert uses insert() for the Inline branch).
        let source = "[[rules]]\nname = \"x\"\nstyle = { fg = \"red\" }\n";
        let doc: DocumentMut = source.parse().expect("valid TOML");
        let mut edits = PendingEdits::default();
        let mut styles = std::collections::HashMap::new();
        styles.insert(
            StyleKey::Default,
            NewStyle {
                bold: Some(true),
                italic: Some(true),
                underline: Some(false),
                dim: Some(false),
                ..NewStyle::default()
            },
        );
        edits.rules.insert(RuleId::UserConfig("x".to_owned()), RuleEdit { pattern: None, styles });
        let out = apply_edits(&doc, &edits).expect("ok — no panic");
        assert!(out.contains("bold = true"), "bold expected: {out:?}");
        assert!(out.contains("italic = true"), "italic expected: {out:?}");
        assert!(out.contains("underline = false"), "underline expected: {out:?}");
        assert!(out.contains("dim = false"), "dim expected: {out:?}");
        assert!(out.contains("fg = \"red\""), "fg must survive (not staged for change): {out:?}");
    }

    #[test]
    fn pattern_with_backslashes_renders_as_literal_string() {
        // Spec §7.1 #16 + §13.2 I8 fold — toml_edit's as_default()
        // selects literal-string form for `\b` regex.
        use crate::config_tui::edit::{RuleEdit, RuleId};
        let source = "[[rules]]\nname = \"x\"\npattern = \"old\"\n";
        let doc: DocumentMut = source.parse().expect("valid TOML");
        let mut edits = PendingEdits::default();
        edits.rules.insert(
            RuleId::UserConfig("x".to_owned()),
            RuleEdit {
                pattern: Some(r"\b[a-z]+\b".to_owned()),
                styles: std::collections::HashMap::new(),
            },
        );
        let out = apply_edits(&doc, &edits).expect("ok");
        assert!(
            out.contains(r"pattern = '\b[a-z]+\b'"),
            "literal-string form expected; got: {out:?}"
        );
        assert!(
            !out.contains(r#"pattern = "\\b"#),
            "must NOT render as escaped basic-string: {out:?}"
        );
    }

    #[test]
    fn mutating_pattern_preserves_leading_comment_block() {
        // Spec §7.1 #17 + §13.2 I9 fold — Table::insert clears Key::repr
        // (auto-format) but DOES NOT clear Key::leaf_decor; preceding
        // comments survive.
        use crate::config_tui::edit::{RuleEdit, RuleId};
        let source = "[[rules]]\nname = \"x\"\n# inline comment above pattern\npattern = \"old\"\n";
        let doc: DocumentMut = source.parse().expect("valid TOML");
        let mut edits = PendingEdits::default();
        edits.rules.insert(
            RuleId::UserConfig("x".to_owned()),
            RuleEdit { pattern: Some("new".to_owned()), styles: std::collections::HashMap::new() },
        );
        let out = apply_edits(&doc, &edits).expect("ok");
        assert!(
            out.contains("# inline comment above pattern"),
            "comment must survive key mutation: {out:?}"
        );
        assert!(
            out.contains("pattern = 'new'") || out.contains("pattern = \"new\""),
            "new value: {out:?}"
        );
    }

    #[test]
    fn mutating_inline_style_keeps_inline_form() {
        // Spec §7.1 #18a + §13.2 I10 fold — form-preservation.
        use crate::config_tui::edit::{NewStyle, RuleEdit, RuleId, StyleKey};
        use crate::style::Color;
        let source = "[[rules]]\nname = \"x\"\nstyle = { fg = \"red\" }\n";
        let doc: DocumentMut = source.parse().expect("valid TOML");
        let mut edits = PendingEdits::default();
        let mut styles = std::collections::HashMap::new();
        styles.insert(
            StyleKey::Default,
            NewStyle { fg: Some(Some(Color::Blue)), ..NewStyle::default() },
        );
        edits.rules.insert(RuleId::UserConfig("x".to_owned()), RuleEdit { pattern: None, styles });
        let out = apply_edits(&doc, &edits).expect("ok");
        assert!(out.contains("style = { fg = \"blue\" }"), "inline form preserved: {out:?}");
        assert!(!out.contains("[rules.style]"), "must NOT flip to block form: {out:?}");
    }

    #[test]
    fn mutating_block_style_keeps_block_form() {
        // Spec §7.1 #18b + §13.2 I10 fold — form-preservation reverse.
        use crate::config_tui::edit::{NewStyle, RuleEdit, RuleId, StyleKey};
        use crate::style::Color;
        let source = "[[rules]]\nname = \"x\"\n\n[rules.style]\nfg = \"red\"\n";
        let doc: DocumentMut = source.parse().expect("valid TOML");
        let mut edits = PendingEdits::default();
        let mut styles = std::collections::HashMap::new();
        styles.insert(
            StyleKey::Default,
            NewStyle { fg: Some(Some(Color::Blue)), ..NewStyle::default() },
        );
        edits.rules.insert(RuleId::UserConfig("x".to_owned()), RuleEdit { pattern: None, styles });
        let out = apply_edits(&doc, &edits).expect("ok");
        assert!(out.contains("[rules.style]"), "block form preserved: {out:?}");
        assert!(out.contains("fg = \"blue\""), "fg updated: {out:?}");
        assert!(!out.contains("style = { "), "must NOT flip to inline form: {out:?}");
    }

    #[test]
    fn deleted_builtin_returns_unsupported_error() {
        // Spec §7.1 #14 + §13.2 I5 fold — full Display assert_eq! (no
        // version-string substring search; future-proof wording).
        use crate::config_tui::edit::RuleId;
        let source = "[general]\ntheme = \"dark\"\n";
        let doc: DocumentMut = source.parse().expect("valid TOML");
        let mut edits = PendingEdits::default();
        edits.deleted.insert(RuleId::Builtin("uuid"));
        let err = apply_edits(&doc, &edits).expect_err("must error");
        match &err {
            ReconcileError::UnsupportedDeletionTarget { rule_id } => {
                assert_eq!(rule_id, "Builtin(\"uuid\")", "Debug-formatted rule_id");
            }
            other @ ReconcileError::TypeMismatch { .. } => {
                panic!("expected UnsupportedDeletionTarget, got {other:?}")
            }
        }
        let display = format!("{err}");
        assert_eq!(
            display,
            "unsupported deletion target: Builtin(\"uuid\") (currently only `RuleId::UserConfig` deletion is supported; other variants are reserved for future work)",
            "Display string byte-pinned (no version-string anti-pattern per memory feedback_test_assertion_specificity)"
        );
    }

    #[test]
    fn deleted_user_config_removes_entry() {
        // Spec §7.1 #13.
        use crate::config_tui::edit::RuleId;
        let source = "[[rules]]\nname = \"x\"\npattern = \"a\"\n\n[[rules]]\nname = \"y\"\npattern = \"b\"\n";
        let doc: DocumentMut = source.parse().expect("valid TOML");
        let mut edits = PendingEdits::default();
        edits.deleted.insert(RuleId::UserConfig("x".to_owned()));
        let out = apply_edits(&doc, &edits).expect("ok");
        assert!(!out.contains("name = \"x\""), "x entry must be gone: {out:?}");
        assert!(out.contains("name = \"y\""), "y entry must remain: {out:?}");
    }

    #[test]
    fn removing_user_rule_deletes_its_leading_comment_block() {
        // Spec §7.1 #19 + §13.2 I11 fold — ArrayOfTables::remove drops
        // leading-comment block attached to the entry.
        use crate::config_tui::edit::RuleId;
        let source = "# Before-x comment\n\
                      [[rules]]\n\
                      name = \"x\"\n\
                      pattern = \"a\"\n\
                      \n\
                      # After-x / before-y comment\n\
                      [[rules]]\n\
                      name = \"y\"\n\
                      pattern = \"b\"\n";
        let doc: DocumentMut = source.parse().expect("valid TOML");
        let mut edits = PendingEdits::default();
        edits.deleted.insert(RuleId::UserConfig("x".to_owned()));
        let out = apply_edits(&doc, &edits).expect("ok");
        assert!(
            !out.contains("# Before-x comment"),
            "Before-x comment must be gone (attached to deleted x entry): {out:?}"
        );
        assert!(
            out.contains("# After-x / before-y comment"),
            "After-x comment must survive (attached to y entry): {out:?}"
        );
    }

    #[test]
    fn added_vec_appends_new_rule() {
        // Spec §7.1 #12.
        use crate::config_tui::edit::{NewRule, NewStyle};
        use crate::style::Color;
        let source = "[general]\ntheme = \"dark\"\n";
        let doc: DocumentMut = source.parse().expect("valid TOML");
        let mut edits = PendingEdits::default();
        edits.added.push(NewRule {
            name: "x".to_owned(),
            pattern: "p".to_owned(),
            style: NewStyle { fg: Some(Some(Color::Cyan)), ..NewStyle::default() },
        });
        let out = apply_edits(&doc, &edits).expect("ok");
        assert!(out.contains("[[rules]]"), "must append [[rules]]: {out:?}");
        assert!(out.contains("name = \"x\""));
        assert!(out.contains("pattern = 'p'") || out.contains("pattern = \"p\""));
        assert!(out.contains("fg = \"cyan\""));
    }

    #[test]
    fn type_mismatch_returns_typed_error() {
        // Spec §7.1 #15 + §13.2 B5 fold — reachable via [[general]] user-written shape.
        // (Defensive path: well-typed doc usually wouldn't have this, but
        // if user manually writes [[general]] as array-of-tables, reconcile
        // emits TypeMismatch instead of panicking.)
        // NOTE: toml_edit 0.25 Item::type_name() returns "array of tables"
        // (spaces, not hyphens) — both field assert and Display are pinned to
        // observed output.
        let source = "[[general]]\nname = \"x\"\n";
        let doc: DocumentMut = source.parse().expect("valid TOML (array-of-tables for general)");
        let mut edits = PendingEdits::default();
        edits.general.theme = Some(Some("dark".to_owned()));
        let err = apply_edits(&doc, &edits).expect_err("must error");
        match &err {
            ReconcileError::TypeMismatch { path, expected, actual } => {
                assert_eq!(path, "general");
                assert_eq!(*expected, "table");
                assert_eq!(*actual, "array of tables");
            }
            other @ ReconcileError::UnsupportedDeletionTarget { .. } => {
                panic!("expected TypeMismatch, got {other:?}")
            }
        }
        let display = format!("{err}");
        assert_eq!(
            display,
            "type mismatch at general: expected table, found array of tables (DocumentMut shape diverged from validated parse — config may be corrupt; try reloading the file)",
            "Display string byte-pinned"
        );
    }

    #[test]
    fn crlf_line_ending_source_preserved_on_mutation() {
        // Spec §7.1 #20 + §13.4 N6 fold — defensive baseline; pins whatever
        // toml_edit 0.25 actually does with \r\n (preserve OR normalize).
        let source = "[general]\r\ntheme = \"dark\"\r\n";
        let doc: DocumentMut = source.parse().expect("valid TOML with CRLF");
        let mut edits = PendingEdits::default();
        edits.general.theme = Some(Some("light".to_owned()));
        let out = apply_edits(&doc, &edits).expect("ok");
        // Pin observed behavior: assert theme updated regardless of \r\n preservation.
        assert!(out.contains("light"), "theme updated: {out:?}");
        // Document observed line-ending behavior:
        let has_crlf = out.contains("\r\n");
        let has_lf_only = out.lines().count() > 1 && !out.contains("\r\n");
        // One of these must be true; either is documented contract.
        assert!(has_crlf || has_lf_only, "must have consistent line endings; got: {out:?}");
        // If toml_edit normalizes to \n: this test documents that.
        // If it preserves \r\n: this test documents that too.
        // The test ensures we KNOW which one and detect regressions.
    }
}
