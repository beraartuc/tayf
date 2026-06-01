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

// reason: the shell_hook functions' only non-test caller (`init::run`) lands
// in the next task; this module-level allow is removed there.
#[allow(dead_code)]
pub(crate) mod shell_hook;
