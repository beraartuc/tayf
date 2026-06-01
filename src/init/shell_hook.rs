//! Shell detection, rc-file resolution, and the managed-block install /
//! uninstall logic for `tayf init`.
//!
//! All filesystem inputs (`$SHELL`, `$HOME`, `$ZDOTDIR`) are passed in by
//! the caller so the pure logic is unit-testable without touching the real
//! environment. Auto-editing is bash/zsh only; fish/other shells receive a
//! printed snippet (see [`managed_block`]).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The shells `tayf init` knows how to set up. `Other` covers anything we
/// will not auto-edit (we print a snippet instead).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shell {
    Zsh,
    Bash,
    Fish,
    Other,
}

/// Resolve the target shell from the `--shell` flag (if any) else the
/// basename of `$SHELL`. Returns `Err(message)` only for an explicit,
/// unrecognized `--shell` value.
pub(crate) fn detect(flag: Option<&str>, shell_env: Option<&str>) -> Result<Shell, String> {
    if let Some(f) = flag {
        return match f.to_ascii_lowercase().as_str() {
            "zsh" => Ok(Shell::Zsh),
            "bash" => Ok(Shell::Bash),
            "fish" => Ok(Shell::Fish),
            other => Err(format!(
                "unknown --shell value '{other}': expected one of zsh, bash, fish"
            )),
        };
    }
    let Some(path) = shell_env else { return Ok(Shell::Other) };
    let base = Path::new(path).file_name().and_then(|s| s.to_str()).unwrap_or("");
    Ok(match base {
        "zsh" => Shell::Zsh,
        "bash" => Shell::Bash,
        "fish" => Shell::Fish,
        _ => Shell::Other,
    })
}

/// Resolve the rc file to edit for `shell`. `zsh` honors `$ZDOTDIR`, then
/// falls back to `$HOME/.zshrc`. `Other` has no auto-edit target.
pub(crate) fn rc_path(shell: Shell, home: Option<&Path>, zdotdir: Option<&Path>) -> Option<PathBuf> {
    match shell {
        Shell::Zsh => zdotdir
            .map(|z| z.join(".zshrc"))
            .or_else(|| home.map(|h| h.join(".zshrc"))),
        Shell::Bash => home.map(|h| h.join(".bashrc")),
        Shell::Fish => home.map(|h| h.join(".config").join("fish").join("config.fish")),
        Shell::Other => None,
    }
}

/// Idempotency / uninstall anchors. The managed block is exactly the text
/// between (and including) these markers.
pub(crate) const BEGIN_MARKER: &str = "# >>> tayf init >>>";
pub(crate) const END_MARKER: &str = "# <<< tayf init <<<";

/// The guard line for `shell`: a static string (no user input) that
/// `exec`s tayf only for an interactive shell on a real TTY that is not
/// already inside a tayf session.
pub(crate) fn guard_line(shell: Shell) -> &'static str {
    match shell {
        Shell::Fish => {
            "status is-interactive; and not set -q TAYF_SESSION; and test -t 1; and exec tayf"
        }
        Shell::Zsh | Shell::Bash | Shell::Other => {
            "[[ $- == *i* && -t 1 && -z $TAYF_SESSION ]] && exec tayf"
        }
    }
}

/// The full marker-delimited block to write into an rc file, ending with a
/// trailing newline.
pub(crate) fn managed_block(shell: Shell) -> String {
    format!(
        "{BEGIN_MARKER}\n# Managed by `tayf init`. Remove with `tayf init --uninstall`.\n{}\n{END_MARKER}\n",
        guard_line(shell)
    )
}

/// Is tayf's managed block already present in `content`?
pub(crate) fn is_installed(content: &str) -> bool {
    content.contains(BEGIN_MARKER)
}

/// Append `block` to `content`, guaranteeing exactly one separating newline
/// so the block starts on its own line. Inverse of [`remove_block`] for a
/// newline-terminated input.
pub(crate) fn append_block(content: &str, block: &str) -> String {
    if content.is_empty() {
        return block.to_owned();
    }
    let mut out = String::with_capacity(content.len() + block.len() + 1);
    out.push_str(content);
    if !content.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(block);
    out
}

/// Remove the managed block (from `BEGIN_MARKER` through `END_MARKER` and
/// the single newline that follows it). Returns `(new_content, removed?)`.
pub(crate) fn remove_block(content: &str) -> (String, bool) {
    let Some(start) = content.find(BEGIN_MARKER) else {
        return (content.to_owned(), false);
    };
    let Some(end_rel) = content[start..].find(END_MARKER) else {
        return (content.to_owned(), false);
    };
    let mut end = start + end_rel + END_MARKER.len();
    if content[end..].starts_with('\n') {
        end += 1;
    }
    let mut out = String::with_capacity(content.len());
    out.push_str(&content[..start]);
    out.push_str(&content[end..]);
    (out, true)
}

/// Outcome of an install attempt, for the caller's report.
pub(crate) struct InstallOutcome {
    /// Path of the backup written before editing (`None` when the rc did
    /// not exist or the block was already present).
    pub(crate) backup: Option<PathBuf>,
    /// `true` when the managed block was already present (no change made).
    pub(crate) already_present: bool,
}

/// `<rc>.tayf-backup-<unix-seconds>` next to the rc file.
fn backup_path(rc: &Path, now: SystemTime) -> PathBuf {
    let secs = now.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let name = rc.file_name().and_then(|s| s.to_str()).unwrap_or("rc");
    rc.with_file_name(format!("{name}.tayf-backup-{secs}"))
}

