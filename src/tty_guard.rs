//! RAII guard that snapshots the controlling-terminal termios state on
//! creation, switches stdin into raw mode, and restores the original state
//! on drop, on panic, and on any normal exit path.
//!
//! Restoring the terminal is the single most security-critical invariant in
//! tayf: a corrupted termios state leaves the user's shell with no echo, no
//! line discipline, or wedged in raw mode after tayf exits. See
//! `docs/superpowers/specs/2026-05-21-tayf-v0.1-design.md` §6 (terminal
//! corruption) and `CLAUDE.md` §3.
//!
//! Restoration paths covered:
//!
//! 1. Normal drop (`Drop` impl).
//! 2. Panic on any thread that unwinds through the guard's owner — handled
//!    by a process-wide `std::panic::set_hook` installed exactly once.
//! 3. Process exit via `ExitCode` propagation up through `main` — the guard
//!    is dropped before `main` returns, so this collapses to case 1.
//!
//! Aborts (`std::process::abort`, `SIGKILL`, segfaults) cannot be intercepted
//! and are out of scope. tayf must avoid these in normal operation; the
//! `disallowed-methods` clippy rule already bans `std::process::exit`.

// reason: `TtyGuard` is consumed by Task 16 (facade) and the Task 19
// integration smoke test. Until those land, the type's only callers are
// behind `#[cfg(test)]` in this same module, so the dead-code lint flags
// the production surface. The allow is module-scoped to keep the
// intentional gap reviewable in one place; it will be removed when the
// facade wires `TtyGuard::engage`.
#![allow(dead_code)]

use std::io::IsTerminal;
use std::os::fd::{AsFd, RawFd};
use std::sync::{Mutex, OnceLock};

use nix::libc::STDIN_FILENO;
use nix::sys::termios::{cfmakeraw, tcgetattr, tcsetattr, SetArg, Termios};

use crate::error::Result;

/// Process-wide slot consulted by the panic hook to restore the terminal.
///
/// `Some` while a guard is alive, `None` otherwise. Always accessed under
/// the mutex; the hook itself is best-effort and ignores poisoning.
static PANIC_RESTORE_STATE: OnceLock<Mutex<Option<Termios>>> = OnceLock::new();

/// RAII guard: snapshot stdin's termios, switch to raw mode, restore on drop.
///
/// Construction calls `tcgetattr` then `tcsetattr` with a `cfmakeraw`-modified
/// copy. Drop calls `tcsetattr` with the original snapshot. A best-effort
/// `std::panic::set_hook` covers unwinding paths.
///
/// Only one guard should be alive per process; nested guards are not
/// supported and will leave the inner guard's "original" snapshot pointing
/// at the outer guard's raw mode, defeating the restore.
pub(crate) struct TtyGuard {
    /// Raw stdin fd, stored for identity/debug purposes. Syscalls always go
    /// through `std::io::stdin().as_fd()` to avoid `unsafe` fd construction.
    fd: RawFd,
    original: Termios,
}

impl TtyGuard {
    /// Engage raw mode. Must be called on the main thread before any other
    /// I/O thread takes ownership of stdin.
    ///
    /// # Errors
    /// Returns [`crate::Error::Tty`] if stdin is not a TTY or if either of
    /// `tcgetattr` / `tcsetattr` fails.
    pub(crate) fn engage() -> Result<Self> {
        let stdin = std::io::stdin();
        if !stdin.is_terminal() {
            return Err(nix::errno::Errno::ENOTTY.into());
        }

        let original = tcgetattr(stdin.as_fd())?;

        let mut raw = original.clone();
        cfmakeraw(&mut raw);
        tcsetattr(stdin.as_fd(), SetArg::TCSANOW, &raw)?;

        install_panic_hook(original.clone());

        Ok(TtyGuard { fd: STDIN_FILENO, original })
    }

    /// The raw fd this guard was constructed against. Provided for debug
    /// logging and tests; not used by restoration paths.
    pub(crate) fn fd(&self) -> RawFd {
        self.fd
    }
}

impl Drop for TtyGuard {
    fn drop(&mut self) {
        // Best-effort restore. If stdin has been closed or replaced under us,
        // there is nothing useful to do here — the terminal is already lost.
        let stdin = std::io::stdin();
        let _ = tcsetattr(stdin.as_fd(), SetArg::TCSANOW, &self.original);

        // Clear the panic-hook snapshot so a later panic in code that runs
        // after the guard has dropped does not re-apply a stale termios.
        if let Some(mux) = PANIC_RESTORE_STATE.get() {
            if let Ok(mut g) = mux.lock() {
                *g = None;
            }
        }
    }
}

/// Install (once per process) a panic hook that restores `original` when any
/// thread panics while a guard is alive. Subsequent calls only update the
/// shared snapshot; the hook itself is registered exactly once.
fn install_panic_hook(original: Termios) {
    static HOOK_INSTALLED: std::sync::Once = std::sync::Once::new();

    let mux = PANIC_RESTORE_STATE.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = mux.lock() {
        *g = Some(original);
    }

    HOOK_INSTALLED.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if let Some(mux) = PANIC_RESTORE_STATE.get() {
                if let Ok(g) = mux.lock() {
                    if let Some(ref original) = *g {
                        let stdin = std::io::stdin();
                        let _ = tcsetattr(stdin.as_fd(), SetArg::TCSANOW, original);
                    }
                }
            }
            prev(info);
        }));
    });
}

#[cfg(test)]
mod tests {
    // Termios tests require a real TTY and would interfere with the test
    // runner's stdin. The integration smoke test (Task 19) exercises the
    // guard end-to-end against a pty allocated by `portable-pty`.
}
