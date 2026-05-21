//! PTY session built on portable-pty.
//!
//! `PtySession::spawn` opens a master/slave pair, sizes it to the current
//! terminal, and launches the requested shell. `into_parts` decomposes the
//! session into four narrow handles owned by separate threads (spec §3.3):
//! a `Reader` (master read half), a `Writer` (master write half), a
//! `Resizer` (shared master for `SIGWINCH`), and a `ChildHandle` (reaping
//! and pid lookup for signal forwarding to the child's process group).
//!
//! This decomposition is what lets the I/O loop avoid a single
//! `Arc<Mutex<PtySession>>` shared across every thread.

// reason: `PtySession` and its decomposed handles are consumed by the I/O
// loop (Task 17) and the facade (Task 16). Until those land the module
// has no in-crate caller, so the dead-code lint flags every public-in-crate
// item. The allow scope is the whole module to keep the surface intentional
// and reviewable in one place; it will be removed when the I/O loop wires
// `spawn` and `into_parts`.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::error::Result;
use crate::shell::ShellSpec;

/// Owning handle to a spawned PTY + child process.
pub(crate) struct PtySession {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
}

/// Read half of the master.
pub(crate) struct Reader {
    inner: Box<dyn Read + Send>,
}

/// Write half of the master.
pub(crate) struct Writer {
    inner: Box<dyn Write + Send>,
}

/// Thread-safe resizer; the master is shared with the signal thread.
pub(crate) struct Resizer {
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
}

/// Child reaper + exit-code accessor.
pub(crate) struct ChildHandle {
    child: Box<dyn Child + Send + Sync>,
}

impl PtySession {
    /// Spawn `spec` in a fresh PTY sized to the current terminal.
    ///
    /// # Errors
    /// Returns `Error::Pty` if the pty system, terminal size lookup, or the
    /// child spawn fails.
    pub(crate) fn spawn(spec: &ShellSpec) -> Result<Self> {
        let pty_system = native_pty_system();
        let size = current_term_size().unwrap_or(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        });

        let pair = pty_system.openpty(size).map_err(io_err)?;

        // portable-pty 0.8's `CommandBuilder` does not expose `arg0`, so the
        // conventional leading-dash argv[0] trick for login shells (e.g.
        // `-zsh`) is not available without forking the crate. We pass `-l`
        // instead, which both zsh and bash honour for login mode; the
        // `ShellSpec::argv0` helper is retained for v0.2 when we drop down
        // to a custom spawn path.
        let mut cmd = CommandBuilder::new(spec.path.as_os_str());
        if spec.login {
            cmd.arg("-l");
        }

        let child = pair.slave.spawn_command(cmd).map_err(io_err)?;
        drop(pair.slave);

        Ok(PtySession { master: pair.master, child })
    }

    /// Split the session into four narrow handles.
    ///
    /// # Errors
    /// Returns `Error::Pty` if the master fd cannot be cloned for reading or
    /// writing.
    pub(crate) fn into_parts(self) -> Result<(Reader, Writer, Resizer, ChildHandle)> {
        let reader = self.master.try_clone_reader().map_err(io_err)?;
        let writer = self.master.take_writer().map_err(io_err)?;

        let master = Arc::new(Mutex::new(self.master));
        let resizer = Resizer { master };

        Ok((
            Reader { inner: reader },
            Writer { inner: writer },
            resizer,
            ChildHandle { child: self.child },
        ))
    }
}

impl Reader {
    pub(crate) fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Writer {
    pub(crate) fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        self.inner.write_all(buf)
    }

    /// Close the master write side so that the input thread receives EPIPE
    /// on its next write and exits cleanly. See spec §3.4.
    pub(crate) fn shutdown(self) {
        drop(self.inner);
    }
}

impl Resizer {
    /// Resize the PTY to `rows` x `cols`.
    ///
    /// # Errors
    /// Returns `Error::Pty` if the underlying `ioctl(TIOCSWINSZ)` fails, or
    /// if the shared master mutex is poisoned by a panicking thread.
    pub(crate) fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        let m = self.master.lock().map_err(|_| {
            crate::error::Error::Pty(std::io::Error::other("resizer mutex poisoned"))
        })?;
        m.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }).map_err(io_err)?;
        Ok(())
    }
}

impl ChildHandle {
    /// Block until the child exits and return its exit code.
    ///
    /// # Errors
    /// Returns `Error::Pty` if waiting on the child fails. Exit codes that do
    /// not fit in `i32` saturate to `70` (sysexits `EX_SOFTWARE`).
    pub(crate) fn wait(&mut self) -> Result<i32> {
        let status = self.child.wait().map_err(io_err)?;
        let code: i32 = status.exit_code().try_into().unwrap_or(70);
        Ok(code)
    }

    /// Process ID of the child (used for signal forwarding to its group).
    pub(crate) fn pid(&self) -> Option<u32> {
        self.child.process_id()
    }
}

// reason: the crate-wide policy is `warn(unsafe_code)` with SAFETY comments;
// this is the sole v0.1 unsafe site (TIOCGWINSZ ioctl). The `-D warnings`
// gate would otherwise reject it.
#[allow(unsafe_code)]
fn current_term_size() -> Option<PtySize> {
    use nix::libc::{ioctl, winsize, STDOUT_FILENO, TIOCGWINSZ};
    // SAFETY: TIOCGWINSZ writes into the local `winsize` from a file descriptor
    // we own (stdout). The struct layout matches the kernel's expectation. On
    // failure we return None.
    let mut ws: winsize = unsafe { std::mem::zeroed() };
    #[allow(clippy::useless_conversion)] // reason: TIOCGWINSZ type differs per-target
    let rc = unsafe { ioctl(STDOUT_FILENO, TIOCGWINSZ as _, std::ptr::addr_of_mut!(ws)) };
    if rc != 0 {
        return None;
    }
    Some(PtySize {
        rows: ws.ws_row,
        cols: ws.ws_col,
        pixel_width: ws.ws_xpixel,
        pixel_height: ws.ws_ypixel,
    })
}

fn io_err(e: impl std::fmt::Display) -> crate::error::Error {
    crate::error::Error::Pty(std::io::Error::other(e.to_string()))
}
