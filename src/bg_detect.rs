//! Background color detection for automatic light/dark theme resolution.
//!
//! Startup-only resolver: COLORFGBG env var → OSC 11 query → dark fallback.
//! See spec §3 for the full algorithm and timing budget.
//!
//! Reference: xterm Operating System Commands ("OSC") — sequence 11 reports
//! the terminal's default background color via the response
//! `\e]11;rgb:RRRR/GGGG/BBBB\e\\` (or BEL-terminated, or 8-bit C1 ST
//! terminated). Rec. 601 weighted luminance (Y = 0.299·R + 0.587·G +
//! 0.114·B) decides light vs dark with threshold 0.5 (inclusive → Light).
//!
//! Termios + panic safety: the OSC 11 path opens `/dev/tty`, snapshots
//! termios, installs a process-wide panic hook that restores on panic
//! (necessary because release builds use `panic = "abort"` and Drop does
//! NOT run on panic), switches to raw mode, queries, reads with a
//! `nix::poll::poll` timeout, then restores termios via Drop. The panic
//! hook clears its slot on Drop so subsequent panics don't re-apply a
//! stale termios.

/// Resolved background theme. Maps directly to v0.2.3 preset theme names:
/// `BgTheme::Light` → `"light"`, `BgTheme::Dark` → `"dark"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BgTheme {
    Light,
    Dark,
}

impl BgTheme {
    /// String identifier matching `themes::load` registry. Stable.
    pub(crate) fn as_theme_name(self) -> &'static str {
        match self {
            BgTheme::Light => "light",
            BgTheme::Dark => "dark",
        }
    }
}

/// Resolve the effective background theme by trying detection paths in
/// order. Never panics. Falls back to `BgTheme::Dark` on any failure.
///
/// Time budget: ≤ 100 ms wall clock for the OSC 11 path; COLORFGBG path
/// is synchronous and zero-I/O. See spec §3.4.
///
/// Side effects: may briefly toggle termios on `/dev/tty` if it reaches
/// the OSC 11 path. All paths restore termios on return (including panic).
pub(crate) fn resolve() -> BgTheme {
    if let Some(t) = detect_from_colorfgbg() {
        debug_log(format_args!("bg_detect: colorfgbg -> {t:?}"));
        return t;
    }
    if let Some(t) = detect_from_osc11() {
        debug_log(format_args!("bg_detect: osc11 -> {t:?}"));
        return t;
    }
    debug_log(format_args!("bg_detect: fallback -> Dark"));
    BgTheme::Dark
}

/// Emit a debug-level trace via the in-crate `log` module. The crate's
/// `log` module exposes `warn_msg!` and `info_msg!` macros but no
/// `debug_msg!`; we route through the lower-level [`crate::log::emit`]
/// entry point directly, gated on the latched log level so the path is
/// zero-cost when `TAYF_LOG` is unset (the default).
fn debug_log(args: std::fmt::Arguments<'_>) {
    if crate::log::enabled(crate::log::LogLevel::Debug) {
        crate::log::emit(crate::log::LogLevel::Debug, args);
    }
}

fn detect_from_colorfgbg() -> Option<BgTheme> {
    let raw = std::env::var("COLORFGBG").ok()?;
    parse_colorfgbg(&raw)
}

/// Parse the COLORFGBG environment variable.
///
/// rxvt / urxvt format: `fg;bg` where bg is an xterm color number 0..15.
/// Some implementations include a third field (`fg;bd;bg`) for default-bd
/// status; we accept both by consulting only the last `;`-separated field.
/// Value `default` (any case) is rejected — no useful signal.
fn parse_colorfgbg(s: &str) -> Option<BgTheme> {
    let bg = s.split(';').next_back()?;
    let bg = bg.trim();
    if bg.is_empty() || bg.eq_ignore_ascii_case("default") {
        return None;
    }
    let n: u8 = bg.parse().ok()?;
    if n > 15 {
        return None;
    }
    if n < 8 {
        Some(BgTheme::Dark)
    } else {
        Some(BgTheme::Light)
    }
}

