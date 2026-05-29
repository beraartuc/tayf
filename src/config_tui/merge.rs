//! AST-level three-way structural merge over [`toml_edit::DocumentMut`].
//!
//! [`merge_three_way`] takes the disk content as it existed at the last
//! successful save (`base`), the TUI's pending-edit projection (`ours`),
//! and the current on-disk content (`theirs`). It walks all keys
//! recursively and returns an auto-merged document plus the set of
//! conflicting keys.
//!
//! The algorithm owns NO IO and depends only on `toml_edit` — the
//! `tests/config_tui_merge_3way.rs` integration tests drive it through
//! the public module boundary. v0.6.2 §3.5.
//!
//! # Algorithm
//!
//! For each top-level key `k` present in `base ∪ ours ∪ theirs`:
//!
//! | base == ours | base == theirs | action                            |
//! |--------------|----------------|-----------------------------------|
//! | yes          | yes            | keep (no change anywhere)          |
//! | yes          | no             | take theirs (only theirs changed)  |
//! | no           | yes            | take ours (only ours changed)      |
//! | no           | no             | convergent if ours == theirs;      |
//! |              |                | else recurse if all three are      |
//! |              |                | tables; else conflict (or whole-   |
//! |              |                | array conflict for `[[tables]]`).  |
//!
//! Item equality is `Display`-equality: `format!("{item}")` round-trips
//! a `toml_edit::Item`. This matches user-visible TOML semantics and
//! transparently handles toml_edit 0.25 quirks (comment-only deltas
//! and CRLF→LF normalization at parse time both round-trip equal).
//! Memory `feedback_toml_edit_025_quirks`.
//!
//! # Known limitations
//!
//! - **Numeric coercion**: `1` (Integer) and `1.0` (Float) compare
//!   distinct — toml_edit makes no coercion. Documented as a test
//!   so a future change is loud.

use toml_edit::{DocumentMut, Item, Table};

/// Dotted key path from the document root, one segment per level.
/// `vec!["general".to_owned(), "theme".to_owned()]` ≡ `general.theme`.
pub type KeyPath = Vec<String>;

/// Format a [`KeyPath`] as a dotted-string for UI surfaces (toast +
/// conflict-list rows).
#[must_use]
pub fn key_path_display(path: &KeyPath) -> String {
    path.join(".")
}

/// Shape classification of a conflict — drives the conflict-list UI's
/// per-row affordances (Leaf rows can offer Ours/Theirs picks; Block
/// rows fall back to Skip).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictValueShape {
    /// Scalar value (string, integer, float, boolean, datetime), or
    /// absent on at least one side with the others scalar.
    Leaf,
    /// Compound value (table, inline table, array, array-of-tables) or
    /// shape-mismatch across the three sides.
    Block,
}

/// One auto-unmergeable key.
#[derive(Debug, Clone)]
pub struct KeyConflict {
    /// Dotted path from the document root.
    pub path: KeyPath,
    /// Rendered base value — `format!("{item}")` of `base[path]`, or
    /// `"(absent)"` if the key did not exist in `base`.
    pub base_value: String,
    /// Rendered ours value — see [`Self::base_value`].
    pub ours_value: String,
    /// Rendered theirs value — see [`Self::base_value`].
    pub theirs_value: String,
    /// Leaf/Block classification — see [`ConflictValueShape`].
    pub shape: ConflictValueShape,
    /// `true` when the conflict is a whole-array replace of an
    /// `[[array-of-tables]]` (v0.6.2 limitation).
    pub is_array_block: bool,
}

/// Output of [`merge_three_way`].
#[derive(Debug, Clone)]
pub struct MergeResult {
    /// `base` with every non-conflicting key resolved. Conflicting keys
    /// retain their `base` value here — the UI applies the user's
    /// per-key choice via [`write_to_path`].
    pub auto_merged: DocumentMut,
    /// Every key the algorithm could not auto-resolve, in
    /// pre-order traversal order.
    pub conflicts: Vec<KeyConflict>,
}

/// Errors returned by [`write_to_path`].
#[derive(Debug, thiserror::Error)]
pub enum WriteToPathError {
    /// `dest` had a non-table value at an intermediate segment, so
    /// the source value cannot be inserted there.
    #[error("type mismatch at {path}: dest is {dest_type}, source is {source_type}")]
    TypeMismatch {
        /// Dotted path where the mismatch was detected.
        path: String,
        /// `toml_edit::Item::type_name()` of the dest item.
        dest_type: String,
        /// `toml_edit::Item::type_name()` of the source item, or
        /// `"table"` when the caller was trying to descend.
        source_type: String,
    },
    /// `source` did not have an item at `path` — caller is asking us
    /// to copy a value that does not exist on the source side.
    #[error("missing intermediate at {path}")]
    MissingIntermediate {
        /// Dotted path where the source descent ran out.
        path: String,
    },
}

