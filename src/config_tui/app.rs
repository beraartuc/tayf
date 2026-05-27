//! TUI App state — [`Tab`] enum, [`TabFocus`], [`Modal`] stack, [`Catalog`].
//!
//! Single-thread design (spec §6.2). No `Arc<Mutex<App>>`.

use std::sync::Arc;
use std::time::Instant;

use crate::config_tui::edit::PendingEdits;
use crate::config_tui::snapshot::ConfigSnapshot;

/// Which top-level tab is focused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tab {
    Patterns,
    Themes,
    Profiles,
    Status,
}

impl Tab {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Patterns => Self::Themes,
            Self::Themes => Self::Profiles,
            Self::Profiles => Self::Status,
            Self::Status => Self::Patterns,
        }
    }
    pub(crate) fn prev(self) -> Self {
        match self {
            Self::Patterns => Self::Status,
            Self::Themes => Self::Patterns,
            Self::Profiles => Self::Themes,
            Self::Status => Self::Profiles,
        }
    }
    pub(crate) fn from_digit(d: u8) -> Option<Self> {
        match d {
            1 => Some(Self::Patterns),
            2 => Some(Self::Themes),
            3 => Some(Self::Profiles),
            4 => Some(Self::Status),
            _ => None,
        }
    }
}

/// Per-tab cursor / scroll state. Each tab maintains its own focus
/// so switching tabs preserves position.
#[derive(Default, Debug)]
pub(crate) struct TabFocus {
    pub(crate) patterns: PatternsFocus,
    pub(crate) themes: ThemesFocus,
    pub(crate) profiles: ProfilesFocus,
    pub(crate) status: StatusFocus,
}

#[derive(Default, Debug)]
pub(crate) struct PatternsFocus {
    pub(crate) selected_idx: usize,
    // reason: written by h/l/Enter dispatch; v0.5.5+ wires the render-side
    // split-pane focus indicator that reads it.
    #[allow(dead_code)]
    pub(crate) detail_focused: bool,
}
#[derive(Default, Debug)]
pub(crate) struct ThemesFocus {
    pub(crate) selected_idx: usize,
    // reason: written by Enter dispatch; v0.5.5+ wires the render-side
    // split-pane focus indicator that reads it.
    #[allow(dead_code)]
    pub(crate) detail_focused: bool,
}
#[derive(Default, Debug)]
pub(crate) struct ProfilesFocus {
    pub(crate) selected_idx: usize,
    // reason: written by Enter dispatch; v0.5.5+ wires the render-side
    // split-pane focus indicator that reads it.
    #[allow(dead_code)]
    pub(crate) detail_focused: bool,
}
#[derive(Default, Debug)]
pub(crate) struct StatusFocus {
    pub(crate) scroll: usize,
}

/// Modal overlay. `App.modal` is `Option<Modal>` — no stacking (spec §7.2);
/// every modal-opening code path is guarded by `modal.is_none()`. Detailed
/// state types for the C4 modals live in widgets/* modules; tag variants
/// declared here so `dispatch_key` can pattern-match them.
#[derive(Debug)]
pub(crate) enum Modal {
    Confirm { msg: String, action: ConfirmAction },
    Error(String),
    QuitWithUnsavedEdits,
    ColorPicker(crate::config_tui::widgets::color_picker::ColorPickerState),
    SaveDiff,
    FullPreview,
    Search,
    SampleSet,
}

#[derive(Debug)]
pub(crate) enum ConfirmAction {
    // reason: v0.5.5+ "Discard edits and reload" Confirm path; tag declared
    // here so events.rs apply_confirm pattern-match stays exhaustive.
    #[allow(dead_code)]
    DiscardEditsAndReload,
    DeleteUserRule(String),
    ResetUserOverride(String),
    // reason: v0.5.5+ Shift+D init-from-dump Confirm path; tag declared
    // here so events.rs apply_confirm pattern-match stays exhaustive.
    #[allow(dead_code)]
    InitFromDump,
}