/// Try OSC 11 background-color query. See spec §3.4 for the full algorithm
/// and timing budget. Returns None if not applicable (no TTY, $STY set,
/// /dev/tty unavailable) or on any I/O / timeout / parse failure.
fn detect_from_osc11() -> Option<BgTheme> {
    if std::env::var_os("STY").is_some() {
        return None;
    }
    if !crate::terminfo::stdout_is_tty() {
        return None;
    }

    let query: &[u8] = if std::env::var_os("TMUX").is_some() {
        // tmux passthrough wrapper. Requires `allow-passthrough on` in
        // tmux 3.3+ (off by default); tmux ≤3.2 enables it by default.
        // When disabled, the wrapped query is silently dropped → read
        // times out → fallback dark (safe).
        //
        // Format: \e P t m u x ; <inner-with-each-ESC-doubled> \e \\
        // Inner: \e]11;?\e\\  →  doubled-ESC form: \e\e]11;?\e\e\\
        b"\x1bPtmux;\x1b\x1b]11;?\x1b\x1b\\\x1b\\"
    } else {
        b"\x1b]11;?\x1b\\"
    };

    let tty = open_dev_tty().ok()?;
    let fd = tty.as_raw_fd();
    let _guard = TtyRawGuard::engage(fd).ok()?;
    write_all_with_timeout(fd, query, OSC11_WRITE_TIMEOUT).ok()?;
    let response = read_until_terminator(fd, OSC11_READ_TIMEOUT).ok()?;
    drain_remaining(fd);
    suppress_query_echo(fd);
    parse_osc11_response(&response)
}

/// Parse a hex channel of 1..=4 ASCII hex nibbles to a 0.0..=1.0 float.
/// `RR` → 0xRR / 0xFF; `RRRR` → 0xRRRR / 0xFFFF. Both forms scale to [0,1].
fn parse_hex_channel(bytes: &[u8]) -> Option<f32> {
    let len = bytes.len();
    if !(1..=4).contains(&len) {
        return None;
    }
    // Channel bytes are guaranteed ASCII hex digits when well-formed.
    // `from_utf8` over ≤4 bytes is bounded; on non-ASCII / non-hex we
    // surface None via the `str::from_utf8` and `from_str_radix` checks.
    let s = std::str::from_utf8(bytes).ok()?;
    let n = u32::from_str_radix(s, 16).ok()?;
    let max: u32 = match len {
        1 => 0xF,
        2 => 0xFF,
        3 => 0xFFF,
        4 => 0xFFFF,
        _ => unreachable!("len bounded by the early check above"),
    };
    #[allow(clippy::cast_precision_loss)]
    // reason: max is at most 0xFFFF (65535), well within f32 mantissa range
    Some(n as f32 / max as f32)
}

/// Rec. 601 weighted luminance with threshold 0.5 (inclusive → Light).
/// Boundary direction (`>=` not `>`) ensures deterministic mapping when
/// float arithmetic lands exactly on 0.5.
fn luminance_to_theme(r: f32, g: f32, b: f32) -> BgTheme {
    let y = 0.299 * r + 0.587 * g + 0.114 * b;
    if y >= 0.5 {
        BgTheme::Light
    } else {
        BgTheme::Dark
    }
}

