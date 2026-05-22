//! Tiny env-gated logger for tayf.
//!
//! Replaces the `tracing` + `tracing-subscriber` dependency pair (which
//! pulled in a second copy of `regex-automata` and ~10 other crates for
//! one `warn!` call site). Activated by setting `TAYF_LOG=<level>` in
//! the environment before launch; emits to stderr.
//!
//! Levels: `off` (default), `warn`, `info`, `debug`, `trace`.
//!
//! Initialization is latched by `std::sync::Once`: only the first call
//! to [`init_from_env`] (or its `_with` variant) has effect. Subsequent
//! calls are no-ops, so it is safe to invoke from `main` on every entry.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Once;

/// Log severity. Values are ordered so that `level >= min` is the
/// "enabled" predicate (higher value = more verbose).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum LogLevel {
    Off = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
    Trace = 4,
}

static LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Off as u8);
static INIT: Once = Once::new();

/// Initialize the logger from the `TAYF_LOG` environment variable.
///
/// Idempotent: only the first call has effect.
pub(crate) fn init_from_env() {
    let raw = std::env::var("TAYF_LOG").ok();
    init_from_env_with(raw.as_deref().unwrap_or(""));
}

/// Internal init taking the level string directly. Called only by
/// [`init_from_env`]; kept `pub(crate)` so future test scaffolding can
/// drive the latch without touching the process environment.
pub(crate) fn init_from_env_with(s: &str) {
    INIT.call_once(|| {
        let lvl = parse_level(s).unwrap_or(LogLevel::Off);
        LEVEL.store(lvl as u8, Ordering::Relaxed);
    });
}

/// Current latched level. Useful for diagnostics and tests.
#[allow(dead_code)]
// reason: exposed for future diagnostic call sites and parity with the
// previous `tracing`-based API; the lib does not currently consult the
// level outside of the `enabled` gate inside `emit`.
pub(crate) fn current_level() -> LogLevel {
    match LEVEL.load(Ordering::Relaxed) {
        0 => LogLevel::Off,
        1 => LogLevel::Warn,
        2 => LogLevel::Info,
        3 => LogLevel::Debug,
        _ => LogLevel::Trace,
    }
}

/// `true` if the current level permits emitting `min`-severity records.
pub(crate) fn enabled(min: LogLevel) -> bool {
    LEVEL.load(Ordering::Relaxed) >= min as u8
}

fn parse_level(s: &str) -> Option<LogLevel> {
    if s.is_empty() {
        return None;
    }
    match s.trim().to_ascii_lowercase().as_str() {
        "off" => Some(LogLevel::Off),
        "warn" => Some(LogLevel::Warn),
        "info" => Some(LogLevel::Info),
        "debug" => Some(LogLevel::Debug),
        "trace" => Some(LogLevel::Trace),
        _ => None,
    }
}

/// Emit a record at `level`. Public-in-crate so the [`warn_msg!`] macro
/// can call it from any module; not intended for direct use.
#[doc(hidden)]
pub(crate) fn emit(level: LogLevel, args: std::fmt::Arguments<'_>) {
    if !enabled(level) {
        return;
    }
    let tag = match level {
        LogLevel::Off => return,
        LogLevel::Warn => "warn",
        LogLevel::Info => "info",
        LogLevel::Debug => "debug",
        LogLevel::Trace => "trace",
    };
    eprintln!("tayf [{tag}] {args}");
}

/// Emit a `warn`-level record. Format-string syntax matches `eprintln!`.
///
/// Named `warn_msg` rather than `warn` to avoid colliding with the
/// built-in `#[warn(...)]` lint attribute (`pub(crate) use warn;` is
/// ambiguous with the attribute name).
macro_rules! warn_msg {
    ($($arg:tt)*) => {
        $crate::log::emit($crate::log::LogLevel::Warn, format_args!($($arg)*))
    };
}

pub(crate) use warn_msg;

#[cfg(test)]
mod tests {
    use super::{parse_level, LogLevel};

    #[test]
    fn parses_level_strings() {
        assert_eq!(parse_level("off"), Some(LogLevel::Off));
        assert_eq!(parse_level("warn"), Some(LogLevel::Warn));
        assert_eq!(parse_level("WARN"), Some(LogLevel::Warn));
        assert_eq!(parse_level("Info"), Some(LogLevel::Info));
        assert_eq!(parse_level("debug"), Some(LogLevel::Debug));
        assert_eq!(parse_level("trace"), Some(LogLevel::Trace));
        assert_eq!(parse_level("  warn  "), Some(LogLevel::Warn));
        assert_eq!(parse_level(""), None);
        assert_eq!(parse_level("bogus"), None);
    }

    #[test]
    fn enabled_is_monotonic() {
        // The level comparison logic the `emit` gate depends on:
        // a more-verbose level must compare greater than a less-verbose one.
        // Direct value comparison avoids touching the global `Once`.
        assert!((LogLevel::Off as u8) < (LogLevel::Warn as u8));
        assert!((LogLevel::Warn as u8) < (LogLevel::Info as u8));
        assert!((LogLevel::Info as u8) < (LogLevel::Debug as u8));
        assert!((LogLevel::Debug as u8) < (LogLevel::Trace as u8));
    }
}
