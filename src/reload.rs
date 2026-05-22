//! Hot-reload orchestrator.
//!
//! Receives [`ReloadRequest`] events from the file watcher and the
//! signal thread, re-reads the config, re-compiles the rule set, and
//! atomically stores the new compiled set into the shared
//! `Arc<ArcSwap<Compiled>>`. Parse/compile failures preserve the
//! previous rule set and emit a `warn_msg!` to stderr.
//!
//! See `docs/superpowers/specs/2026-05-22-tayf-v0.2.1-hot-reload.md` §3.2.

/// Source of a reload trigger. Both variants route to the same
/// orchestrator code path; the variant is preserved purely for
/// diagnostic logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
// reason: scaffold-only in this task — the watcher (Task 5) and signals
// path (Task 6) wire the producers, and the orchestrator thread (Task 7)
// wires the consumer. Allow is removed at first non-test use site.
pub(crate) enum ReloadRequest {
    /// The file watcher observed a change to the config path.
    FileChanged,
    /// SIGHUP was delivered to the tayf process.
    SignalHup,
}

use std::path::Path;
use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::error::Result;
use crate::rules::Compiled;
use crate::terminfo::ColorDepth;

/// Re-load the config from `path` (or rebuild built-ins when `None`),
/// re-compile the rule set at `depth`, and atomically store the new
/// `Arc<Compiled>` into `handle`.
///
/// On failure (parse error, regex compile error, etc.) the previous
/// `Arc<Compiled>` remains in `handle` untouched and the error is
/// returned to the caller.
///
/// # Errors
/// Returns any error surfaced by [`crate::config::load`] (with the
/// caller's explicit `path`) or [`crate::rules::Compiled::load`].
#[allow(dead_code)]
// reason: tested in this task, wired to the orchestrator thread in
// Task 6. Allow is removed at that first non-test use site.
pub(crate) fn reload_once(
    handle: &ArcSwap<Compiled>,
    path: Option<&Path>,
    depth: ColorDepth,
) -> Result<()> {
    // Load using the same resolver the initial run used. `config::load`
    // returns `Some((config, path_loaded_from))` when a file was read,
    // or `None` when nothing applies.
    let loaded = crate::config::load(path)?;
    let cfg = loaded.as_ref().map(|(c, _)| c);
    let path_str = loaded.as_ref().map(|(_, p)| p.display().to_string());

    let compiled = Compiled::load(cfg, path_str.as_deref(), depth)?;
    handle.store(Arc::new(compiled));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reload_request_variants_are_distinct() {
        assert_ne!(ReloadRequest::FileChanged, ReloadRequest::SignalHup);
    }

    #[test]
    fn reload_request_is_copy() {
        // Compile-time guarantee — a copy through assignment.
        let a = ReloadRequest::FileChanged;
        let b = a;
        let _ = a; // would fail to compile if non-Copy
        assert_eq!(a, b);
    }

    use std::path::PathBuf;
    use std::sync::Arc;

    use arc_swap::ArcSwap;
    use tempfile::TempDir;

    use crate::rules::Compiled;
    use crate::terminfo::ColorDepth;

    fn write(dir: &TempDir, body: &str) -> PathBuf {
        let p = dir.path().join("config.toml");
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn reload_once_swaps_on_valid_config() {
        let handle = Arc::new(ArcSwap::from_pointee(Compiled::load_builtins().unwrap()));
        let before = handle.load_full();

        let dir = tempfile::tempdir().unwrap();
        let path = write(
            &dir,
            r#"
[[rules]]
name = "log_level"
style = { fg = "yellow", bold = true }
"#,
        );

        super::reload_once(&handle, Some(&path), ColorDepth::Truecolor).unwrap();

        let after = handle.load_full();
        assert!(!Arc::ptr_eq(&before, &after), "reload_once must replace the Arc on success");
    }

    #[test]
    fn reload_once_preserves_old_arc_on_parse_error() {
        let handle = Arc::new(ArcSwap::from_pointee(Compiled::load_builtins().unwrap()));
        let before = handle.load_full();

        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "this is = not valid = toml\n");

        let err = super::reload_once(&handle, Some(&path), ColorDepth::Truecolor)
            .expect_err("invalid toml must fail reload");
        assert!(matches!(err, crate::error::Error::Config { .. }));

        let after = handle.load_full();
        assert!(Arc::ptr_eq(&before, &after), "reload_once must NOT swap the Arc on failure");
    }

    // Deliberately NO `reload_once_with_none_path_*` test here — see plan
    // rationale (the test would be host-dependent on $XDG_CONFIG_HOME and
    // $HOME). The production code path always passes Some(path) when a
    // config was loaded at startup.
}
