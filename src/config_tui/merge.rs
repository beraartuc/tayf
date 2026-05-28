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
//! # v0.6.2 limitations
//!
//! - **Array-of-tables (`[[rules]]`)**: changes inside such an array
//!   yield a whole-array conflict, not per-element merging. v0.7+
//!   may add identity-keyed per-element merge.
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
            // v0.6.2: whole-array conflict; per-element merge lives in v0.7+.
            conflicts.push(KeyConflict {
                path: path.clone(),
                base_value: render_item(bv),
                ours_value: render_item(ov),
                theirs_value: render_item(tv),
                shape: ConflictValueShape::Block,
                is_array_block: true,
            });
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
