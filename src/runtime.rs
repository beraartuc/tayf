//! Two-thread runtime + shutdown orchestration. See spec §3.4.
//!
//! Output thread reads PTY master, feeds Pipeline, writes stdout. When the
//! child exits, the slave side closes; on Linux the master read returns EIO,
//! on macOS Ok(0). Either way the output thread drains its partial line and
//! exits. Main joins the output thread, then returns the child's exit code.
//!
//! Input thread reads stdin, writes PTY master. **v0.1 limitation:** the
//! input thread is NOT joined and may remain blocked on `stdin.read()` when
//! tayf exits. The OS reaps it on process exit. Future versions will add a
//! self-pipe wakeup so the thread terminates promptly (spec §3.4 step 7).

use std::io::{self, Read, Write};
use std::thread::{self, JoinHandle};

use crate::error::Result;
use crate::pipeline::Pipeline;
use crate::pty::{ChildHandle, Reader, Writer};
use crate::rules::Compiled;

/// Read chunk size for the master fd. 8 KiB is the canonical pipe buffer on
/// macOS and a common Linux default; larger sizes do not measurably help
/// throughput at the latencies tayf cares about.
const READ_BUF_BYTES: usize = 8 * 1024;

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
            loop {
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
                    // Spurious EINTR (e.g. SIGWINCH delivered to this
                    // thread): loop and retry the read.
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
