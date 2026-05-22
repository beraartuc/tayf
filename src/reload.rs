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
}