/// Walk `base`, `ours`, and `theirs` together and produce a structural
/// 3-way merge. See module docs for the per-key resolution table.
#[must_use]
pub fn merge_three_way(
    base: &DocumentMut,
    ours: &DocumentMut,
    theirs: &DocumentMut,
) -> MergeResult {
    let mut auto_merged = DocumentMut::new();
    let mut conflicts = Vec::new();
    let mut path: KeyPath = Vec::new();
    merge_table(
        base.as_table(),
        ours.as_table(),
        theirs.as_table(),
        auto_merged.as_table_mut(),
        &mut path,
        &mut conflicts,
    );
    MergeResult { auto_merged, conflicts }
}

// reason: bv/ov/tv and bv_eq_ov/bv_eq_tv encode the algorithm vocabulary
// (base / ours / theirs). Renaming for clippy::similar_names would obscure
// the per-arm comparison invariants. Spec §3.5.
#[allow(clippy::similar_names)]
fn merge_table(
    base: &Table,
    ours: &Table,
    theirs: &Table,
    out: &mut Table,
    path: &mut KeyPath,
    conflicts: &mut Vec<KeyConflict>,
) {
    let mut keys: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (k, _) in base {
        keys.insert(k.to_owned());
    }
    for (k, _) in ours {
        keys.insert(k.to_owned());
    }
    for (k, _) in theirs {
        keys.insert(k.to_owned());
    }

    for k in keys {
        path.push(k.clone());

        let bv = base.get(&k);
        let ov = ours.get(&k);
        let tv = theirs.get(&k);

        let bv_eq_ov = items_eq(bv, ov);
        let bv_eq_tv = items_eq(bv, tv);
        let ov_eq_tv = items_eq(ov, tv);

        if bv_eq_ov && bv_eq_tv {
            // No change on any side — propagate base.
            set_or_remove(out, &k, bv);
        } else if bv_eq_ov {
            // Only theirs changed → take theirs.
            set_or_remove(out, &k, tv);
        } else if bv_eq_tv {
            // Only ours changed → take ours.
            set_or_remove(out, &k, ov);
        } else if ov_eq_tv {
            // Convergent — both changed to the same value.
            set_or_remove(out, &k, ov);
        } else if is_table(bv) && is_table(ov) && is_table(tv) {
            // All three are tables → recurse into a fresh sub-table
            // injected at `k`. Allocating fresh keeps the recursion
            // semantics decoupled from toml_edit's implicit-table
            // bookkeeping (which an earlier `or_insert_with` attempt
            // silently dropped child mutations from).
            let bt = bv.and_then(Item::as_table).expect("is_table verified");
            let ot = ov.and_then(Item::as_table).expect("is_table verified");
            let tt = tv.and_then(Item::as_table).expect("is_table verified");
            out.insert(&k, Item::Table(Table::new()));
            let out_table =
                out.get_mut(&k).and_then(Item::as_table_mut).expect("just inserted as Item::Table");
            merge_table(bt, ot, tt, out_table, path, conflicts);
        } else if is_array_of_tables(bv) && is_array_of_tables(ov) && is_array_of_tables(tv) {
            // v0.7: per-element name-keyed merge. Fallback to whole-array
            // conflict on missing identity, same-side duplicate, or order
            // divergence. Spec §3.2.
            let baot = bv.and_then(Item::as_array_of_tables).expect("is_array_of_tables");
            let oaot = ov.and_then(Item::as_array_of_tables).expect("is_array_of_tables");
            let taot = tv.and_then(Item::as_array_of_tables).expect("is_array_of_tables");
            merge_array_of_tables(baot, oaot, taot, out, &k, path, conflicts);
        } else {
            // Genuine leaf-or-shape-mismatch conflict.
            conflicts.push(KeyConflict {
                path: path.clone(),
                base_value: render_item(bv),
                ours_value: render_item(ov),
                theirs_value: render_item(tv),
                shape: classify_shape(bv, ov, tv),
                is_array_block: false,
            });
        }

        path.pop();
    }
}