/// Parse OSC 11 response: `\e]11;rgb:RRRR/GGGG/BBBB<terminator>` or
/// `\e]11;rgb:RR/GG/BB<terminator>`. Terminator is one of BEL (0x07),
/// 7-bit ST start (ESC = 0x1B), or 8-bit C1 ST (0x9C).
///
/// Some terminals (notably tmux relays) strip the leading ESC — we accept
/// both forms by looking for the byte sequence `]11;rgb:`. Byte-wise
/// throughout — no `str::from_utf8` over the whole input, so 0x9C is fine.
fn parse_osc11_response(bytes: &[u8]) -> Option<BgTheme> {
    let prefix = b"]11;rgb:";
    let i = bytes.windows(prefix.len()).position(|w| w == prefix)?;
    let payload = &bytes[i + prefix.len()..];
    let end =
        payload.iter().position(|&b| matches!(b, 0x07 | 0x1B | 0x9C)).unwrap_or(payload.len());
    let triple = &payload[..end];
    let mut parts = triple.split(|&b| b == b'/');
    let r = parse_hex_channel(parts.next()?)?;
    let g = parse_hex_channel(parts.next()?)?;
    let b = parse_hex_channel(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some(luminance_to_theme(r, g, b))
}

use std::fs::OpenOptions;
use std::os::fd::{AsRawFd, BorrowedFd, RawFd};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::poll::{PollFd, PollFlags, PollTimeout};
use nix::sys::termios::{cfmakeraw, tcgetattr, tcsetattr, SetArg, Termios};
use nix::unistd::{read, write};

const OSC11_WRITE_TIMEOUT: Duration = Duration::from_millis(50);
const OSC11_READ_TIMEOUT: Duration = Duration::from_millis(100);
const OSC11_RESPONSE_CAP: usize = 128;

/// Process-wide slot consulted by the bg-detect panic hook to restore
/// `/dev/tty`'s termios. Parallel to `tty_guard::PANIC_RESTORE_STATE`
/// (which restores stdin). Each guard populates its own slot on engage
/// and clears on drop.
static PANIC_RESTORE_STATE: OnceLock<Mutex<Option<(RawFd, Termios)>>> = OnceLock::new();

/// Short-lived raw-mode guard for the bg-detect OSC 11 path. Runs BEFORE
/// the main `tty_guard::TtyGuard` engages, so it manages its own termios
/// snapshot AND its own panic hook (necessary because release builds use
/// `panic = "abort"` — Drop does NOT run on panic).
struct TtyRawGuard {
    fd: RawFd,
    original: Termios,
}

impl TtyRawGuard {
    fn engage(fd: RawFd) -> crate::error::Result<Self> {
        // SAFETY: caller holds the owning `File` for `fd` for the duration
        // of the guard; we only borrow the fd for the termios syscalls.
        // reason: crate-wide policy is `warn(unsafe_code)` with SAFETY
        // comments; allow is scoped to the single borrow.
        #[allow(unsafe_code)]
        let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
        let original = tcgetattr(borrowed)?;

        install_panic_hook(fd, original.clone());

        let mut raw = original.clone();
        cfmakeraw(&mut raw);
        tcsetattr(borrowed, SetArg::TCSANOW, &raw)?;

        Ok(TtyRawGuard { fd, original })
    }
}

impl Drop for TtyRawGuard {
    fn drop(&mut self) {
        // Best-effort restore. Matches `tty_guard::TtyGuard::Drop` pattern:
        // if `tcsetattr` fails on restore the terminal is already lost.
        // SAFETY: while Drop runs, the owning `File` for `self.fd` is still
        // alive in the calling scope (drop order: locals dropped LIFO,
        // guard before File).
        // reason: crate-wide policy is `warn(unsafe_code)` with SAFETY
        // comments; allow is scoped to the single borrow.
        #[allow(unsafe_code)]
        let borrowed = unsafe { BorrowedFd::borrow_raw(self.fd) };
        let _ = tcsetattr(borrowed, SetArg::TCSANOW, &self.original);

        // Clear the panic-hook slot so a later panic does not re-apply a
        // stale termios from this guard's lifetime.
        if let Some(mux) = PANIC_RESTORE_STATE.get() {
            if let Ok(mut g) = mux.lock() {
                *g = None;
            }
        }
    }
}

/// Install (once per process) a panic hook that restores `/dev/tty`'s
/// termios when any thread panics while a bg-detect guard is alive.
/// Subsequent calls only update the shared slot; the hook itself is
/// registered exactly once.
fn install_panic_hook(fd: RawFd, original: Termios) {
    static HOOK_INSTALLED: std::sync::Once = std::sync::Once::new();

    let mux = PANIC_RESTORE_STATE.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = mux.lock() {
        *g = Some((fd, original));
    }

    HOOK_INSTALLED.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if let Some(mux) = PANIC_RESTORE_STATE.get() {
                if let Ok(g) = mux.lock() {
                    if let Some((fd, ref original)) = *g {
                        // SAFETY: the slot is populated only while a
                        // `TtyRawGuard` owns this fd via the calling `File`;
                        // panics during that window mean the `File` is
                        // still alive on the panicking thread's stack.
                        // reason: crate-wide policy is `warn(unsafe_code)`
                        // with SAFETY comments; allow scoped to the borrow.
                        #[allow(unsafe_code)]
                        let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
                        let _ = tcsetattr(borrowed, SetArg::TCSANOW, original);
                    }
                }
            }
            prev(info);
        }));
    });
}

