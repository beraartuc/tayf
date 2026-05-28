//! Integration tests for v0.6.2 G7 — Item 5: AST-level 3-way merge.
//!
//! Pure-logic TDD: every assertion drives a specific branch in
//! `merge_three_way` / `write_to_path` against handcrafted
//! `toml_edit::DocumentMut` triples. Spec §3.5. Memory
//! `feedback_test_assertion_specificity` — exact-match conflict counts +
//! pinned shape variants rather than "contains".

#![allow(clippy::expect_used)] // integration-test convenience

use std::str::FromStr;

use tayf::config_tui::merge::{
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
fn merge_array_of_tables_yields_whole_array_block_conflict_v0_6_2_limitation() {
    // v0.6.2 does NOT element-wise merge `[[rules]]` array-of-tables.
    // Whole-array conflict is the documented limitation. Test name pins
    // the limitation per memory `feedback_collision_pin_pattern` — a
    // future per-element AoT merge will rename and force this assertion
    // off the `_v0_6_2_limitation` suffix grep.
    let base = doc("[[rules]]\nname = \"a\"\npattern = \"A\"\n");
    let ours = doc("[[rules]]\nname = \"a\"\npattern = \"B\"\n");
    let theirs = doc("[[rules]]\nname = \"a\"\npattern = \"C\"\n");
    let merge = merge_three_way(&base, &ours, &theirs);
    assert_eq!(merge.conflicts.len(), 1, "whole-array conflict, not per-element");
    let c = &merge.conflicts[0];
    assert!(c.is_array_block, "array-of-tables flagged as array block");
    assert_eq!(c.shape, ConflictValueShape::Block);
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
    assert!(dest.to_string().contains("light"), "dest carries the source's leaf value after write");
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