// reason: ba/oa/ta encode the algorithm vocabulary (base/ours/theirs) and
// match the surrounding `merge_table` style. Spec §3.2.
#[allow(clippy::similar_names)]
fn merge_array_of_tables(
    base: &toml_edit::ArrayOfTables,
    ours: &toml_edit::ArrayOfTables,
    theirs: &toml_edit::ArrayOfTables,
    out: &mut Table,
    key: &str,
    path: &mut KeyPath,
    conflicts: &mut Vec<KeyConflict>,
) {
    use std::collections::BTreeSet;

    // Identity validation. Any element without a String `name` → fallback.
    let identity_ok = |aot: &toml_edit::ArrayOfTables| -> bool {
        aot.iter().all(|t| t.get("name").and_then(Item::as_str).is_some())
    };
    if !(identity_ok(base) && identity_ok(ours) && identity_ok(theirs)) {
        conflicts.push(KeyConflict {
            path: path.clone(),
            base_value: render_item(Some(&Item::ArrayOfTables(base.clone()))),
            ours_value: render_item(Some(&Item::ArrayOfTables(ours.clone()))),
            theirs_value: render_item(Some(&Item::ArrayOfTables(theirs.clone()))),
            shape: ConflictValueShape::Block,
            is_array_block: true,
        });
        return;
    }

    let collect_names = |aot: &toml_edit::ArrayOfTables| -> Vec<String> {
        aot.iter()
            .map(|t| t.get("name").and_then(Item::as_str).expect("identity_ok verified").to_owned())
            .collect()
    };
    let base_names = collect_names(base);
    let ours_names = collect_names(ours);
    let theirs_names = collect_names(theirs);

    // Same-side duplicate guard (defensive — apply_user_rules invariant).
    let has_dup = |names: &[String]| -> bool {
        let mut sorted = names.to_vec();
        sorted.sort();
        sorted.windows(2).any(|w| w[0] == w[1])
    };
    if has_dup(&ours_names) {
        debug_assert!(false, "ours-side duplicate name violates apply_user_rules invariant");
    }
    if has_dup(&base_names) || has_dup(&ours_names) || has_dup(&theirs_names) {
        conflicts.push(KeyConflict {
            path: path.clone(),
            base_value: render_item(Some(&Item::ArrayOfTables(base.clone()))),
            ours_value: render_item(Some(&Item::ArrayOfTables(ours.clone()))),
            theirs_value: render_item(Some(&Item::ArrayOfTables(theirs.clone()))),
            shape: ConflictValueShape::Block,
            is_array_block: true,
        });
        return;
    }

    // Order-divergence guard. Same set, different order → whole-array conflict.
    let base_set: BTreeSet<&String> = base_names.iter().collect();
    let ours_set: BTreeSet<&String> = ours_names.iter().collect();
    let theirs_set: BTreeSet<&String> = theirs_names.iter().collect();
    if base_set == ours_set && base_set == theirs_set && ours_names != theirs_names {
        conflicts.push(KeyConflict {
            path: path.clone(),
            base_value: render_item(Some(&Item::ArrayOfTables(base.clone()))),
            ours_value: render_item(Some(&Item::ArrayOfTables(ours.clone()))),
            theirs_value: render_item(Some(&Item::ArrayOfTables(theirs.clone()))),
            shape: ConflictValueShape::Block,
            is_array_block: true,
        });
        return;
    }

    // Distinct ordered name list: base first, then ours-only, then theirs-only.
    let mut order: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for n in base_names.iter().chain(ours_names.iter()).chain(theirs_names.iter()) {
        if seen.insert(n.clone()) {
            order.push(n.clone());
        }
    }

    let find_by_name = |aot: &toml_edit::ArrayOfTables, n: &str| -> Option<Table> {
        aot.iter().find(|t| t.get("name").and_then(Item::as_str) == Some(n)).cloned()
    };

    // Build the resulting ArrayOfTables on `out` at `key`.
    let mut result = toml_edit::ArrayOfTables::new();
    for name in &order {
        let be = find_by_name(base, name);
        let oe = find_by_name(ours, name);
        let te = find_by_name(theirs, name);
        merge_aot_element(
            be.as_ref(),
            oe.as_ref(),
            te.as_ref(),
            name,
            path,
            &mut result,
            conflicts,
        );
    }
    out.insert(key, Item::ArrayOfTables(result));
}

