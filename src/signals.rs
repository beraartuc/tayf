//! Signal handling thread.
//!
//! Spawns a dedicated thread that consumes signals from the `signal_hook`
//! iterator API. The thread forwards `SIGWINCH` to the PTY master and
//! `SIGINT`/`SIGTERM`/`SIGHUP` to the child's process group. `SIGHUP`
//! additionally triggers a config reload when the hot-reload pipeline
//! is wired (`reload_tx` is `Some`); when not wired (no config file
//! present or `--no-hot-reload` set), `SIGHUP` is still forwarded — a
//! BEHAVIOR CHANGE from v0.2.1–v0.3.2, where `SIGHUP` was silently
//! dropped without a forward fallback (see CHANGELOG v0.3.3).
//!
//! Per CLAUDE.md §3, terminal signals must reach the child's process *group*
//! (`killpg`) and not the child PID alone — otherwise foreground programs
//! launched by the shell (e.g. an editor under `bash`) survive `^C` as
//! orphans.
//!
//! ## Shutdown triggers
//!
//! Forwarding alone is not enough. An interactive shell may ignore
//! `SIGTERM`/`SIGHUP`, and a terminal emulator that closes sends `SIGHUP` to
//! tayf rather than ending the process. The signal thread therefore also
//! requests a shutdown through the runtime's shutdown channel:
//!
//! - `SIGTERM` always requests shutdown after forwarding.
//! - `SIGHUP` requests shutdown only when stdin is already hung up. A
//!   *manual* `kill -HUP <pid>` keeps the terminal alive and remains the
//!   documented config-reload trigger; a terminal close reports
//!   `POLLHUP`/`POLLERR` on stdin and is treated as a real hangup. Linux and
//!   macOS do not expose a reliable `siginfo` distinction between the two
//!   origins, so the stdin state is the discriminator.
//!
//! The main thread owns the child handle and performs the actual
//! kill/escalate/reap sequence; see [`crate::runtime`]. `SIGINT` is
//! forward-only: `^C` must cancel the shell's foreground job, not end the
//! tayf session.
//!
//! Lifetime: [`spawn_handler`] returns a [`SignalGuard`] whose `Drop` closes
//! the `signal_hook` iterator (waking the blocked thread) and joins it. This
//! keeps signal teardown deterministic and tied to the I/O loop's scope.

use std::io::IsTerminal;
use std::os::fd::AsFd;
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};

use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM, SIGWINCH};
use signal_hook::iterator::Signals;