/// Open the controlling terminal as read+write. Unlike `stdin`, `/dev/tty`
/// reliably refers to the process's controlling terminal even if stdin is
/// redirected (`tayf < file`).
fn open_dev_tty() -> std::io::Result<std::fs::File> {
    OpenOptions::new().read(true).write(true).open("/dev/tty")
}

/// Block until the fd is writable or the timeout elapses, then write all
/// bytes. Returns `Err` on poll error, timeout, or partial write.
fn write_all_with_timeout(fd: RawFd, bytes: &[u8], timeout: Duration) -> std::io::Result<()> {
    // SAFETY: caller holds the owning `File` for `fd` for the duration of
    // this call; we only borrow the fd for poll/write syscalls.
    // reason: crate-wide policy is `warn(unsafe_code)` with SAFETY comments;
    // allow is scoped to the single borrow.
    #[allow(unsafe_code)]
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
    let mut pollfds = [PollFd::new(borrowed, PollFlags::POLLOUT)];
    let timeout_arg = poll_timeout_from_duration(timeout);
    let ready = nix::poll::poll(&mut pollfds, timeout_arg)
        .map_err(|e| std::io::Error::other(format!("poll write: {e}")))?;
    if ready == 0 {
        return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "write timeout"));
    }
    let mut written = 0usize;
    while written < bytes.len() {
        let n = write(borrowed, &bytes[written..])
            .map_err(|e| std::io::Error::other(format!("write: {e}")))?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::WriteZero, "write returned 0"));
        }
        written += n;
    }
    Ok(())
}

/// Loop-read from the fd up to `OSC11_RESPONSE_CAP` bytes, stopping when:
/// - A terminator byte (BEL/ESC/0x9C) is observed, OR
/// - The buffer reaches `OSC11_RESPONSE_CAP`, OR
/// - The overall timeout elapses.
///
/// Returns the bytes read on success. Returns `Err` on poll/read failure
/// or timeout-with-empty-buffer.
fn read_until_terminator(fd: RawFd, timeout: Duration) -> std::io::Result<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut buf: Vec<u8> = Vec::with_capacity(64);
    // SAFETY: caller holds the owning `File` for `fd` for the duration of
    // this call; we only borrow the fd for poll/read syscalls.
    // reason: crate-wide policy is `warn(unsafe_code)` with SAFETY comments;
    // allow is scoped to the single borrow.
    #[allow(unsafe_code)]
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
    loop {
        let now = Instant::now();
        if now >= deadline {
            if buf.is_empty() {
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "read timeout"));
            }
            return Ok(buf);
        }
        let remaining = deadline - now;
        let mut pollfds = [PollFd::new(borrowed, PollFlags::POLLIN)];
        let timeout_arg = poll_timeout_from_duration(remaining);
        let ready = nix::poll::poll(&mut pollfds, timeout_arg)
            .map_err(|e| std::io::Error::other(format!("poll read: {e}")))?;
        if ready == 0 {
            if buf.is_empty() {
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "read timeout"));
            }
            return Ok(buf);
        }
        let mut chunk = [0u8; 64];
        let n = read(borrowed.as_raw_fd(), &mut chunk)
            .map_err(|e| std::io::Error::other(format!("read: {e}")))?;
        if n == 0 {
            return Ok(buf);
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.iter().any(|&b| matches!(b, 0x07 | 0x1B | 0x9C)) {
            return Ok(buf);
        }
        if buf.len() >= OSC11_RESPONSE_CAP {
            return Ok(buf);
        }
    }
}