// reason: be/oe/te encode the algorithm vocabulary (base/ours/theirs element)
// matching the surrounding merge_array_of_tables style. Spec §3.2.
#[allow(clippy::similar_names, clippy::too_many_arguments)]
fn merge_aot_element(
    be: Option<&Table>,
    oe: Option<&Table>,
    te: Option<&Table>,
    name: &str,
    path: &mut KeyPath,
    result: &mut toml_edit::ArrayOfTables,
    conflicts: &mut Vec<KeyConflict>,
) {
    path.push(name.to_owned());

    let tables_match = |a: Option<&Table>, b: Option<&Table>| -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(x), Some(y)) => tables_eq(x, y),
            _ => false,
        }
    };

    let beq_oe = tables_match(be, oe);
    let beq_te = tables_match(be, te);
    let oeq_te = tables_match(oe, te);

    if beq_oe && beq_te {
        // No change — propagate base.
        if let Some(t) = be {
            result.push(t.clone());
        }
    } else if beq_oe {
        // Only theirs changed → take theirs.
        if let Some(t) = te {
            result.push(t.clone());
        }
    } else if beq_te {
        // Only ours changed → take ours.
        if let Some(t) = oe {
            result.push(t.clone());
        }
    } else if oeq_te {
        // Convergent — both changed to the same element.
        if let Some(t) = oe {
            result.push(t.clone());
        }
    } else {
        match (be, oe, te) {
            // Delete-modify pairs.
            (Some(_), None, Some(_)) | (Some(_), Some(_), None) => {
                let be_item = be.map(item_from_table_ref);
                let oe_item = oe.map(item_from_table_ref);
                let te_item = te.map(item_from_table_ref);
                conflicts.push(KeyConflict {
                    path: path.clone(),
                    base_value: render_item(be_item.as_ref()),
                    ours_value: render_item(oe_item.as_ref()),
                    theirs_value: render_item(te_item.as_ref()),
                    shape: ConflictValueShape::Block,
                    is_array_block: false,
                });
                if let Some(t) = be {
                    result.push(t.clone());
                }
            }
            // Insert collision (be absent, oe and te both present, oe != te).
            (None, Some(_), Some(_)) => {
                let oe_item = oe.map(item_from_table_ref);
                let te_item = te.map(item_from_table_ref);
                conflicts.push(KeyConflict {
                    path: path.clone(),
                    base_value: "(absent)".to_owned(),
                    ours_value: render_item(oe_item.as_ref()),
                    theirs_value: render_item(te_item.as_ref()),
                    shape: ConflictValueShape::Block,
                    is_array_block: false,
                });
            }
            // All three present, all different → recurse on element fields.
            (Some(b), Some(o), Some(t)) => {
                let mut element = Table::new();
                merge_table(b, o, t, &mut element, path, conflicts);
                result.push(element);
            }
            // Unreachable: equality short-circuits earlier in this function.
            (Some(_) | None, None, None) | (None, None, Some(_)) | (None, Some(_), None) => {
                unreachable!("equality cases handled above in merge_aot_element");
            }
        }
    }

    path.pop();
}

fn item_from_table_ref(t: &Table) -> Item {
    Item::Table(t.clone())
}

fn items_eq(a: Option<&Item>, b: Option<&Item>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => items_eq_inner(x, y),
        _ => false,
    }
}

fn items_eq_inner(a: &Item, b: &Item) -> bool {
    match (a, b) {
        // Item::Table's Display is empty (it relies on the parent
        // document to emit section headers), so a string compare here
        // would call every pair of tables "equal" and short-circuit the
        // recursion. Compare by walking entries instead.
        (Item::Table(ta), Item::Table(tb)) => tables_eq(ta, tb),
        (Item::ArrayOfTables(aa), Item::ArrayOfTables(ab)) => array_of_tables_eq(aa, ab),
        // Item::Value's Display is the literal TOML for the scalar
        // (or inline table / array literal), which is what we want.
        _ => format!("{a}") == format!("{b}"),
    }
}

fn tables_eq(a: &Table, b: &Table) -> bool {
    let ka: std::collections::BTreeSet<&str> = a.iter().map(|(k, _)| k).collect();
    let kb: std::collections::BTreeSet<&str> = b.iter().map(|(k, _)| k).collect();
    if ka != kb {
        return false;
    }
    for k in ka {
        if !items_eq(a.get(k), b.get(k)) {
            return false;
        }
    }
    true
}

fn array_of_tables_eq(a: &toml_edit::ArrayOfTables, b: &toml_edit::ArrayOfTables) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).all(|(x, y)| tables_eq(x, y))
}

fn is_table(it: Option<&Item>) -> bool {
    matches!(it, Some(Item::Table(_)))
}

fn is_array_of_tables(it: Option<&Item>) -> bool {
    matches!(it, Some(Item::ArrayOfTables(_)))
}

fn classify_shape(b: Option<&Item>, o: Option<&Item>, t: Option<&Item>) -> ConflictValueShape {
    let is_scalar_or_absent = |it: Option<&Item>| -> bool {
        match it {
            Some(Item::Value(v)) => matches!(
                v,
                toml_edit::Value::String(_)
                    | toml_edit::Value::Integer(_)
                    | toml_edit::Value::Float(_)
                    | toml_edit::Value::Boolean(_)
                    | toml_edit::Value::Datetime(_)
            ),
            None => true,
            _ => false,
        }
    };
    if is_scalar_or_absent(b) && is_scalar_or_absent(o) && is_scalar_or_absent(t) {
        ConflictValueShape::Leaf
    } else {
        ConflictValueShape::Block
    }
}

fn render_item(it: Option<&Item>) -> String {
    match it {
        Some(i) => format!("{i}"),
        None => "(absent)".to_owned(),
    }
}

fn set_or_remove(out: &mut Table, key: &str, item: Option<&Item>) {
    match item {
        Some(i) => {
            out.insert(key, i.clone());
        }
        None => {
            out.remove(key);
        }
    }
}

