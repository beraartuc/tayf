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
//! Input thread reads stdin, writes PTY master. **v0.1 limitation:** the
//! input thread is NOT joined and may remain blocked on `stdin.read()` when
//! tayf exits. The OS reaps it on process exit. Future versions will add a
//! self-pipe wakeup so the thread terminates promptly (spec §3.4 step 7).

use std::io::{self, Read, Write};
use std::os::fd::BorrowedFd;
use std::thread::{self, JoinHandle};

use nix::poll::{poll, PollFd, PollFlags};

use crate::error::Result;
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
/// thread (stdin → PTY). The main thread waits on the child, then joins
/// the output thread. The input thread is not joined — it may still be
/// blocked on `stdin.read()`; the OS reaps it on process exit (spec §3.4
/// step 7, v0.1 limit documented there).
///
/// # Errors
/// Returns `Error::Pty` if waiting on the child fails. Otherwise returns
/// the child's exit code.
pub(crate) fn run(
    reader: Reader,
    writer: Writer,
    mut child: ChildHandle,
    rules: Compiled,
    apply_colors: bool,
) -> Result<i32> {
    let pipeline = Pipeline::new(rules);

    let output_handle = spawn_output_thread(reader, pipeline, apply_colors);
    let _input_handle = spawn_input_thread(writer);

    let code = child.wait()?;

    // Drain the output thread. It should already be exiting because the
    // child closed the slave; we just join. We intentionally ignore the
    // join result: a panicked or errored output thread does not change the
    // child's exit code, and the terminal state is restored by `TtyGuard`
    // regardless.
    let _ = output_handle.join();

    Ok(code)
}

// reason: crate-wide policy is `warn(unsafe_code)` with SAFETY comments; the
// single `unsafe { BorrowedFd::borrow_raw(raw_fd) }` below would otherwise be
// rejected by `-D warnings`. See the SAFETY block inside for the invariant.
#[allow(unsafe_code)]
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
                // `Tayf::run`'s scope for the entire lifetime of this
                // thread, so the fd is valid for the borrow below.
                let borrowed = unsafe { BorrowedFd::borrow_raw(raw_fd) };
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

fn spawn_input_thread(mut writer: Writer) -> JoinHandle<()> {
    // reason: see `spawn_output_thread` — thread spawn failure is treated as
    // unrecoverable in v0.1.
    thread::Builder::new()
        .name("tayf-input".into())
        .spawn(move || {
            let mut stdin = io::stdin().lock();
            let mut buf = vec![0u8; READ_BUF_BYTES];
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if writer.write_all(&buf[..n]).is_err() {
                            // EPIPE — master write end is gone (facade
                            // called `Writer::shutdown`, or the child
                            // closed). Exit quietly.
                            break;
                        }
                    }
                    // Spurious EINTR: loop and retry.
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        })
        .expect("input thread must spawn")
}
