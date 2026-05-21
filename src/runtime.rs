//! Two-thread runtime + shutdown orchestration. See spec §3.4.
//!
//! Output thread reads PTY master, feeds Pipeline, writes stdout. When the
//! child exits, the slave side closes; on Linux the master read returns EIO,
//! on macOS Ok(0). Either way the output thread drains its partial line and
//! exits. Main joins the output thread, then returns the child's exit code.
//!
//! The output thread is **`poll(2)`-driven** with a 50ms timeout so that
//! interactive prompts (which do not end in `\n`) are flushed through
//! `Pipeline::tick` within one tick instead of waiting for either a newline
//! or the 64KB line-buffer cap (spec §3.4, `LineBuffer::FLUSH_TIMEOUT`).
//!
//! Input thread reads stdin, writes PTY master. The input thread blocks on
//! `nix::poll::poll([stdin, self_pipe_read], None)` instead of `stdin.read`
//! directly, so the runtime can wake it up at shutdown by writing one byte
//! to the self-pipe write end. After `child.wait()` returns, the runtime
//! writes the wake-up byte and joins the input thread cleanly. This removes
//! the v0.1 "OS reaps blocked input thread at process exit" limit recorded
//! in spec §3.4 step 7.

use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::thread::{self, JoinHandle};

use nix::poll::{poll, PollFd, PollFlags};

use crate::error::{Error, Result};
use crate::pipeline::Pipeline;
use crate::pty::{ChildHandle, Reader, Writer};
use crate::rules::Compiled;

/// Read chunk size for the master fd. 8 KiB is the canonical pipe buffer on
/// macOS and a common Linux default; larger sizes do not measurably help
/// throughput at the latencies tayf cares about.
const READ_BUF_BYTES: usize = 8 * 1024;

/// Idle-flush tick interval. Matches `LineBuffer::FLUSH_TIMEOUT` so a partial
/// line idle for one cap is guaranteed to flush on the next poll wake-up.
const POLL_TIMEOUT_MS: nix::libc::c_int = 50;

/// Run the I/O loop until the child exits.
///
/// Spawns the output thread (PTY → stdout via `Pipeline`) and the input
/// thread (stdin → PTY). The main thread waits on the child, joins the
/// output thread, then wakes the input thread via the self-pipe and joins
/// it as well (spec §3.4 step 7).
///
/// # Errors
/// Returns `Error::Pty` if the self-pipe cannot be created or if waiting on
/// the child fails. Otherwise returns the child's exit code.
pub(crate) fn run(
    reader: Reader,
    writer: Writer,
    mut child: ChildHandle,
    rules: Compiled,
    apply_colors: bool,
) -> Result<i32> {
    let pipeline = Pipeline::new(rules);

    let (shutdown_read, shutdown_write) = make_self_pipe()?;

    let output_handle = spawn_output_thread(reader, pipeline, apply_colors);
    let input_handle = spawn_input_thread(writer, shutdown_read);

    let code = child.wait()?;

    // Drain the output thread. It should already be exiting because the
    // child closed the slave; we just join. We intentionally ignore the
    // join result: a panicked or errored output thread does not change the
    // child's exit code, and the terminal state is restored by `TtyGuard`
    // regardless.
    let _ = output_handle.join();

    // Wake the input thread by writing one byte to the self-pipe. The
    // thread is currently blocked in `poll(2)` on both stdin and the pipe
    // read end; the write makes the pipe readable so `poll` returns and the
    // thread exits its loop. `OwnedFd` will close the write end on drop,
    // which would also wake the poll (POLLHUP) — writing the byte first is
    // explicit and races-free regardless of platform pipe semantics.
    signal_shutdown(&shutdown_write);
    drop(shutdown_write);
    let _ = input_handle.join();

    Ok(code)
}

