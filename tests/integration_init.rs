//! Integration tests for `tayf init` — config creation, `--print`, and the
//! bash/zsh rc install/uninstall round-trip. Non-PTY: `tayf init` only
//! writes files and prints, so plain `Command` + `output()` is enough.

use std::path::{Path, PathBuf};
use std::process::Command;

fn tayf_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tayf"))
}

#[test]
fn init_no_shell_hook_creates_config_idempotently() {
    let xdg = tempfile::tempdir().expect("xdg");
    let cfg = xdg.path().join("tayf").join("config.toml");

    // First run: creates the config.
    let out = Command::new(tayf_bin())
        .args(["init", "--no-shell-hook", "--shell", "bash"])
        .env("XDG_CONFIG_HOME", xdg.path())
        .output()
        .expect("run init");
    assert!(out.status.success(), "exit 0; stderr={}", String::from_utf8_lossy(&out.stderr));
    assert!(cfg.exists(), "config created at {}", cfg.display());

    // Second run without --force: reported as already existing, exit 0.
    let out = Command::new(tayf_bin())
        .args(["init", "--no-shell-hook", "--shell", "bash"])
        .env("XDG_CONFIG_HOME", xdg.path())
        .output()
        .expect("run init again");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("already exists"));

    // --force overwrites a clobbered file with defaults.
    std::fs::write(&cfg, "# clobbered\n").expect("clobber");
    let out = Command::new(tayf_bin())
        .args(["init", "--no-shell-hook", "--shell", "bash", "--force"])
        .env("XDG_CONFIG_HOME", xdg.path())
        .output()
        .expect("run init --force");
    assert!(out.status.success());
    assert!(std::fs::read_to_string(&cfg).unwrap().starts_with("# tayf default configuration"));
}

#[test]
fn init_print_writes_nothing() {
    let xdg = tempfile::tempdir().expect("xdg");
    let cfg = xdg.path().join("tayf").join("config.toml");

    let out = Command::new(tayf_bin())
        .args(["init", "--print", "--shell", "fish"])
        .env("XDG_CONFIG_HOME", xdg.path())
        // Set HOME too so the printed fish rc path is hermetic (not the
        // developer's real ~), even though --print writes nothing.
        .env("HOME", xdg.path())
        .output()
        .expect("run init --print");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("# >>> tayf init >>>"));
    assert!(stdout.contains("status is-interactive; and not set -q TAYF_SESSION"));
    assert!(!cfg.exists(), "--print must not create the config");
}

#[test]
fn init_zsh_installs_backs_up_and_uninstalls() {
    let home = tempfile::tempdir().expect("home");
    let xdg = tempfile::tempdir().expect("xdg");
    let rc = home.path().join(".zshrc");
    let original = "# my zshrc\nexport FOO=1\n";
    std::fs::write(&rc, original).expect("seed rc");

    // Install.
    let out = Command::new(tayf_bin())
        .args(["init", "--shell", "zsh"])
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .env_remove("ZDOTDIR")
        .output()
        .expect("run init zsh");
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));
    let after = std::fs::read_to_string(&rc).unwrap();
    // The hook is installed at the TOP (before any prompt framework can
    // redirect stdout and fail the `-t 1` guard); user content follows.
    assert!(after.starts_with("# >>> tayf init >>>"));
    assert!(after.ends_with(original));
    assert!(backup_count(home.path()) >= 1, "a backup was written");

    // Uninstall restores the original.
    let out = Command::new(tayf_bin())
        .args(["init", "--uninstall", "--shell", "zsh"])
        .env("HOME", home.path())
        .env_remove("ZDOTDIR")
        .output()
        .expect("run init --uninstall");
    assert!(out.status.success());
    assert_eq!(std::fs::read_to_string(&rc).unwrap(), original);
}

fn backup_count(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .expect("read_dir")
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains(".zshrc.tayf-backup-"))
        .count()
}
