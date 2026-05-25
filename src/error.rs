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
    /// A `styles` map key references a regex group name that is not present
    /// in the rule's compiled regex. `name` is the user-supplied key
    /// (sanitized by the `Display` impl). `available` lists the regex's
    /// actual named groups in `capture_names()` positional order
    /// (left-to-right, `None` filtered out). An empty `available` means
    /// the regex has no named groups at all (the regex's capture groups
    /// are all positional, e.g., a user rule that uses `(...)` without
    /// `(?P<name>...)`). See spec §2.4 + reviewer I-2/I-4.
    CaptureGroupNameUnknown {
        /// The raw `styles` map key as written in the user's TOML
        /// (sanitized in the `Display` impl).
        name: String,
        /// Named groups in the regex, in `capture_names()` positional
        /// order (left-to-right). Empty when the regex has no named groups.
        available: Vec<String>,
    },
    /// A `styles` map defines two keys that resolve to the same capture-
    /// group index: one numeric (positional, e.g., `"1"`) and one named
    /// (e.g., `"scheme"`). Set exactly one. Within a single regex the
    /// `regex` crate forbids duplicate named groups, so this variant only
    /// arises from a positional/named clash. See spec §2.4 + reviewer I-3.
    CaptureGroupDuplicateTarget {
        /// The numeric (positional) form of the key (e.g., `"1"`).
        positional: String,
        /// The named form of the key (e.g., `"scheme"`).
        named: String,
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
                if n == 0 {
                    write!(
                        f,
                        "styles.\"{group}\": rule's regex has no capture groups; styles cannot be set"
                    )
                } else {
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
            Self::CaptureGroupNameUnknown { name, available } => {
                if available.is_empty() {
                    write!(
                        f,
                        "styles.\"{}\": rule's regex has no named capture groups",
                        sanitize_for_display(name)
                    )
                } else {
                    write!(
                        f,
                        "styles.\"{}\": rule's regex has no capture group named '{}' (available: {})",
                        sanitize_for_display(name),
                        sanitize_for_display(name),
                        available
                            .iter()
                            .map(|s| sanitize_for_display(s))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            }
            Self::CaptureGroupDuplicateTarget { positional, named } => {
                // `positional` is grammar-validated numeric (1..N) — parsing
                // here is safe; emit as integer so the diagnostic reads
                // "index N" without extra quoting. If parsing somehow fails
                // (shouldn't, given grammar gate upstream), fall back to the
                // sanitized raw string.
                let idx_display: String = positional
                    .parse::<usize>()
                    .map_or_else(|_| sanitize_for_display(positional), |n| n.to_string());
                write!(
                    f,
                    "styles.\"{}\" and styles.{} target the same capture group (index {}); set exactly one",
                    sanitize_for_display(positional),
                    sanitize_for_display(named),
                    idx_display
                )
            }
        }
    }
}

/// Per-profile load/parse/compile failure classification.
///
/// Wrapped by [`Error::Profile`]; for collected per-rule validation
/// failures see [`Error::ProfileValidation`] + [`ProfileRuleError`].
///
/// `#[non_exhaustive]` so future profile load failure modes can be added
/// without a major version bump. No `Clone` derive — the variants carry
/// owned `String` messages captured at construction time (matching the
/// `Error` enum, which also does not derive `Clone`).
#[derive(Debug)]
#[non_exhaustive]
pub enum ProfileErrorKind {
    /// Neither disk nor embedded source resolved the profile name.
    /// `searched` lists the absolute paths that were attempted, in
    /// order, for the diagnostic.
    NotFound {
        /// Candidate paths that were tried, in discovery order.
        searched: Vec<std::path::PathBuf>,
    },
    /// TOML deserialization failure on the profile source. `message` is
    /// captured at construction via `e.to_string()` so the variant does
    /// not couple the public API to `toml::de::Error`.
    ParseError {
        /// `toml::de::Error::to_string()` snapshot.
        message: String,
    },
    /// `std::fs::canonicalize` or downstream path-safety checks failed.
    /// `message` is captured at construction via `e.to_string()`.
    PathCanonicalization {
        /// The non-canonical path the check was attempted against.
        path: std::path::PathBuf,
        /// `std::io::Error::to_string()` snapshot.
        message: String,
    },
    /// `regex::Regex::new` rejected an `append_rules` entry's pattern.
    /// Profile authors fix one pattern at a time; fail-fast (not
    /// collected). Pattern bytes go through [`sanitize_for_display`] in
    /// the wrapper to honour CLAUDE.md §3 BEL-leak invariant.
    RegexCompile {
        /// The offending profile rule's `name` field.
        rule_name: String,
        /// The raw pattern (sanitized by the Display wrapper).
        pattern: String,
        /// `regex::Error::to_string()` snapshot.
        message: String,
    },
}

/// A single per-rule validation failure inside a profile. Bundled into
/// [`Error::ProfileValidation::errors`] when one or more issues are found.
///
/// Surfaced from [`crate::profiles::validate_profile`] (Phase 1) and the
/// capture-group key dispatch path in `Compiled::load_with_theme` (Phase 2,
/// landing in v0.5.2 Phase 3 work).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRuleError {
    /// The offending rule's `name` field, copied verbatim from the TOML.
    /// Sentinels: `"<rules>"` for [`ProfileRuleErrorKind::RuleUnknown`]
    /// (where the offending entry is a whitelist name, not an
    /// `append_rules` entry); `"<theme>"` for
    /// [`ProfileRuleErrorKind::ThemeNameInvalid`].
    pub rule_name: String,
    /// What's wrong.
    pub kind: ProfileRuleErrorKind,
}