use crate::error::{Error, Result};
use crate::pty::Resizer;
use crate::reload::ReloadRequest;
use crate::shutdown::ShutdownReason;

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
/// `SIGINT`/`SIGTERM`/`SIGHUP`; when `None` (rare — `portable_pty` could not
/// report a pid) forwarding is silently skipped, since there is no group to
/// target.
/// `reload_tx`, when `Some`, receives a
/// [`ReloadRequest::SignalHup`](crate::reload::ReloadRequest::SignalHup) on
/// every delivered `SIGHUP` IN ADDITION to the SIGHUP being forwarded to
/// the child's process group. When `None`, `SIGHUP` is still forwarded
/// to the child PG — only the reload trigger is skipped. v0.3.3 fixed a
/// v0.2.1 regression where `SIGHUP` with `None` `reload_tx` was silently
/// dropped (CHANGELOG v0.3.3).
///
/// `shutdown_tx` receives a [`ShutdownReason::Signal`] when the delivered
/// signal must end the session (`SIGTERM`, or a `SIGHUP` whose stdin is
/// already hung up). The target signal is forwarded to the child PG first;
/// the runtime's main thread then reaps the child and escalates to
/// `SIGKILL` if it ignores the signal.
///
/// # Errors
/// Returns [`Error::Signal`] if `signal_hook` cannot register the requested
/// signals or the OS refuses to spawn the dedicated thread.
pub(crate) fn spawn_handler(
    resizer: Resizer,
    child_pid: Option<u32>,
    reload_tx: Option<Sender<ReloadRequest>>,
    shutdown_tx: Sender<ShutdownReason>,
) -> Result<SignalGuard> {
    let mut signals = Signals::new([SIGWINCH, SIGINT, SIGTERM, SIGHUP]).map_err(Error::Signal)?;
    let closer = signals.handle();

    let handle = thread::Builder::new()
        .name("tayf-signals".into())
        .spawn(move || {
            for sig in &mut signals {
                match sig {
                    SIGWINCH => {
                        if let Some((rows, cols)) = crate::terminfo::winsize() {
                            let _ = resizer.resize(rows, cols);
                        }
                    }
                    SIGINT => {
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
                    SIGTERM => {
                        if let Some(pid) = child_pid {
                            if let Ok(pid_i32) = i32::try_from(pid) {
                                forward_to_pgid(pid_i32, sig);
                            }
                        }
                        // Forwarding is not enough: the child may ignore
                        // SIGTERM (interactive shells often do). Ask the
                        // main thread to reap it and escalate to SIGKILL.
                        let _ = shutdown_tx.send(ShutdownReason::Signal(SIGTERM));
                    }
                    SIGHUP => {
                        // v0.3.3: SIGHUP is forwarded to the child process
                        // group unconditionally, mirroring SIGINT/SIGTERM.
                        // This fixes a v0.2.1 regression where SIGHUP was
                        // silently dropped when hot-reload was not wired
                        // (no config file present, or --no-hot-reload),
                        // leaving the child shell unaware of tmux detach /
                        // terminal close and orphaning its foreground
                        // processes. Order: forward first (kernel path,
                        // immediate), reload-notify after (mpsc channel).
                        // The post-forward reload is intentional residual
                        // work — see spec §1.2 (Rev2 I-4): the child may
                        // die from the forwarded HUP before the reload
                        // completes; the wasted work is benign (no
                        // functional impact, minor info_msg! stderr
                        // artifact gated by TAYF_LOG=info).
                        if let Some(pid) = child_pid {
                            if let Ok(pid_i32) = i32::try_from(pid) {
                                forward_to_pgid(pid_i32, sig);
                            }
                        }
                        if let Some(tx) = &reload_tx {
                            let _ = tx.send(ReloadRequest::SignalHup);
                        }
                        // A manual `kill -HUP` leaves the terminal intact;
                        // a terminal hangup makes stdin report POLLHUP /
                        // POLLERR. Only the latter should end the session,
                        // so the documented reload trigger keeps working.
                        if stdin_hung_up() {
                            let _ = shutdown_tx.send(ShutdownReason::Signal(SIGHUP));
                        }
                    }
                    _ => {}
                }
            }
        })
        .map_err(Error::Signal)?;

    Ok(SignalGuard { handle: Some(handle), closer })
}

fn forward_to_pgid(child_pid: i32, sig: i32) {
    use nix::sys::signal::{killpg, Signal};
    use nix::unistd::Pid;
    if let Ok(s) = Signal::try_from(sig) {
        let _ = killpg(Pid::from_raw(child_pid), s);
    }
}

/// True when stdin is already disconnected.
///
/// Closing the terminal emulator leaves tayf's stdin open as an fd but in a
/// hung-up state: `poll(2)` returns `POLLHUP` (often alongside `POLLIN` and
/// an EOF read). A manual `kill -HUP <pid>` on a live terminal does not set
/// those flags. This is the portable discriminator available to us —
/// `siginfo_t::si_code` is not reliable across Linux and macOS for
/// kernel-generated hangups (macOS reports `SI_USER` for both).
///
/// A pending read is intentionally left to the input thread; `poll` only
/// observes readiness and does not consume bytes.
fn stdin_hung_up() -> bool {
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        // No terminal to hang up; treat a detached stdin as a hangup so a
        // SIGHUP cannot leave tayf waiting on a dead session.
        return true;
    }
    let mut pollfds = [PollFd::new(stdin.as_fd(), PollFlags::POLLIN)];
    match poll(&mut pollfds, PollTimeout::ZERO) {
        Ok(_) => pollfds[0].revents().is_some_and(|events| {
            events.intersects(PollFlags::POLLHUP | PollFlags::POLLERR | PollFlags::POLLNVAL)
        }),
        Err(_) => false,
    }
}
