//! Single top-level error type for the tayf crate.
//!
//! All public functions surface errors through this enum so callers (including
//! `main`) can map them to user-facing messages and exit codes in one place.
//! See spec §4.

use std::fmt::Write as _;
use std::io;

/// A single violation in a theme's `[[rules]]` list. Bundled into
/// [`Error::ThemeValidation::errors`] when one or more issues are found.
///
/// Surfaced from [`crate::themes::validate_theme_rules`]. The `Display`
/// impl on [`Error::ThemeValidation`] composes multi-line output from
/// these; for structured access (e.g. machine-readable diagnostics),
/// pattern-match on `Error::ThemeValidation { errors, .. }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeRuleError {
    /// The offending rule's `name` field, copied verbatim from the TOML.
    /// For the `[general]` section violation the sentinel `"<general>"`
    /// is used (angle brackets are rejected by `themes::name_is_valid`,
    /// so this cannot collide with a user rule name).
    pub rule_name: String,
    /// What's wrong.
    pub kind: ThemeRuleErrorKind,
}

/// Classification of a [`ThemeRuleError`]. One variant per validation
/// rule enforced by [`crate::themes::validate_theme_rules`].
///
/// `#[non_exhaustive]` so v0.4+ can add new validation rules (e.g.
/// `RuleNameWhitespace`) without a major version bump.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ThemeRuleErrorKind {
    /// `name` does not match any entry in [`crate::rules::BUILTIN_NAMES`].
    UnknownName,
    /// `pattern` is set — disallowed (themes only override style).
    PatternForbidden,
    /// `enabled = false` is set — disallowed (themes only override style;
    /// use a `[[rules]]` block in the user config to disable a built-in).
    EnabledFalseForbidden,
    /// The theme TOML carries a `[general]` section. Themes only override
    /// style; `[general]` fields belong in the user config. The
    /// accompanying [`ThemeRuleError::rule_name`] is set to the sentinel
    /// `"<general>"`.
    GeneralSectionForbidden,
    /// A `styles` map key is not a positive decimal. v0.3.5 grammar:
    /// `^[1-9][0-9]*$`. The raw user-supplied key is echoed via
    /// [`sanitize_for_display`] so the diagnostic shows what the user typed.
    /// See spec §1.3.3.
    CaptureGroupKeyMalformed {
        /// The raw `styles` map key as written in the user's TOML. Passed
        /// through [`sanitize_for_display`] by the `Display` impl so any
        /// embedded control bytes are escaped before reaching a terminal.
        key: String,
    },
    /// A `styles` map sets key `"0"`. Group 0 is the entire match, which
    /// is already covered by the rule's `style` field. See spec §1.3.1.
    CaptureGroupIndexZeroForbidden,
    /// A `styles` map sets a key whose integer value is `>=` the rule's
    /// regex `captures_len()`. Valid range is `1..=captures_len - 1`.
    /// See spec §1.3.2.
    CaptureGroupIndexOutOfRange {
        /// The offending integer key from the user's `styles` map.
        group: usize,
        /// The rule's regex `captures_len()` (group count + 1).
        captures_len: usize,
    },
}

impl std::fmt::Display for ThemeRuleErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownName => {
                f.write_str("not a built-in name; themes may only override built-ins")
            }
            Self::PatternForbidden => {
                f.write_str("must not set 'pattern' (themes only override style)")
            }
            Self::EnabledFalseForbidden => {
                f.write_str("must not set 'enabled = false' (themes only override style)")
            }
            Self::GeneralSectionForbidden => f.write_str(
                "themes must not set [general] (themes only override style; \
                 use the user config for [general] fields)",
            ),
            Self::CaptureGroupKeyMalformed { key } => write!(
                f,
                "styles.\"{}\": capture-group key must be a positive decimal \
                 (1, 2, ..., N) with no leading zeros",
                sanitize_for_display(key)
            ),
            Self::CaptureGroupIndexZeroForbidden => f.write_str(
                "styles.\"0\": group 0 is the entire match; use the 'style' field instead",
            ),
            Self::CaptureGroupIndexOutOfRange { group, captures_len } => {
                let n = captures_len.saturating_sub(1);
                write!(
                    f,
                    "styles.\"{}\": rule's regex has {} capture group{} (valid: 1..={})",
                    group,
                    n,
                    if n == 1 { "" } else { "s" },
                    n
                )
            }
        }
    }
}

