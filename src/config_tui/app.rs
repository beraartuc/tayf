//! TUI App state — [`Tab`] enum, [`TabFocus`], [`Modal`] stack, [`Catalog`].
//!
//! Single-thread design (spec §6.2). No `Arc<Mutex<App>>`.

use std::time::Instant;

use crate::bg_detect::BgTheme;
use crate::config_tui::chrome::AccentPalette;
use crate::config_tui::edit::{PendingEdits, RuleId};
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
    pub(crate) detail_focused: bool,
}
#[derive(Default, Debug)]
pub(crate) struct ThemesFocus {
    pub(crate) selected_idx: usize,
    pub(crate) detail_focused: bool,
}
#[derive(Default, Debug)]
pub(crate) struct ProfilesFocus {
    pub(crate) selected_idx: usize,
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
    Confirm {
        msg: String,
        action: ConfirmAction,
    },
    Error(String),
    QuitWithUnsavedEdits,
    ColorPicker(crate::config_tui::widgets::color_picker::ColorPickerState),
    SaveDiff,
    FullPreview,
    Search,
    SampleSet,
    NewPattern {
        phase: NewPatternPhase,
        draft: PatternDraft,
    },
    /// Inline regex-source editor (spec §12.4 D3). `e` keystroke on the
    /// Patterns tab; buffer initialized via `events::pattern_for_rule_id`.
    /// Enter commits to `edits.rules[rule_id].pattern`; Esc cancels.
    EditRegex {
        rule_id: crate::config_tui::edit::RuleId,
        buffer: String,
        error: Option<String>,
    },
    /// Read-only keybinding cheat-sheet overlay (spec §12.4 D4). Opened
    /// by `?` or F1; any key dismisses + key is discarded (vim/less
    /// convention). Content lives in `events::HELP_MODAL_CONTENT`.
    Help,
    /// G8 (§3.6): per-key conflict resolution surface opened when
    /// [`crate::config_tui::widgets::save_diff::build_initial_state`]
    /// produces a `SaveDiffState::MergePending`. Keymap:
    /// `j`/`k` navigate rows; `o`/`t`/`s` toggle the focused row's
    /// pick (Block-shaped rows can only Skip); Enter applies all
    /// selections via `commit_bytes`; Esc cancels and resets
    /// `pending_save_and_quit`.
    ConflictList(crate::config_tui::widgets::conflict_list::ConflictListState),
}

/// 3-phase wizard state for the `n` keystroke new-pattern modal
/// (spec §12.4 D2). Each phase owns a distinct input surface:
/// `Name` and `Regex` write to the draft text buffers; `Style`
/// delegates to the embedded `ColorPickerState`.
#[derive(Debug)]
pub(crate) enum NewPatternPhase {
    Name,
    Regex,
    Style,
}

/// Mutable draft accumulator for `Modal::NewPattern`. Lives inside
/// the modal variant so Esc back-paths preserve in-progress input
/// (TUI reviewer I4 fold — phase-aware back-out without data loss).
#[derive(Debug)]
pub(crate) struct PatternDraft {
    pub(crate) name: String,
    pub(crate) pattern: String,
    pub(crate) pattern_error: Option<String>,
    pub(crate) picker_state: crate::config_tui::widgets::color_picker::ColorPickerState,
    pub(crate) draft_style: crate::config_tui::edit::NewStyle,
}

impl PatternDraft {
    pub(crate) fn new() -> Self {
        Self {
            name: String::new(),
            pattern: String::new(),
            pattern_error: None,
            picker_state: crate::config_tui::widgets::color_picker::ColorPickerState::default(),
            draft_style: crate::config_tui::edit::NewStyle::default(),
        }
    }
}