/// Append the managed block to `rc`, backing it up first. Idempotent: if
/// the block is already present, makes no change.
pub(crate) fn install_to_rc(
    rc: &Path,
    shell: Shell,
    now: SystemTime,
) -> std::io::Result<InstallOutcome> {
    let existing = std::fs::read_to_string(rc).unwrap_or_default();
    if is_installed(&existing) {
        return Ok(InstallOutcome { backup: None, already_present: true });
    }
    let backup = if rc.exists() {
        let b = backup_path(rc, now);
        std::fs::copy(rc, &b)?;
        Some(b)
    } else {
        None
    };
    let new_content = append_block(&existing, &managed_block(shell));
    crate::config_tui::save::write_atomic_to(rc, &new_content)?;
    Ok(InstallOutcome { backup, already_present: false })
}

/// Remove the managed block from `rc`, backing it up first. Returns `false`
/// when there was no block (or no file) to remove.
pub(crate) fn uninstall_from_rc(rc: &Path, now: SystemTime) -> std::io::Result<bool> {
    let Ok(existing) = std::fs::read_to_string(rc) else {
        return Ok(false);
    };
    let (new_content, removed) = remove_block(&existing);
    if !removed {
        return Ok(false);
    }
    let _ = std::fs::copy(rc, backup_path(rc, now))?;
    crate::config_tui::save::write_atomic_to(rc, &new_content)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_resolves_flag_and_env() {
        assert_eq!(detect(Some("zsh"), None), Ok(Shell::Zsh));
        assert_eq!(detect(Some("BASH"), None), Ok(Shell::Bash));
        assert_eq!(detect(None, Some("/usr/bin/fish")), Ok(Shell::Fish));
        assert_eq!(detect(None, Some("/bin/zsh")), Ok(Shell::Zsh));
        assert_eq!(detect(None, Some("/bin/sh")), Ok(Shell::Other));
        assert_eq!(detect(None, None), Ok(Shell::Other));
        assert!(detect(Some("powershell"), None).is_err());
    }

    #[test]
    fn rc_path_per_shell() {
        let home = PathBuf::from("/home/u");
        let z = PathBuf::from("/z");
        assert_eq!(rc_path(Shell::Zsh, Some(&home), Some(&z)), Some(PathBuf::from("/z/.zshrc")));
        assert_eq!(rc_path(Shell::Zsh, Some(&home), None), Some(PathBuf::from("/home/u/.zshrc")));
        assert_eq!(rc_path(Shell::Bash, Some(&home), None), Some(PathBuf::from("/home/u/.bashrc")));
        assert_eq!(
            rc_path(Shell::Fish, Some(&home), None),
            Some(PathBuf::from("/home/u/.config/fish/config.fish"))
        );
        assert_eq!(rc_path(Shell::Other, Some(&home), None), None);
        assert_eq!(rc_path(Shell::Zsh, None, None), None);
    }

    #[test]
    fn managed_block_shape_per_shell() {
        let z = managed_block(Shell::Zsh);
        assert!(z.starts_with(BEGIN_MARKER));
        assert!(z.trim_end().ends_with(END_MARKER));
        assert!(z.contains("[[ $- == *i* && -t 1 && -z $TAYF_SESSION ]] && exec tayf"));
        assert_eq!(guard_line(Shell::Bash), guard_line(Shell::Zsh));
        let f = managed_block(Shell::Fish);
        assert!(f.contains("status is-interactive; and not set -q TAYF_SESSION; and test -t 1; and exec tayf"));
        assert!(z.ends_with('\n') && f.ends_with('\n'));
    }

    #[test]
    fn append_then_remove_is_identity_for_newline_terminated_file() {
        let original = "# my zshrc\nexport FOO=1\n";
        let block = managed_block(Shell::Zsh);
        assert!(!is_installed(original));
        let installed = append_block(original, &block);
        assert!(is_installed(&installed));
        assert!(installed.starts_with(original));
        let (restored, removed) = remove_block(&installed);
        assert!(removed);
        assert_eq!(restored, original);
        let (same, removed2) = remove_block(original);
        assert!(!removed2);
        assert_eq!(same, original);
    }

    #[test]
    fn append_block_into_empty_file_is_just_the_block() {
        let block = managed_block(Shell::Bash);
        assert_eq!(append_block("", &block), block);
    }

    #[test]
    fn install_backup_idempotent_then_uninstall() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let rc = tmp.path().join(".zshrc");
        let original = "# my zshrc\nexport FOO=1\n";
        std::fs::write(&rc, original).expect("seed rc");
        let now = SystemTime::now();

        let out = install_to_rc(&rc, Shell::Zsh, now).expect("install");
        assert!(!out.already_present);
        assert!(out.backup.as_deref().is_some_and(Path::exists));
        assert!(is_installed(&std::fs::read_to_string(&rc).unwrap()));

        let out2 = install_to_rc(&rc, Shell::Zsh, now).expect("reinstall");
        assert!(out2.already_present);
        assert!(out2.backup.is_none());

        let removed = uninstall_from_rc(&rc, now).expect("uninstall");
        assert!(removed);
        assert_eq!(std::fs::read_to_string(&rc).unwrap(), original);

        assert!(!uninstall_from_rc(&rc, now).expect("uninstall-again"));
    }
}