/// All recoverable errors produced by tayf.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Could not determine which shell to launch.
    #[error("could not determine shell: {0}. Set $SHELL or pass --shell <path>.")]
    ShellDiscovery(String),

    /// PTY operation failed (open, spawn, read, write, resize).
    #[error(
        "PTY operation failed: {0}. If your terminal supports PTY allocation, please file a bug at https://github.com/beraartuc/tayf/issues."
    )]
    Pty(#[from] io::Error),

    /// Terminal control failed (termios get/set, ioctl).
    #[error("terminal control failed: {0}. tayf must be launched from a real terminal; piping stdin or running without a TTY is not supported in v0.1.")]
    Tty(#[from] nix::errno::Errno),

    /// Built-in or user regex failed to compile.
    #[error("regex compilation failed: {0}. Check the pattern syntax.")]
    RegexCompile(#[from] regex::Error),

    /// Signal handler installation failed.
    #[error(
        "signal installation failed: {0}. Try running again; if persistent, please file a bug."
    )]
    Signal(#[source] io::Error),

    /// Failed to load or validate the user TOML config.
    ///
    /// `line` is 1-based when available; pass `0` for errors with no line
    /// context (path resolution, size limit, IO). `0` was chosen over
    /// `Option<NonZeroUsize>` because thiserror's format-string support is
    /// terser this way; the sentinel is constant across the codebase.
    ///
    /// **Display contract:** both the `path` and `message` fields pass through
    /// `sanitize_for_display` in the `Display` impl so that any user-supplied
    /// content echoed back (e.g. a color string from a config rule, or a
    /// hostile `XDG_CONFIG_HOME`) cannot smuggle an escape sequence onto the
    /// user's terminal — CLAUDE.md §3 invariant. Callers that read these
    /// fields directly (e.g. for structured logging) get the raw bytes;
    /// format through `Display` or sanitize yourself before printing to a
    /// terminal.
    #[error("config error in {}{}: {}", sanitize_for_display(path), line_suffix(*line), sanitize_for_display(message))]
    Config {
        /// Absolute path to the config file the error originated from.
        path: String,
        /// 1-based line number, or `0` for "no line context".
        line: usize,
        /// Human-readable description ending in actionable guidance.
        message: String,
    },

    /// Returned when `--theme <NAME>` or `[general] theme = "..."` names a theme
    /// that is not in the embedded registry and not found on disk. The available
    /// list merges built-ins with disk-discovered names at construction time,
    /// minus any case-insensitive collisions (built-ins always win the name).
    #[error("theme '{}' not found; available: {}. Run with --theme <name>.", sanitize_for_display(name), available.join(", "))]
    Theme { name: String, available: Vec<String> },

    /// File-watcher operation failed (start, register path, event channel).
    ///
    /// Uses `#[source]` rather than `#[from]` so call sites in the watcher
    /// and reload orchestrator construct `Error::Watch(...)` explicitly — the
    /// conversion is part of the contract there, not an implicit coercion.
    #[error("file watcher error: {0}")]
    Watch(#[source] notify::Error),

    /// One or more validation errors collected from a single pass over a
    /// theme's `[[rules]]` list. `theme` is the requested theme name
    /// (matching `--theme <name>` or `[general] theme`). `source_path` is
    /// the embedded synthetic path for shipped presets
    /// (e.g. `<embedded:theme/dark>`) or the absolute canonical disk path
    /// for disk-loaded themes (e.g. `/home/u/.config/tayf/themes/mine.toml`).
    ///
    /// **Display contract:** the `source_path`, each `rule_name`, and the
    /// theme name itself pass through [`sanitize_for_display`] in the
    /// `Display` impl so any user-supplied control byte (a hostile config
    /// path or a rule name with `\x1b`) cannot smuggle a terminal control
    /// sequence onto the user's terminal — CLAUDE.md §3 invariant.
    #[error("{}", format_theme_validation(theme, source_path, errors))]
    ThemeValidation {
        /// The user-facing theme name (`--theme <name>` or
        /// `[general] theme`). Asymmetric with `Error::Theme.name` —
        /// `theme` reads more naturally inside the Display string.
        theme: String,
        /// `<embedded:theme/{name}>` for shipped presets, canonical disk
        /// path for disk-loaded themes.
        source_path: String,
        /// At least one entry; an empty Vec would be a constructor bug.
        errors: Vec<ThemeRuleError>,
    },

    /// A line exceeded the buffer cap; flushed as-is without rule application.
    ///
    /// **Non-fatal — INVARIANT:** This variant must only be constructed for
    /// `crate::log::warn_msg!` logging, never returned from `Result` to propagate via
    /// `?`. The line-buffer module signals overflow through the dedicated
    /// `(Vec<_>, Option<Error>)` return shape (spec §5 / Task 5), keeping this
    /// variant out of any normal control-flow path. Future contributors who
    /// find themselves writing `return Err(Error::BufferOverflow { .. })`
    /// should reach for a `Warning` type instead.
    #[error("line buffer exceeded {cap} bytes; flushing as-is")]
    BufferOverflow {
        /// The cap (in bytes) that was exceeded.
        cap: usize,
    },
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, Error>;

fn line_suffix(line: usize) -> String {
    if line == 0 {
        String::new()
    } else {
        format!(":{line}")
    }
}

/// Replace ASCII control bytes in a diagnostic message with their `\xNN`
/// escape form so a hostile config string (e.g. `"\x1b[2J"` in a color value)
/// cannot execute as a terminal control sequence when the error is printed
/// to stderr. Preserves common whitespace (`\n`, `\t`, regular space) since
/// those round-trip safely through a terminal.
fn sanitize_for_display(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    for ch in message.chars() {
        // `is_control()` covers ASCII C0 (0x00..=0x1F + 0x7F) AND Unicode
        // C1 (U+0080..U+009F). U+009B is the 8-bit CSI introducer — a hostile
        // config string could otherwise smuggle "\u{009B}2J" past the gate.
        if ch.is_control() && ch != '\n' && ch != '\t' {
            // `write!` into a String is infallible; the discard is
            // explicit per the plan's clippy::format_push_string fallback.
            let _ = write!(out, "\\x{:02x}", ch as u32);
        } else {
            out.push(ch);
        }
    }
    out
}

fn format_theme_validation(theme: &str, source_path: &str, errors: &[ThemeRuleError]) -> String {
    let n = errors.len();
    let plural = if n == 1 { "error" } else { "errors" };
    let mut out = format!(
        "theme '{theme}' (loaded from {path}) has {n} validation {plural}:",
        theme = sanitize_for_display(theme),
        path = sanitize_for_display(source_path),
    );
    for e in errors {
        // Literal single quotes around rule_name match Error::Theme's
        // Display contract (`theme 'foo' not found`). `{name:?}` Debug
        // formatting would produce double quotes and break visual
        // consistency.
        let _ = write!(
            out,
            "\n  - rule '{name}': {msg}",
            name = sanitize_for_display(&e.rule_name),
            msg = e.kind,
        );
    }
    out
}

impl Error {
    /// Build a [`Error::Config`] from a `toml::de::Error`, extracting the
    /// 1-based line number when the source span is available.
    #[allow(clippy::needless_pass_by_value)]
    // reason: the diagnostic is single-shot — callers obtain `err` from
    // `toml::from_str(..).unwrap_err()` and never reuse it. Taking by value
    // matches that lifecycle and keeps the signature stable for Task 4.
    pub(crate) fn config_from_toml(path: String, source: &str, err: toml::de::Error) -> Self {
        let line = err.span().map_or(0, |range| line_from_offset(source, range.start));
        Error::Config { path, line, message: err.message().to_string() }
    }

    /// Build a [`Error::Config`] for a regex compile failure inside a named
    /// rule. `line` is 0 unless the caller already knows the source line.
    #[allow(clippy::needless_pass_by_value)]
    // reason: `regex::Error` is the single-shot return of `Regex::new(..)`;
    // callers move it in directly. Matches the by-value signature established
    // for `config_from_toml` so the two construction helpers are symmetric.
    pub(crate) fn config_regex(path: String, rule_name: &str, source: regex::Error) -> Self {
        Error::Config {
            path,
            line: 0,
            message: format!("rule '{rule_name}': {source}. Check the pattern syntax."),
        }
    }
}

#[allow(clippy::naive_bytecount)]
// reason: pulling the `bytecount` crate for a one-shot diagnostic helper
// violates the dependency-minimalism policy; config errors are not on any
// hot path and the linear scan is bounded by the 1 MiB config size cap.
fn line_from_offset(source: &str, offset: usize) -> usize {
    // Count newline bytes before `offset`. Operates on `.as_bytes()` rather
    // than slicing `&str` so a non-char-boundary `offset` can never panic —
    // CLAUDE.md §2 ("no panics in library code") applies even when current
    // callers happen to pass char-aligned offsets.
    let upper = offset.min(source.len());
    source.as_bytes()[..upper].iter().filter(|&&b| b == b'\n').count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_discovery_message_includes_remediation() {
        let e = Error::ShellDiscovery("no $SHELL".into());
        let msg = e.to_string();
        assert!(msg.contains("could not determine shell"));
        assert!(msg.contains("--shell"), "remediation hint required: {msg}");
    }

    #[test]
    fn pty_message_includes_remediation() {
        let e: Error = io::Error::from(io::ErrorKind::PermissionDenied).into();
        let msg = e.to_string();
        assert!(msg.contains("PTY operation failed"));
        assert!(msg.contains("file a bug"), "remediation hint required: {msg}");
    }

    #[test]
    fn tty_message_includes_remediation() {
        let e: Error = nix::errno::Errno::EIO.into();
        let msg = e.to_string();
        assert!(msg.contains("terminal control failed"));
        assert!(msg.contains("real terminal") || msg.contains("not supported"));
    }

    #[test]
    fn regex_message_includes_remediation() {
        // Build the pattern at runtime so clippy's `invalid_regex` lint
        // (which inspects string literals) does not flag the test source.
        let pattern = String::from("(") + "invalid";
        let bad = regex::Regex::new(&pattern).unwrap_err();
        let e: Error = bad.into();
        let msg = e.to_string();
        assert!(msg.contains("regex compilation failed"));
        assert!(msg.contains("pattern syntax"));
    }

    #[test]
    fn signal_message_includes_remediation() {
        let e = Error::Signal(io::Error::from(io::ErrorKind::Other));
        let msg = e.to_string();
        assert!(msg.contains("signal installation failed"));
        assert!(msg.contains("file a bug"));
    }

    #[test]
    fn buffer_overflow_message_is_descriptive() {
        let e = Error::BufferOverflow { cap: 65536 };
        let msg = e.to_string();
        assert!(msg.contains("65536"));
        assert!(msg.contains("flushing as-is"));
    }

    #[test]
    fn pty_from_io_error_preserves_source_chain() {
        use std::error::Error as _;
        let io = io::Error::from(io::ErrorKind::PermissionDenied);
        let e: Error = io.into();
        assert!(e.source().is_some(), "Pty variant must carry source");
    }

    #[test]
    fn signal_preserves_source_chain_via_source_attr() {
        use std::error::Error as _;
        let e = Error::Signal(io::Error::from(io::ErrorKind::Other));
        assert!(e.source().is_some(), "Signal variant must carry source via #[source]");
    }

    #[test]
    fn from_io_error_routes_to_pty_variant() {
        let io = io::Error::from(io::ErrorKind::PermissionDenied);
        let e: Error = io.into();
        assert!(matches!(e, Error::Pty(_)));
    }

    #[test]
    fn config_message_includes_path_line_and_message() {
        let e = Error::Config {
            path: "/home/u/.config/tayf/config.toml".into(),
            line: 12,
            message: "unknown color name 'turquoise'".into(),
        };
        let msg = e.to_string();
        assert!(msg.contains("/home/u/.config/tayf/config.toml"));
        assert!(msg.contains("12"));
        assert!(msg.contains("turquoise"));
    }

    #[test]
    fn config_message_omits_line_when_zero() {
        let e = Error::Config {
            path: "/etc/tayf.toml".into(),
            line: 0,
            message: "file too large: 2097152 bytes (max 1048576)".into(),
        };
        let msg = e.to_string();
        assert!(msg.contains("/etc/tayf.toml"));
        assert!(!msg.contains(":0:"), "line 0 sentinel must not surface in message: {msg}");
        assert!(msg.contains("too large"));
    }

    #[test]
    fn config_from_toml_parse_error_carries_line() {
        // Unterminated inline-table — guaranteed parse failure in toml 0.9.
        let bad = "rules = [ { unterminated\n";
        let err: toml::de::Error = toml::from_str::<toml::Table>(bad).unwrap_err();
        let cfg = Error::config_from_toml("/tmp/cfg.toml".into(), bad, err);
        let msg = cfg.to_string();
        assert!(msg.contains("/tmp/cfg.toml"));
    }

    #[test]
    fn config_message_escapes_control_bytes_to_prevent_terminal_injection() {
        // A hostile config string echoed back in an error message must not
        // execute as a terminal control sequence when Display'd to stderr.
        let e = Error::Config {
            path: "/tmp/cfg.toml".into(),
            line: 7,
            message: "rule 'evil': fg: unknown color name '\x1b[2J\x1b[H'".into(),
        };
        let rendered = e.to_string();
        assert!(!rendered.contains('\x1b'), "raw ESC must not survive Display: {rendered:?}");
        assert!(rendered.contains("\\x1b"), "ESC must be escaped as \\x1b: {rendered:?}");
        // Newline and tab pass through unchanged (safe whitespace).
        let e2 = Error::Config { path: "/x".into(), line: 0, message: "ok\nfine\there".into() };
        let r2 = e2.to_string();
        assert!(r2.contains("ok\nfine\there"), "safe whitespace must pass: {r2:?}");
    }

    #[test]
    fn config_path_escapes_control_bytes_too() {
        // Sanitization gate must cover `path` symmetrically with `message`.
        // A hostile XDG_CONFIG_HOME or --config arg could contain ESC.
        let e = Error::Config {
            path: "/tmp/\x1b[2J/cfg.toml".into(),
            line: 0,
            message: "anything".into(),
        };
        let rendered = e.to_string();
        assert!(!rendered.contains('\x1b'), "raw ESC must not survive in path: {rendered:?}");
        assert!(rendered.contains("\\x1b"), "ESC must be escaped as \\x1b in path: {rendered:?}");
    }

    #[test]
    fn config_message_escapes_c1_control_introducer() {
        // U+009B is the 8-bit CSI introducer — same threat class as ESC [.
        // Regression guard for the `is_ascii_control` -> `is_control` fix.
        let e = Error::Config { path: "/x".into(), line: 0, message: "fg: '\u{009b}2J'".into() };
        let rendered = e.to_string();
        assert!(
            !rendered.contains('\u{009b}'),
            "raw U+009B must not survive Display: {rendered:?}"
        );
        assert!(rendered.contains("\\x9b"), "U+009B must be escaped as \\x9b: {rendered:?}");
    }

    #[test]
    fn watch_error_display_is_helpful() {
        let inner = notify::Error::generic("permission denied");
        let err = crate::error::Error::Watch(inner);
        let msg = err.to_string();
        assert!(msg.contains("file watcher error"));
        assert!(msg.contains("permission denied"));
    }

    #[test]
    fn theme_display_shows_name_and_alternatives() {
        let e = Error::Theme { name: "foo".into(), available: vec!["dark".into(), "light".into()] };
        let s = e.to_string();
        assert!(s.contains("'foo'"), "should quote unknown name; got: {s}");
        assert!(s.contains("dark"), "should list 'dark'; got: {s}");
        assert!(s.contains("light"), "should list 'light'; got: {s}");
        assert!(s.contains("--theme"), "should suggest --theme; got: {s}");
    }

    #[test]
    fn theme_display_sanitizes_control_bytes_in_name() {
        // Defense-in-depth: a hostile `--theme $'\x1b[2J'` must not let an ESC
        // sequence reach the terminal via the error path. Mirrors the gate that
        // `Error::Config` applies to `path` and `message`.
        let e = Error::Theme {
            name: "\x1b[2Jevil".into(),
            available: vec!["dark".into(), "light".into()],
        };
        let s = e.to_string();
        assert!(!s.contains('\x1b'), "raw ESC must not survive Display; got: {s:?}");
        assert!(s.contains("\\x1b"), "ESC must appear as \\x1b escape; got: {s:?}");
    }

    #[test]
    fn theme_rule_error_kind_display_unknown_name() {
        let s = ThemeRuleErrorKind::UnknownName.to_string();
        assert!(s.contains("not a built-in name"), "got: {s}");
        assert!(s.contains("themes may only override"), "got: {s}");
    }

    #[test]
    fn theme_rule_error_kind_display_pattern_forbidden() {
        let s = ThemeRuleErrorKind::PatternForbidden.to_string();
        assert!(s.contains("must not set 'pattern'"), "got: {s}");
        assert!(s.contains("themes only override style"), "got: {s}");
    }

    #[test]
    fn theme_rule_error_kind_display_enabled_false_forbidden() {
        let s = ThemeRuleErrorKind::EnabledFalseForbidden.to_string();
        assert!(s.contains("must not set 'enabled = false'"), "got: {s}");
    }

    #[test]
    fn theme_rule_error_kind_display_general_section_forbidden() {
        let s = ThemeRuleErrorKind::GeneralSectionForbidden.to_string();
        assert!(s.contains("themes must not set [general]"), "got: {s}");
        assert!(s.contains("use the user config"), "got: {s}");
    }

    #[test]
    fn theme_rule_error_kind_capture_group_key_malformed_display() {
        let k = ThemeRuleErrorKind::CaptureGroupKeyMalformed { key: "01".to_owned() };
        let s = k.to_string();
        assert!(
            s.contains("styles.\"01\"")
                && s.contains("capture-group key must be a positive decimal"),
            "got: {s}"
        );
    }

    #[test]
    fn theme_rule_error_kind_capture_group_index_zero_forbidden_display() {
        let s = ThemeRuleErrorKind::CaptureGroupIndexZeroForbidden.to_string();
        assert!(
            s.contains("styles.\"0\"") && s.contains("entire match") && s.contains("'style' field"),
            "got: {s}"
        );
    }

    #[test]
    fn theme_rule_error_kind_capture_group_index_out_of_range_display() {
        let k = ThemeRuleErrorKind::CaptureGroupIndexOutOfRange { group: 7, captures_len: 4 };
        let s = k.to_string();
        assert!(
            s.contains("styles.\"7\"")
                && s.contains("3 capture groups")
                && s.contains("valid: 1..=3"),
            "got: {s}"
        );
    }

    #[test]
    fn theme_rule_error_kind_out_of_range_singular_one_group() {
        let k = ThemeRuleErrorKind::CaptureGroupIndexOutOfRange { group: 5, captures_len: 2 };
        let s = k.to_string();
        assert!(s.contains("1 capture group ") && !s.contains("groups"), "expect singular: {s}");
        assert!(s.contains("valid: 1..=1"));
    }

    #[test]
    fn theme_rule_error_kind_capture_group_key_sanitized() {
        let k = ThemeRuleErrorKind::CaptureGroupKeyMalformed { key: "\x07abc".to_owned() };
        let s = k.to_string();
        // sanitize_for_display escapes control bytes — the literal \x07 must not appear.
        assert!(!s.as_bytes().contains(&0x07), "raw control byte leaked: {s:?}");
    }

    #[test]
    fn theme_rule_error_kind_implements_clone_not_copy() {
        // Sanity: ThemeRuleErrorKind no longer impl Copy (two new variants carry payload).
        // The `assert_copy` test from v0.3.4 is removed; assert_clone retained below.
        fn assert_clone<T: Clone>() {}
        assert_clone::<ThemeRuleErrorKind>();
    }

    fn rule_err(name: &str, kind: ThemeRuleErrorKind) -> ThemeRuleError {
        ThemeRuleError { rule_name: name.to_owned(), kind }
    }

    #[test]
    fn theme_validation_display_includes_theme_name_and_path() {
        let e = Error::ThemeValidation {
            theme: "mine".into(),
            source_path: "/home/u/.config/tayf/themes/mine.toml".into(),
            errors: vec![rule_err("log_level", ThemeRuleErrorKind::PatternForbidden)],
        };
        let s = e.to_string();
        assert!(s.contains("theme 'mine'"), "should quote theme; got: {s}");
        assert!(
            s.contains("/home/u/.config/tayf/themes/mine.toml"),
            "should include path; got: {s}"
        );
    }

    #[test]
    fn theme_validation_display_uses_single_quotes_around_rule_name() {
        let e = Error::ThemeValidation {
            theme: "x".into(),
            source_path: "<embedded:theme/x>".into(),
            errors: vec![rule_err("ipv4", ThemeRuleErrorKind::PatternForbidden)],
        };
        let s = e.to_string();
        assert!(s.contains("rule 'ipv4'"), "literal single quotes; got: {s}");
        assert!(!s.contains("rule \"ipv4\""), "must not be Debug quoted; got: {s}");
    }

    #[test]
    fn theme_validation_display_uses_two_space_indent_for_rule_lines() {
        let e = Error::ThemeValidation {
            theme: "x".into(),
            source_path: "<x>".into(),
            errors: vec![rule_err("a", ThemeRuleErrorKind::UnknownName)],
        };
        let s = e.to_string();
        assert!(s.contains("\n  - rule 'a':"), "2-space indent + dash; got: {s}");
    }

    #[test]
    fn theme_validation_display_singular_vs_plural() {
        let one = Error::ThemeValidation {
            theme: "x".into(),
            source_path: "<x>".into(),
            errors: vec![rule_err("a", ThemeRuleErrorKind::UnknownName)],
        };
        let many = Error::ThemeValidation {
            theme: "x".into(),
            source_path: "<x>".into(),
            errors: vec![
                rule_err("a", ThemeRuleErrorKind::UnknownName),
                rule_err("b", ThemeRuleErrorKind::PatternForbidden),
            ],
        };
        assert!(one.to_string().contains("1 validation error:"), "{one}");
        assert!(!one.to_string().contains("1 validation errors:"), "no plural");
        assert!(many.to_string().contains("2 validation errors:"), "{many}");
    }

    #[test]
    fn theme_validation_display_lists_each_kind_message() {
        let e = Error::ThemeValidation {
            theme: "x".into(),
            source_path: "<x>".into(),
            errors: vec![
                rule_err("u", ThemeRuleErrorKind::UnknownName),
                rule_err("p", ThemeRuleErrorKind::PatternForbidden),
                rule_err("e", ThemeRuleErrorKind::EnabledFalseForbidden),
                rule_err("<general>", ThemeRuleErrorKind::GeneralSectionForbidden),
            ],
        };
        let s = e.to_string();
        assert!(s.contains("not a built-in name"), "UnknownName text: {s}");
        assert!(s.contains("must not set 'pattern'"), "PatternForbidden text: {s}");
        assert!(s.contains("must not set 'enabled = false'"), "EnabledFalseForbidden: {s}");
        assert!(s.contains("themes must not set [general]"), "GeneralSection: {s}");
    }

    #[test]
    fn theme_validation_display_sanitizes_control_bytes_in_path() {
        let e = Error::ThemeValidation {
            theme: "x".into(),
            source_path: "/tmp/\x1b[2J/x.toml".into(),
            errors: vec![rule_err("a", ThemeRuleErrorKind::UnknownName)],
        };
        let s = e.to_string();
        assert!(!s.contains('\x1b'), "raw ESC must not survive: {s:?}");
        assert!(s.contains("\\x1b"), "ESC must be escaped: {s:?}");
    }

    #[test]
    fn theme_validation_display_sanitizes_control_bytes_in_rule_name() {
        let e = Error::ThemeValidation {
            theme: "x".into(),
            source_path: "<x>".into(),
            errors: vec![rule_err("evil\x1b[2J", ThemeRuleErrorKind::UnknownName)],
        };
        let s = e.to_string();
        assert!(!s.contains('\x1b'), "raw ESC must not survive: {s:?}");
        assert!(s.contains("\\x1b"), "ESC must be escaped: {s:?}");
    }

    #[test]
    fn theme_validation_display_sanitizes_control_bytes_in_theme_name() {
        let e = Error::ThemeValidation {
            theme: "evil\x1b[2J".into(),
            source_path: "<x>".into(),
            errors: vec![rule_err("a", ThemeRuleErrorKind::UnknownName)],
        };
        let s = e.to_string();
        assert!(!s.contains('\x1b'), "raw ESC must not survive: {s:?}");
        assert!(s.contains("\\x1b"), "ESC must be escaped: {s:?}");
    }
}