#[derive(Debug)]
pub(crate) enum ConfirmAction {
    /// "Discard edits and reload" path — opened by `Ctrl+R` when
    /// `app.edits.is_dirty()`. Reloads the snapshot from disk and
    /// clears staged edits. Spec v0.6.1 §3.3.
    DiscardEditsAndReload,
    /// Delete a rule by its full `RuleId` (R-I3 + T-I8: payload widens
    /// `String` → `RuleId`). Wired from the `'d'` / `Delete` keystroke on
    /// the Patterns tab; confirmed by `apply_confirm` which inserts into
    /// `PendingEdits::deleted`. Spec v0.6.2 §3.3.
    DeleteRule(RuleId),
    /// Reset all staged overrides (style + regex + delete) for a rule.
    /// Symmetric with `DeleteRule`. Spec v0.6.2 §3.3.
    ResetOverride(RuleId),
    /// First-run init-from-dump path — opened by `Shift+D` when the
    /// bound config path does not yet exist. Writes the built-in
    /// default config and reloads the snapshot. Spec v0.6.1 §3.3.
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
pub(crate) struct PreviewState {
    pub(crate) compiled: std::sync::Arc<arc_swap::ArcSwap<crate::rules::Compiled>>,
    pub(crate) compile_error: Option<String>,
    pub(crate) debouncer: crate::config_tui::debounce::Debouncer,
    pub(crate) runs: Vec<Vec<crate::pipeline::StyleSpan>>,
    pub(crate) scratch: crate::pipeline::PipelineScratch,
}

impl std::fmt::Debug for PreviewState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewState")
            .field("compiled", &"<ArcSwap<Compiled>>")
            .field("compile_error", &self.compile_error)
            .field("debouncer", &self.debouncer)
            .field("runs", &self.runs.len())
            .field("scratch", &"<PipelineScratch>")
            .finish()
    }
}

impl PreviewState {
    pub(crate) fn new(compiled: crate::rules::Compiled) -> Self {
        Self {
            compiled: std::sync::Arc::new(arc_swap::ArcSwap::from_pointee(compiled)),
            compile_error: None,
            debouncer: crate::config_tui::debounce::Debouncer::default(),
            runs: Vec::new(),
            scratch: crate::pipeline::PipelineScratch::default(),
        }
    }

    /// Run `apply_rules_spans` across all sample lines, populating `runs`.
    pub(crate) fn recompile(&mut self, sample_text: &str) {
        self.runs.clear();
        for line in sample_text.lines() {
            let spans = crate::pipeline::apply_rules_spans(
                line.as_bytes(),
                &self.compiled,
                &mut self.scratch,
            );
            self.runs.push(spans);
        }
    }
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

/// Terminal environment snapshot, resolved ONCE at TUI startup (before raw
/// mode) and threaded into [`App`]. Carries the background polarity, the
/// derived chrome accent, and the terminal color depth. Powers preview
/// fidelity (§5) and chrome color (§7).
#[derive(Debug, Clone, Copy)]
pub(crate) struct TuiEnv {
    pub(crate) bg: BgTheme,
    pub(crate) accent: AccentPalette,
    /// Detected terminal color depth. The live preview compiles at THIS depth
    /// (not always Truecolor) so its colors match what real `tayf` renders on
    /// the same terminal — at 256/16 depth the runtime downsamples, so the
    /// preview must too. Manual-review fidelity finding.
    pub(crate) depth: crate::terminfo::ColorDepth,
}

impl TuiEnv {
    /// Resolve from the real terminal. MUST be called before raw mode / alt
    /// screen — `bg_detect::resolve()` manages its own `/dev/tty` termios.
    pub(crate) fn resolve() -> Self {
        let bg = crate::bg_detect::resolve();
        Self { bg, accent: AccentPalette::from_bg(bg), depth: crate::terminfo::detect_depth() }
    }

