//! Integration tests for shutdown escalation.
//!
//! Regression coverage for the orphan-process bug: tayf used to forward
//! `SIGTERM`/`SIGHUP` to the child process group and then keep waiting in
//! `child.wait()` forever when the child ignored those catchable signals.
//! Closing the terminal left the same leak, and PTY EOF/EIO only stopped the
//! output thread while the main thread kept waiting on the child.
//!
//! These tests use shells that deliberately ignore `TERM`/`HUP` so only the
//! runtime's `SIGKILL` escalation can make tayf exit. The shell tests also run
//! a signal-ignoring foreground job: with job control it lives in its own
//! process group, so the escalation must reach the PTY's foreground group, not
//! just the shell's. Each test bounds the
//! wait and kills tayf on failure so a regression cannot wedge the suite.

#![cfg(unix)]
#![allow(clippy::expect_used)] // reason: tests, not library code

use std::io::Write;
use std::time::{Duration, Instant};

use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use portable_pty::{Child, CommandBuilder, MasterPty};

const SETTLE_MS: u64 = 500;
const EXIT_BUDGET_MS: u64 = 5_000;

fn tayf_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_tayf"))
}

/// Spawn tayf (wrapping `shell`) inside a fresh outer PTY.
fn spawn_tayf(
    shell: &std::path::Path,
) -> (Box<dyn MasterPty + Send>, Box<dyn Child + Send + Sync>) {
    let pair = portable_pty::native_pty_system()
        .openpty(portable_pty::PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");

    let mut builder = CommandBuilder::new(tayf_bin());
    builder.env("TAYF_DISABLE_BG_DETECT", "1");
    builder.arg("--shell");
    builder.arg(shell);

    let child = pair.slave.spawn_command(builder).expect("spawn tayf");
    drop(pair.slave);
    (pair.master, child)
}

/// Poll `child` until it exits or `budget` elapses. On timeout, kill the
/// process so a failing test does not leak a tayf instance into the runner.
fn wait_for_exit(child: &mut Box<dyn Child + Send + Sync>, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        if child.try_wait().expect("try_wait").is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let _ = child.kill();
    let _ = child.wait();
    false
}

fn read_child_pid(path: &std::path::Path) -> i32 {
    std::fs::read_to_string(path).expect("read child pid file").trim().parse().expect("parse pid")
}

/// Poll until `path` holds a pid, so a test never signals tayf before the
/// shell has finished its setup.
fn wait_for_pid_file(path: &std::path::Path) -> i32 {
    let deadline = Instant::now() + Duration::from_millis(EXIT_BUDGET_MS);
    loop {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(pid) = text.trim().parse() {
                return pid;
            }
        }
        assert!(Instant::now() < deadline, "pid file {} never appeared", path.display());
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Make the wrapped interactive shell ignore `TERM`/`HUP` and start a
/// foreground job that ignores them too. Returns `(shell_pid, job_pid)`.
///
/// With job control on, the shell puts the job in its own process group, so
/// signalling the shell's group alone leaves the job behind as an orphan. The
/// job records its own pid (`$$` of the inner `sh`, kept across `exec`) so
/// the test can assert it is gone as well. Ignored dispositions are inherited
/// across `fork` and `exec`, so only `SIGKILL` can end either process.
fn start_signal_ignoring_job(writer: &mut dyn Write, dir: &std::path::Path) -> (i32, i32) {
    let shell_pid_file = dir.join("shell.pid");
    let job_pid_file = dir.join("job.pid");
    let setup = format!(
        "echo $$ > '{}'\ntrap '' TERM HUP\nsh -c 'echo $$ > \"{}\"; exec sleep 60'\n",
        shell_pid_file.display(),
        job_pid_file.display()
    );
    writer.write_all(setup.as_bytes()).expect("write setup");
    writer.flush().expect("flush");
    (wait_for_pid_file(&shell_pid_file), wait_for_pid_file(&job_pid_file))
}

/// Assert that a process spawned under tayf (recorded in a pid file) is
/// really gone. Checking only that tayf exited would also pass for an
/// implementation that abandons the child — the exact leak these tests guard
/// against.
fn assert_child_gone(pid: i32, context: &str) {
    let deadline = Instant::now() + Duration::from_millis(EXIT_BUDGET_MS);
    loop {
        match kill(Pid::from_raw(pid), None::<Signal>) {
            Err(nix::errno::Errno::ESRCH) => return,
            _ if Instant::now() >= deadline => {
                panic!("{context}: child {pid} is still alive after tayf exited")
            }
            _ => std::thread::sleep(Duration::from_millis(25)),
        }
    }
}

#[test]
fn sigterm_terminates_tayf_when_child_ignores_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (master, mut child) = spawn_tayf(std::path::Path::new("/bin/sh"));
    let tayf_pid = child.process_id().expect("tayf pid");

    let mut writer = master.take_writer().expect("take writer");
    let _reader = master.try_clone_reader().expect("clone reader");

    std::thread::sleep(Duration::from_millis(SETTLE_MS));
    let (shell_pid, job_pid) = start_signal_ignoring_job(&mut writer, dir.path());

    let pid = i32::try_from(tayf_pid).expect("pid fits i32");
    kill(Pid::from_raw(pid), Signal::SIGTERM).expect("kill -TERM tayf");

    assert!(
        wait_for_exit(&mut child, Duration::from_millis(EXIT_BUDGET_MS)),
        "tayf must exit after SIGTERM even when the child ignores TERM/HUP; \
         shutdown escalation did not reach SIGKILL"
    );
    assert_child_gone(shell_pid, "SIGTERM shell");
    assert_child_gone(job_pid, "SIGTERM foreground job");
}

#[test]
fn terminal_hangup_terminates_tayf_when_child_ignores_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (master, mut child) = spawn_tayf(std::path::Path::new("/bin/sh"));

    let mut writer = master.take_writer().expect("take writer");
    let reader = master.try_clone_reader().expect("clone reader");

    std::thread::sleep(Duration::from_millis(SETTLE_MS));
    let (shell_pid, job_pid) = start_signal_ignoring_job(&mut writer, dir.path());

    // Closing every master fd is what a terminal-emulator window close does:
    // tayf's stdin reports POLLHUP and the kernel delivers SIGHUP. The shell
    // and its foreground job ignore both, so tayf must escalate to SIGKILL
    // and exit itself.
    drop(writer);
    drop(reader);
    drop(master);

    assert!(
        wait_for_exit(&mut child, Duration::from_millis(EXIT_BUDGET_MS)),
        "tayf must exit on terminal hangup (SIGHUP + stdin EOF) instead of \
         leaving the shell as an orphan"
    );
    assert_child_gone(shell_pid, "terminal hangup shell");
    assert_child_gone(job_pid, "terminal hangup foreground job");
}

