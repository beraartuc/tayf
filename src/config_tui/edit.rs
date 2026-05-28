//! `PendingEdits` aggregator + `RuleEdit` + `StyleKey` + `RuleId`.
//!
//! Accumulates in-TUI mutations not yet on disk. `is_dirty()` is
//! consulted by the quit-confirm modal and the save button.

// reason: RuleId::{Embedded, DiskProfile} variants are constructed
// inside compile_pending's overlay loop (v0.6.1 §3.2) and read by
// reconcile.rs + events.rs, but no top-level tab still constructs
// them via user keystrokes (profile/theme override-copy lands in
// v0.7+). StyleKey::{Numbered, Named} are future capture-group
// style slots (v0.7+). Module-level allow until those TUI wires land.
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};

use crate::style::Color;

/// Identity of a rule across catalog sources. Used as the merge
/// key in conflict-mode save (spec §8.4 D / I-6 fold).
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub(crate) enum RuleId {
    Builtin(&'static str),
    UserConfig(String),
    Embedded { profile: &'static str, rule: String },
    DiskProfile { profile: String, rule: String },
}

/// One per-capture-group style slot. `Default` = the rule's
/// top-level `style = { ... }`; `Numbered(i)` / `Named(name)` =
/// `styles."1"` / `styles."matchname"` entry.
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub(crate) enum StyleKey {
    Default,
    Numbered(u32),
    Named(String),
}

/// A staged style mutation. `None` means "leave that axis unchanged
/// from current source"; `Some(None)` (on `fg`/`bg` and on the
/// boolean axes) means "set explicitly to none / clear".
///
/// `Option<Option<bool>>` on the three boolean axes is **load-bearing**
/// for the TUI tri-state: outer `None` = unedited (no axis interaction
/// this session), `Some(None)` = explicitly cleared by the user (`c`
/// in the color picker), `Some(Some(b))` = explicit set. The two
/// `None` shapes are distinguished downstream so explicit clear can
/// remove the axis key from the inline TOML table — see `reconcile.rs`
/// `write_style_table`. `dim` stays `Option<bool>` because the picker
/// has no `c`-clear keystroke for it (CLI-flag mandate; spec §3.1).
//
// reason: the `Option<Option<_>>` shape on fg/bg + the three bool
// axes is the load-bearing tri-state for the TUI (unedited /
// set-explicitly / explicitly-cleared); replacing with a custom enum
// would just rename the same three states. Spec §6.1 + §3.1.
#[allow(clippy::option_option)]
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub(crate) struct NewStyle {
    pub(crate) fg: Option<Option<Color>>,
    pub(crate) bg: Option<Option<Color>>,
    pub(crate) bold: Option<Option<bool>>,
    pub(crate) italic: Option<Option<bool>>,
    pub(crate) underline: Option<Option<bool>>,
    pub(crate) dim: Option<bool>,
}

/// A staged rule mutation — pattern source edit + per-style-key
/// overlay edits. Empty == no edits for that rule.
#[derive(Default, Clone, Debug)]
pub(crate) struct RuleEdit {
    pub(crate) pattern: Option<String>,
    pub(crate) styles: HashMap<StyleKey, NewStyle>,
}

impl RuleEdit {
    fn is_empty(&self) -> bool {
        self.pattern.is_none() && self.styles.is_empty()
    }
}

/// `[general]` table staged mutations.
///
/// Outer `Option` = "field edited?"; inner `Option<String>` =
/// "set to (None = clear, Some(s) = name)".
//
// reason: the `Option<Option<String>>` shape is the load-bearing
// tri-state for `[general]` edits (unedited / set-to-name /
// explicitly-cleared); replacing with a custom enum would just
// rename the same three states. Spec §6.1.
#[allow(clippy::option_option)]
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub(crate) struct GeneralEdits {
    pub(crate) theme: Option<Option<String>>,
    pub(crate) profile: Option<Option<String>>,
}

impl GeneralEdits {
    fn is_empty(&self) -> bool {
        self.theme.is_none() && self.profile.is_none()
    }
}

/// A wholly new rule (TUI `n` shortcut). Persisted to user-config
/// `[[rules]]` on save.
#[derive(Clone, Debug)]
pub(crate) struct NewRule {
    pub(crate) name: String,
    pub(crate) pattern: String,
    pub(crate) style: NewStyle,
}

/// Top-level edits accumulator.
#[derive(Default, Debug)]
pub(crate) struct PendingEdits {
    pub(crate) general: GeneralEdits,
    pub(crate) rules: HashMap<RuleId, RuleEdit>,
    pub(crate) added: Vec<NewRule>,
    pub(crate) deleted: HashSet<RuleId>,
}

impl PendingEdits {
    /// True iff any mutation has been staged.
    pub(crate) fn is_dirty(&self) -> bool {
        !self.general.is_empty()
            || self.rules.values().any(|e| !e.is_empty())
            || !self.added.is_empty()
            || !self.deleted.is_empty()
    }

    /// Drop all staged edits (used by `m` discard-and-reload in
    /// `SaveDiff` conflict modal).
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_pending_edits_is_not_dirty() {
        let p = PendingEdits::default();
        assert!(!p.is_dirty());
    }

    #[test]
    fn staged_general_theme_marks_dirty() {
        let mut p = PendingEdits::default();
        p.general.theme = Some(Some("dark".to_owned()));
        assert!(p.is_dirty());
    }

    #[test]
    fn staged_rule_edit_with_pattern_marks_dirty() {
        let mut p = PendingEdits::default();
        let id = RuleId::Builtin("uuid");
        p.rules.insert(id, RuleEdit { pattern: Some(r"\bx\b".to_owned()), styles: HashMap::new() });
        assert!(p.is_dirty());
    }

    #[test]
    fn empty_rule_edit_in_map_does_not_mark_dirty() {
        // A no-op RuleEdit (created on focus + abandoned) must not
        // flip the dirty bit. Guard against TUI focus-but-no-change
        // false positives.
        let mut p = PendingEdits::default();
        p.rules.insert(RuleId::Builtin("uuid"), RuleEdit::default());
        assert!(!p.is_dirty());
    }

    #[test]
    fn clear_resets_dirty() {
        let mut p = PendingEdits::default();
        p.added.push(NewRule {
            name: "x".to_owned(),
            pattern: "x".to_owned(),
            style: NewStyle::default(),
        });
        assert!(p.is_dirty());
        p.clear();
        assert!(!p.is_dirty());
    }

    #[test]
    fn delete_then_readd_same_id_consistent_state() {
        let mut p = PendingEdits::default();
        let id = RuleId::UserConfig("xx".to_owned());
        p.deleted.insert(id.clone());
        assert!(p.is_dirty());
        // simulate re-add → deletion no longer applies.
        p.deleted.remove(&id);
        assert!(!p.is_dirty());
    }
}