/// Create a self-pipe used to wake the input thread at shutdown.
///
/// Returns `(read_end, write_end)` as `OwnedFd` so both ends are closed on
/// drop without manual `libc::close`. Errors are surfaced as `Error::Pty`
/// (an `io::Error` chained from the underlying errno).
///
/// Close-on-exec is intentionally **not** set: nix 0.27 does not expose
/// `pipe2` on all targets tayf supports, and tayf does not `exec` any
/// further child after this point (the user's shell was already spawned
/// in `PtySession::spawn` higher up in `Tayf::run`), so there is no
/// observable inheritance hazard.
fn make_self_pipe() -> Result<(OwnedFd, OwnedFd)> {
    let (r, w) = nix::unistd::pipe().map_err(|e| Error::Pty(io::Error::from(e)))?;
    // SAFETY: `r` and `w` are freshly created by `pipe(2)` above and not
    // owned by anything else; `OwnedFd::from_raw_fd` takes ownership of
    // each fd and will close them on drop. Per its documented contract,
    // the fds must be open, owned, and not used by any other code path —
    // all three hold here because the two integers are only visible
    // inside this function until they are wrapped.
    // reason: crate-wide policy is `warn(unsafe_code)` with SAFETY blocks;
    // this is the only `unsafe` in `runtime::run` outside the existing
    // `borrow_master_fd` helper.
    #[allow(unsafe_code)]
    let owned = unsafe { (OwnedFd::from_raw_fd(r), OwnedFd::from_raw_fd(w)) };
    Ok(owned)
}

/// Write one byte to the self-pipe write end to wake the input thread.
///
/// Errors are intentionally ignored: the only documented failures here are
/// `EPIPE` (read end already closed — input thread already exited) and
/// `EINTR`/`EAGAIN`. In every case the input thread either has already
/// woken or will wake on the next poll iteration when the write end is
/// dropped by the caller.
fn signal_shutdown(write_end: &OwnedFd) {
    let fd = write_end.as_raw_fd();
    // A single-byte payload is sufficient; the input thread only checks
    // for readability and drains whatever is there before exiting.
    let _ = nix::unistd::write(fd, &[0u8]);
}

/// Borrow the PTY master fd for the duration of one `poll(2)` call.
///
/// # Safety
/// The caller must guarantee that `raw_fd` refers to an open file
/// descriptor that remains valid (i.e. not closed by any other code path)
/// for the entire lifetime of the returned `BorrowedFd`. In the tayf
/// output thread:
///
/// * `raw_fd` is the PTY master fd. The `MasterPty` that owns it is held
///   alive by the `Resizer` retained in `Tayf::run`'s scope (via
///   `SignalGuard`) for the entire lifetime of the output thread, so the
///   underlying fd is not closed while the borrow is live.
/// * `Reader::inner` holds a separate `dup`'d fd (a different fd integer
///   pointing at the same open file description). Closing that dup on
///   `Reader` drop does not invalidate `raw_fd`, so the borrow remains
///   sound even across `Reader` mutation.
///
/// The returned `'static` lifetime is a lie that the caller MUST shorten
/// by only using the value within a stack frame whose lifetime is
/// covered by the invariants above; never store it past that frame.
// reason: crate-wide policy is `warn(unsafe_code)` with SAFETY comments;
// this is the single point in the runtime that must opt in. The helper
// isolates the unsafe block so no future edit to `spawn_output_thread`
// can accidentally introduce a second unsafe operation under one allow.
#[allow(unsafe_code)]
unsafe fn borrow_master_fd(raw_fd: RawFd) -> BorrowedFd<'static> {
    // SAFETY: Delegated to the caller — see the `# Safety` section above.
    // In the output thread this is upheld by the `Resizer`-anchored
    // lifetime of the `MasterPty` and the dup-fd non-aliasing argument.
    unsafe { BorrowedFd::borrow_raw(raw_fd) }
}

