//! Profiles tab — default + disk profile management (spec §6.1).
//!
//! Lists the `default` profile (`config.toml`) plus every disk profile under
//! `~/.config/tayf/profiles/*.toml`. Keymap (Enter = focus detail, uniform
//! spec §12.3):
//!   - `Space` — set the selected profile active. `default` clears
//!     `[general] profile`; a named profile sets it. Staged into
//!     `edits.general.profile`; `Ctrl+S` persists.
//!   - `n` — create a new profile (name prompt; clone the active rule set or
//!     start empty). Opens [`crate::config_tui::app::Modal::NewProfile`].
//!   - `d` — delete the selected disk profile (confirm modal). The synthetic
//!     `default` profile cannot be deleted.
//!
//! In-TUI editing of a named profile's rules is deferred (spec §6.3): all
//! rule edits target `config.toml` (the default profile). When a named
//! profile is active the Detail pane shows an affordance to edit its file
//! directly.

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::config_tui::app::{App, ConfirmAction, Modal};

/// Synthetic list entry standing in for `config.toml`'s rule set — the
/// implicit profile that is active when `[general] profile` is unset.
pub(crate) const DEFAULT_PROFILE_LABEL: &str = "default";

/// The full Profiles-tab list: the synthetic `default` entry followed by the
/// disk-profile stems under `~/.config/tayf/profiles/`. Computed fresh on
/// every render/dispatch so an in-session create/delete is reflected at once.
/// A missing root or `profiles/` dir yields just `["default"]`.
pub(crate) fn list_profile_names(_app: &App) -> Vec<String> {
    let mut names = vec![DEFAULT_PROFILE_LABEL.to_owned()];
    if let Some(root) = crate::config_tui::save::tayf_config_root() {
        names.extend(crate::profiles::list_names_with_root(&root));
    }
    names
}

/// The effective active profile name: the staged edit wins over the snapshot.
/// `None` (no profile) maps to the synthetic [`DEFAULT_PROFILE_LABEL`].
fn active_profile_label(app: &App) -> String {
    let active = match &app.edits.general.profile {
        Some(staged) => staged.clone(),
        None => app.snapshot.parsed.profile.clone(),
    };
    active.unwrap_or_else(|| DEFAULT_PROFILE_LABEL.to_owned())
}

pub(crate) fn render(frame: &mut Frame, area: Rect, app: &App) {
    let chunks =
        Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).split(area);
    render_list(frame, chunks[0], app);
    render_detail(frame, chunks[1], app);
}

fn filtered_names(app: &App) -> Vec<String> {
    let filter = app.search_filter.as_deref().unwrap_or("").to_lowercase();
    list_profile_names(app)
        .into_iter()
        .filter(|n| filter.is_empty() || n.to_lowercase().contains(&filter))
        .collect()
}

fn render_list(frame: &mut Frame, area: Rect, app: &App) {
    let active = active_profile_label(app);
    let names = filtered_names(app);
    let items: Vec<ListItem> = names
        .iter()
        .map(|name| {
            let marker = if *name == active { "● " } else { "  " };
            ListItem::new(format!("{marker}{name}"))
        })
        .collect();
    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(app.focus.profiles.selected_idx.min(items.len() - 1)));
    }
    let accent = app.tui_env.accent;
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(accent.border())
                .title("Profiles")
                .title_style(accent.header()),
        )
        .highlight_style(accent.selection())
        .highlight_symbol("▶ ");
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_detail(frame: &mut Frame, area: Rect, app: &App) {
    let names = filtered_names(app);
    let selected = names.get(app.focus.profiles.selected_idx).cloned().unwrap_or_default();
    let active = active_profile_label(app);

    let body = if selected.is_empty() {
        "(no profile selected)".to_owned()
    } else if selected == DEFAULT_PROFILE_LABEL {
        "Profile: default (config.toml)\n\nThe default profile is config.toml's rule set — \
         the rules you edit on the Patterns tab.\n\nPress Space to make it active\nPress 'n' \
         to create a new profile"
            .to_owned()
    } else {
        format!(
            "Profile: {selected}\n\nSource: ~/.config/tayf/profiles/{selected}.toml\n\n\
             Press Space to set as active\nPress 'd' to delete this profile\nPress 'n' to \
             create a new profile"
        )
    };

    // Affordance (spec §6.3): when a NAMED profile is active, the TUI's rule
    // edits still target the default profile (config.toml). Editing the named
    // profile's rules in-TUI is deferred — the user edits its file directly.
    let body = if active == DEFAULT_PROFILE_LABEL {
        body
    } else {
        format!(
            "{body}\n\nActive profile: {active} — edit its file directly; rule edits here \
             target the default profile (config.toml)."
        )
    };

    let accent = app.tui_env.accent;
    frame.render_widget(
        Paragraph::new(body).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(accent.border())
                .title("Detail")
                .title_style(accent.header()),
        ),
        area,
    );
}

