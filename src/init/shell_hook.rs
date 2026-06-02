//! Shell detection, rc-file resolution, and the managed-block install /
//! uninstall logic for `tayf init`.
//!
//! All filesystem inputs (`$SHELL`, `$HOME`, `$ZDOTDIR`) are passed in by
//! the caller so the pure logic is unit-testable without touching the real
//! environment. Auto-editing is bash/zsh only; fish/other shells receive a
//! printed snippet (see [`managed_block`]).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Atomically write `content` to `rc`, preserving a symlinked rc (write
/// through to the link target so dotfile-manager symlinks survive) and the
/// existing file's permission mode. New rc files default to 0o644 (rc files
/// are conventionally group/world-readable, unlike tayf's own 0o600 config).
fn write_rc_atomic(rc: &Path, content: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

    // Resolve through a symlink (handles dangling links too — read_link does
    // not require the target to exist) so we update the real file and keep
    // the link itself intact. Relative link targets resolve against rc's dir.
    let target = match std::fs::read_link(rc) {
        Ok(link) if link.is_absolute() => link,
        Ok(link) => rc.parent().unwrap_or_else(|| Path::new(".")).join(link),
        Err(_) => rc.to_path_buf(), // not a symlink (or unreadable) → write rc directly
    };

    // Preserve the existing file's mode; default 0o644 for a brand-new rc.
    let mode = std::fs::metadata(&target).map_or(0o644, |m| m.permissions().mode() & 0o777);

    let parent = target.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "rc path has no parent directory")
    })?;
    std::fs::create_dir_all(parent)?;

    // tmpfile in the target's own dir (EXDEV-safe rename), then atomic rename.
    let pid = std::process::id();
    let stamp =
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let stem = target.file_name().and_then(|s| s.to_str()).unwrap_or("rc");
    let tmp = parent.join(format!("{stem}.tayf-tmp-{pid}-{stamp}"));

    let write_result = (|| -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(mode)
            .open(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, &target)
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&tmp); // best-effort cleanup; don't mask the real error
    }
    write_result
}

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
            other => {
                Err(format!("unknown --shell value '{other}': expected one of zsh, bash, fish"))
            }
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
pub(crate) fn rc_path(
    shell: Shell,
    home: Option<&Path>,
    zdotdir: Option<&Path>,
) -> Option<PathBuf> {
    match shell {
        Shell::Zsh => zdotdir.map(|z| z.join(".zshrc")).or_else(|| home.map(|h| h.join(".zshrc"))),
        Shell::Bash => home.map(|h| h.join(".bashrc")),
        Shell::Fish => home.map(|h| h.join(".config").join("fish").join("config.fish")),
        Shell::Other => None,
    }
}

/// Idempotency / uninstall anchors. The managed block is exactly the text
/// between (and including) these markers.
pub(crate) const BEGIN_MARKER: &str = "# >>> tayf init >>>";
pub(crate) const END_MARKER: &str = "# <<< tayf init <<<";