fn spawn_output_thread(
    mut reader: Reader,
    mut pipeline: Pipeline,
    apply_colors: bool,
) -> JoinHandle<io::Result<()>> {
    // reason: `thread::Builder::spawn` only fails when the OS refuses to
    // create a thread (resource exhaustion). In that state tayf cannot do
    // its job at all, and `TtyGuard`'s `Drop` will restore the terminal
    // when the resulting panic unwinds. v0.1 accepts the panic; v0.2 may
    // propagate this as `Error::Pty` once the facade is shaped to surface
    // pre-loop spawn failures.
    thread::Builder::new()
        .name("tayf-output".into())
        .spawn(move || -> io::Result<()> {
            let mut stdout = io::stdout().lock();
            let mut buf = vec![0u8; READ_BUF_BYTES];
            let raw_fd = reader.as_raw_fd();
            loop {
                // SAFETY: `raw_fd` is the PTY master fd. Per the invariant
                // documented on `Reader::master_fd`, the `MasterPty` that
                // owns this fd is held alive by the `Resizer` retained in
                // `Tayf::run`'s scope (via `SignalGuard`) for the entire
                // lifetime of this thread, so the fd is valid for the
                // borrow below. The `dup`'d fd held by `Reader::inner` is
                // a different fd integer and closing it on `Reader` drop
                // does not affect this borrow. The returned `'static`
                // borrow is used only within this iteration of the loop.
                // reason: the helper's `# Safety` contract is satisfied
                // above; the allow scopes only to this one statement so a
                // future `unsafe` slip elsewhere in this function still
                // trips the crate-level `warn(unsafe_code)` lint.
                #[allow(unsafe_code)]
                let borrowed = unsafe { borrow_master_fd(raw_fd) };
                let mut pollfds = [PollFd::new(&borrowed, PollFlags::POLLIN)];
                match poll(&mut pollfds, POLL_TIMEOUT_MS) {
                    Ok(0) => {
                        // Timeout: flush any idle partial line via the
                        // pipeline's tick. In passthrough mode the pipeline
                        // was never fed, so the tick is skipped to avoid
                        // emitting an empty SGR pair on the next prompt
                        // refresh.
                        if apply_colors {
                            pipeline.tick(&mut stdout)?;
                            stdout.flush()?;
                        }
                        continue;
                    }
                    Ok(_) => {}
                    // Spurious EINTR (e.g. SIGWINCH delivered to this
                    // thread): loop and retry the poll.
                    Err(nix::errno::Errno::EINTR) => continue,
                    Err(e) => return Err(io::Error::from(e)),
                }
                match reader.read(&mut buf) {
                    Ok(0) => break, // macOS: child closed
                    Ok(n) => {
                        if apply_colors {
                            pipeline.feed(&buf[..n], &mut stdout)?;
                        } else {
                            stdout.write_all(&buf[..n])?;
                        }
                        stdout.flush()?;
                    }
                    // Linux signals child-closed by surfacing EIO on the
                    // master fd; macOS returns Ok(0). Treat both as EOF.
                    Err(e) if e.raw_os_error() == Some(nix::libc::EIO) => break,
                    // Spurious EINTR: loop and retry the read.
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(e),
                }
            }
            // Final drain of any pending partial line (no trailing newline,
            // e.g. a prompt). Skipped when colors are disabled because the
            // pipeline was never fed.
            if apply_colors {
                pipeline.drain(&mut stdout)?;
                stdout.flush()?;
            }
            Ok(())
        })
        .expect("output thread must spawn")
}

fn spawn_input_thread(mut writer: Writer, shutdown_read: OwnedFd) -> JoinHandle<()> {
    // reason: see `spawn_output_thread` — thread spawn failure is treated as
    // unrecoverable in v0.1.
    thread::Builder::new()
        .name("tayf-input".into())
        .spawn(move || {
            let stdin = io::stdin();
            let mut handle = stdin.lock();
            let mut buf = vec![0u8; READ_BUF_BYTES];
            loop {
                // Re-borrow both fds each iteration. `StdinLock` is `AsFd`
                // (the process-global stdin lives forever), and
                // `shutdown_read` is moved into this closure so its
                // `OwnedFd` outlives every loop iteration.
                let stdin_fd = handle.as_fd();
                let shutdown_fd = shutdown_read.as_fd();
                let mut pollfds = [
                    PollFd::new(&stdin_fd, PollFlags::POLLIN),
                    PollFd::new(&shutdown_fd, PollFlags::POLLIN),
                ];
                // `-1` = block forever. There is no idle work on the input
                // side; we only wake on stdin readiness, stdin EOF/error,
                // or the shutdown signal.
                match poll(&mut pollfds, -1i32) {
                    Ok(_) => {}
                    // Spurious EINTR (e.g. SIGWINCH delivered to this
                    // thread): loop and retry the poll.
                    Err(nix::errno::Errno::EINTR) => continue,
                    Err(_) => break,
                }
                // Shutdown signal: pipe is readable (the main thread of
                // `runtime::run` wrote a byte after `child.wait`). Exit
                // without draining; the pipe is dropped on thread exit.
                if pollfds[1].any().unwrap_or(false) {
                    break;
                }
                if !pollfds[0].any().unwrap_or(false) {
                    // No events on either fd we care about; loop. Should
                    // not happen with timeout=-1 but is defensive.
                    continue;
                }
                match handle.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if writer.write_all(&buf[..n]).is_err() {
                            // EPIPE — master write end is gone (the child
                            // closed). Exit quietly.
                            break;
                        }
                    }
                    // Spurious EINTR: loop and retry the read on the next
                    // poll iteration.
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        })
        .expect("input thread must spawn")
}