    /// Deterministic env (always Dark, fixed Truecolor depth, no `/dev/tty`
    /// I/O). Used by snapshot/unit tests AND by `crate::__test_api`
    /// integration-test boot helpers.
    pub(crate) fn deterministic() -> Self {
        let bg = BgTheme::Dark;
        Self {
            bg,
            accent: AccentPalette::from_bg(bg),
            depth: crate::terminfo::ColorDepth::Truecolor,
        }
    }
}

/// Top-level App state.
// reason: `mini_preview_visible`, `should_quit`, `pending_save_and_quit`,
// and `needs_redraw` are four distinct single-bit sentinel flags; a state
// machine would add indirection with no clarity gain in a single-threaded TUI.
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct App {
    pub(crate) snapshot: ConfigSnapshot,
    pub(crate) edits: PendingEdits,
    pub(crate) catalog: Catalog,
    pub(crate) preview: PreviewState,
    pub(crate) tui_env: TuiEnv,
    pub(crate) tab: Tab,
    pub(crate) focus: TabFocus,
    pub(crate) modal: Option<Modal>,
    pub(crate) sample_input: SampleInput,
    pub(crate) toast: Option<Toast>,
    pub(crate) mini_preview_visible: bool,
    pub(crate) save_diff: Option<crate::config_tui::widgets::save_diff::SaveDiffState>,
    /// Vertical scroll offset for the save-diff modal `Paragraph` render.
    /// Reset to 0 when the modal closes (Commit ok / `CloseModal` /
    /// Esc tier-2). v0.6.1 §3.7.
    pub(crate) save_diff_scroll: u16,
    pub(crate) search_filter: Option<String>,
    pub(crate) search_state: Option<crate::config_tui::widgets::search::SearchState>,
    pub(crate) sample_set_state: Option<crate::config_tui::widgets::sample_set::SampleSetState>,
    pub(crate) should_quit: bool,
    /// Set when user invokes save-and-quit via the `QuitWithUnsavedEdits`
    /// modal's `s` option. **Invariant**: MUST be cleared on every
    /// `SaveDiff` exit path that does NOT set `should_quit`. Enforced by
    /// `test_pending_save_and_quit_resets_on_every_non_commit_exit`.
    pub(crate) pending_save_and_quit: bool,
    /// Dirty flag for the event loop: draw only when set (spec §8). Start
    /// `true` so the first frame renders. Set on key/resize/state change.
    pub(crate) needs_redraw: bool,
}

impl App {
    /// Build the `App` from a snapshot. Catalog populated from existing
    /// tayf accessors.
    pub(crate) fn from_snapshot(snapshot: ConfigSnapshot, tui_env: TuiEnv) -> Self {
        let builtin_rule_names: Vec<&'static str> = crate::rules::BUILTIN_NAMES.to_vec();
        let builtin_theme_names: Vec<&'static str> = crate::themes::names().to_vec();
        let embedded_profile_names: Vec<&'static str> =
            crate::profiles::embedded_profile_names().collect();

        let config_theme = snapshot.parsed.theme.as_deref();
        let profile = snapshot.parsed.profile.as_deref();
        // Mirror the runtime theme precedence so the preview matches real tayf
        // (spec §5): config theme > profile.theme > bg-detect default.
        let effective_theme = crate::config_tui::theme_resolve::resolve_from_snapshot(
            config_theme,
            profile,
            tui_env.bg,
        );
        let synth_config = crate::config::Config {
            general: snapshot.parsed.general.clone(),
            rules: snapshot.parsed.rules.clone(),
        };
        let (compiled, compile_error) = match crate::rules::compile_from_config(
            &synth_config,
            Some(effective_theme.as_str()),
            profile,
            tui_env.depth,
        ) {
            Ok(c) => (c, None),
            Err(e) => (crate::rules::Compiled::empty(), Some(e.to_string())),
        };
        let mut preview = PreviewState::new(compiled);
        preview.compile_error = compile_error;

        let mut app = Self {
            snapshot,
            edits: PendingEdits::default(),
            catalog: Catalog { builtin_rule_names, builtin_theme_names, embedded_profile_names },
            preview,
            tui_env,
            tab: Tab::Patterns,
            focus: TabFocus::default(),
            modal: None,
            sample_input: SampleInput::default(),
            toast: None,
            mini_preview_visible: true,
            save_diff: None,
            save_diff_scroll: 0,
            search_filter: None,
            search_state: None,
            sample_set_state: None,
            should_quit: false,
            pending_save_and_quit: false,
            needs_redraw: true,
        };
        app.preview.recompile(&app.sample_input.text);
        app
    }

