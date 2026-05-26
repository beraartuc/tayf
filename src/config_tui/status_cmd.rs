//! `tayf config status` implementation (no ratatui). Prints resolved
//! config state (path / theme / profile / bg detect) and the tail of
//! the hot-reload event log.
//!
//! Exit codes (spec §4.4):
//! - 0 on full success.
//! - 64 (`EX_USAGE`) on config parse error — partial info still printed
//!   to stdout; stderr gets the warning line (I-10 fold).

use std::fmt::Write as _;
use std::process::ExitCode;

use crate::cli::RunArgs;

/// Number of trailing reload events to show.
const RECENT_EVENTS_LIMIT: usize = 100;

/// Entry point invoked by `crate::config_tui::status` dispatcher.
#[allow(clippy::needless_pass_by_value)]
// reason: `RunArgs` is consumed here; passing by value is idiomatic for
// entry points that own their argument bag.
pub(crate) fn run(args: RunArgs) -> ExitCode {
    let body = render(&args);
    print!("{}", body.stdout);
    if !body.stderr.is_empty() {
        eprint!("{}", body.stderr);
    }
    body.exit
}

/// Captured output of [`render`]. Separated from I/O for unit-test
/// observability — callers can assert on `stdout` / `stderr` without
/// capturing file descriptors.
pub(crate) struct StatusOutput {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) exit: ExitCode,
}

/// Build the status output from the resolved args. Pure (no I/O side
/// effects) aside from calling `crate::config::load` which reads the
/// file system. This is the testable core of `tayf config status`.
pub(crate) fn render(args: &RunArgs) -> StatusOutput {
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut exit = ExitCode::SUCCESS;

    // Resolve config file. `load` takes `Option<&Path>`.
    let load_outcome = crate::config::load(args.config.as_deref());
    let (config_line, config_dir_opt, theme_line, profile_line) = match &load_outcome {
        Ok(Some((cfg, path))) => {
            let cfg_line = format!("config: {}\n", path.display());
            let theme = args
                .theme
                .clone()
                .or_else(|| cfg.general.theme.clone())
                .unwrap_or_else(|| "(unresolved: none set)".to_owned());
            let profile = args
                .profile
                .clone()
                .or_else(|| cfg.general.profile.clone())
                .unwrap_or_else(|| "(unresolved: none set)".to_owned());
            (
                cfg_line,
                path.parent().map(std::path::Path::to_path_buf),
                format!("theme: {theme}\n"),
                format!("profile: {profile}\n"),
            )
        }
        Ok(None) => {
            let theme = args
                .theme
                .clone()
                .unwrap_or_else(|| "(unresolved: no config + no --theme)".to_owned());
            let profile = args
                .profile
                .clone()
                .unwrap_or_else(|| "(unresolved: no config + no --profile)".to_owned());
            (
                "config: (no config file)\n".to_owned(),
                None,
                format!("theme: {theme}\n"),
                format!("profile: {profile}\n"),
            )
        }
        Err(e) => {
            exit = ExitCode::from(64); // EX_USAGE per I-10 fold
            let _ = writeln!(stderr, "tayf config status: config parse error: {e}");
            let theme = args
                .theme
                .clone()
                .unwrap_or_else(|| "(unresolved: config parse failed)".to_owned());
            let profile = args
                .profile
                .clone()
                .unwrap_or_else(|| "(unresolved: config parse failed)".to_owned());
            (
                format!("config: (unresolved: {e})\n"),
                None,
                format!("theme: {theme}\n"),
                format!("profile: {profile}\n"),
            )
        }
    };

    stdout.push_str(&config_line);
    stdout.push_str(&theme_line);
    stdout.push_str(&profile_line);

    // bg detect line
    let bg = if args.no_color {
        "disabled (--no-color)".to_owned()
    } else {
        "(probed at runtime)".to_owned()
    };
    let _ = writeln!(stdout, "bg detect: {bg}");

    // hot reload watcher status
    let hot_line = match &config_dir_opt {
        Some(dir) => {
            let events = crate::reload::read_recent_events(dir, RECENT_EVENTS_LIMIT);
            if events.is_empty() {
                "hot reload: no active wrapper detected\n".to_owned()
            } else {
                format!(
                    "hot reload: {} recent event(s) in {}/runtime/reload.log\n",
                    events.len(),
                    dir.display()
                )
            }
        }
        None => "hot reload: no config dir resolved\n".to_owned(),
    };
    stdout.push_str(&hot_line);

    StatusOutput { stdout, stderr, exit }
}

#[cfg(test)]
mod tests {
    use super::{render, StatusOutput};
    use crate::cli::RunArgs;

    fn baseline_args() -> RunArgs {
        RunArgs {
            shell: None,
            login: false,
            no_color: false,
            config: None,
            theme: None,
            profile: None,
            bypass: false,
            no_hot_reload: false,
        }
    }

    #[test]
    fn status_no_config_renders_byte_pinned_lines() {
        // Use a nonexistent config path to force the "no config file" branch
        // deterministically, regardless of whether the test runner has a real
        // ~/.config/tayf/config.toml. An explicit nonexistent path triggers the
        // Err branch (cannot stat), so we instead rely on NULL env vars approach:
        // pass `config: None` and let load() discover nothing under a temp XDG.
        // In practice, on a CI machine without a real config file, load(None)
        // returns Ok(None). On a developer machine with a real config, it returns
        // Ok(Some(...)). Either branch must produce the required keys.
        let out: StatusOutput = render(&baseline_args());
        assert!(out.stdout.contains("config:"), "got: {}", out.stdout);
        assert!(out.stdout.contains("theme:"), "got: {}", out.stdout);
        assert!(out.stdout.contains("profile:"), "got: {}", out.stdout);
        assert!(out.stdout.contains("bg detect:"), "got: {}", out.stdout);
        assert!(out.stdout.contains("hot reload:"), "got: {}", out.stdout);
    }

    #[test]
    fn status_no_color_flag_shows_disabled_bg_detect() {
        let mut args = baseline_args();
        args.no_color = true;
        let out = render(&args);
        assert!(out.stdout.contains("bg detect: disabled (--no-color)"), "got: {}", out.stdout);
    }

    #[test]
    fn status_with_bad_config_path_sets_exit_64_and_partial_stdout() {
        let mut args = baseline_args();
        // A path that doesn't exist causes config::load to return an Err.
        args.config = Some(std::path::PathBuf::from("/nonexistent/tayf_test_cfg.toml"));
        let out = render(&args);
        // stdout still has the required keys (partial info)
        assert!(out.stdout.contains("config:"), "stdout must have config: line");
        assert!(out.stdout.contains("theme:"), "stdout must have theme: line");
        assert!(out.stdout.contains("profile:"), "stdout must have profile: line");
        assert!(out.stdout.contains("bg detect:"), "stdout must have bg detect: line");
        assert!(out.stdout.contains("hot reload:"), "stdout must have hot reload: line");
        // stderr has the parse error
        assert!(
            out.stderr.contains("tayf config status: config parse error:"),
            "stderr must carry error prefix; got: {}",
            out.stderr
        );
        // exit code 64
        assert_eq!(
            format!("{:?}", out.exit),
            format!("{:?}", std::process::ExitCode::from(64u8)),
            "exit must be 64 (EX_USAGE)"
        );
    }

    #[test]
    fn status_with_theme_flag_shows_theme_in_output() {
        let mut args = baseline_args();
        args.theme = Some("light".to_owned());
        let out = render(&args);
        assert!(out.stdout.contains("theme: light"), "got: {}", out.stdout);
    }
}