/// The guard snippet for `shell`: a static string (no user input) that
/// `exec`s the tayf binary only for an interactive shell on a real TTY that
/// is not already inside a tayf session.
///
/// The binary is located by trying the standard install directories with an
/// absolute path (`~/.local/bin` for `install.sh`, `~/.cargo/bin` for
/// `cargo install`, Homebrew's `/opt/homebrew/bin` and `/usr/local/bin`),
/// then falling back to a `PATH` lookup. A bare `exec tayf` is *not* enough:
/// the snippet is installed at the top of the rc file (see [`install_to_rc`])
/// so it runs before a prompt framework (e.g. Powerlevel10k instant prompt)
/// can redirect stdout, but at that point `$PATH` may not yet include the
/// install directory — the absolute paths make the guard robust regardless.
pub(crate) fn guard_line(shell: Shell) -> &'static str {
    match shell {
        Shell::Fish => {
            "if status is-interactive; and not set -q TAYF_SESSION; and test -t 1
    for _tayf in $HOME/.local/bin/tayf $HOME/.cargo/bin/tayf /opt/homebrew/bin/tayf /usr/local/bin/tayf
        test -x $_tayf; and exec $_tayf
    end
    type -q tayf; and exec tayf
end"
        }
        Shell::Zsh | Shell::Bash | Shell::Other => {
            r#"if [[ $- == *i* && -t 1 && -z $TAYF_SESSION ]]; then
  for _tayf in "$HOME/.local/bin/tayf" "$HOME/.cargo/bin/tayf" /opt/homebrew/bin/tayf /usr/local/bin/tayf; do
    [[ -x "$_tayf" ]] && exec "$_tayf"
  done
  command -v tayf >/dev/null 2>&1 && exec tayf
fi"#
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

/// Prepend `block` to `content` so tayf's guard runs *before* any prompt
/// framework (e.g. Powerlevel10k instant prompt) further down the rc file can
/// redirect stdout — at the top, the `-t 1` guard still sees the real
/// terminal, so the `exec` fires. `block` is newline-terminated by
/// [`managed_block`], so `content` always begins on its own line. Inverse of
/// [`remove_block`].
pub(crate) fn prepend_block(content: &str, block: &str) -> String {
    let mut out = String::with_capacity(block.len() + content.len());
    out.push_str(block);
    out.push_str(content);
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

/// Install the managed block at the *top* of `rc`, backing it up first.
///
/// Self-healing: a no-op only when the current block is already the very
/// first thing in the file. If a block from an older version is present —
/// appended at the bottom, or carrying a stale guard — it is removed and
/// re-installed at the top, so the guard runs before a prompt framework can
/// redirect stdout (which would make the `-t 1` check fail). Placing it last
/// was the v0.12.0/v0.12.1 onboarding bug this fixes.
pub(crate) fn install_to_rc(
    rc: &Path,
    shell: Shell,
    now: SystemTime,
) -> std::io::Result<InstallOutcome> {
    let existing = std::fs::read_to_string(rc).unwrap_or_default();
    let block = managed_block(shell);
    // Already at the top with the current text → nothing to do.
    if existing.starts_with(&block) {
        return Ok(InstallOutcome { backup: None, already_present: true });
    }
    let backup = if rc.exists() {
        let b = backup_path(rc, now);
        std::fs::copy(rc, &b)?;
        Some(b)
    } else {
        None
    };
    // Drop any prior block (wherever it sits) before prepending the fresh one,
    // so re-running `tayf init` relocates/refreshes rather than duplicating.
    let (stripped, _) = remove_block(&existing);
    let new_content = prepend_block(&stripped, &block);
    write_rc_atomic(rc, &new_content)?;
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
    write_rc_atomic(rc, &new_content)?;
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
        // The guard opens the interactive/TTY/no-session check, then tries the
        // tayf binary at the standard install dirs by absolute path before a
        // bare PATH `exec` (so it survives PATH not yet being set up).
        assert!(z.contains("if [[ $- == *i* && -t 1 && -z $TAYF_SESSION ]]; then"));
        assert!(z.contains("/opt/homebrew/bin/tayf"));
        assert!(z.contains(r#"[[ -x "$_tayf" ]] && exec "$_tayf""#));
        assert!(z.contains("command -v tayf >/dev/null 2>&1 && exec tayf"));
        // A bare PATH-relative `exec tayf` as the *only* mechanism was the bug.
        assert!(!z.contains("]] && exec tayf\n"));
        assert_eq!(guard_line(Shell::Bash), guard_line(Shell::Zsh));
        let f = managed_block(Shell::Fish);
        assert!(f.contains("status is-interactive; and not set -q TAYF_SESSION; and test -t 1"));
        assert!(f.contains("/opt/homebrew/bin/tayf"));
        assert!(f.contains("test -x $_tayf; and exec $_tayf"));
        assert!(z.ends_with('\n') && f.ends_with('\n'));
    }

    #[test]
    fn prepend_then_remove_is_identity_for_newline_terminated_file() {
        let original = "# my zshrc\nexport FOO=1\n";
        let block = managed_block(Shell::Zsh);
        assert!(!original.contains(BEGIN_MARKER));
        let installed = prepend_block(original, &block);
        assert!(installed.contains(BEGIN_MARKER));
        // The block goes to the TOP; the user's content follows untouched.
        assert!(installed.starts_with(&block));
        assert!(installed.ends_with(original));
        let (restored, removed) = remove_block(&installed);
        assert!(removed);
        assert_eq!(restored, original);
        let (same, removed2) = remove_block(original);
        assert!(!removed2);
        assert_eq!(same, original);
    }

    #[test]
    fn prepend_block_into_empty_file_is_just_the_block() {
        let block = managed_block(Shell::Bash);
        assert_eq!(prepend_block("", &block), block);
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
        let after = std::fs::read_to_string(&rc).unwrap();
        assert!(after.starts_with(&managed_block(Shell::Zsh))); // installed at the top
        assert!(after.ends_with(original)); // user content preserved below

        let out2 = install_to_rc(&rc, Shell::Zsh, now).expect("reinstall");
        assert!(out2.already_present);
        assert!(out2.backup.is_none());

        let removed = uninstall_from_rc(&rc, now).expect("uninstall");
        assert!(removed);
        assert_eq!(std::fs::read_to_string(&rc).unwrap(), original);

        assert!(!uninstall_from_rc(&rc, now).expect("uninstall-again"));
    }

    #[test]
    fn install_through_symlink_preserves_the_link_and_mode() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().expect("tmpdir");
        // Real rc lives in a "dotfiles" dir; ~/.zshrc is a symlink to it.
        let dotfiles = tmp.path().join("dotfiles");
        std::fs::create_dir_all(&dotfiles).expect("mkdir dotfiles");
        let real = dotfiles.join("zshrc");
        std::fs::write(&real, "# real zshrc\nexport FOO=1\n").expect("seed real");
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        let link = tmp.path().join(".zshrc");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");

        let out = install_to_rc(&link, Shell::Zsh, SystemTime::now()).expect("install");
        assert!(!out.already_present);

        // The link is still a symlink (NOT clobbered into a regular file).
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        // The block landed in the real target (through the link).
        assert!(std::fs::read_to_string(&real).unwrap().contains("# >>> tayf init >>>"));
        // The real file's mode is preserved (0o644), not tightened to 0o600.
        assert_eq!(std::fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o644);
    }

    #[test]
    fn install_into_new_rc_uses_0o644() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().expect("tmpdir");
        let rc = tmp.path().join(".bashrc"); // does not exist yet
        let out = install_to_rc(&rc, Shell::Bash, SystemTime::now()).expect("install");
        assert!(out.backup.is_none()); // nothing to back up
        assert!(rc.exists());
        assert_eq!(std::fs::metadata(&rc).unwrap().permissions().mode() & 0o777, 0o644);
    }

    #[test]
    fn prepend_block_puts_block_first_even_when_content_has_no_trailing_newline() {
        let block = managed_block(Shell::Zsh);
        // The block (newline-terminated) leads; content follows verbatim, so a
        // content tail without its own newline is left exactly as given.
        let out = prepend_block("export FOO=1", &block);
        assert_eq!(out, format!("{block}export FOO=1"));
        assert!(out.contains(BEGIN_MARKER));
    }

    #[test]
    fn reinstall_relocates_old_bottom_block_to_top() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let rc = tmp.path().join(".zshrc");
        // Simulate a pre-v0.12.2 install: the block was appended at the BOTTOM
        // with the old bare-PATH guard.
        let body = "# my zshrc\nexport FOO=1\n";
        let old_block = "# >>> tayf init >>>\n# Managed by `tayf init`. Remove with `tayf init --uninstall`.\n[[ $- == *i* && -t 1 && -z $TAYF_SESSION ]] && exec tayf\n# <<< tayf init <<<\n";
        std::fs::write(&rc, format!("{body}{old_block}")).expect("seed rc");
        let now = SystemTime::now();

        // Re-running init relocates + refreshes the block to the top.
        let out = install_to_rc(&rc, Shell::Zsh, now).expect("relocate");
        assert!(!out.already_present, "relocation is a change, not a no-op");
        assert!(out.backup.as_deref().is_some_and(Path::exists));
        let content = std::fs::read_to_string(&rc).unwrap();
        assert!(content.starts_with(&managed_block(Shell::Zsh)), "now at the top");
        assert!(content.contains("export FOO=1"), "user content kept");
        // The stale bare-PATH guard is gone (no leftover at the bottom).
        assert!(!content.contains("]] && exec tayf"));
        assert_eq!(content.matches(BEGIN_MARKER).count(), 1, "no duplicate block");

        // A second run is now a clean no-op.
        let out2 = install_to_rc(&rc, Shell::Zsh, now).expect("reinstall");
        assert!(out2.already_present);
    }
}