pub(crate) fn dispatch_key(app: &mut App, k: KeyEvent) {
    let names = filtered_names(app);
    let len = names.len();
    match k.code {
        KeyCode::Char('j') | KeyCode::Down => {
            app.focus.profiles.selected_idx =
                (app.focus.profiles.selected_idx + 1).min(len.saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.focus.profiles.selected_idx = app.focus.profiles.selected_idx.saturating_sub(1);
        }
        KeyCode::Char('g') => app.focus.profiles.selected_idx = 0,
        KeyCode::Char('G') => app.focus.profiles.selected_idx = len.saturating_sub(1),
        KeyCode::Enter => app.focus.profiles.detail_focused = true,
        KeyCode::Char(' ') => {
            let Some(name) = names.get(app.focus.profiles.selected_idx) else {
                return;
            };
            if name == DEFAULT_PROFILE_LABEL {
                // Selecting the default clears `[general] profile`.
                app.edits.general.profile = Some(None);
                app.toast = Some(crate::config_tui::app::Toast::ok(
                    "staged default profile (config.toml); Ctrl+S to save".to_owned(),
                ));
            } else {
                app.edits.general.profile = Some(Some(name.clone()));
                app.toast = Some(crate::config_tui::app::Toast::ok(format!(
                    "staged profile = {name}; Ctrl+S to save"
                )));
            }
        }
        KeyCode::Char('n') if app.modal.is_none() => {
            app.modal =
                Some(Modal::NewProfile { buffer: String::new(), clone_rules: true, error: None });
        }
        KeyCode::Char('d') => {
            let Some(name) = names.get(app.focus.profiles.selected_idx) else {
                return;
            };
            if name == DEFAULT_PROFILE_LABEL {
                app.toast = Some(crate::config_tui::app::Toast::warn(
                    "The default profile (config.toml) cannot be deleted".to_owned(),
                ));
                return;
            }
            if app.modal.is_none() {
                app.modal = Some(Modal::Confirm {
                    msg: format!("Delete profile '{name}' from disk?"),
                    action: ConfirmAction::DeleteProfile(name.clone()),
                });
            }
        }
        _ => {}
    }
}

/// Serialize + save/restore `XDG_CONFIG_HOME` for tests that need a
/// deterministic, empty config root (the Profiles list reads it live).
/// Returns an RAII guard that restores the prior value on drop. Holds a
/// process-wide mutex for its lifetime so parallel lib tests do not race the
/// shared env key. Dependency-free; only used in-crate by render-snapshot
/// + dispatch tests.
#[cfg(test)]
pub(crate) fn scoped_empty_config_root(dir: &std::path::Path) -> EnvVarGuard {
    EnvVarGuard::set("XDG_CONFIG_HOME", dir.as_os_str())
}

/// RAII guard restoring an env var to its prior value on drop. See
/// [`scoped_empty_config_root`].
#[cfg(test)]
pub(crate) struct EnvVarGuard {
    key: &'static str,
    prior: Option<std::ffi::OsString>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl EnvVarGuard {
    fn set(key: &'static str, value: &std::ffi::OsStr) -> Self {
        static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let lock = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let prior = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, prior, _lock: lock }
    }
}

#[cfg(test)]
impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.prior {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_tui::app::{App, TuiEnv};
    use crate::config_tui::snapshot::ConfigSnapshot;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    fn render_to_string(app: &App) -> String {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("backend");
        terminal.draw(|f| render(f, Rect::new(0, 0, 80, 24), app)).expect("draw");
        let buf = terminal.backend().buffer();
        let area = buf.area;
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    /// Build a deterministic Profiles-tab app rooted at `config_root` (so
    /// the disk listing is scoped). Returns the app plus the env guard the
    /// caller must keep alive for the test's duration.
    fn app_on_profiles_tab(config_root: &std::path::Path) -> (App, EnvVarGuard) {
        let guard = scoped_empty_config_root(config_root);
        let mut app = App::from_snapshot(ConfigSnapshot::empty(), TuiEnv::deterministic());
        app.tab = crate::config_tui::app::Tab::Profiles;
        (app, guard)
    }

    fn write_disk_profile(config_root: &std::path::Path, name: &str, body: &str) {
        let dir = config_root.join("tayf").join("profiles");
        std::fs::create_dir_all(&dir).expect("mkdir profiles");
        std::fs::write(dir.join(format!("{name}.toml")), body).expect("write profile");
    }

    #[test]
    fn list_contains_default_first_then_disk_profiles() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        write_disk_profile(tmp.path(), "work", "");
        let (app, _g) = app_on_profiles_tab(tmp.path());
        let names = list_profile_names(&app);
        assert_eq!(names.first().map(String::as_str), Some("default"), "default is first");
        assert!(names.iter().any(|n| n == "work"), "disk profile listed; got {names:?}");
    }

    #[test]
    fn space_on_disk_profile_stages_general_profile() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        write_disk_profile(tmp.path(), "work", "");
        let (mut app, _g) = app_on_profiles_tab(tmp.path());
        let idx = list_profile_names(&app).iter().position(|n| n == "work").expect("work listed");
        app.focus.profiles.selected_idx = idx;
        dispatch_key(&mut app, key(KeyCode::Char(' ')));
        assert_eq!(
            app.edits.general.profile,
            Some(Some("work".to_owned())),
            "Space on a named profile stages [general] profile = work"
        );
    }

    #[test]
    fn space_on_default_clears_general_profile() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let (mut app, _g) = app_on_profiles_tab(tmp.path());
        app.edits.general.profile = Some(Some("work".to_owned()));
        app.focus.profiles.selected_idx = 0; // default
        dispatch_key(&mut app, key(KeyCode::Char(' ')));
        assert_eq!(
            app.edits.general.profile,
            Some(None),
            "Space on default clears the active-profile pointer"
        );
    }

    // ---- Task 5.2: create + delete ------------------------------------

    #[test]
    fn n_opens_new_profile_modal() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let (mut app, _g) = app_on_profiles_tab(tmp.path());
        dispatch_key(&mut app, key(KeyCode::Char('n')));
        assert!(
            matches!(app.modal, Some(Modal::NewProfile { clone_rules: true, .. })),
            "n opens the NewProfile name prompt (clone default)"
        );
    }

    #[test]
    fn create_empty_profile_writes_parseable_file() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let (mut app, _g) = app_on_profiles_tab(tmp.path());
        // Open modal, toggle to empty, type a name, commit.
        dispatch_key(&mut app, key(KeyCode::Char('n')));
        crate::config_tui::events::dispatch_key(&mut app, key(KeyCode::Tab)); // → empty
        for c in "staging".chars() {
            crate::config_tui::events::dispatch_key(&mut app, key(KeyCode::Char(c)));
        }
        crate::config_tui::events::dispatch_key(&mut app, key(KeyCode::Enter));
        assert!(app.modal.is_none(), "modal closes on successful create");
        let path = tmp.path().join("tayf").join("profiles").join("staging.toml");
        assert!(path.exists(), "profiles/staging.toml written");
        let body = std::fs::read_to_string(&path).expect("read");
        let parsed: crate::profiles::Profile =
            toml::from_str(&body).expect("created empty profile parses");
        assert!(parsed.rules.is_empty(), "empty profile has no rules");
        assert!(list_profile_names(&app).iter().any(|n| n == "staging"));
    }

    #[test]
    fn create_clone_profile_serializes_active_rules() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let guard = scoped_empty_config_root(tmp.path());
        // Snapshot with a user rule so the clone has something to serialize.
        let mut snap = ConfigSnapshot::empty();
        snap.parsed.rules.push(crate::config::UserRule {
            name: "ticket".to_owned(),
            pattern: Some(r"JIRA-\d+".to_owned()),
            style: None,
            enabled: true,
            styles: None,
            priority: None,
        });
        let mut app = App::from_snapshot(snap, TuiEnv::deterministic());
        app.tab = crate::config_tui::app::Tab::Profiles;
        dispatch_key(&mut app, key(KeyCode::Char('n'))); // clone_rules = true by default
        for c in "mine".chars() {
            crate::config_tui::events::dispatch_key(&mut app, key(KeyCode::Char(c)));
        }
        crate::config_tui::events::dispatch_key(&mut app, key(KeyCode::Enter));
        let path = tmp.path().join("tayf").join("profiles").join("mine.toml");
        let parsed: crate::profiles::Profile =
            toml::from_str(&std::fs::read_to_string(&path).expect("read")).expect("clone parses");
        assert_eq!(parsed.rules.len(), 1, "cloned the active rule set");
        assert_eq!(parsed.rules[0].name, "ticket");
        assert_eq!(parsed.rules[0].pattern.as_deref(), Some(r"JIRA-\d+"));
        drop(guard);
    }

    #[test]
    fn create_duplicate_name_keeps_modal_open_with_error() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        write_disk_profile(tmp.path(), "work", "");
        let (mut app, _g) = app_on_profiles_tab(tmp.path());
        dispatch_key(&mut app, key(KeyCode::Char('n')));
        for c in "work".chars() {
            crate::config_tui::events::dispatch_key(&mut app, key(KeyCode::Char(c)));
        }
        crate::config_tui::events::dispatch_key(&mut app, key(KeyCode::Enter));
        assert!(
            matches!(app.modal, Some(Modal::NewProfile { error: Some(_), .. })),
            "duplicate name keeps the modal open with an error"
        );
    }

    #[test]
    fn d_on_disk_profile_opens_confirm_then_deletes() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        write_disk_profile(tmp.path(), "work", "");
        let (mut app, _g) = app_on_profiles_tab(tmp.path());
        let idx = list_profile_names(&app).iter().position(|n| n == "work").expect("work listed");
        app.focus.profiles.selected_idx = idx;
        dispatch_key(&mut app, key(KeyCode::Char('d')));
        assert!(
            matches!(
                &app.modal,
                Some(Modal::Confirm { action: ConfirmAction::DeleteProfile(n), .. }) if n == "work"
            ),
            "d opens a delete-confirm for the selected disk profile"
        );
        crate::config_tui::events::dispatch_key(&mut app, key(KeyCode::Char('y')));
        let path = tmp.path().join("tayf").join("profiles").join("work.toml");
        assert!(!path.exists(), "confirmed delete removes the profile file");
    }

    #[test]
    fn d_on_default_refuses_and_warns() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let (mut app, _g) = app_on_profiles_tab(tmp.path());
        app.focus.profiles.selected_idx = 0; // default
        dispatch_key(&mut app, key(KeyCode::Char('d')));
        assert!(app.modal.is_none(), "default delete opens no confirm modal");
        assert_eq!(
            app.toast.as_ref().map(|t| t.text.as_str()),
            Some("The default profile (config.toml) cannot be deleted"),
            "default delete warns instead"
        );
    }

    // ---- Task 5.3: affordance when a named profile is active ----------

    #[test]
    fn affordance_shows_when_named_profile_active() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        write_disk_profile(tmp.path(), "work", "");
        let (mut app, _g) = app_on_profiles_tab(tmp.path());
        app.edits.general.profile = Some(Some("work".to_owned()));
        let rendered = render_to_string(&app);
        assert!(
            rendered.contains("edit its file directly"),
            "named-active affordance must mention editing the file directly; got:\n{rendered}"
        );
    }

    #[test]
    fn no_affordance_when_default_active() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let (app, _g) = app_on_profiles_tab(tmp.path());
        let rendered = render_to_string(&app);
        assert!(
            !rendered.contains("edit its file directly"),
            "default-active must not show the named-profile affordance; got:\n{rendered}"
        );
    }
}