/// Classification of a [`ProfileRuleError`].
///
/// Phase 1 ([`crate::profiles::validate_profile`]) collects the first
/// five variants. Phase 2 (capture-group key dispatch in
/// `Compiled::load_with_theme`) collects [`Self::StylesKey`] via the
/// existing capture-group key validation path, reusing
/// [`ThemeRuleErrorKind`] for byte-equal Display semantics across all
/// rule sources.
///
/// `#[non_exhaustive]` so future profile validation rules can be added
/// without a major version bump.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProfileRuleErrorKind {
    /// A `rules` whitelist entry names something that is not a built-in
    /// rule. `name` is the offending entry; `known` is the list of
    /// built-in names in alphabetical order, surfaced in the Display
    /// for a pedagogical diagnostic.
    RuleUnknown {
        /// The unknown name the user wrote in `rules = [...]`.
        name: String,
        /// Built-in names in alphabetical order.
        known: Vec<String>,
    },
    /// An `append_rules` entry's `name` field fails the same predicate
    /// as theme names ([`crate::themes::name_is_valid`] — ASCII
    /// alphanumeric with `-` or `_`).
    RuleNameInvalid {
        /// The offending name as written.
        name: String,
    },
    /// An `append_rules` entry's `name` matches a built-in rule name.
    /// Profile concerns and user-override concerns are separated; the
    /// user-override path is `[[rules]]` at the user-config level.
    AppendRuleConflictsWithBuiltin {
        /// The built-in name the entry tried to redefine.
        name: String,
    },
    /// Two `append_rules` entries within the same profile share a name.
    AppendRuleConflictsWithOther {
        /// The duplicated name.
        name: String,
    },
    /// `profile.theme` fails [`crate::themes::name_is_valid`]. Existence
    /// of the referenced theme is checked at theme-load time, not here.
    ThemeNameInvalid {
        /// The invalid theme name.
        name: String,
    },
    /// Capture-group key validation in an `append_rules.styles` map
    /// failed. Reuses [`ThemeRuleErrorKind`] (`KeyMalformed`,
    /// `IndexZeroForbidden`, `IndexOutOfRange`, `NameUnknown`,
    /// `DuplicateTarget`) for byte-equal `Display` semantics across all
    /// rule sources.
    StylesKey(ThemeRuleErrorKind),
}