/// Toast — auto-dismiss after 3 s.
#[derive(Debug)]
pub(crate) struct Toast {
    pub(crate) text: String,
    pub(crate) kind: ToastKind,
    pub(crate) shown_at: Instant,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ToastKind {
    Ok,
    Warn,
}

impl Toast {
    pub(crate) fn ok(text: impl Into<String>) -> Self {
        Self { text: text.into(), kind: ToastKind::Ok, shown_at: Instant::now() }
    }
    pub(crate) fn warn(text: impl Into<String>) -> Self {
        Self { text: text.into(), kind: ToastKind::Warn, shown_at: Instant::now() }
    }
    pub(crate) fn expired(&self) -> bool {
        self.shown_at.elapsed() >= std::time::Duration::from_secs(3)
    }
}

/// Catalog — read-only enumeration of available rules / themes /
/// profiles. Built once at `App` init from existing tayf accessors.
// reason: field names must carry their category prefix for clarity even though
// they share the `_names` postfix — renaming would obscure the domain.
#[allow(clippy::struct_field_names)]
#[derive(Default, Debug)]
pub(crate) struct Catalog {
    pub(crate) builtin_rule_names: Vec<&'static str>,
    pub(crate) builtin_theme_names: Vec<&'static str>,
    pub(crate) embedded_profile_names: Vec<&'static str>,
}

/// Live-preview state. `compiled` is the rule set the preview applies;
/// `compile_error` is set when the latest debounced recompile failed.
#[derive(Default, Debug)]
pub(crate) struct PreviewState {
    // reason: populated by v0.5.5+ recompile_preview body when the
    // span-emitting preview pipeline (spec §5.4 DOKUNULMAZ blocker)
    // is unblocked; v0.5.4 ships the scaffold field only.
    #[allow(dead_code)]
    pub(crate) compiled: Option<Arc<crate::rules::Compiled>>,
    pub(crate) compile_error: Option<String>,
    pub(crate) debouncer: crate::config_tui::debounce::Debouncer,
}

/// Session sample input shown in the live-preview strip.
#[derive(Debug)]
pub(crate) struct SampleInput {
    pub(crate) text: String,
}

impl Default for SampleInput {
    fn default() -> Self {
        Self { text: crate::config_tui::render::DEFAULT_PREVIEW_SAMPLE.to_owned() }
    }
}

/// Top-level App state.
pub(crate) struct App {
    pub(crate) snapshot: ConfigSnapshot,
    pub(crate) edits: PendingEdits,
    pub(crate) catalog: Catalog,
    pub(crate) preview: PreviewState,
    pub(crate) tab: Tab,
    pub(crate) focus: TabFocus,
    pub(crate) modal: Option<Modal>,
    pub(crate) sample_input: SampleInput,
    pub(crate) toast: Option<Toast>,
    pub(crate) mini_preview_visible: bool,
    pub(crate) save_diff: Option<crate::config_tui::widgets::save_diff::SaveDiffState>,
    pub(crate) search_filter: Option<String>,
    pub(crate) search_state: Option<crate::config_tui::widgets::search::SearchState>,
    pub(crate) sample_set_state: Option<crate::config_tui::widgets::sample_set::SampleSetState>,
    pub(crate) should_quit: bool,
}

impl App {
    /// Build the `App` from a snapshot. Catalog populated from existing
    /// tayf accessors.
    pub(crate) fn from_snapshot(snapshot: ConfigSnapshot) -> Self {
        let builtin_rule_names: Vec<&'static str> = crate::rules::BUILTIN_NAMES.to_vec();
        let builtin_theme_names: Vec<&'static str> = crate::themes::names().to_vec();
        let embedded_profile_names: Vec<&'static str> =
            crate::profiles::embedded_profile_names().collect();
        Self {
            snapshot,
            edits: PendingEdits::default(),
            catalog: Catalog { builtin_rule_names, builtin_theme_names, embedded_profile_names },
            preview: PreviewState::default(),
            tab: Tab::Patterns,
            focus: TabFocus::default(),
            modal: None,
            sample_input: SampleInput::default(),
            toast: None,
            mini_preview_visible: true,
            save_diff: None,
            search_filter: None,
            search_state: None,
            sample_set_state: None,
            should_quit: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_cycle_forward_and_backward() {
        assert_eq!(Tab::Patterns.next(), Tab::Themes);
        assert_eq!(Tab::Themes.next(), Tab::Profiles);
        assert_eq!(Tab::Profiles.next(), Tab::Status);
        assert_eq!(Tab::Status.next(), Tab::Patterns);
        assert_eq!(Tab::Patterns.prev(), Tab::Status);
    }

    #[test]
    fn tab_from_digit_pins_1_to_4() {
        assert_eq!(Tab::from_digit(1), Some(Tab::Patterns));
        assert_eq!(Tab::from_digit(2), Some(Tab::Themes));
        assert_eq!(Tab::from_digit(3), Some(Tab::Profiles));
        assert_eq!(Tab::from_digit(4), Some(Tab::Status));
        assert_eq!(Tab::from_digit(5), None);
    }

    #[test]
    fn app_init_no_dirty_no_modal_patterns_tab_default() {
        let snap = crate::config_tui::snapshot::ConfigSnapshot::empty();
        let app = App::from_snapshot(snap);
        assert_eq!(app.tab, Tab::Patterns);
        assert!(app.modal.is_none());
        assert!(!app.edits.is_dirty());
        assert!(!app.should_quit);
    }
}