/// Copy the leaf at `path` from `source` into `doc`, creating any
/// missing intermediate tables in `doc` along the way.
///
/// Used by the conflict-list UI: when the user picks Theirs (or Ours)
/// for a conflicting key, the chosen side's value at that path overwrites
/// the auto-merged document's value.
///
/// # Errors
///
/// - [`WriteToPathError::TypeMismatch`] when an intermediate segment in
///   `doc` has a non-table value (cannot descend without overwriting).
/// - [`WriteToPathError::MissingIntermediate`] when `source` does not
///   have a value at `path` (caller asked us to copy something that
///   does not exist).
pub fn write_to_path(
    doc: &mut DocumentMut,
    path: &[String],
    source: &DocumentMut,
) -> Result<(), WriteToPathError> {
    if path.is_empty() {
        return Err(WriteToPathError::MissingIntermediate { path: "(empty)".to_owned() });
    }
    let display_path = path.join(".");

    // Resolve the source leaf by descending the path.
    let mut src_item: &Item = source.as_item();
    for seg in path {
        let Some(t) = src_item.as_table() else {
            return Err(WriteToPathError::MissingIntermediate { path: display_path.clone() });
        };
        let Some(next) = t.get(seg) else {
            return Err(WriteToPathError::MissingIntermediate { path: display_path.clone() });
        };
        src_item = next;
    }
    let src_leaf = src_item.clone();

    // Walk `doc`, creating intermediate tables as needed.
    let mut cur: &mut Item = doc.as_item_mut();
    for (i, seg) in path.iter().enumerate() {
        let is_leaf = i == path.len() - 1;
        let cur_type = cur.type_name().to_owned();
        let Some(t) = cur.as_table_mut() else {
            return Err(WriteToPathError::TypeMismatch {
                path: display_path.clone(),
                dest_type: cur_type,
                source_type: "table".to_owned(),
            });
        };
        if is_leaf {
            t.insert(seg, src_leaf.clone());
            return Ok(());
        }
        let entry = t.entry(seg).or_insert_with(|| Item::Table(Table::new()));
        cur = entry;
    }
    Ok(())
}

