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
// reason: fields wired by C3 per-tab dispatch; dead until C3 lands.
#[allow(dead_code)]
#[derive(Default, Debug)]
pub(crate) struct TabFocus {
    pub(crate) patterns: PatternsFocus,
    pub(crate) themes: ThemesFocus,
    pub(crate) profiles: ProfilesFocus,
    pub(crate) status: StatusFocus,
}

// reason: fields wired by C3 patterns tab; dead until C3 lands.
#[allow(dead_code)]
#[derive(Default, Debug)]
pub(crate) struct PatternsFocus {
    pub(crate) selected_idx: usize,
    pub(crate) detail_focused: bool,
}
// reason: fields wired by C3 themes tab; dead until C3 lands.
#[allow(dead_code)]
#[derive(Default, Debug)]
pub(crate) struct ThemesFocus {
    pub(crate) selected_idx: usize,
    pub(crate) detail_focused: bool,
}
// reason: fields wired by C3 profiles tab; dead until C3 lands.
#[allow(dead_code)]
#[derive(Default, Debug)]
pub(crate) struct ProfilesFocus {
    pub(crate) selected_idx: usize,
    pub(crate) detail_focused: bool,
}
// reason: field wired by C3 status tab; dead until C3 lands.
#[allow(dead_code)]
#[derive(Default, Debug)]
pub(crate) struct StatusFocus {
    pub(crate) scroll: usize,
}

/// Modal overlay. `App.modal` is `Option<Modal>` — no stacking (spec §7.2);
/// every modal-opening code path is guarded by `modal.is_none()`.
/// Detailed state types land in C4 (widgets/* modules) — C2a holds
/// only the tag variants needed for key dispatch.
// reason: Confirm and Error variants wired by C3/C4; ColorPicker/SaveDiff/Search/SampleSet
// land in C4; tag variants declared here now so dispatch_key can pattern-match them.
#[allow(dead_code)]
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

// reason: ConfirmAction variants constructed by C3/C4 action handlers; dead until those land.
#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum ConfirmAction {
    DiscardEditsAndReload,
    DeleteUserRule(String),
    ResetUserOverride(String),
    InitFromDump,
}

/// Toast — auto-dismiss after 3 s.
// reason: text and kind read by C2b render.rs; dead until C2b lands.
#[allow(dead_code)]
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
/// (C3 fills detail; C2a holds the type only.)
// reason: field names must carry their category prefix for clarity even though
// they share the `_names` postfix — renaming would obscure the domain.
#[allow(clippy::struct_field_names)]
// reason: catalog fields read by C3 tab dispatch; dead until C3 lands.
#[allow(dead_code)]
#[derive(Default, Debug)]
pub(crate) struct Catalog {
    pub(crate) builtin_rule_names: Vec<&'static str>,
    pub(crate) builtin_theme_names: Vec<&'static str>,
    pub(crate) embedded_profile_names: Vec<&'static str>,
}

/// Live-preview state (C4 wires real recompile).
// reason: fields wired by C4 preview recompile; dead until C4 lands.
#[allow(dead_code)]
#[derive(Default, Debug)]
pub(crate) struct PreviewState {
    pub(crate) compiled: Option<Arc<crate::rules::Compiled>>,
    pub(crate) compile_error: Option<String>,
    pub(crate) debounce_pending: bool,
}

/// Session sample input shown in the live-preview strip.
// reason: text field read by C2b render.rs; dead until C2b lands.
#[allow(dead_code)]
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
// reason: scaffold fields consumed by C2b/C3/C4; dead until those tasks land.
#[allow(dead_code)]
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