impl std::fmt::Display for ProfileRuleErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RuleUnknown { name, known } => write!(
                f,
                "rules entry \"{}\": not a built-in name (known: {})",
                sanitize_for_display(name),
                known
                    .iter()
                    .map(|s| sanitize_for_display(s))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            Self::RuleNameInvalid { name } => write!(
                f,
                "append_rules entry \"{}\": name must be ASCII alphanumeric with '-' or '_'",
                sanitize_for_display(name),
            ),
            Self::AppendRuleConflictsWithBuiltin { name } => write!(
                f,
                "append_rules entry \"{}\": collides with built-in rule; use [[rules]] at the user-config level to override built-ins",
                sanitize_for_display(name),
            ),
            Self::AppendRuleConflictsWithOther { name } => write!(
                f,
                "append_rules: duplicate entry \"{}\"",
                sanitize_for_display(name),
            ),
            Self::ThemeNameInvalid { name } => write!(
                f,
                "theme \"{}\": name must be ASCII alphanumeric with '-' or '_'",
                sanitize_for_display(name),
            ),
            Self::StylesKey(inner) => write!(f, "{inner}"),
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

    /// A profile failed to load — file not found, parse error, path
    /// canonicalisation, or pattern compile. Single-error (not
    /// collected). See [`ProfileErrorKind`].
    ///
    /// **Display contract:** `name`, `source_path`, and the inner
    /// `kind`'s string-typed fields all pass through
    /// [`sanitize_for_display`] in the `Display` impl so any
    /// user-supplied control byte (a hostile profile name or
    /// `XDG_CONFIG_HOME` path) cannot smuggle a terminal control
    /// sequence onto the user's terminal — CLAUDE.md §3 invariant.
    #[error("{}", format_profile_load(name, source_path, kind))]
    Profile {
        /// The profile name as the user wrote it (CLI arg or
        /// `[general] profile` value).
        name: String,
        /// `<embedded:profile/{name}>` for shipped profiles (none in
        /// v0.5.2); canonical disk path for disk-loaded profiles.
        source_path: String,
        /// Classification + payload for the underlying failure.
        kind: ProfileErrorKind,
    },

    /// A profile's parsed body failed [`crate::profiles::validate_profile`]
    /// (Phase 1) or `Compiled::load_with_theme` capture-group key
    /// dispatch (Phase 2, landing with v0.5.2 rules-side work).
    /// Fail-collected — every violation gathered into a single error.
    ///
    /// **Display contract:** `profile`, `source_path`, and each rule's
    /// `rule_name` pass through [`sanitize_for_display`] in the
    /// `Display` impl per the same CLAUDE.md §3 invariant as the other
    /// validation-error variants.
    #[error("{}", format_profile_validation(profile, source_path, errors))]
    ProfileValidation {
        /// The user-facing profile name (CLI `--profile` or
        /// `[general] profile`).
        profile: String,
        /// `<embedded:profile/{name}>` for shipped profiles (none in
        /// v0.5.2); canonical disk path for disk-loaded profiles.
        source_path: String,
        /// At least one entry; an empty Vec would be a constructor bug.
        errors: Vec<ProfileRuleError>,
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

fn format_profile_load(name: &str, source_path: &str, kind: &ProfileErrorKind) -> String {
    match kind {
        ProfileErrorKind::NotFound { searched } => {
            let paths = searched
                .iter()
                .map(|p| sanitize_for_display(&p.display().to_string()))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "profile '{name}' not found (searched: {paths})",
                name = sanitize_for_display(name)
            )
        }
        ProfileErrorKind::ParseError { message } => format!(
            "profile '{name}' at {path}: {message}",
            name = sanitize_for_display(name),
            path = sanitize_for_display(source_path),
            message = sanitize_for_display(message),
        ),
        ProfileErrorKind::PathCanonicalization { path, message } => format!(
            "profile '{name}' path '{path}': {message}",
            name = sanitize_for_display(name),
            path = sanitize_for_display(&path.display().to_string()),
            message = sanitize_for_display(message),
        ),
        ProfileErrorKind::RegexCompile { rule_name, pattern, message } => format!(
            "profile '{name}': failed to compile rule '{rule_name}' pattern '{pattern}': {message}",
            name = sanitize_for_display(name),
            rule_name = sanitize_for_display(rule_name),
            pattern = sanitize_for_display(pattern),
            message = sanitize_for_display(message),
        ),
    }
}

fn format_profile_validation(
    profile: &str,
    source_path: &str,
    errors: &[ProfileRuleError],
) -> String {
    let n = errors.len();
    let plural = if n == 1 { "error" } else { "errors" };
    let mut out = format!(
        "profile '{profile}' (loaded from {path}) has {n} validation {plural}:",
        profile = sanitize_for_display(profile),
        path = sanitize_for_display(source_path),
    );
    for e in errors {
        // Literal single quotes around rule_name match Error::Theme's
        // Display contract for visual consistency with ThemeValidation.
        let _ = write!(
            out,
            "\n  - rule '{name}': {msg}",
            name = sanitize_for_display(&e.rule_name),
            msg = e.kind,
        );
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
    fn theme_rule_error_kind_out_of_range_no_capture_groups_specialized() {
        let k = ThemeRuleErrorKind::CaptureGroupIndexOutOfRange { group: 1, captures_len: 1 };
        let s = k.to_string();
        assert!(s.contains("styles.\"1\""), "got: {s}");
        assert!(s.contains("no capture groups"), "got: {s}");
        assert!(s.contains("styles cannot be set"), "got: {s}");
        assert!(!s.contains("valid: 1..=0"), "regression guard: {s}");
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

    #[test]
    fn display_capture_group_name_unknown_with_available_byte_exact() {
        let kind = ThemeRuleErrorKind::CaptureGroupNameUnknown {
            name: "foo".to_owned(),
            available: vec!["scheme".to_owned(), "sep".to_owned(), "body".to_owned()],
        };
        assert_eq!(
            kind.to_string(),
            "styles.\"foo\": rule's regex has no capture group named 'foo' (available: scheme, sep, body)"
        );
    }

    #[test]
    fn display_capture_group_name_unknown_empty_available_byte_exact() {
        let kind = ThemeRuleErrorKind::CaptureGroupNameUnknown {
            name: "foo".to_owned(),
            available: vec![],
        };
        assert_eq!(kind.to_string(), "styles.\"foo\": rule's regex has no named capture groups");
    }

    #[test]
    fn display_capture_group_duplicate_target_byte_exact() {
        let kind = ThemeRuleErrorKind::CaptureGroupDuplicateTarget {
            positional: "1".to_owned(),
            named: "date".to_owned(),
        };
        assert_eq!(
            kind.to_string(),
            "styles.\"1\" and styles.date target the same capture group (index 1); set exactly one"
        );
    }

    // ---- v0.5.2 profile error Display byte-pin tests ----

    #[test]
    fn display_profile_rule_unknown_byte_exact() {
        let kind = ProfileRuleErrorKind::RuleUnknown {
            name: "ipv5".to_owned(),
            known: vec!["ipv4".to_owned(), "ipv6".to_owned(), "url".to_owned()],
        };
        assert_eq!(
            kind.to_string(),
            "rules entry \"ipv5\": not a built-in name (known: ipv4, ipv6, url)"
        );
    }

    #[test]
    fn display_profile_rule_name_invalid_byte_exact() {
        let kind = ProfileRuleErrorKind::RuleNameInvalid { name: "bad name".to_owned() };
        assert_eq!(
            kind.to_string(),
            "append_rules entry \"bad name\": name must be ASCII alphanumeric with '-' or '_'"
        );
    }

    #[test]
    fn display_profile_append_rule_conflicts_with_builtin_byte_exact() {
        let kind = ProfileRuleErrorKind::AppendRuleConflictsWithBuiltin { name: "ipv4".to_owned() };
        assert_eq!(
            kind.to_string(),
            "append_rules entry \"ipv4\": collides with built-in rule; use [[rules]] at the user-config level to override built-ins"
        );
    }

    #[test]
    fn display_profile_append_rule_conflicts_with_other_byte_exact() {
        let kind = ProfileRuleErrorKind::AppendRuleConflictsWithOther { name: "foo".to_owned() };
        assert_eq!(kind.to_string(), "append_rules: duplicate entry \"foo\"");
    }

    #[test]
    fn display_profile_theme_name_invalid_byte_exact() {
        let kind = ProfileRuleErrorKind::ThemeNameInvalid { name: "bad name".to_owned() };
        assert_eq!(
            kind.to_string(),
            "theme \"bad name\": name must be ASCII alphanumeric with '-' or '_'"
        );
    }

    #[test]
    fn display_profile_styles_key_delegates_to_theme_rule_error_kind() {
        let inner = ThemeRuleErrorKind::CaptureGroupKeyMalformed { key: "01".to_owned() };
        let outer = ProfileRuleErrorKind::StylesKey(inner.clone());
        // Wrapper Display must be byte-equal to the inner Display
        // (no prefix, no suffix — pure delegation per spec §6.3).
        assert_eq!(outer.to_string(), inner.to_string());
    }

    #[test]
    fn format_profile_validation_byte_exact_singular_and_plural() {
        let one = Error::ProfileValidation {
            profile: "myaws".into(),
            source_path: "/home/u/.config/tayf/profiles/myaws.toml".into(),
            errors: vec![ProfileRuleError {
                rule_name: "<rules>".into(),
                kind: ProfileRuleErrorKind::RuleUnknown {
                    name: "ipv5".into(),
                    known: vec!["ipv4".into()],
                },
            }],
        };
        let s = one.to_string();
        assert!(s.contains("profile 'myaws'"), "must quote profile name; got: {s}");
        assert!(
            s.contains("(loaded from /home/u/.config/tayf/profiles/myaws.toml)"),
            "must include canonical path; got: {s}"
        );
        assert!(s.contains("1 validation error:"), "singular form; got: {s}");
        assert!(!s.contains("1 validation errors:"), "must not pluralize 1; got: {s}");
        assert!(
            s.contains(
                "  - rule '<rules>': rules entry \"ipv5\": not a built-in name (known: ipv4)"
            ),
            "byte-pinned per-rule line; got: {s}"
        );

        let many = Error::ProfileValidation {
            profile: "myaws".into(),
            source_path: "<p>".into(),
            errors: vec![
                ProfileRuleError {
                    rule_name: "foo".into(),
                    kind: ProfileRuleErrorKind::RuleNameInvalid { name: "foo".into() },
                },
                ProfileRuleError {
                    rule_name: "<theme>".into(),
                    kind: ProfileRuleErrorKind::ThemeNameInvalid { name: "bad".into() },
                },
            ],
        };
        let s = many.to_string();
        assert!(s.contains("2 validation errors:"), "plural form; got: {s}");
    }

    #[test]
    fn format_profile_load_not_found_byte_exact() {
        let e = Error::Profile {
            name: "bogus".into(),
            source_path: "<embedded:profile/bogus>".into(),
            kind: ProfileErrorKind::NotFound {
                searched: vec![std::path::PathBuf::from("/a/b/profiles/bogus.toml")],
            },
        };
        let s = e.to_string();
        assert_eq!(
            s, "profile 'bogus' not found (searched: /a/b/profiles/bogus.toml)",
            "byte-pinned NotFound; got: {s}"
        );
    }

    #[test]
    fn format_profile_load_parse_error_byte_exact() {
        // Appendix A.3: ParseError carries an owned String (no
        // toml::de::Error coupling on the public surface).
        let e = Error::Profile {
            name: "myaws".into(),
            source_path: "/home/u/.config/tayf/profiles/myaws.toml".into(),
            kind: ProfileErrorKind::ParseError {
                message: "expected `=`, found newline at line 3 column 4".into(),
            },
        };
        assert_eq!(
            e.to_string(),
            "profile 'myaws' at /home/u/.config/tayf/profiles/myaws.toml: \
             expected `=`, found newline at line 3 column 4"
        );
    }

    #[test]
    fn format_profile_load_path_canonicalization_byte_exact() {
        let e = Error::Profile {
            name: "myaws".into(),
            source_path: "/path/before/canonicalisation".into(),
            kind: ProfileErrorKind::PathCanonicalization {
                path: std::path::PathBuf::from("/home/u/.config/tayf/profiles/myaws.toml"),
                message: "No such file or directory (os error 2)".into(),
            },
        };
        assert_eq!(
            e.to_string(),
            "profile 'myaws' path '/home/u/.config/tayf/profiles/myaws.toml': \
             No such file or directory (os error 2)"
        );
    }

    #[test]
    fn format_profile_load_regex_compile_byte_exact() {
        let e = Error::Profile {
            name: "myaws".into(),
            source_path: "/home/u/.config/tayf/profiles/myaws.toml".into(),
            kind: ProfileErrorKind::RegexCompile {
                rule_name: "instance_id".into(),
                pattern: "i-[0-9a-f".into(),
                message: "regex parse error: unclosed character class".into(),
            },
        };
        assert_eq!(
            e.to_string(),
            "profile 'myaws': failed to compile rule 'instance_id' pattern 'i-[0-9a-f': \
             regex parse error: unclosed character class"
        );
    }

    #[test]
    fn format_profile_load_sanitizes_control_bytes() {
        // Defense-in-depth: a hostile profile name / message must
        // not let an ESC sequence reach the terminal via the error
        // path. Mirror of the gate other Display impls apply.
        let e = Error::Profile {
            name: "evil\x1b[2J".into(),
            source_path: "/path".into(),
            kind: ProfileErrorKind::ParseError { message: "boom\x1b[2J".into() },
        };
        let s = e.to_string();
        assert!(!s.contains('\x1b'), "raw ESC must not survive Display: {s:?}");
        assert!(s.contains("\\x1b"), "ESC must be escaped as \\x1b: {s:?}");
    }
}
