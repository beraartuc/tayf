//! OSC 11 hang diagnostic. Spawn under portable-pty subprocess on macOS CI
//! to measure where the bg_detect path stalls. Mirrors detect_from_osc11
//! phase-by-phase (open / tcgetattr / install_panic_hook / cfmakeraw /
//! tcsetattr / write / read loop / drain / suppress / restore). Prints
//! per-phase wall-clock timing to stderr; exit 0 regardless of outcome.
//!
//! Usage: cargo run --example repro_osc11_hang
//! CI usage: tests/integration_bg_detect.rs spawns the tayf binary itself
//! in portable-pty WITHOUT the v0.3.1 COLORFGBG workaround to verify the
//! production bg_detect path no longer hangs. This binary is the developer
//! triage tool for when that test starts failing in the future.
//!
//! See docs/superpowers/specs/2026-05-23-tayf-v0.3.2-pattern-polish-tech-debt.md §3.6, §4.3.

use std::fs::OpenOptions;
use std::os::fd::{AsRawFd, BorrowedFd, RawFd};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::poll::{PollFd, PollFlags, PollTimeout};
use nix::sys::termios::{cfmakeraw, tcgetattr, tcsetattr, SetArg, Termios};
use nix::unistd::{read, write};

const OSC11_READ_TIMEOUT_MS: u32 = 100;

static PANIC_SLOT: OnceLock<Mutex<Option<(RawFd, Termios)>>> = OnceLock::new();

fn main() {
    let overall = Instant::now();
    let mut phases: Vec<(&'static str, Duration)> = Vec::new();

    // Phase 1: open /dev/tty
    let t = Instant::now();
    let tty = match OpenOptions::new().read(true).write(true).open("/dev/tty") {
        Ok(f) => f,
        Err(e) => {
            eprintln!("open /dev/tty failed: {e}");
            print_phases(&phases, overall);
            return;
        }
    };
    phases.push(("open", t.elapsed()));
    let fd = tty.as_raw_fd();
    // SAFETY: tty owns the fd for the lifetime of main; we only borrow.
    // reason: crate-wide policy is `warn(unsafe_code)` with SAFETY
    // comments; allow is scoped to the single borrow.
    #[allow(unsafe_code)]
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };

    // Phase 2: tcgetattr
    let t = Instant::now();
    let original = match tcgetattr(borrowed) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("tcgetattr failed: {e}");
            print_phases(&phases, overall);
            return;
        }
    };
    phases.push(("tcgetattr", t.elapsed()));

    // Phase 3: install_panic_hook (OnceLock + Mutex contention path mirror)
    let t = Instant::now();
    let mux = PANIC_SLOT.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = mux.lock() {
        *g = Some((fd, original.clone()));
    }
    // (Skipping actual std::panic::set_hook — the contention shape on
    // OnceLock+Mutex is what production hits; the hook callback itself is
    // not on the OSC 11 happy path.)
    phases.push(("install_panic_hook", t.elapsed()));

    // Phase 4: cfmakeraw (in-place mutation, no syscall)
    let t = Instant::now();
    let mut raw = original.clone();
    cfmakeraw(&mut raw);
    phases.push(("cfmakeraw", t.elapsed()));

    // Phase 5: tcsetattr(raw)
    let t = Instant::now();
    if let Err(e) = tcsetattr(borrowed, SetArg::TCSANOW, &raw) {
        eprintln!("tcsetattr raw failed: {e}");
        print_phases(&phases, overall);
        return;
    }
    phases.push(("tcsetattr_raw", t.elapsed()));

    // Phase 6: write OSC 11 query
    let t = Instant::now();
    let _ = write(borrowed, b"\x1b]11;?\x1b\\");
    phases.push(("write_osc11", t.elapsed()));

    // Phase 7: read_until_terminator loop (mirrors bg_detect::read_until_terminator)
    let t = Instant::now();
    let deadline = Instant::now() + Duration::from_millis(u64::from(OSC11_READ_TIMEOUT_MS));
    let mut buf: Vec<u8> = Vec::with_capacity(64);
    let mut poll_iters = 0usize;
    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let remaining = deadline - now;
        let mut pollfds = [PollFd::new(borrowed, PollFlags::POLLIN)];
        let timeout_ms = i32::try_from(remaining.as_millis()).unwrap_or(i32::MAX);
        let timeout = PollTimeout::try_from(timeout_ms).unwrap_or(PollTimeout::MAX);
        poll_iters += 1;
        let ready = match nix::poll::poll(&mut pollfds, timeout) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("poll error: {e}");
                break;
            }
        };
        if ready == 0 {
            break;
        }
        let mut chunk = [0u8; 64];
        match read(borrowed.as_raw_fd(), &mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.iter().any(|&b| matches!(b, 0x07 | 0x1B | 0x9C)) {
                    break;
                }
                if buf.len() >= 128 {
                    break;
                }
            }
        }
    }
    eprintln!("  read_loop_iters: {poll_iters}");
    eprintln!("  read_bytes: {}", buf.len());
    phases.push(("read_until_terminator", t.elapsed()));

    // Phase 8: drain_remaining (O_NONBLOCK toggle + read loop + flag restore)
    let t = Instant::now();
    if let Ok(prior_bits) = fcntl(borrowed.as_raw_fd(), FcntlArg::F_GETFL) {
        let prior = OFlag::from_bits_truncate(prior_bits);
        if fcntl(borrowed.as_raw_fd(), FcntlArg::F_SETFL(prior | OFlag::O_NONBLOCK)).is_ok() {
            let mut drain_buf = [0u8; 64];
            for _ in 0..256 {
                match read(borrowed.as_raw_fd(), &mut drain_buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            let _ = fcntl(borrowed.as_raw_fd(), FcntlArg::F_SETFL(prior));
        }
    }
    phases.push(("drain_remaining", t.elapsed()));

    // Phase 9: suppress_query_echo
    let t = Instant::now();
    let _ = write(borrowed, b"\r\x1b[K");
    phases.push(("suppress_echo", t.elapsed()));

    // Phase 10: tcsetattr(restore) + panic-hook slot clear
    let t = Instant::now();
    let _ = tcsetattr(borrowed, SetArg::TCSANOW, &original);
    if let Some(mux) = PANIC_SLOT.get() {
        if let Ok(mut g) = mux.lock() {
            *g = None;
        }
    }
    phases.push(("tcsetattr_restore", t.elapsed()));

    print_phases(&phases, overall);
}

fn print_phases(phases: &[(&'static str, Duration)], overall: Instant) {
    eprintln!("overall: {:?}", overall.elapsed());
    for (name, d) in phases {
        eprintln!("  {name}: {d:?}");
    }
}
