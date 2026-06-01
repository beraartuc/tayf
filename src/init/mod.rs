//! `tayf init` — first-run setup.
//!
//! Creates the default config file and installs the always-on shell-rc
//! guard so tayf runs automatically in new terminals. Pure shell logic
//! lives in [`shell_hook`] (env/paths injected for testability); this
//! module owns the orchestration and the real-env reads.
//!
//! Public API:
//! - `run` — entry point dispatched from `main.rs` for `Cmd::Init` (added
//!   in the orchestration task).

pub(crate) mod shell_hook;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use crate::cli::InitArgs;
use shell_hook::Shell;

/// Result of the config-file creation step (for reporting).
pub(crate) enum ConfigStep {
    Created,
    AlreadyExists,
}

/// Create the default config at `target` unless it exists and `force` is
/// false. Reuses the shared `default_config_toml` generator and the atomic
/// writer (creates parent dirs).
pub(crate) fn run_config_step(target: &Path, force: bool) -> std::io::Result<ConfigStep> {
    if target.exists() && !force {
        return Ok(ConfigStep::AlreadyExists);
    }
    crate::config_tui::save::write_atomic_to(target, &crate::config::default_config_toml())?;
    Ok(ConfigStep::Created)
}

/// Resolve the config target: explicit `--config`, else `<base>/config.toml`.
fn resolve_config_target(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return Some(p.to_path_buf());
    }
    crate::config::config_base(
        || std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        || std::env::var_os("HOME").map(PathBuf::from),
    )
    .map(|b| b.join("config.toml"))
}

/// Print the rc snippet + a manual instruction (fish/other shells, or when
/// no rc path resolves).
fn print_manual_snippet(shell: Shell, home: Option<&Path>, zdotdir: Option<&Path>) {
    let block = shell_hook::managed_block(shell);
    match shell_hook::rc_path(shell, home, zdotdir) {
        Some(rc) => println!("tayf init: add the following to {}:\n{block}", rc.display()),
        None => println!("tayf init: add the following to your shell's startup file:\n{block}"),
    }
}

/// Install the shell hook into the resolved rc file for bash/zsh, printing
/// status messages. Returns `Err(ExitCode)` if the edit fails.
fn install_hook_to_rc(
    rc: &Path,
    shell: Shell,
) -> Result<(), ExitCode> {
    match shell_hook::install_to_rc(rc, shell, SystemTime::now()) {
        Ok(out) if out.already_present => {
            println!("tayf init: shell hook already present in {}", rc.display());
        }
        Ok(out) => {
            println!("tayf init: added the tayf hook to {}", rc.display());
            if let Some(b) = out.backup {
                println!("tayf init: backed up your previous rc to {}", b.display());
            }
            if matches!(shell, Shell::Bash) {
                println!(
                    "tayf init: note — macOS login Terminals read ~/.bash_profile; \
                     add the same line there if new windows do not pick it up"
                );
            }
        }
        Err(e) => {
            eprintln!("tayf init: failed to edit {}: {e}", rc.display());
            return Err(ExitCode::from(70));
        }
    }
    Ok(())
}

/// `tayf init` entry point. See [`InitArgs`] for the surface.
#[allow(clippy::needless_pass_by_value, clippy::must_use_candidate)]
// reason: ExitCode is returned for main.rs to propagate; InitArgs is the CLI
// contract type taken by value, matching the config_tui::run shape.
pub fn run(args: InitArgs) -> ExitCode {
    // Resolve the shell first; an explicit bad --shell is a usage error.
    let shell = match shell_hook::detect(
        args.shell.as_deref(),
        std::env::var("SHELL").ok().as_deref(),
    ) {
        Ok(s) => s,
        Err(msg) => {
            eprintln!("tayf init: {msg}");
            return ExitCode::from(64);
        }
    };

    if args.print && args.uninstall {
        eprintln!("tayf init: --print and --uninstall cannot be combined");
        return ExitCode::from(64);
    }

    let home = std::env::var_os("HOME").map(PathBuf::from);
    let zdotdir = std::env::var_os("ZDOTDIR").map(PathBuf::from);

    // --print: emit the snippet, write nothing.
    if args.print {
        let block = shell_hook::managed_block(shell);
        match shell_hook::rc_path(shell, home.as_deref(), zdotdir.as_deref()) {
            Some(rc) => println!("# Add the following to {}:\n{block}", rc.display()),
            None => println!("# Add the following to your shell's startup file:\n{block}"),
        }
        return ExitCode::SUCCESS;
    }

    // --uninstall: remove the block, leave config alone.
    if args.uninstall {
        let Some(rc) = shell_hook::rc_path(shell, home.as_deref(), zdotdir.as_deref()) else {
            eprintln!("tayf init: no rc file to uninstall from for this shell");
            return ExitCode::from(64);
        };
        return match shell_hook::uninstall_from_rc(&rc, SystemTime::now()) {
            Ok(true) => {
                println!("tayf init: removed the tayf block from {}", rc.display());
                ExitCode::SUCCESS
            }
            Ok(false) => {
                println!("tayf init: no tayf block found in {}", rc.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("tayf init: failed to edit {}: {e}", rc.display());
                ExitCode::from(70)
            }
        };
    }

    // Default flow: create config, then install the hook (bash/zsh) or print
    // a snippet (fish/other), then an activation hint.
    let Some(target) = resolve_config_target(args.config.as_deref()) else {
        eprintln!(
            "tayf init: cannot determine a config location: set $HOME or \
             $XDG_CONFIG_HOME, or pass --config <path>"
        );
        return ExitCode::from(70);
    };
    match run_config_step(&target, args.force) {
        Ok(ConfigStep::Created) => println!("tayf init: created {}", target.display()),
        Ok(ConfigStep::AlreadyExists) => {
            println!(
                "tayf init: config already exists at {} (use --force to overwrite)",
                target.display()
            );
        }
        Err(e) => {
            eprintln!("tayf init: failed to write {}: {e}", target.display());
            return ExitCode::from(70);
        }
    }

    if !args.no_shell_hook {
        match shell {
            Shell::Zsh | Shell::Bash => {
                match shell_hook::rc_path(shell, home.as_deref(), zdotdir.as_deref()) {
                    Some(rc) => {
                        if let Err(code) = install_hook_to_rc(&rc, shell) {
                            return code;
                        }
                    }
                    None => print_manual_snippet(shell, home.as_deref(), zdotdir.as_deref()),
                }
            }
            Shell::Fish | Shell::Other => {
                print_manual_snippet(shell, home.as_deref(), zdotdir.as_deref());
            }
        }
    }

    println!("tayf init: done. tayf will start in new terminals. To start now: exec tayf");
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_step_creates_then_reports_existing_then_force_overwrites() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let target = tmp.path().join("tayf").join("config.toml");

        // First call: file absent -> created (parent dir created too).
        let r = run_config_step(&target, false).expect("first create");
        assert!(matches!(r, ConfigStep::Created));
        assert!(target.exists());
        assert!(std::fs::read_to_string(&target).unwrap().starts_with("# tayf default configuration"));

        // Second call without --force: reported as already existing.
        let r = run_config_step(&target, false).expect("second");
        assert!(matches!(r, ConfigStep::AlreadyExists));

        // With --force: overwrites a clobbered file back to defaults.
        std::fs::write(&target, "# clobbered\n").expect("clobber");
        let r = run_config_step(&target, true).expect("force");
        assert!(matches!(r, ConfigStep::Created));
        assert!(std::fs::read_to_string(&target).unwrap().starts_with("# tayf default configuration"));
    }
}