/// Return `true` if `doc` has a value at `path`. Read-only mirror of the
/// dest-side descent in [`write_to_path`].
///
/// Used by `events::build_final_doc` to short-circuit the `Skip`-on-
/// `Block`-shape conflict arm when the base side also has no value at
/// the path: calling [`write_to_path`] in that case would surface a
/// misleading "missing intermediate at <key>" toast (v0.6.2 cross-
/// cutting review I3). `auto_merged` already carries no value at
/// conflicting keys by construction in [`merge_three_way`], so skipping
/// the write is the correct no-op.
#[must_use]
pub(crate) fn path_exists(doc: &DocumentMut, path: &[String]) -> bool {
    if path.is_empty() {
        return false;
    }
    let mut cur: &Item = doc.as_item();
    for seg in path {
        let Some(t) = cur.as_table() else { return false };
        let Some(next) = t.get(seg) else { return false };
        cur = next;
    }
    true
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    //! Pure-logic tests for `merge_three_way` and `write_to_path`.
    //!
    //! Memory `feedback_test_assertion_specificity` — exact-match
    //! conflict counts + pinned shape variants rather than "contains".
    //!
    //! Moved inline from the former `tests/config_tui_merge_3way.rs`
    //! integration suite in v0.6.3 when `config_tui::merge` was demoted
    //! to `pub(crate)` to drop the `toml_edit::DocumentMut` re-export from
    //! the crate's public surface. The algorithm is pure (no IO), so the
    //! integration-vs-unit boundary added no coverage.

    use std::str::FromStr;

    use super::{
        key_path_display, merge_three_way, write_to_path, ConflictValueShape, WriteToPathError,
    };
    use toml_edit::DocumentMut;

    fn doc(s: &str) -> DocumentMut {
        DocumentMut::from_str(s).expect("test fixture is valid TOML")
    }

    // -----------------------------------------------------------------------
    // merge_three_way pure cases
    // -----------------------------------------------------------------------

    #[test]
    fn merge_disjoint_keys_auto_merges_both_into_result() {
        let base = doc("[general]\ntheme = \"dark\"\n");
        let ours = doc("[general]\ntheme = \"dark\"\nshell = \"fish\"\n");
        let theirs = doc("[general]\ntheme = \"dark\"\nverbose = true\n");
        let merge = merge_three_way(&base, &ours, &theirs);
        assert!(merge.conflicts.is_empty(), "disjoint keys must not conflict");
        let merged = merge.auto_merged.to_string();
        assert!(merged.contains("shell"), "ours-only key 'shell' survives merge");
        assert!(merged.contains("verbose"), "theirs-only key 'verbose' survives merge");
    }

    #[test]
    fn merge_same_key_both_changed_to_different_values_yields_one_leaf_conflict() {
        let base = doc("[general]\ntheme = \"dark\"\n");
        let ours = doc("[general]\ntheme = \"tokyo\"\n");
        let theirs = doc("[general]\ntheme = \"light\"\n");
        let merge = merge_three_way(&base, &ours, &theirs);
        assert_eq!(merge.conflicts.len(), 1, "exactly one conflict at general.theme");
        let c = &merge.conflicts[0];
        assert_eq!(c.path, vec!["general".to_owned(), "theme".to_owned()]);
        assert_eq!(c.shape, ConflictValueShape::Leaf, "scalar string is a Leaf");
        assert!(!c.is_array_block, "single scalar is not an array block");
    }

    #[test]
    fn merge_same_key_both_changed_to_same_value_is_no_conflict() {
        let base = doc("[general]\ntheme = \"dark\"\n");
        let ours = doc("[general]\ntheme = \"light\"\n");
        let theirs = doc("[general]\ntheme = \"light\"\n");
        let merge = merge_three_way(&base, &ours, &theirs);
        assert!(merge.conflicts.is_empty(), "convergent value is auto-merge");
    }

    #[test]
    fn merge_ours_only_change_takes_ours_no_conflict() {
        let base = doc("[general]\ntheme = \"dark\"\n");
        let ours = doc("[general]\ntheme = \"tokyo\"\n");
        let theirs = base.clone();
        let merge = merge_three_way(&base, &ours, &theirs);
        assert!(merge.conflicts.is_empty());
        assert!(
            merge.auto_merged.to_string().contains("tokyo"),
            "ours-only change kept by auto-merger"
        );
    }

    #[test]
    fn merge_theirs_only_change_takes_theirs_no_conflict() {
        let base = doc("[general]\ntheme = \"dark\"\n");
        let ours = base.clone();
        let theirs = doc("[general]\ntheme = \"light\"\n");
        let merge = merge_three_way(&base, &ours, &theirs);
        assert!(merge.conflicts.is_empty());
        assert!(
            merge.auto_merged.to_string().contains("light"),
            "theirs-only change kept by auto-merger"
        );
    }

    #[test]
    fn merge_nested_table_disjoint_changes_recurse_and_auto_merge() {
        let base = doc("[general.colors]\nbg = \"black\"\n");
        let ours = doc("[general.colors]\nbg = \"black\"\nfg = \"white\"\n");
        let theirs = doc("[general.colors]\nbg = \"black\"\naccent = \"red\"\n");
        let merge = merge_three_way(&base, &ours, &theirs);
        let s = merge.auto_merged.to_string();
        assert!(
            merge.conflicts.is_empty(),
            "disjoint nested keys auto-merge; conflicts={:?}; merged=\n{}",
            merge.conflicts.iter().map(|c| key_path_display(&c.path)).collect::<Vec<_>>(),
            s,
        );
        assert!(s.contains("fg"), "ours-only nested key survives; merged=\n{s}");
        assert!(s.contains("accent"), "theirs-only nested key survives; merged=\n{s}");
    }

    #[test]
    fn merge_comment_only_change_in_theirs_is_no_conflict_toml_edit_025_quirk() {
        // toml_edit 0.25 normalizes Item value comparison: comment-only deltas
        // round-trip equal via `format!("{item}")`. Memory
        // `feedback_toml_edit_025_quirks`.
        let base = doc("[general]\ntheme = \"dark\"\n");
        let ours = base.clone();
        let theirs = doc("[general]\n# new comment\ntheme = \"dark\"\n");
        let merge = merge_three_way(&base, &ours, &theirs);
        assert!(merge.conflicts.is_empty(), "comment-only change is no-conflict");
    }

    #[test]
    fn merge_crlf_to_lf_normalization_on_theirs_is_no_conflict_toml_edit_025_quirk() {
        // toml_edit 0.25 normalizes CRLF→LF at parse time. Memory
        // `feedback_toml_edit_025_quirks`.
        let base = doc("[general]\ntheme = \"dark\"\n");
        let ours = base.clone();
        let theirs = doc("[general]\r\ntheme = \"dark\"\r\n");
        let merge = merge_three_way(&base, &ours, &theirs);
        assert!(merge.conflicts.is_empty(), "CRLF normalized; no conflict");
    }

    #[test]
    fn merge_array_of_tables_per_element_yields_field_level_conflict() {
        // v0.7 spec §3.4 #1 — rename + assert flip of the v0.6.2 limitation pin.
        // base/ours/theirs each have one [[rules]] with name="a" but the
        // `pattern` field differs across the three. Per-element merge
        // descends into the element and produces a single Leaf conflict
        // at path = ["rules", "a", "pattern"].
        let base = doc("[[rules]]\nname = \"a\"\npattern = \"A\"\n");
        let ours = doc("[[rules]]\nname = \"a\"\npattern = \"B\"\n");
        let theirs = doc("[[rules]]\nname = \"a\"\npattern = \"C\"\n");
        let merge = merge_three_way(&base, &ours, &theirs);
        assert_eq!(merge.conflicts.len(), 1, "exactly one field-level conflict");
        let c = &merge.conflicts[0];
        assert_eq!(c.path, vec!["rules".to_owned(), "a".to_owned(), "pattern".to_owned()]);
        assert_eq!(c.shape, ConflictValueShape::Leaf, "scalar field is Leaf");
        assert!(!c.is_array_block, "per-element conflict is not array-block");
    }

    #[test]
    fn merge_three_way_convergent_deletion_removes_key() {
        // v0.6.2 cross-cutting review NIT b: both sides converge on
        // "remove key" (bv = Some, ov = None, tv = None → ov_eq_tv = true
        // → set_or_remove(out, &k, None)). Pins the contract so a future
        // merge_table refactor can't silently regress.
        let base = doc("[general]\ntheme = \"dark\"\nverbose = true\n");
        let ours = doc("[general]\ntheme = \"dark\"\n");
        let theirs = doc("[general]\ntheme = \"dark\"\n");
        let merge = merge_three_way(&base, &ours, &theirs);
        assert!(merge.conflicts.is_empty(), "convergent deletion is no-conflict");
        let merged = merge.auto_merged.to_string();
        assert!(
            !merged.contains("verbose"),
            "convergent-deleted key removed from auto_merged; got: {merged:?}"
        );
        assert!(merged.contains("theme = \"dark\""), "unchanged key kept; got: {merged:?}");
    }

    #[test]
    fn path_exists_traverses_existing_segments_and_returns_false_on_first_miss() {
        // v0.6.3 I3 helper — companion to the Skip+absent-base fix in
        // events::build_final_doc.
        let d = doc("[general]\ntheme = \"dark\"\n[a.b]\nc = 1\n");
        assert!(super::path_exists(&d, &["general".to_owned()]));
        assert!(super::path_exists(&d, &["general".to_owned(), "theme".to_owned()]));
        assert!(super::path_exists(&d, &["a".to_owned(), "b".to_owned(), "c".to_owned()]));
        assert!(!super::path_exists(&d, &["general".to_owned(), "missing".to_owned()]));
        assert!(!super::path_exists(&d, &["rules".to_owned()]));
        assert!(!super::path_exists(&d, &[]), "empty path returns false (no key to test)");
    }

    #[test]
    fn merge_integer_vs_float_treated_as_distinct_values_no_coercion() {
        // toml_edit 0.25: `1` (Integer) and `1.0` (Float) are NOT equal —
        // their `Display` impls differ. Documenting this so a future
        // numeric-coercion change is a loud test break.
        let base = doc("x = 0\n");
        let ours = doc("x = 1\n");
        let theirs = doc("x = 1.0\n");
        let merge = merge_three_way(&base, &ours, &theirs);
        assert_eq!(merge.conflicts.len(), 1, "Integer(1) != Float(1.0)");
    }

    // -----------------------------------------------------------------------
    // write_to_path pure cases
    // -----------------------------------------------------------------------

    #[test]
    fn write_to_path_replaces_leaf_scalar_with_source_value() {
        let mut dest = doc("[general]\ntheme = \"dark\"\n");
        let source = doc("[general]\ntheme = \"light\"\n");
        write_to_path(&mut dest, &["general".to_owned(), "theme".to_owned()], &source)
            .expect("write_to_path on existing leaf must succeed");
        assert!(
            dest.to_string().contains("light"),
            "dest carries the source's leaf value after write"
        );
        assert!(!dest.to_string().contains("dark"), "old value is replaced, not appended");
    }

    #[test]
    fn write_to_path_nested_inline_table_replaces_leaf() {
        let mut dest = doc("[a.b]\nc = 1\n");
        let source = doc("[a.b]\nc = 99\n");
        write_to_path(&mut dest, &["a".to_owned(), "b".to_owned(), "c".to_owned()], &source)
            .expect("nested-leaf write must succeed");
        assert!(dest.to_string().contains("99"), "leaf written to nested path");
    }

    #[test]
    fn write_to_path_type_mismatch_returns_error_not_panic() {
        let mut dest = doc("x = 1\n");
        let source = doc("[x]\nnested = 1\n");
        let res = write_to_path(&mut dest, &["x".to_owned(), "nested".to_owned()], &source);
        assert!(
            matches!(res, Err(WriteToPathError::TypeMismatch { .. })),
            "writing into a non-table dest is a clean error, not a panic; got: {res:?}"
        );
    }

    // -----------------------------------------------------------------------
    // AoT positive-path pins (v0.7 spec §3.4 #2, #5, #9)
    // -----------------------------------------------------------------------

    #[test]
    fn merge_array_of_tables_disjoint_insertions_auto_merge() {
        // Spec §3.4 #2. ours appends name="x", theirs appends name="y" →
        // both auto-merge; deterministic order = base + ours-only + theirs-only.
        let base = doc("[[rules]]\nname = \"a\"\n");
        let ours = doc("[[rules]]\nname = \"a\"\n[[rules]]\nname = \"x\"\n");
        let theirs = doc("[[rules]]\nname = \"a\"\n[[rules]]\nname = \"y\"\n");
        let merge = merge_three_way(&base, &ours, &theirs);
        assert!(merge.conflicts.is_empty(), "disjoint inserts must not conflict");
        let s = merge.auto_merged.to_string();
        assert!(s.contains("name = \"a\""));
        assert!(s.contains("name = \"x\""));
        assert!(s.contains("name = \"y\""));
    }

    #[test]
    fn merge_array_of_tables_convergent_deletion_drops_element() {
        // Spec §3.4 #5. base has "a" + "b", ours drops "b", theirs drops "b".
        let base = doc("[[rules]]\nname = \"a\"\n[[rules]]\nname = \"b\"\n");
        let ours = doc("[[rules]]\nname = \"a\"\n");
        let theirs = doc("[[rules]]\nname = \"a\"\n");
        let merge = merge_three_way(&base, &ours, &theirs);
        assert!(merge.conflicts.is_empty(), "convergent deletion = auto-merge");
        let s = merge.auto_merged.to_string();
        assert!(s.contains("name = \"a\""), "kept");
        assert!(!s.contains("name = \"b\""), "dropped");
    }

    #[test]
    fn merge_array_of_tables_order_preserves_base_then_appends_ours_then_theirs() {
        // Spec §3.4 #9. base [a,b], ours [a,b,x], theirs [a,b,y] → [a,b,x,y].
        let base = doc("[[rules]]\nname = \"a\"\n[[rules]]\nname = \"b\"\n");
        let ours =
            doc("[[rules]]\nname = \"a\"\n[[rules]]\nname = \"b\"\n[[rules]]\nname = \"x\"\n");
        let theirs =
            doc("[[rules]]\nname = \"a\"\n[[rules]]\nname = \"b\"\n[[rules]]\nname = \"y\"\n");
        let merge = merge_three_way(&base, &ours, &theirs);
        assert!(merge.conflicts.is_empty());
        let s = merge.auto_merged.to_string();
        let pos_a = s.find("name = \"a\"").expect("a present");
        let pos_b = s.find("name = \"b\"").expect("b present");
        let pos_x = s.find("name = \"x\"").expect("x present");
        let pos_y = s.find("name = \"y\"").expect("y present");
        assert!(pos_a < pos_b && pos_b < pos_x && pos_x < pos_y, "deterministic order");
    }

    #[test]
    fn merge_array_of_tables_convergent_insertion_no_conflict() {
        // Spec §3.4 #3. Both sides add same name="z" with same content.
        let base = doc("[[rules]]\nname = \"a\"\n");
        let ours = doc("[[rules]]\nname = \"a\"\n[[rules]]\nname = \"z\"\npattern = \"Z\"\n");
        let theirs = doc("[[rules]]\nname = \"a\"\n[[rules]]\nname = \"z\"\npattern = \"Z\"\n");
        let merge = merge_three_way(&base, &ours, &theirs);
        assert!(merge.conflicts.is_empty(), "convergent insertion auto-merges");
        let s = merge.auto_merged.to_string();
        assert_eq!(s.matches("name = \"z\"").count(), 1, "single z element");
    }

    #[test]
    fn merge_array_of_tables_convergent_insertion_divergent_content_conflicts() {
        // Spec §3.4 #4. ours adds name="z" pattern="A", theirs adds name="z"
        // pattern="B" → element-level conflict at path ["rules", "z"].
        let base = doc("[[rules]]\nname = \"a\"\n");
        let ours = doc("[[rules]]\nname = \"a\"\n[[rules]]\nname = \"z\"\npattern = \"A\"\n");
        let theirs = doc("[[rules]]\nname = \"a\"\n[[rules]]\nname = \"z\"\npattern = \"B\"\n");
        let merge = merge_three_way(&base, &ours, &theirs);
        assert_eq!(merge.conflicts.len(), 1, "insert-collision yields one conflict");
        let c = &merge.conflicts[0];
        assert_eq!(c.path, vec!["rules".to_owned(), "z".to_owned()]);
        assert_eq!(c.shape, ConflictValueShape::Block, "element-level Block");
        assert!(!c.is_array_block, "per-element conflict, not array-block");
        assert_eq!(c.base_value, "(absent)", "base side absent");
    }
}