    /// Deterministic test constructor — empty config, defaults across the
    /// board, no modal. Used by render snapshot tests (spec §6.7).
    #[cfg(test)]
    #[must_use]
    pub(crate) fn default_for_test() -> Self {
        let snapshot = crate::config_tui::snapshot::ConfigSnapshot::empty();
        Self::from_snapshot(snapshot, TuiEnv::deterministic())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// fg color of the preview span that starts at `needle` on sample line
    /// `line_idx`, or None.
    // Assumes the matched rule produces one span starting exactly at `needle`'s
    // byte offset (true for the builtin ipv4 full-address rule).
    fn preview_fg(app: &App, line_idx: usize, needle: &str) -> Option<crate::style::Color> {
        let line = app.sample_input.text.lines().nth(line_idx)?;
        let start = line.find(needle)?;
        app.preview.runs.get(line_idx)?.iter().find(|s| s.start == start).and_then(|s| s.style.fg)
    }

    /// Runtime reference: the fg the real `tayf` pipeline assigns to the ipv4
    /// span of the default sample's first line under an explicit theme. Read
    /// LIVE from the compiler — never a hardcoded hex — so it survives default
    /// palette re-tones (e.g. commit 06c11c8 ipv4 → #33c7ff).
    fn runtime_ipv4_fg(theme: &str) -> Option<crate::style::Color> {
        let cfg = crate::config::Config {
            general: crate::config::GeneralSection::default(),
            rules: Vec::new(),
        };
        // Truecolor here mirrors `app_with_bg`'s deterministic depth, so this
        // oracle isolates THEME resolution (depth fidelity is covered separately).
        let compiled = crate::rules::compile_from_config(
            &cfg,
            Some(theme),
            None,
            crate::terminfo::ColorDepth::Truecolor,
        )
        .expect("compile theme");
        let handle = arc_swap::ArcSwap::from_pointee(compiled);
        let mut scratch = crate::pipeline::PipelineScratch::default();
        let line =
            crate::config_tui::render::DEFAULT_PREVIEW_SAMPLE.lines().next().expect("line 0");
        let spans = crate::pipeline::apply_rules_spans(line.as_bytes(), &handle, &mut scratch);
        let start = line.find("192.168.1.42").expect("ip in sample");
        spans.iter().find(|s| s.start == start).and_then(|s| s.style.fg)
    }

    fn app_with_bg(bg: crate::bg_detect::BgTheme) -> App {
        let env = TuiEnv {
            bg,
            accent: crate::config_tui::chrome::AccentPalette::from_bg(bg),
            depth: crate::terminfo::ColorDepth::Truecolor,
        };
        App::from_snapshot(crate::config_tui::snapshot::ConfigSnapshot::empty(), env)
    }

    #[test]
    fn preview_compiles_at_the_tui_color_depth_not_always_truecolor() {
        // Manual-review fidelity: the preview must downsample to the terminal's
        // detected depth so it matches `tayf cat` on the same terminal. At
        // Indexed256 the ipv4 fg must be a downsampled `Indexed`, not full Rgb
        // (the previous hardcoded-Truecolor preview showed Rgb → mismatch).
        let env = TuiEnv {
            bg: crate::bg_detect::BgTheme::Dark,
            accent: crate::config_tui::chrome::AccentPalette::from_bg(
                crate::bg_detect::BgTheme::Dark,
            ),
            depth: crate::terminfo::ColorDepth::Indexed256,
        };
        let app = App::from_snapshot(crate::config_tui::snapshot::ConfigSnapshot::empty(), env);
        let fg = preview_fg(&app, 0, "192.168.1.42");
        assert!(
            matches!(fg, Some(crate::style::Color::Indexed(_))),
            "preview ipv4 must downsample to Indexed at 256-color depth, got {fg:?}"
        );
    }

    #[test]
    fn preview_matches_runtime_under_light_bg() {
        // Spec §5.2 oracle: on a light terminal the preview must EQUAL the
        // runtime's light-theme colorization, and must DIFFER from the dark
        // built-ins (proving the theme is actually applied). Palette-agnostic —
        // no hardcoded hex, survives default re-tones.
        let app = app_with_bg(crate::bg_detect::BgTheme::Light);
        assert_eq!(
            preview_fg(&app, 0, "192.168.1.42"),
            runtime_ipv4_fg("light"),
            "light-bg preview ipv4 must equal the runtime light-theme color"
        );
        assert_ne!(
            preview_fg(&app, 0, "192.168.1.42"),
            runtime_ipv4_fg("dark"),
            "light-bg preview must NOT fall back to the dark built-ins (the bug)"
        );
    }

    #[test]
    fn preview_matches_runtime_under_dark_bg() {
        // Dark path stays equal to the runtime dark theme (== built-ins): guards
        // that the fidelity fix does not regress the dark terminal.
        let app = app_with_bg(crate::bg_detect::BgTheme::Dark);
        let dark_runtime = runtime_ipv4_fg("dark");
        assert!(dark_runtime.is_some(), "dark theme must assign a color to the ipv4 span");
        assert_eq!(
            preview_fg(&app, 0, "192.168.1.42"),
            dark_runtime,
            "dark-bg preview ipv4 equals the runtime dark-theme color"
        );
    }

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
        let app = App::from_snapshot(snap, TuiEnv::deterministic());
        assert_eq!(app.tab, Tab::Patterns);
        assert!(app.modal.is_none());
        assert!(!app.edits.is_dirty());
        assert!(!app.should_quit);
    }
}