/// `Duration` → `i32` millisecond count saturated at `i32::MAX` for the
/// `nix::poll::poll` API.
fn duration_millis_i32(d: Duration) -> i32 {
    d.as_millis().try_into().unwrap_or(i32::MAX)
}

/// `Duration` → `PollTimeout`, saturating at `PollTimeout::MAX` for any
/// overflow. nix 0.28's `poll` accepts `Into<PollTimeout>`; `i32` only has
/// `TryFrom`, so we wrap here to keep the call sites clean.
fn poll_timeout_from_duration(d: Duration) -> PollTimeout {
    let millis = duration_millis_i32(d);
    PollTimeout::try_from(millis).unwrap_or(PollTimeout::MAX)
}

/// Non-blocking drain of any remaining bytes on `/dev/tty` after a
/// successful OSC 11 response read. Consumes bytes until `read` returns
/// `EAGAIN`/`EWOULDBLOCK` or any other error. Discards the bytes.
///
/// Caller guarantees the OSC 11 read returned Ok before invoking drain —
/// otherwise we would risk swallowing pre-typed user keystrokes.
fn drain_remaining(fd: RawFd) {
    // SAFETY: caller still owns the underlying File for `fd`; we only
    // borrow the fd for the fcntl/read syscalls below.
    // reason: crate-wide policy is `warn(unsafe_code)` with SAFETY comments;
    // allow is scoped to the single borrow.
    #[allow(unsafe_code)]
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
    let Ok(prior_bits) = fcntl(borrowed.as_raw_fd(), FcntlArg::F_GETFL) else {
        return;
    };
    let prior = OFlag::from_bits_truncate(prior_bits);
    if fcntl(borrowed.as_raw_fd(), FcntlArg::F_SETFL(prior | OFlag::O_NONBLOCK)).is_err() {
        return;
    }

    let mut buf = [0u8; 64];
    loop {
        match read(borrowed.as_raw_fd(), &mut buf) {
            // EAGAIN/EWOULDBLOCK lands in `Err(_)`; treat EOF (Ok(0)) and any
            // error as "done draining".
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }

    let _ = fcntl(borrowed.as_raw_fd(), FcntlArg::F_SETFL(prior));
}

/// Write `\r\e[K` (CR + `EraseInLine`) to `/dev/tty` to wipe any literal
/// echo of the OSC 11 query by terminals that don't recognize OSC 11.
/// Best-effort; ignores write errors.
fn suppress_query_echo(fd: RawFd) {
    // SAFETY: caller still owns the underlying File for `fd`; we only
    // borrow the fd for the single write syscall.
    // reason: crate-wide policy is `warn(unsafe_code)` with SAFETY comments;
    // allow is scoped to the single borrow.
    #[allow(unsafe_code)]
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
    let _ = write(borrowed, b"\r\x1b[K");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colorfgbg_two_field_dark() {
        assert_eq!(parse_colorfgbg("15;0"), Some(BgTheme::Dark));
    }

    #[test]
    fn colorfgbg_two_field_light() {
        assert_eq!(parse_colorfgbg("0;15"), Some(BgTheme::Light));
    }

    #[test]
    fn colorfgbg_three_field_with_non_numeric_middle() {
        // Parser must use only the last field; middle is ignored.
        assert_eq!(parse_colorfgbg("0;garbage;15"), Some(BgTheme::Light));
    }

    #[test]
    fn colorfgbg_default_keyword_returns_none() {
        assert_eq!(parse_colorfgbg("0;default"), None);
        assert_eq!(parse_colorfgbg("0;DEFAULT"), None);
    }

    #[test]
    fn colorfgbg_malformed_returns_none() {
        assert_eq!(parse_colorfgbg(""), None);
        assert_eq!(parse_colorfgbg("abc"), None);
        assert_eq!(parse_colorfgbg("0;99"), None);
        assert_eq!(parse_colorfgbg("0;-1"), None);
    }

    #[test]
    fn parse_hex_channel_1_nibble() {
        let v = parse_hex_channel(b"f").unwrap();
        // 0xF / 0xF = 1.0
        assert!((v - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_hex_channel_2_nibble() {
        let v = parse_hex_channel(b"ff").unwrap();
        // 0xFF / 0xFF = 1.0
        assert!((v - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_hex_channel_4_nibble_half() {
        let v = parse_hex_channel(b"7fff").unwrap();
        // 0x7FFF / 0xFFFF ≈ 0.4999847412
        #[allow(clippy::cast_precision_loss)]
        // reason: 0xFFFF (65535) fits exactly in f32 mantissa; test-only constants
        let expected = 0x7FFF_u32 as f32 / 0xFFFF_u32 as f32;
        assert!((v - expected).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_hex_channel_rejects_zero_length_and_too_long() {
        assert_eq!(parse_hex_channel(b""), None);
        assert_eq!(parse_hex_channel(b"abcde"), None);
    }

    #[test]
    fn parse_hex_channel_rejects_non_hex() {
        assert_eq!(parse_hex_channel(b"zz"), None);
    }

    #[test]
    fn luminance_threshold_inclusive_at_half() {
        // RGB (0.5, 0.5, 0.5) → Y = 0.5 (modulo IEEE 754 rounding).
        // Threshold `>= 0.5 → Light` makes the boundary deterministic.
        assert_eq!(luminance_to_theme(0.5, 0.5, 0.5), BgTheme::Light);
    }

    #[test]
    fn luminance_dark_gray_below_threshold() {
        assert_eq!(luminance_to_theme(0.2, 0.2, 0.2), BgTheme::Dark);
    }

    #[test]
    fn parse_osc11_four_digit_hex_dark() {
        let bytes = b"\x1b]11;rgb:0000/0000/0000\x1b\\";
        assert_eq!(parse_osc11_response(bytes), Some(BgTheme::Dark));
    }

    #[test]
    fn parse_osc11_four_digit_hex_light() {
        let bytes = b"\x1b]11;rgb:ffff/ffff/ffff\x1b\\";
        assert_eq!(parse_osc11_response(bytes), Some(BgTheme::Light));
    }

    #[test]
    fn parse_osc11_two_digit_hex_with_bel() {
        let bytes = b"\x1b]11;rgb:ff/ff/ff\x07";
        assert_eq!(parse_osc11_response(bytes), Some(BgTheme::Light));
    }

    #[test]
    fn parse_osc11_eight_bit_c1_st_terminator() {
        // 0x9C alone is invalid UTF-8 — byte-wise parser must accept it.
        let bytes = b"\x1b]11;rgb:ffff/ffff/ffff\x9c";
        assert_eq!(parse_osc11_response(bytes), Some(BgTheme::Light));
    }

    #[test]
    fn parse_osc11_missing_terminator_still_parses() {
        let bytes = b"\x1b]11;rgb:0000/0000/0000";
        assert_eq!(parse_osc11_response(bytes), Some(BgTheme::Dark));
    }

    #[test]
    fn parse_osc11_malformed_returns_none() {
        assert_eq!(parse_osc11_response(b"garbage"), None);
        // Non-hex channel:
        assert_eq!(parse_osc11_response(b"\x1b]11;rgb:zz/zz/zz\x07"), None);
        // Missing field:
        assert_eq!(parse_osc11_response(b"\x1b]11;rgb:ff/ff\x07"), None);
        // Extra field:
        assert_eq!(parse_osc11_response(b"\x1b]11;rgb:ff/ff/ff/ff\x07"), None);
    }
}
