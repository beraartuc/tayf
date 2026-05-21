//! Signal handling thread.
//!
//! Spawns a dedicated thread that consumes signals from the `signal_hook`
//! iterator API. The thread forwards `SIGWINCH` to the PTY master and
//! `SIGINT`/`SIGTERM` to the child's process group.
//!
//! Per CLAUDE.md §3, terminal signals must reach the child's process *group*
//! (`killpg`) and not the child PID alone — otherwise foreground programs
//! launched by the shell (e.g. an editor under `bash`) survive `^C` as
//! orphans.
//!
//! Lifetime: [`spawn_handler`] returns a [`SignalGuard`] whose `Drop` closes
//! the `signal_hook` iterator (waking the blocked thread) and joins it. This
//! keeps signal teardown deterministic and tied to the I/O loop's scope.

// reason: this module is wired up by the I/O loop (Task 17) and the facade
// (Task 16); until those land the public-in-crate items have no in-crate
// caller and the dead-code lint flags them. The scope mirrors `pty.rs`.
#![allow(dead_code)]

use std::thread::{self, JoinHandle};

use signal_hook::consts::{SIGINT, SIGTERM, SIGWINCH};
use signal_hook::iterator::Signals;

use crate::error::{Error, Result};
use crate::pty::Resizer;

/// Owning handle to the signal thread. Drop closes the signal iterator and
/// joins the thread.
pub(crate) struct SignalGuard {
    handle: Option<JoinHandle<()>>,
    closer: signal_hook::iterator::Handle,
}

impl Drop for SignalGuard {
    fn drop(&mut self) {
        self.closer.close();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Install signal handling. `resizer` is moved into the signal thread.
/// `child_pid` is forwarded to as the process-group leader on
/// `SIGINT`/`SIGTERM`; when `None` (rare — `portable_pty` could not report a
/// pid) forwarding is silently skipped, since there is no group to target.
///
/// # Errors
/// Returns [`Error::Signal`] if `signal_hook` cannot register the requested
/// signals or the OS refuses to spawn the dedicated thread.
pub(crate) fn spawn_handler(resizer: Resizer, child_pid: Option<u32>) -> Result<SignalGuard> {
    let mut signals = Signals::new([SIGWINCH, SIGINT, SIGTERM]).map_err(Error::Signal)?;
    let closer = signals.handle();

    let handle = thread::Builder::new()
        .name("tayf-signals".into())
        .spawn(move || {
            for sig in &mut signals {
                match sig {
                    SIGWINCH => {
                        if let Some((rows, cols)) = read_winsize() {
                            let _ = resizer.resize(rows, cols);
                        }
                    }
                    SIGINT | SIGTERM => {
                        if let Some(pid) = child_pid {
                            // PIDs above i32::MAX cannot exist on any supported
                            // Unix; if `try_from` somehow fails we drop the
                            // forward rather than wrap around to a negative
                            // (and thus wrong-group) target.
                            if let Ok(pid_i32) = i32::try_from(pid) {
                                forward_to_pgid(pid_i32, sig);
                            }
                        }
                    }
                    _ => {}
                }
            }
        })
        .map_err(Error::Signal)?;

    Ok(SignalGuard { handle: Some(handle), closer })
}

// reason: crate-wide policy is `warn(unsafe_code)` with SAFETY comments; the
// `-D warnings` gate would otherwise reject the TIOCGWINSZ call.
#[allow(unsafe_code)]
fn read_winsize() -> Option<(u16, u16)> {
    use nix::libc::{ioctl, winsize, STDOUT_FILENO, TIOCGWINSZ};
    // SAFETY: TIOCGWINSZ is a read-only ioctl; the kernel writes into our
    // local `winsize` from a file descriptor we own (stdout). The struct
    // layout matches the kernel's expectation.
    let mut ws: winsize = unsafe { std::mem::zeroed() };
    #[allow(clippy::useless_conversion)] // reason: TIOCGWINSZ type differs per-target
    let rc = unsafe { ioctl(STDOUT_FILENO, TIOCGWINSZ as _, std::ptr::addr_of_mut!(ws)) };
    if rc != 0 {
        None
    } else {
        Some((ws.ws_row, ws.ws_col))
    }
}

fn forward_to_pgid(child_pid: i32, sig: i32) {
    use nix::sys::signal::{killpg, Signal};
    use nix::unistd::Pid;
    if let Ok(s) = Signal::try_from(sig) {
        let _ = killpg(Pid::from_raw(child_pid), s);
    }
}