#[test]
fn pty_eof_terminates_tayf_when_child_keeps_running() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("close-stdio.c");
    let helper = dir.path().join("close-stdio");
    let pid_file = dir.path().join("child.pid");

    // A small C helper instead of a shell script: `/bin/sh` on macOS keeps
    // the controlling terminal alive after `exec 0<&- 1>&- 2>&-`, so closing
    // the slave fds never produces a master EOF. A process that closes the
    // fds directly (as the C program does) does produce EOF on both macOS and
    // Linux. Dispositions set with `signal(SIG_IGN)` survive `exec`, so the
    // trailing `sleep` still ignores TERM/HUP and forces the SIGKILL rung.
    let source_text = format!(
        "#include <signal.h>\n\
         #include <stdio.h>\n\
         #include <unistd.h>\n\
         \n\
         int main(void) {{\n\
             FILE *f = fopen(\"{}\", \"w\");\n\
             if (f) {{ fprintf(f, \"%ld\", (long)getpid()); fclose(f); }}\n\
             signal(SIGTERM, SIG_IGN);\n\
             signal(SIGHUP, SIG_IGN);\n\
             close(0);\n\
             close(1);\n\
             close(2);\n\
             execl(\"/bin/sleep\", \"sleep\", \"60\", (char *)0);\n\
             return 1;\n\
         }}\n",
        pid_file.display()
    );
    std::fs::write(&source, source_text).expect("write helper source");

    // Rust builds already require a working C toolchain (the linker), so
    // compiling this fixture adds no new prerequisite to `cargo test`.
    let compiler = std::env::var("CC").unwrap_or_else(|_| "cc".to_owned());
    let status = std::process::Command::new(compiler)
        .arg("-o")
        .arg(&helper)
        .arg(&source)
        .status()
        .expect("run C compiler");
    assert!(status.success(), "C compiler failed to build the close-stdio helper");

    let (_master, mut child) = spawn_tayf(&helper);

    // The helper closes its slave-side fds, so tayf's PTY master reports
    // EOF/EIO while the (signal-ignoring) `sleep` process is still alive.
    // The output thread must trigger shutdown, and the runtime must kill the
    // child instead of waiting forever.
    assert!(
        wait_for_exit(&mut child, Duration::from_millis(EXIT_BUDGET_MS)),
        "tayf must terminate when the PTY returns EOF/EIO; the child keeps \
         running with its tty closed and ignores TERM/HUP"
    );
    assert_child_gone(read_child_pid(&pid_file), "PTY EOF");
}
