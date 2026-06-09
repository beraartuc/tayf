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

use std::io::{Read, Write};
use std::os::fd::RawFd;
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
    /// Raw fd of the master, captured at `PtySession::into_parts`. Used to
    /// poll for readiness with a 50ms idle timeout in the output thread
    /// (`runtime::spawn_output_thread`). The reader's actual bytes come from a
    /// `dup`'d fd inside `inner`, but `dup`'d fds share the same open file
    /// description, so polling this fd accurately reports readiness for
    /// `inner.read`.
    ///
    /// **Lifetime invariant:** the underlying open file is owned by the
    /// `MasterPty` held inside the `Resizer` returned alongside this `Reader`.
    /// The facade (`Tayf::run`) keeps that `Resizer` alive — via the signal
    /// thread's `SignalGuard` — until after the output thread joins, so the fd
    /// stays open for the entire lifetime of this `Reader`.
    master_fd: RawFd,
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
        let (rows, cols) = crate::terminfo::winsize().unwrap_or((24, 80));
        let size = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };

        let pair = pty_system.openpty(size).map_err(io_err)?;

        // portable-pty's public `CommandBuilder` does not expose `arg0`
        // (still true as of 0.9), so the conventional leading-dash argv[0]
        // trick for login shells (e.g. `-zsh`) is not available without
        // forking the crate. We pass `-l` instead, which both zsh and bash
        // honour for login mode.
        let mut cmd = CommandBuilder::new(spec.path.as_os_str());
        if spec.login {
            cmd.arg("-l");
        }

        // Mark the child environment so always-on wrappers (rc `exec tayf`
        // guards) and tools can detect they are running inside tayf. Constant
        // value; CommandBuilder inherits the parent env (no env_clear), so this
        // is purely additive and does not affect the direct-argv invocation.
        cmd.env("TAYF_SESSION", "1");

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
        let master_fd = self.master.as_raw_fd().ok_or_else(|| {
            crate::error::Error::Pty(std::io::Error::other(
                "master pty does not expose a raw fd; required for poll-driven I/O loop",
            ))
        })?;

        let master = Arc::new(Mutex::new(self.master));
        let resizer = Resizer { master };

        Ok((
            Reader { inner: reader, master_fd },
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

    /// Raw fd suitable for `poll(2)`. See the `Reader::master_fd` field
    /// doc-comment for the lifetime invariant the caller must uphold.
    pub(crate) fn as_raw_fd(&self) -> RawFd {
        self.master_fd
    }
}

impl Writer {
    pub(crate) fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        self.inner.write_all(buf)
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

fn io_err(e: impl std::fmt::Display) -> crate::error::Error {
    crate::error::Error::Pty(std::io::Error::other(e.to_string()))
}
