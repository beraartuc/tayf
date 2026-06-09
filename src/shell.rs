//! Shell discovery cascade per spec §1 / brainstorming Decision 3.

use std::env;
use std::path::{Path, PathBuf};

use crate::error::Result;

/// What to launch and how.
#[derive(Debug, Clone)]
pub(crate) struct ShellSpec {
    pub(crate) path: PathBuf,
    pub(crate) login: bool,
}

/// Production entry point. Resolves the shell via `$SHELL` then `/etc/passwd`
/// then `/bin/sh`. The `override_path` (typically the `--shell` CLI flag)
/// short-circuits the cascade.
///
/// # Errors
/// Currently never errors — falls through to `/bin/sh`. The `Result` shape is
/// kept so adding real failure modes (e.g. validating `--shell` paths) does
/// not churn call sites.
pub(crate) fn discover(override_path: Option<&Path>, login: bool) -> Result<ShellSpec> {
    let mut spec = discover_with_env(
        override_path,
        || env::var("SHELL").ok().filter(|s| !s.is_empty()),
        passwd_shell,
    )?;
    spec.login = login;
    Ok(spec)
}

// reason: shape mirrors `discover`'s `Result` return so the two stay
// interchangeable if real error modes (e.g. validating `--shell` paths) are
// ever added. Dropping the wrapper now would force a churning refactor then.
#[allow(clippy::unnecessary_wraps)]
fn discover_with_env(
    override_path: Option<&Path>,
    env_lookup: impl FnOnce() -> Option<String>,
    passwd_lookup: impl FnOnce() -> Option<String>,
) -> Result<ShellSpec> {
    if let Some(p) = override_path {
        return Ok(ShellSpec { path: p.to_path_buf(), login: false });
    }
    if let Some(env_path) = env_lookup() {
        return Ok(ShellSpec { path: PathBuf::from(env_path), login: false });
    }
    if let Some(passwd_path) = passwd_lookup() {
        return Ok(ShellSpec { path: PathBuf::from(passwd_path), login: false });
    }
    Ok(ShellSpec { path: PathBuf::from("/bin/sh"), login: false })
}

fn passwd_shell() -> Option<String> {
    use nix::unistd::{getuid, User};
    let user = User::from_uid(getuid()).ok().flatten()?;
    let shell = user.shell.to_string_lossy().to_string();
    if shell.is_empty() {
        None
    } else {
        Some(shell)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_override_path_wins() {
        let spec = discover_with_env(
            Some(Path::new("/usr/local/bin/fish")),
            || Some("/bin/zsh".into()),
            || Some("/bin/bash".into()),
        )
        .unwrap();
        assert_eq!(spec.path, PathBuf::from("/usr/local/bin/fish"));
    }

    #[test]
    fn shell_env_var_used_when_no_override() {
        let spec = discover_with_env(None, || Some("/bin/zsh".into()), || Some("/bin/bash".into()))
            .unwrap();
        assert_eq!(spec.path, PathBuf::from("/bin/zsh"));
    }

    #[test]
    fn passwd_used_when_env_empty() {
        let spec = discover_with_env(None, || None, || Some("/bin/bash".into())).unwrap();
        assert_eq!(spec.path, PathBuf::from("/bin/bash"));
    }

    #[test]
    fn sh_fallback_used_when_both_absent() {
        let spec = discover_with_env(None, || None, || None).unwrap();
        assert_eq!(spec.path, PathBuf::from("/bin/sh"));
    }
}
