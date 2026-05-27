//! [`PendingEdits`] → [`DocumentMut`] walk + serialize.
//!
//! Pure data-transform module: takes a frozen [`DocumentMut`] + staged
//! edits, returns the new TOML source bytes. No I/O, no side effects.
//!
//! Public-to-crate API:
//!   `pub(crate) fn apply_edits(doc, edits) -> Result<String, ReconcileError>`
//!
//! Spec §5 — handler-by-handler walk. Spec §6 — [`ReconcileError`] variants.

use crate::config_tui::edit::PendingEdits;
use toml_edit::DocumentMut;

// reason: variants consumed by Task B5 (apply_deletion) + Task B1 (ensure_general_table);
// v0.5.5 Phase A2 skeleton.
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

/// Walk `edits` into `doc` (cloned internally) and serialize to TOML
/// string. Spec §5 algorithm. **Implementation note:** `doc` is cloned
/// internally (`DocumentMut::clone()` is O(n) tree-walk; acceptable for
/// save-on-Ctrl+S frequency, not a hot path). Caller's snapshot.doc is
/// not mutated.
// reason: unnecessary_wraps — Phase A3 facade now calls this, but the
// body is still the pass-through skeleton; ReconcileError variants are
// only produced in Phase B handlers (B1 + B5 consumers).
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn apply_edits(
    doc: &DocumentMut,
    _edits: &PendingEdits,
) -> Result<String, ReconcileError> {
    // Phase A2 skeleton: pass-through. Phase B fills in per-handler walk.
    let working = doc.clone();
    Ok(working.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
