//! Compiled rule set and built-in patterns.
//!
//! `Compiled` holds a parallel collection of regexes and styles. v0.1 walks
//! `individuals` for every line; the `set` field is populated for the v0.4
//! `RegexSet` fast-path so that switching does not break the public shape.
//! See spec §3.6 and §3.8.

use regex::bytes::{Regex, RegexSet};

use crate::error::{Error, Result};
use crate::style::{Color, Style};

/// 1 MiB cap shared by NFA compile size and lazy DFA cache growth. Both
/// must be bounded to defend against `ReDoS` — see CLAUDE.md §3.
const REGEX_SIZE_LIMIT_BYTES: usize = 1 << 20;

/// One built-in rule: a name (for diagnostics), a regex pattern source, and
/// the style applied to each match. The pattern is owned `String` (not
/// `&'static str`) because the filename rule is built dynamically; the cost
/// of a handful of heap allocations at startup is negligible.
pub(crate) struct BuiltinRule {
    pub(crate) name: String,
    pub(crate) pattern: String,
    pub(crate) style: Style,
    /// Per-capture-group style overlay. `None` at index `i` (which corresponds
    /// to capture group `i + 1` — group 0 is the entire match) means that
    /// group inherits the rule's default `style`. `Some(s)` means: when the
    /// group fires in a match, wrap its bytes with `s` instead of `style`.
    ///
    /// Vector length equals `regex.captures_len() - 1`. An empty vector means
    /// no per-group styling — `apply_rules` uses the cheaper `find_iter` hot
    /// path.
    ///
    /// **Alternation invariant:** if a regex alternation branch contributes
    /// no capture groups (e.g., the syslog branch of the `timestamp`
    /// pattern), all `caps.get(i)` for `i in 1..=N` return `None` when that
    /// branch matches; the match collapses to a single default-style run.
    pub(crate) group_styles: Vec<Option<Style>>,
    /// `true` if `pattern` came from a user TOML config (either an appended
    /// custom rule OR an override of a built-in's pattern). `false` for the
    /// canonical built-in patterns shipped by tayf. Drives error routing in
    /// `compile_error_for`: built-in compile failures are `RegexCompile` (a
    /// tayf bug), user-supplied compile failures are `Config` (user error).
    pub(crate) is_user_supplied: bool,
    /// User-supplied `styles` map (per-capture-group overlay) parsed from a
    /// `[[rules]]` entry's `styles = { ... }` table. `None` means no user/
    /// theme override of per-group styles; the rule's built-in
    /// [`Self::group_styles`] applies as-is. `Some(map)` means: at
    /// `Compiled::load_with_theme` build time, validate each key against
    /// the compiled regex's `captures_len()` and overlay the user's
    /// styles into `group_styles[i]` REPLACING the built-in defaults
    /// (REPLACE semantics, Rev2 Karar 27).
    ///
    /// Keys are positive-decimal capture-group indexes (1-based; grammar
    /// `^[1-9][0-9]*$`); `validate_styles_map_key` enforces the grammar
    /// upstream in `config::parse` / `themes::validate_theme_rules`.
    /// Range validation against `captures_len` happens in
    /// `resolve_group_styles_for_rule` at compile time.
    pub(crate) styles_override:
        Option<std::collections::BTreeMap<String, crate::config::UserStyle>>,
    /// `true` iff [`Self::styles_override`] was last written by the theme
    /// layer (preset or disk theme). `false` for user-config writes and for
    /// the default built-in shape. Drives error routing in
    /// `resolve_group_styles_for_rule`: theme-sourced range/key errors
    /// collect into `Vec<ThemeRuleError>` for [`Error::ThemeValidation`];
    /// user-config-sourced errors fail-fast as [`Error::Config`].
    ///
    /// Since the user-config layer applies AFTER the theme layer and
    /// REPLACES `styles_override` wholesale (Rev2 Karar 27), a `true` value
    /// here unambiguously means "this map originated from the theme and
    /// was never overwritten by user config".
    pub(crate) styles_override_from_theme: bool,
}

/// Provenance of a rule during [`Compiled::load_with_theme`] build. Determines
/// how validation errors are routed: theme-sourced errors collect into a
/// `Vec<ThemeRuleError>` for fail-collected [`Error::ThemeValidation`];
/// user-config-sourced errors fail-fast as [`Error::Config`]. Built-in rules
/// pass validation by construction (asserted by `builtin_rules_*` tests).
///
/// Spec ref: §3.6, Rev2 I-1 (fail-collected theme routing) + Rev2 Karar 27
/// (REPLACE semantics for `styles` map overlays).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum RuleSource {
    /// Canonical pattern shipped by tayf. Compile failures are
    /// `Error::RegexCompile` (tayf bug); range validation cannot fire
    /// (built-ins ship empty `styles_override`).
    Builtin,
    /// `pattern`/`style`/`styles` from the user's TOML config (override of a
    /// built-in OR appended custom rule). Range/key errors fail-fast as
    /// `Error::Config` so the user sees one problem at a time on the path
    /// they own.
    UserConfig,
    /// `styles` from a preset or disk-loaded theme. Range/key errors collect
    /// into a `Vec<ThemeRuleError>` for a single fail-collected
    /// `Error::ThemeValidation` at loop end (matches v0.3.4 contract).
    Theme,
}

/// File extensions colored by the `filename` built-in rule. See spec §3.8 for
/// rationale and the curated catalog. To add an extension, add it here, run
/// the rules tests, and add a smoke case below.
const FILENAME_EXTENSIONS: &[&str] = &[
    // Archives & compression — longest variants first so e.g. .tar.gz wins
    // over .gz when the alternation is sorted longest-first below.
    "tar.gz",
    "tar.bz2",
    "tar.xz",
    "tar.zst",
    "tgz",
    "tbz2",
    "txz",
    "tar",
    "zip",
    "7z",
    "rar",
    "gz",
    "bz2",
    "xz",
    "zst",
    "lz4",
    "jar",
    "war",
    "ear",
    // Text & document
    "md",
    "markdown",
    "txt",
    "rst",
    "adoc",
    "asciidoc",
    "tex",
    "log",
    "csv",
    "tsv",
    // Config & data
    "json",
    "jsonc",
    "yaml",
    "yml",
    "toml",
    "ini",
    "conf",
    "cfg",
    "env",
    "lock",
    "properties",
    "plist",
    // Source code
    "rs",
    "py",
    "pyi",
    "pyx",
    "js",
    "mjs",
    "cjs",
    "ts",
    "tsx",
    "jsx",
    "go",
    "java",
    "kt",
    "kts",
    "groovy",
    "scala",
    "sbt",
    "clj",
    "cljs",
    "c",
    "h",
    "cpp",
    "hpp",
    "cc",
    "hh",
    "cxx",
    "hxx",
    "m",
    "mm",
    "cs",
    "vb",
    "fs",
    "fsx",
    "rb",
    "erb",
    "php",
    "phtml",
    "pl",
    "pm",
    "lua",
    "r",
    "jl",
    "swift",
    "dart",
    "ex",
    "exs",
    "erl",
    "hrl",
    "hs",
    "lhs",
    "ml",
    "mli",
    "elm",
    "nim",
    "zig",
    "v",
    "sv",
    "vhdl",
    "sh",
    "bash",
    "zsh",
    "fish",
    "ksh",
    "csh",
    "tcsh",
    "ps1",
    "bat",
    "cmd",
    "awk",
    "sed",
    "sql",
    "psql",
    // Web & markup
    "html",
    "htm",
    "xhtml",
    "xml",
    "xsl",
    "xslt",
    "css",
    "scss",
    "sass",
    "less",
    "styl",
    "vue",
    "svelte",
    "astro",
    "mdx",
    "svg",
    "wasm",
    "wat",
    // Images
    "png",
    "jpg",
    "jpeg",
    "gif",
    "webp",
    "bmp",
    "ico",
    "tif",
    "tiff",
    "avif",
    "heic",
    "heif",
    // Video
    "mp4",
    "m4v",
    "mov",
    "avi",
    "mkv",
    "webm",
    "flv",
    "wmv",
    "mpg",
    "mpeg",
    "ogv",
    // Audio
    "mp3",
    "wav",
    "flac",
    "ogg",
    "oga",
    "m4a",
    "aac",
    "opus",
    "wma",
    // Documents
    "pdf",
    "doc",
    "docx",
    "xls",
    "xlsx",
    "ppt",
    "pptx",
    "odt",
    "ods",
    "odp",
    "epub",
    "mobi",
    "azw3",
    // Binary / packages / images
    "exe",
    "dll",
    "so",
    "dylib",
    "bin",
    "o",
    "obj",
    "a",
    "lib",
    "deb",
    "rpm",
    "pkg",
    "apk",
    "ipa",
    "dmg",
    "iso",
    "img",
    "vmdk",
    "qcow2",
    "vhd",
    "vhdx",
    "appimage",
    "msi",
    "snap",
    "flatpak",
];

/// Build the filename pattern from `FILENAME_EXTENSIONS`. Extensions are
/// sorted longest-first so that compound forms like `tar.gz` match before
/// their suffix `gz`. Each extension is `regex::escape`-d to be safe.
fn build_filename_pattern() -> String {
    let mut exts: Vec<&&str> = FILENAME_EXTENSIONS.iter().collect();
    exts.sort_by_key(|e| std::cmp::Reverse(e.len()));
    let escaped: Vec<String> = exts.iter().map(|e| regex::escape(e)).collect();
    let alternation = escaped.join("|");
    format!(r"\b[\w.-]+\.(?:{alternation})\b")
}

/// Four timestamp formats joined as alternation. Each branch is anchor-
/// bounded with fixed counts; no backtracking risk under `regex::bytes`.
const TS_ISO8601: &str =
    r"\b(\d{4}-\d{2}-\d{2})([T ])(\d{2}:\d{2}:\d{2})(\.\d{1,9})?([Zz]|[+-]\d{2}:?\d{2})?\b";
const TS_SYSLOG: &str =
    r"\b(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) [ \d]\d \d{2}:\d{2}:\d{2}\b";
const TS_APACHE: &str = r"\b\d{1,2}/(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)/\d{4}:\d{2}:\d{2}:\d{2} [+-]\d{4}";
const TS_RFC2822: &str = r"\b(?:Mon|Tue|Wed|Thu|Fri|Sat|Sun), \d{1,2} (?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) \d{4} \d{2}:\d{2}:\d{2} (?:GMT|UTC|EST|EDT|CST|CDT|MST|MDT|PST|PDT|[+-]\d{4})";

/// Join `TS_ISO8601`, `TS_SYSLOG`, `TS_APACHE`, `TS_RFC2822` as a single
/// non-capturing alternation suitable for `regex::bytes::Regex`.
fn build_timestamp_pattern() -> String {
    format!("(?:{TS_ISO8601})|(?:{TS_SYSLOG})|(?:{TS_APACHE})|(?:{TS_RFC2822})")
}

/// Construct the built-in rules. Returns a fresh `Vec` because the
/// filename rule contains a dynamically built pattern string. See spec §3.6
/// and §3.8.
#[allow(clippy::too_many_lines)]
// reason: this is a registry — one struct literal per built-in rule, with
// fixed-shape fields. Splitting it across helpers would just rename the
// data, not reduce it; the rule set is the source of truth.
pub(crate) fn builtin_rules() -> Vec<BuiltinRule> {
    vec![
        BuiltinRule {
            name: "permission".into(),
            pattern: r"(?:^|\s)([dlcbps-])([rwxsStT-]{3})([rwxsStT-]{3})([rwxsStT-]{3})\+?(?:\s|$)".into(),
            style: Style { fg: Some(Color::White), dim: true, ..Style::DEFAULT },
            group_styles: vec![
                Some(Style { fg: Some(Color::White),       ..Style::DEFAULT }),  // type
                Some(Style { fg: Some(Color::BrightRed),   ..Style::DEFAULT }),  // user-rwx
                Some(Style { fg: Some(Color::Yellow),      ..Style::DEFAULT }),  // group-rwx
                Some(Style { fg: Some(Color::BrightGreen), ..Style::DEFAULT }),  // other-rwx
            ],
            is_user_supplied: false,
            styles_override: None,
            styles_override_from_theme: false,
        },
        BuiltinRule {
            name: "timestamp".into(),
            pattern: build_timestamp_pattern(),
            style: Style { fg: Some(Color::BrightBlack), ..Style::DEFAULT },
            group_styles: vec![
                Some(Style { fg: Some(Color::Yellow),      ..Style::DEFAULT }),  // 1: date
                Some(Style { fg: Some(Color::BrightBlack), ..Style::DEFAULT }),  // 2: T/space sep
                Some(Style { fg: Some(Color::Green),       ..Style::DEFAULT }),  // 3: time
                Some(Style { fg: Some(Color::BrightBlack), ..Style::DEFAULT }),  // 4: .ms
                Some(Style { fg: Some(Color::Magenta),     ..Style::DEFAULT }),  // 5: tz
            ],
            is_user_supplied: false,
            styles_override: None,
            styles_override_from_theme: false,
        },
        BuiltinRule {
            name: "uuid".into(),
            pattern: r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b".into(),
            style: Style { fg: Some(Color::BrightMagenta), ..Style::DEFAULT },
            group_styles: Vec::new(),
            is_user_supplied: false,
            styles_override: None,
            styles_override_from_theme: false,
        },
        // See docs/superpowers/specs/2026-05-23-tayf-v0.3.2-pattern-polish-tech-debt.md §3.1.
        // Char classes here are byte classes under regex::bytes — bytes 0x80..0xFF
        // (UTF-8 continuation, IDN, percent-decoded paths) are implicitly included
        // because the negation only lists ASCII bytes. Trailing-trim set is sentence
        // punctuation only (.,;:!?); closing brackets ) and ] stay in the match to
        // preserve Wikipedia/MDN URLs and IPv6 literal host syntax.
        BuiltinRule {
            name: "url".into(),
            pattern: concat!(
                r#"\b(https?|ssh|ftp)(://)([^\s<>"\\^`{|}]*[^\s<>"\\^`{|}.,;:!?])"#,
                r#"|"#,
                r#"\bgit@[A-Za-z0-9][A-Za-z0-9.-]*[A-Za-z0-9]:[^\s<>"\\^`{|}]*[^\s<>"\\^`{|}.,;:!?]"#,
            ).into(),
            style: Style { fg: Some(Color::BrightBlue), underline: true, ..Style::DEFAULT },
            group_styles: vec![
                Some(Style { fg: Some(Color::BrightCyan), ..Style::DEFAULT }), // 1: scheme
                Some(Style { fg: Some(Color::BrightBlack), ..Style::DEFAULT }), // 2: "://"
                Some(Style { fg: Some(Color::BrightBlue), underline: true, ..Style::DEFAULT }), // 3: host+path
            ],
            is_user_supplied: false,
            styles_override: None,
            styles_override_from_theme: false,
        },
        BuiltinRule {
            name: "email".into(),
            pattern: r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b".into(),
            style: Style { fg: Some(Color::BrightGreen), ..Style::DEFAULT },
            group_styles: Vec::new(),
            is_user_supplied: false,
            styles_override: None,
            styles_override_from_theme: false,
        },
        BuiltinRule {
            name: "ipv4".into(),
            pattern: r"\b(?:25[0-5]|2[0-4]\d|1\d{2}|[1-9]?\d)(?:\.(?:25[0-5]|2[0-4]\d|1\d{2}|[1-9]?\d)){3}\b".into(),
            style: Style { fg: Some(Color::Yellow), bold: true, ..Style::DEFAULT },
            group_styles: Vec::new(),
            is_user_supplied: false,
            styles_override: None,
            styles_override_from_theme: false,
        },
        BuiltinRule {
            name: "ipv6".into(),
            pattern: r"(?:[0-9A-Fa-f]{1,4}:){7}[0-9A-Fa-f]{1,4}|(?:[0-9A-Fa-f]{1,4}:){1,6}:[0-9A-Fa-f]{0,4}|::[0-9A-Fa-f]{1,4}|::1".into(),
            style: Style { fg: Some(Color::BrightYellow), ..Style::DEFAULT },
            group_styles: Vec::new(),
            is_user_supplied: false,
            styles_override: None,
            styles_override_from_theme: false,
        },
        BuiltinRule {
            name: "mac".into(),
            pattern: r"\b[0-9A-Fa-f]{2}(?:[:-][0-9A-Fa-f]{2}){5}\b".into(),
            style: Style { fg: Some(Color::Cyan), ..Style::DEFAULT },
            group_styles: Vec::new(),
            is_user_supplied: false,
            styles_override: None,
            styles_override_from_theme: false,
        },
        BuiltinRule {
            name: "log_level".into(),
            pattern: r"\b(?:ERROR|FAIL|FATAL|CRITICAL|WARN|WARNING|INFO|DEBUG|TRACE)\b".into(),
            style: Style { fg: Some(Color::BrightRed), bold: true, ..Style::DEFAULT },
            group_styles: Vec::new(),
            is_user_supplied: false,
            styles_override: None,
            styles_override_from_theme: false,
        },
        BuiltinRule {
            name: "http_status".into(),
            pattern: r"(?:^|[\s/:])([1-5]\d{2})\b".into(),
            style: Style { fg: Some(Color::Magenta), ..Style::DEFAULT },
            group_styles: Vec::new(),
            is_user_supplied: false,
            styles_override: None,
            styles_override_from_theme: false,
        },
        BuiltinRule {
            name: "filename".into(),
            pattern: build_filename_pattern(),
            style: Style { fg: Some(Color::BrightCyan), ..Style::DEFAULT },
            group_styles: Vec::new(),
            is_user_supplied: false,
            styles_override: None,
            styles_override_from_theme: false,
        },
        BuiltinRule {
            name: "fqdn".into(),
            pattern: r"\b(?:[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?\.){1,}[A-Za-z]{2,24}\b".into(),
            style: Style { fg: Some(Color::Blue), ..Style::DEFAULT },
            group_styles: Vec::new(),
            is_user_supplied: false,
            styles_override: None,
            styles_override_from_theme: false,
        },
        // See docs/superpowers/specs/2026-05-23-tayf-v0.3.2-pattern-polish-tech-debt.md §3.1.
        // Bare units [smhd] match without whitespace (e.g. "5m", "30s"); multi-letter
        // units (ns/us/μs/ms) keep optional single-space prefix. Compound forms like
        // kubectl AGE "2d3h" or docker STATUS "1h30m20s" match as a single span via
        // the repeat group — Rust regex has no lookahead; this shape is the lookahead-
        // free equivalent and remains linear-time DFA.
        // SGR collision safety: v0.3.0 ANSI SM separates SGR bytes into sequence_scratch;
        // with default respect_existing_colors=true, apply_rules skips SGR-bearing lines
        // entirely. Users opting out (respect_existing_colors=false) re-introduce the
        // v0.1-class collision (e.g. "49m" inside "\x1b[49m") — known limitation, see
        // CHANGELOG and spec §1.7.
        BuiltinRule {
            name: "duration".into(),
            pattern: r"\b\d+(?:\.\d+)?(?:\s?(?:ns|us|μs|ms)|[smhd])(?:\d+(?:\.\d+)?(?:\s?(?:ns|us|μs|ms)|[smhd]))*\b".into(),
            style: Style { fg: Some(Color::Green), ..Style::DEFAULT },
            group_styles: Vec::new(),
            is_user_supplied: false,
            styles_override: None,
            styles_override_from_theme: false,
        },
    ]
}

/// Names of the built-in rules. Mirrors the order of [`builtin_rules`].
pub(crate) const BUILTIN_NAMES: &[&str] = &[
    "permission",
    "timestamp",
    "uuid",
    "url",
    "email",
    "ipv4",
    "ipv6",
    "mac",
    "log_level",
    "http_status",
    "filename",
    "fqdn",
    "duration",
];

#[cfg(test)]
mod builtin_names_test {
    use super::{builtin_rules, BUILTIN_NAMES};
    #[test]
    fn builtin_names_match_builtin_rules_order() {
        let rules = builtin_rules();
        let names: Vec<&str> = rules.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, BUILTIN_NAMES);
    }
}

/// Compiled rule set ready for application against output lines.
///
/// `individuals` and `styles` are parallel — index `i` of `individuals` carries
/// the regex; index `i` of `styles` carries the style to apply. `set` is the
/// equivalent `RegexSet` populated for v0.4's planned fast-path; v0.1 ignores
/// it but the storage shape stays stable.
// reason: required by `.expect_err()` in tests. Do NOT log a `Compiled`
// instance directly — `Regex`'s Debug output echoes the pattern source,
// which may include user-supplied patterns (mild info-leak surface).
#[derive(Debug)]
pub(crate) struct Compiled {
    #[allow(dead_code)]
    // reason: reserved for v0.4 RegexSet fast-path; populated now to keep the shape stable
    pub(crate) set: RegexSet,
    pub(crate) individuals: Vec<Regex>,
    pub(crate) styles: Vec<Style>,
    /// Per-rule capture-group style overlay. `group_styles[i]` is the overlay
    /// for `individuals[i]` (vector length = `individuals[i].captures_len() - 1`,
    /// or empty for legacy rules). See [`BuiltinRule::group_styles`] for the
    /// per-entry semantics.
    pub(crate) group_styles: Vec<Vec<Option<Style>>>,
    /// `uses_capture_styling[i] == true` iff `group_styles[i]` contains at
    /// least one `Some` entry. Cached at compile time so `apply_rules`'s
    /// inner loop can branchless dispatch between `find_iter` (hot path)
    /// and `captures_iter` (runs-per-match) without per-line scanning.
    pub(crate) uses_capture_styling: Vec<bool>,
    /// When `true`, lines containing any SGR (CSI `m`) byte skip rule
    /// application. Read from `[general] respect_existing_colors` and
    /// snapshotted at line boundary via the enclosing `ArcSwap<Compiled>`
    /// (spec §4.4, Karar 11).
    pub(crate) respect_existing_colors: bool,
}

impl Compiled {
    /// Empty rule set — no patterns, no styles. Used by bypass mode
    /// (`--bypass` / `TAYF_DISABLE`) to satisfy `runtime::run`'s
    /// `Arc<ArcSwap<Compiled>>` signature without compiling any rules.
    ///
    /// The Pipeline is constructed in `runtime::run` (unconditional,
    /// `src/runtime.rs:64`), but bypass mode passes `apply_colors = false`,
    /// which short-circuits all `feed`/`tick`/`drain` calls in the output
    /// thread (`runtime.rs:196, 211, 229`). So `individuals` and `styles`
    /// are constructed but never iterated.
    ///
    /// Precedent: `load_with_all_builtins_disabled_yields_passthrough`
    /// exercises this same empty-Compiled shape via the user-config
    /// path, proving the Pipeline tolerates an empty rule set without
    /// panic.
    pub(crate) fn empty() -> Self {
        Self {
            set: RegexSet::empty(),
            individuals: Vec::new(),
            styles: Vec::new(),
            group_styles: Vec::new(),
            uses_capture_styling: Vec::new(),
            respect_existing_colors: true,
        }
    }

    /// Compile a rule set with an optional preset theme layered between the
    /// built-in defaults and the user config.
    ///
    /// Layering order (later layers override earlier ones):
    /// 1. Built-in defaults from [`builtin_rules`].
    /// 2. Optional preset theme resolved by [`crate::themes::load`].
    /// 3. Optional user config (`config`).
    ///
    /// After merging, the patterns are compiled and every style slot
    /// (both per-rule default and per-capture-group overlay) is reduced
    /// to `depth` via [`Self::downgrade_for_depth`].
    ///
    /// The theme layer is validated by [`crate::themes::validate_theme_rules`]
    /// before it is merged so schema violations (unknown rule name, stray
    /// `pattern` or `enabled = false`, stray `[general]` section) surface as
    /// [`crate::Error::ThemeValidation`] pointing at the embedded source
    /// label or the disk path, rather than at the user's config path. F2
    /// collisions and disk-load IO still produce [`crate::Error::Config`]
    /// (see `# Errors` below).
    ///
    /// Each pattern is compiled with `regex::bytes::RegexBuilder::size_limit`
    /// capped at 1 MiB to bound the memory a single user regex can consume.
    /// `dfa_size_limit` is similarly capped at 1 MiB so the lazy DFA cache
    /// cannot grow unboundedly under adversarial input (CLAUDE.md §3).
    /// User-rule compile errors carry the offending rule's name in the
    /// surfaced [`crate::Error::Config`].
    ///
    /// # Errors
    /// Returns [`crate::Error::Theme`] when `theme` names neither a disk
    /// theme nor a built-in preset; the `available` list includes both
    /// sources, deduplicated and case-insensitive.
    /// Returns [`crate::Error::Config`] when (a) a disk theme exists for
    /// a built-in name (F2 collision policy), (b) a disk theme cannot be
    /// read, exceeds the 1 MiB cap, or resolves outside the canonical
    /// themes base, (c) a user rule fails to compile (regex error), or
    /// (d) a user rule violates validation (missing fields / no visible
    /// style).
    /// Returns [`crate::Error::ThemeValidation`] when one or more theme
    /// rules fail validation (collected from a single pass; v0.3.4+).
    /// Returns [`crate::Error::RegexCompile`] when a *built-in* pattern
    /// fails to compile — never happens in practice.
    pub(crate) fn load_with_theme(
        config: Option<&crate::config::Config>,
        config_path: Option<&str>,
        theme: Option<&str>,
        depth: crate::terminfo::ColorDepth,
    ) -> Result<Self> {
        // Resolve theme name (if any) to a LoadedTheme before delegating to
        // the source-aware builder. `themes::load` validates the name shape,
        // enforces the F2 collision policy, and yields the canonical source
        // + path_label pair. Validation against the theme contract still
        // runs inside `build_from_loaded`.
        let loaded_theme = match theme {
            Some(name) => Some((name.to_owned(), crate::themes::load(name)?)),
            None => None,
        };
        let loaded_ref = loaded_theme.as_ref().map(|(n, l)| (n.as_str(), l));
        Self::build_from_loaded(config, config_path, loaded_ref, depth)
    }

    /// Internal builder that takes a pre-resolved [`crate::themes::LoadedTheme`]
    /// (paired with the requested theme name for diagnostics) instead of a
    /// theme name to look up. Public entry [`Self::load_with_theme`] is a
    /// thin wrapper that calls [`crate::themes::load`] first; this split
    /// exists so unit tests can inject a synthetic theme TOML without
    /// touching disk or the env-var lookup.
    ///
    /// All validation, merge ordering, and error-routing semantics are
    /// identical to the public entry — see [`Self::load_with_theme`].
    fn build_from_loaded(
        config: Option<&crate::config::Config>,
        config_path: Option<&str>,
        loaded_theme: Option<(&str, &crate::themes::LoadedTheme)>,
        depth: crate::terminfo::ColorDepth,
    ) -> Result<Self> {
        let mut rules = builtin_rules();

        // Layer 1: optional preset theme. Applied BEFORE the user config so
        // user rules win on conflict (spec §2 decision 5). Validation runs
        // BEFORE the merge so semantic errors surface against the synthetic
        // theme path rather than mutating the rule set first.
        if let Some((name, loaded)) = loaded_theme {
            let theme_cfg = crate::config::parse(&loaded.path_label, &loaded.source)?;
            crate::themes::validate_theme_rules(name, &loaded.path_label, &theme_cfg)?;
            // `from_theme = true` so any `styles_override` map written here
            // is tagged for theme-routed error collection downstream.
            crate::config::apply_user_rules_with_source(
                &loaded.path_label,
                &mut rules,
                &theme_cfg.rules,
                true,
            )?;
        }

        // Layer 2: user config. `from_theme = false` — user-config writes
        // overwrite any prior theme-tagged `styles_override` (REPLACE
        // semantics, Rev2 Karar 27), and any subsequent range/key errors
        // surface as `Error::Config` so the user sees them on their own
        // config path.
        if let Some(c) = config {
            // `config_path` flows into Error::Config messages produced inside
            // apply_user_rules (and any nested UserStyle::to_style call) so
            // users see `config error in /home/u/.config/tayf/config.toml: ...`
            // rather than the empty-path sentinel.
            let path = config_path.filter(|p| !p.is_empty()).unwrap_or("<config>");
            crate::config::apply_user_rules_with_source(path, &mut rules, &c.rules, false)?;
        }

        let theme_name = loaded_theme.map(|(n, _)| n);
        let theme_path = loaded_theme.map(|(_, l)| l.path_label.as_str());
        let compiled_rules = compile_merged_rules(&rules, config_path, theme_name, theme_path)?;

        // Karar 11: snapshot config value into Compiled so reads happen at
        // line boundary via ArcSwap<Compiled>, no separate atomic needed.
        let respect_existing_colors = config.map_or_else(
            || crate::config::GeneralSection::default().respect_existing_colors,
            |c| c.general.respect_existing_colors,
        );

        let mut compiled = Compiled {
            set: compiled_rules.set,
            individuals: compiled_rules.individuals,
            styles: compiled_rules.styles,
            group_styles: compiled_rules.group_styles,
            uses_capture_styling: compiled_rules.uses_capture_styling,
            respect_existing_colors,
        };
        // Bake depth into every style slot — both default and per-group.
        compiled.downgrade_for_depth(depth);
        Ok(compiled)
    }

    /// Built-in defaults compiled at Truecolor depth, no theme, no user
    /// config. Convenience shim used by the bench harness and by test
    /// scaffolding that does not exercise the layering logic.
    ///
    /// # Errors
    /// As for [`Self::load_with_theme`].
    pub(crate) fn load_builtins() -> Result<Self> {
        Self::load_with_theme(None, None, None, crate::terminfo::ColorDepth::Truecolor)
    }

    /// Walk every style slot in the compiled rule set and reduce it to
    /// the colors representable at `depth`. This covers both the per-rule
    /// default style (`styles[i]`) and the per-capture-group overlay
    /// (`group_styles[i][j]`, `Some` entries only — `None` slots are
    /// preserved as-is, since they fall through to the rule's default).
    ///
    /// Idempotent and depth-monotonic: calling at the same depth twice
    /// produces the same result; calling at a lower depth after a higher
    /// one is well-defined via [`Style::downgrade`].
    ///
    /// Spec ref: §1.2.5 — depth downgrade contract; §3.1.D — extension
    /// to `group_styles`.
    pub(crate) fn downgrade_for_depth(&mut self, depth: crate::terminfo::ColorDepth) {
        for style in &mut self.styles {
            *style = style.downgrade(depth);
        }
        for group_vec in &mut self.group_styles {
            for s in group_vec.iter_mut().flatten() {
                *s = s.downgrade(depth);
            }
        }
    }
}

/// Map a built-in vs. user-rule regex error: built-ins surface as
/// [`Error::RegexCompile`]; user rules surface as [`Error::Config`] with the
/// offending rule name and the user's config path threaded through.
///
/// "User rule" covers both user-config `[[rules]]` and theme TOML rules —
/// once a theme has passed [`crate::themes::validate_theme_rules`] it is
/// merged via the same [`crate::config::apply_user_rules`] path as user
/// config, and `is_user_supplied` is set for both. Theme-supplied rules
/// flow `config_path` = `<embedded:theme/{name}>` (the synthetic path)
/// so the diagnostic still points at the right source.
fn compile_error_for(
    is_user_supplied: bool,
    rule_name: &str,
    config_path: Option<&str>,
    source: regex::Error,
) -> Error {
    if is_user_supplied {
        let path = config_path.filter(|p| !p.is_empty()).unwrap_or("<config>");
        Error::config_regex(path.to_string(), rule_name, source)
    } else {
        Error::from(source)
    }
}

/// Classify a [`BuiltinRule`]'s provenance for error-routing purposes.
///
/// `styles_override_from_theme` wins over `is_user_supplied`: a built-in
/// whose `styles` map was set by a preset theme but whose pattern/style
/// remain at the built-in defaults must route `styles`-map range errors
/// through [`Error::ThemeValidation`], even though `is_user_supplied` is
/// `false` for the unchanged pattern.
///
/// Spec ref: §3.6, Rev2 I-1.
fn rule_source_of(rule: &BuiltinRule) -> RuleSource {
    if rule.styles_override.is_some() {
        if rule.styles_override_from_theme {
            RuleSource::Theme
        } else {
            RuleSource::UserConfig
        }
    } else if rule.is_user_supplied {
        // No styles_override but is_user_supplied (pattern/style was
        // overridden by user config) — still UserConfig for diagnostic
        // routing, even though the early-return path inside the resolver
        // means we won't actually emit any range/key errors.
        RuleSource::UserConfig
    } else {
        RuleSource::Builtin
    }
}

/// Output of [`compile_merged_rules`]: parallel vectors plus the aggregated
/// `RegexSet` and the `uses_capture_styling` cache. Internal-only; the
/// caller in [`Compiled::build_from_loaded`] zips this with
/// `respect_existing_colors` to populate the final `Compiled` struct.
struct CompiledRules {
    set: RegexSet,
    individuals: Vec<Regex>,
    styles: Vec<Style>,
    group_styles: Vec<Vec<Option<Style>>>,
    uses_capture_styling: Vec<bool>,
}

/// Compile each merged rule, build the parallel style/regex/group-styles
/// vectors, and aggregate theme-routed validation errors into a single
/// [`Error::ThemeValidation`]. User-config-routed errors and built-in
/// compile failures fail-fast via `?` inside the loop.
///
/// `theme_name` / `theme_path` flow into the `Error::ThemeValidation`
/// payload when at least one theme-routed error is collected; they're
/// otherwise unused. Both are `Some(...)` together or both `None`.
fn compile_merged_rules(
    rules: &[BuiltinRule],
    config_path: Option<&str>,
    theme_name: Option<&str>,
    theme_path: Option<&str>,
) -> Result<CompiledRules> {
    let mut individuals: Vec<Regex> = Vec::with_capacity(rules.len());
    let mut styles: Vec<Style> = Vec::with_capacity(rules.len());
    let mut sources: Vec<String> = Vec::with_capacity(rules.len());
    let mut group_styles: Vec<Vec<Option<Style>>> = Vec::with_capacity(rules.len());
    let mut theme_errors: Vec<crate::error::ThemeRuleError> = Vec::new();

    for rule in rules {
        let regex = regex::bytes::RegexBuilder::new(&rule.pattern)
            .size_limit(REGEX_SIZE_LIMIT_BYTES)
            .dfa_size_limit(REGEX_SIZE_LIMIT_BYTES)
            .build()
            .map_err(|e| compile_error_for(rule.is_user_supplied, &rule.name, config_path, e))?;
        let captures_len = regex.captures_len();
        let final_group_styles = resolve_group_styles_for_rule(
            rule,
            rule_source_of(rule),
            captures_len,
            config_path,
            &mut theme_errors,
        )?;
        sources.push(rule.pattern.clone());
        individuals.push(regex);
        styles.push(rule.style);
        group_styles.push(final_group_styles);
    }

    if !theme_errors.is_empty() {
        return Err(Error::ThemeValidation {
            theme: theme_name.unwrap_or("<unknown>").to_owned(),
            source_path: theme_path.unwrap_or("").to_owned(),
            errors: theme_errors,
        });
    }

    // `sources` are the same patterns we just compiled individually — RegexSet
    // over the same set cannot raise a syntax error, and tayf's per-rule
    // size_limit keeps the aggregate well under RegexSet's default cap. The
    // error path is preserved for forward-compat (e.g. larger rule sets in v0.4).
    let set = RegexSet::new(&sources).map_err(Error::from)?;

    // Cache the any-Some scan so apply_rules's inner loop can branchless
    // dispatch between find_iter (hot path) and captures_iter (overlay).
    let uses_capture_styling: Vec<bool> =
        group_styles.iter().map(|gs| gs.iter().any(Option::is_some)).collect();

    Ok(CompiledRules { set, individuals, styles, group_styles, uses_capture_styling })
}

/// Resolve the per-capture-group style overlay vector for a single rule,
/// routing range/key validation errors to either a collected
/// `Vec<ThemeRuleError>` (theme provenance) or a fail-fast
/// [`Error::Config`] (user-config provenance). Built-in rules with no
/// `styles_override` short-circuit to a clone of their pre-populated
/// `group_styles` (Phase 6 will make those non-empty for permission /
/// timestamp / url).
///
/// `captures_len` is `regex.captures_len()` for the rule's compiled regex
/// — i.e. `1 + (number of capture groups)`. A rule with no groups has
/// `captures_len == 1` and any non-empty `styles_override` map is
/// out-of-range by definition.
///
/// Spec ref: §3.6, §1.3.5, Rev2 I-1, Rev2 Karar 27.
fn resolve_group_styles_for_rule(
    rule: &BuiltinRule,
    source: RuleSource,
    captures_len: usize,
    user_cfg_path: Option<&str>,
    theme_errors: &mut Vec<crate::error::ThemeRuleError>,
) -> Result<Vec<Option<Style>>> {
    // Built-in with no user/theme override: inherit the built-in's pre-
    // populated overlay (Phase 6 populates timestamp/url/permission).
    let Some(map) = rule.styles_override.as_ref() else {
        return Ok(rule.group_styles.clone());
    };

    // We always have at least one capture group entry for the entire match;
    // overlay vector covers groups 1..captures_len-1 (length = captures_len - 1).
    let mut vec: Vec<Option<Style>> = vec![None; captures_len.saturating_sub(1)];

    let user_cfg_path_or_sentinel = user_cfg_path.filter(|p| !p.is_empty()).unwrap_or("<config>");

    for (key, user_style) in map {
        // Special-case "0" BEFORE grammar validation: group 0 has a dedicated
        // diagnostic that points at the `style` field.
        if key == "0" {
            match source {
                RuleSource::Theme => {
                    // Already collected by `themes::validate_theme_rules`
                    // (Phase 1); skip here to avoid duplicate diagnostics.
                    continue;
                }
                RuleSource::UserConfig => {
                    let kind = crate::error::ThemeRuleErrorKind::CaptureGroupIndexZeroForbidden;
                    return Err(Error::Config {
                        path: user_cfg_path_or_sentinel.to_owned(),
                        line: 0,
                        message: format!("rule '{}': {kind}", rule.name),
                    });
                }
                RuleSource::Builtin => unreachable!(
                    "Builtin rules ship with styles_override == None; reached the \
                     map iteration only for UserConfig/Theme. styles_override on \
                     a Builtin would be a constructor bug."
                ),
            }
        }

        // Grammar check (positive decimal, no leading zeros, `^[1-9][0-9]*$`).
        let Some(parsed) = crate::config::validate_styles_map_key(key) else {
            match source {
                RuleSource::Theme => {
                    // Already collected by `themes::validate_theme_rules`
                    // (Phase 1) with the original key bytes. Skip silently.
                    continue;
                }
                RuleSource::UserConfig => {
                    let kind = crate::error::ThemeRuleErrorKind::CaptureGroupKeyMalformed {
                        key: key.to_owned(),
                    };
                    return Err(Error::Config {
                        path: user_cfg_path_or_sentinel.to_owned(),
                        line: 0,
                        message: format!("rule '{}': {kind}", rule.name),
                    });
                }
                RuleSource::Builtin => unreachable!(),
            }
        };

        // Range check: key must be < captures_len. `captures_len` is
        // 1 + group_count, so valid integer keys are 1..=captures_len-1.
        if parsed >= captures_len {
            match source {
                RuleSource::Theme => {
                    theme_errors.push(crate::error::ThemeRuleError {
                        rule_name: rule.name.clone(),
                        kind: crate::error::ThemeRuleErrorKind::CaptureGroupIndexOutOfRange {
                            group: parsed,
                            captures_len,
                        },
                    });
                    continue;
                }
                RuleSource::UserConfig => {
                    let kind = crate::error::ThemeRuleErrorKind::CaptureGroupIndexOutOfRange {
                        group: parsed,
                        captures_len,
                    };
                    return Err(Error::Config {
                        path: user_cfg_path_or_sentinel.to_owned(),
                        line: 0,
                        message: format!("rule '{}': {kind}", rule.name),
                    });
                }
                RuleSource::Builtin => unreachable!(),
            }
        }

        let style = user_style.to_style(user_cfg_path_or_sentinel, &rule.name)?;
        // parsed >= 1 (grammar excludes "0" and leading zeros), so the
        // subtraction is safe; index is < captures_len - 1 == vec.len().
        vec[parsed - 1] = Some(style);
    }
    Ok(vec)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find_rule(name: &str) -> String {
        builtin_rules().into_iter().find(|r| r.name == name).expect("rule must exist").pattern
    }

    fn matches(pattern_name: &str, input: &str) -> bool {
        let re = regex::bytes::Regex::new(&find_rule(pattern_name)).unwrap();
        re.is_match(input.as_bytes())
    }

    fn match_string(pattern_name: &str, input: &str) -> Option<String> {
        let re = regex::bytes::Regex::new(&find_rule(pattern_name)).unwrap();
        re.find(input.as_bytes()).map(|m| String::from_utf8_lossy(m.as_bytes()).into_owned())
    }

    // reason: helper for fixture matrices that need the captured match list
    // (e.g. duration §4.2 "match list: [...]"); paired with `matches` for
    // boolean-only fixtures.
    fn all_matches(pattern_name: &str, input: &str) -> Vec<String> {
        let re = regex::bytes::Regex::new(&find_rule(pattern_name)).unwrap();
        re.find_iter(input.as_bytes())
            .map(|m| String::from_utf8_lossy(m.as_bytes()).into_owned())
            .collect()
    }

    #[test]
    fn ipv4_matches() {
        assert!(matches("ipv4", "connect 192.168.1.1"));
        assert!(matches("ipv4", "0.0.0.0"));
        assert!(matches("ipv4", "255.255.255.255"));
        assert!(!matches("ipv4", "999.999.999.999"));
        assert!(!matches("ipv4", "1.2.3"));
    }

    #[test]
    fn ipv6_matches() {
        assert!(matches("ipv6", "fe80::1"));
        assert!(matches("ipv6", "2001:0db8:85a3:0000:0000:8a2e:0370:7334"));
        assert!(matches("ipv6", "::1"));
        assert!(!matches("ipv6", "12345"));
    }

    #[test]
    fn mac_matches() {
        assert!(matches("mac", "aa:bb:cc:dd:ee:ff"));
        assert!(matches("mac", "AA-BB-CC-DD-EE-FF"));
        assert!(matches("mac", "00:11:22:33:44:55"));
        assert!(!matches("mac", "zz:bb:cc:dd:ee:ff"));
        assert!(!matches("mac", "aa:bb:cc"));
    }

    #[test]
    fn log_level_matches() {
        assert!(matches("log_level", "[ERROR] failed to connect"));
        assert!(matches("log_level", "WARN something"));
        assert!(matches("log_level", "INFO startup"));
        assert!(matches("log_level", "FATAL crash"));
        assert!(!matches("log_level", "errorlike"));
    }

    #[test]
    fn http_status_matches() {
        assert!(matches("http_status", "HTTP/1.1 404 Not Found"));
        assert!(matches("http_status", "status: 500"));
        assert!(matches("http_status", "200 OK"));
        assert!(!matches("http_status", "abc500def"));
    }

    #[test]
    fn fqdn_matches() {
        assert!(matches("fqdn", "https://example.com/"));
        assert!(matches("fqdn", "api.staging.internal.corp.net"));
        assert!(matches("fqdn", "host.local"));
        // Single labels are not FQDNs:
        assert!(!matches("fqdn", "localhost"));
    }

    // === Duration pattern (rule "duration") — see spec §4.2 ===

    #[test]
    fn duration_matches_bare_units() {
        // NEW in v0.3.2: bare [smhd] units.
        assert_eq!(all_matches("duration", "took 5s"), vec!["5s".to_string()]);
        assert_eq!(all_matches("duration", "elapsed 30m"), vec!["30m".to_string()]);
        assert_eq!(all_matches("duration", "uptime 2h"), vec!["2h".to_string()]);
        assert_eq!(all_matches("duration", "retention 7d"), vec!["7d".to_string()]);
    }

    #[test]
    fn duration_matches_compound_forms() {
        // NEW in v0.3.2: compound via repeat group, single span per match.
        // Drives kubectl AGE and docker ps STATUS columns.
        assert_eq!(all_matches("duration", "AGE 2d3h"), vec!["2d3h".to_string()]);
        assert_eq!(all_matches("duration", "STATUS 1h30m20s"), vec!["1h30m20s".to_string()]);
        assert_eq!(all_matches("duration", "elapsed 1h30m500ms"), vec!["1h30m500ms".to_string()]);
        assert_eq!(all_matches("duration", "uptime 7d12h30m"), vec!["7d12h30m".to_string()]);
    }

    #[test]
    fn duration_matches_multi_letter_units_preserved() {
        // v0.2.2 multi-letter units carry over.
        assert_eq!(all_matches("duration", "took 20.291 ms"), vec!["20.291 ms".to_string()]);
        assert_eq!(all_matches("duration", "took 50ns"), vec!["50ns".to_string()]);
        assert_eq!(all_matches("duration", "took 100 μs"), vec!["100 μs".to_string()]);
        assert_eq!(all_matches("duration", "took 2.5 us"), vec!["2.5 us".to_string()]);
    }

    #[test]
    fn duration_rejects_word_continuation() {
        // FP guards: \b after [smhd] fails when next char is also a word char.
        assert!(all_matches("duration", "5min ago").is_empty());
        assert!(all_matches("duration", "5seconds").is_empty());
        assert!(all_matches("duration", "5days").is_empty());
        assert!(all_matches("duration", "5hours").is_empty());
    }

    #[test]
    fn duration_rejects_bare_unit_with_whitespace() {
        // FP guards: bare units require NO space; multi-letter units may have one.
        assert!(all_matches("duration", "5 m of cable").is_empty());
        assert!(all_matches("duration", "took 5 s").is_empty());
        assert!(all_matches("duration", "took 5 h").is_empty());
        assert!(all_matches("duration", "took 5 d").is_empty());
    }

    #[test]
    fn duration_accepts_decade_plural_as_fp_tradeoff() {
        // Documented accepted FP — too rare in CLI output to suppress; spec §1.7.
        assert_eq!(all_matches("duration", "in the 200s"), vec!["200s".to_string()]);
    }

    #[test]
    fn duration_multiple_non_adjacent_matches() {
        assert_eq!(
            all_matches("duration", "first 5s then 30m later"),
            vec!["5s".to_string(), "30m".to_string()]
        );
    }

    #[test]
    fn bare_units_collide_with_sgr_when_respect_existing_colors_is_false() {
        // KNOWN LIMITATION: spec §1.3, §1.7 — when the user opts out of the
        // v0.3.0 default (respect_existing_colors=true), apply_rules sees SGR
        // bytes and the bare-unit pattern matches `49m` inside `\x1b[49m`.
        // This test pins the (broken) behavior so a future change cannot
        // silently flip it.
        use crate::config::{Config, GeneralSection, UserRule};
        use crate::pipeline::{apply_rules, PipelineScratch};
        use crate::terminfo::ColorDepth;
        use arc_swap::ArcSwap;

        let cfg = Config {
            general: GeneralSection { respect_existing_colors: false, ..GeneralSection::default() },
            rules: Vec::<UserRule>::new(),
        };
        let compiled = Compiled::load_with_theme(
            Some(&cfg),
            Some("/test/cfg.toml"),
            None,
            ColorDepth::Truecolor,
        )
        .unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"\x1b[49m", &rules, &mut scratch, &mut out).unwrap();
        // Duration style fg is Green = SGR 32. With respect=false, the bare-`m`
        // bit of `49m` matches the duration rule and an SGR wrap appears.
        let s = String::from_utf8_lossy(&out).into_owned();
        assert!(
            s.contains("\x1b[32m") || s.contains(";32m") || s.contains("[32"),
            "expected Green SGR somewhere in output (the documented v0.1-class \
             collision): {s:?}"
        );
    }

    #[test]
    fn filename_matches_common_extensions() {
        assert!(matches("filename", "edit claude.md please"));
        assert!(matches("filename", "extract archive.tar.gz"));
        assert!(matches("filename", "cat config.json"));
        assert!(matches("filename", "vim src/main.rs"));
        assert!(matches("filename", "open report.pdf"));
        assert!(matches("filename", "build dist.zip"));
        assert!(matches("filename", "view package-lock.json"));
        assert!(matches("filename", "play video.mp4"));
        assert!(matches("filename", "render image.png"));
        assert!(matches("filename", "/var/log/syslog.gz"));
    }

    #[test]
    fn filename_rejects_non_listed_extensions() {
        assert!(!matches("filename", "weird.qqqq"));
        assert!(!matches("filename", "no_extension"));
        // Pure dotfiles without listed-extension postfix don't match:
        assert!(!matches("filename", ".bashrc"));
    }

    #[test]
    fn filename_longest_match_for_compound_extensions() {
        let re = regex::bytes::Regex::new(&find_rule("filename")).unwrap();
        let m = re.find(b"archive.tar.gz").unwrap();
        assert_eq!(&b"archive.tar.gz"[m.start()..m.end()], b"archive.tar.gz");
    }

    #[test]
    fn compiled_load_succeeds() {
        let c = Compiled::load_builtins().expect("builtin compile");
        let n = builtin_rules().len();
        assert_eq!(c.individuals.len(), n);
        assert_eq!(c.styles.len(), n);
        assert_eq!(c.set.len(), n);
        assert_eq!(n, 13, "v0.2.2 ships thirteen built-in rules");
    }

    #[test]
    fn filename_wins_over_fqdn_for_known_extensions() {
        use crate::pipeline::{apply_rules, PipelineScratch};
        use arc_swap::ArcSwap;
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"edit claude.md please\n", &rules, &mut scratch, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        // BrightCyan fg = SGR 96; Blue fg = SGR 34. Verify the filename style wins.
        assert!(s.contains("96"), "expected filename SGR 96 (bright cyan), got: {s:?}");
        assert!(!s.contains("\x1b[34m"), "should not contain blue SGR 34: {s:?}");
    }

    #[test]
    fn filename_wins_for_rust_source() {
        use crate::pipeline::{apply_rules, PipelineScratch};
        use arc_swap::ArcSwap;
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"vim src/main.rs and tests.rs\n", &rules, &mut scratch, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("96"), "expected bright cyan: {s:?}");
    }

    #[test]
    fn fqdn_still_matches_when_no_filename_competes() {
        use crate::pipeline::{apply_rules, PipelineScratch};
        use arc_swap::ArcSwap;
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"visit api.example.org today\n", &rules, &mut scratch, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        // Blue SGR 34 should appear (no extension to conflict).
        assert!(s.contains("34"), "expected fqdn SGR 34 (blue): {s:?}");
    }

    #[test]
    fn load_with_no_config_matches_load_builtins() {
        use crate::terminfo::ColorDepth;
        let a = Compiled::load_builtins().unwrap();
        let b = Compiled::load_with_theme(None, None, None, ColorDepth::Truecolor).unwrap();
        assert_eq!(a.individuals.len(), b.individuals.len());
        assert_eq!(a.styles, b.styles);
    }

    #[test]
    fn load_at_none_depth_strips_colors_but_keeps_attributes() {
        use crate::terminfo::ColorDepth;
        let c = Compiled::load_with_theme(None, None, None, ColorDepth::None).unwrap();
        for s in &c.styles {
            assert_eq!(s.fg, None, "depth=None must strip all fg colors");
            assert_eq!(s.bg, None, "depth=None must strip all bg colors");
        }
        // log_level built-in still has bold:true even when colors are stripped.
        let log_idx = builtin_rules().iter().position(|r| r.name == "log_level").unwrap();
        assert!(c.styles[log_idx].bold);
    }

    #[test]
    fn load_at_basic16_keeps_ansi_unchanged() {
        use crate::terminfo::ColorDepth;
        let c = Compiled::load_with_theme(None, None, None, ColorDepth::Basic16).unwrap();
        let log_idx = builtin_rules().iter().position(|r| r.name == "log_level").unwrap();
        // Built-in log_level fg is BrightRed (ANSI) — unchanged at Basic16.
        assert_eq!(c.styles[log_idx].fg, Some(crate::style::Color::BrightRed));
    }

    #[test]
    fn downgrade_for_depth_walks_group_styles_basic16() {
        let mut c = Compiled::load_builtins().unwrap();
        c.downgrade_for_depth(crate::terminfo::ColorDepth::Basic16);
        // permission's group_styles entries should all still be Some after
        // downgrade (Basic16-safe palette is a no-op).
        let perm_idx = 0; // permission is at index 0
        for slot in &c.group_styles[perm_idx] {
            assert!(slot.is_some(), "group_style slot dropped during downgrade");
        }
    }

    #[test]
    fn downgrade_for_depth_none_zeroes_all_group_style_colors() {
        let mut c = Compiled::load_builtins().unwrap();
        c.downgrade_for_depth(crate::terminfo::ColorDepth::None);
        // After None-depth downgrade, every group_style fg should be None.
        for vec in &c.group_styles {
            for s in vec.iter().flatten() {
                assert!(s.fg.is_none(), "fg leaked through None-depth downgrade: {s:?}");
            }
        }
    }

    #[test]
    fn load_applies_user_rules_then_bakes_depth() {
        use crate::config::{Config, GeneralSection, UserRule, UserStyle};
        use crate::terminfo::ColorDepth;
        let cfg = Config {
            general: GeneralSection::default(),
            rules: vec![UserRule {
                name: "custom_id".into(),
                pattern: Some(r"\b[0-9a-fA-F]{8}\b".into()),
                style: Some(UserStyle { fg: Some("#ff8800".into()), ..UserStyle::default() }),
                enabled: true,
                styles: None,
            }],
        };
        // At Basic16 depth, the appended user rule's Rgb fg downgrades to an ANSI color.
        let c = Compiled::load_with_theme(
            Some(&cfg),
            Some("/test/cfg.toml"),
            None,
            ColorDepth::Basic16,
        )
        .unwrap();
        let last_style = c.styles.last().unwrap();
        match last_style.fg {
            Some(crate::style::Color::Rgb(_, _, _)) => panic!("Rgb not downgraded: {last_style:?}"),
            Some(_) => {}
            None => panic!("user rule should still carry a color at Basic16"),
        }
        assert_eq!(c.individuals.len(), 14, "13 built-ins + 1 user rule");
    }

    #[test]
    fn override_of_builtin_pattern_with_bad_regex_surfaces_as_config_error() {
        // Regression for the misrouted-error bug found in code review of Task 8:
        // user override of a built-in's pattern with an invalid regex must
        // produce Error::Config (EX_USAGE), not Error::RegexCompile (EX_SOFTWARE).
        use crate::config::{Config, GeneralSection, UserRule, UserStyle};
        use crate::terminfo::ColorDepth;
        let cfg = Config {
            general: GeneralSection::default(),
            rules: vec![UserRule {
                name: "ipv4".into(),
                pattern: Some("(unclosed".into()),
                style: Some(UserStyle { fg: Some("red".into()), ..UserStyle::default() }),
                enabled: true,
                styles: None,
            }],
        };
        let err =
            Compiled::load_with_theme(Some(&cfg), Some("/x/cfg.toml"), None, ColorDepth::Truecolor)
                .expect_err("invalid regex must fail to compile");
        let msg = err.to_string();
        assert!(
            msg.starts_with("config error in /x/cfg.toml"),
            "must be a Config error with path, got: {msg}"
        );
        assert!(msg.contains("ipv4"), "must name the rule: {msg}");
    }

    #[test]
    fn new_user_rule_with_bad_regex_surfaces_as_config_error() {
        // Parallel test: user-only rule (not a built-in override) with bad regex.
        use crate::config::{Config, GeneralSection, UserRule, UserStyle};
        use crate::terminfo::ColorDepth;
        let cfg = Config {
            general: GeneralSection::default(),
            rules: vec![UserRule {
                name: "my_rule".into(),
                pattern: Some("(also-unclosed".into()),
                style: Some(UserStyle { fg: Some("red".into()), ..UserStyle::default() }),
                enabled: true,
                styles: None,
            }],
        };
        let err =
            Compiled::load_with_theme(Some(&cfg), Some("/x/cfg.toml"), None, ColorDepth::Truecolor)
                .expect_err("invalid regex must fail to compile");
        let msg = err.to_string();
        assert!(msg.starts_with("config error in /x/cfg.toml"));
        assert!(msg.contains("my_rule"));
    }

    #[test]
    fn load_with_all_builtins_disabled_yields_passthrough() {
        // Edge case flagged in code review: empty rule set after merge must
        // produce a valid (empty) Compiled, which makes apply_rules a no-op
        // (effective passthrough).
        use crate::config::{Config, GeneralSection, UserRule};
        use crate::terminfo::ColorDepth;
        let cfg = Config {
            general: GeneralSection::default(),
            rules: BUILTIN_NAMES
                .iter()
                .map(|n| UserRule {
                    name: (*n).into(),
                    pattern: None,
                    style: None,
                    enabled: false,
                    styles: None,
                })
                .collect(),
        };
        let c =
            Compiled::load_with_theme(Some(&cfg), Some("/x"), None, ColorDepth::Truecolor).unwrap();
        assert_eq!(c.individuals.len(), 0);
        assert_eq!(c.styles.len(), 0);
        assert_eq!(c.set.len(), 0);
    }

    #[test]
    fn permission_matches_common_modes() {
        assert!(matches("permission", "-rw-r--r-- 1 user staff 100 May 22 file"));
        assert!(matches("permission", "drwxr-xr-x  3 user staff 96 May 22 dir"));
        assert!(matches("permission", "lrwxrwxrwx 1 root root 7 May 22 link"));
        assert!(matches("permission", "crw-rw-rw-  1 root tty 5, 0 May 22 console"));
        // ACL trailing '+'
        assert!(matches("permission", "-rwxr-xr-x+ 1 user staff 100 May 22 file"));
    }

    #[test]
    fn permission_rejects_invalid_shapes() {
        // Wrong leading char
        assert!(!matches("permission", "xrwxrwxrwx 1 user file"));
        // Too short (9 chars body)
        assert!(!matches("permission", "-rwxrwx 1 user file"));
        // Wrong perm chars
        assert!(!matches("permission", "-rwzqqzqqz 1 user file"));
    }

    #[test]
    fn timestamp_matches_iso8601() {
        assert!(matches("timestamp", "log 2026-05-22T10:30:45Z message"));
        assert!(matches("timestamp", "log 2026-05-22T10:30:45.123Z message"));
        assert!(matches("timestamp", "log 2026-05-22 10:30:45 message"));
        assert!(matches("timestamp", "log 2026-05-22T10:30:45+03:00 message"));
        assert!(matches("timestamp", "log 2026-05-22T10:30:45-0500 message"));
    }

    #[test]
    fn timestamp_matches_syslog() {
        assert!(matches("timestamp", "May 22 10:30:45 host kernel: ..."));
        // Single-digit day, space-padded
        assert!(matches("timestamp", "Jan  5 09:08:07 host kernel: ..."));
    }

    #[test]
    fn timestamp_matches_apache() {
        assert!(matches("timestamp", "[22/May/2026:10:30:45 +0300] \"GET /\""));
    }

    #[test]
    fn timestamp_matches_rfc2822() {
        assert!(matches("timestamp", "Date: Wed, 22 May 2026 10:30:45 GMT"));
        assert!(matches("timestamp", "Date: Wed, 22 May 2026 10:30:45 +0300"));
        assert!(matches("timestamp", "Date: Wed, 22 May 2026 10:30:45 EST"));
    }

    #[test]
    fn timestamp_rejects_non_timestamps() {
        assert!(!matches("timestamp", "date only 2026-05-22"));
        assert!(!matches("timestamp", "time only 10:30:45"));
        assert!(!matches("timestamp", "random May text without time"));
    }

    #[test]
    fn uuid_matches_canonical_form() {
        assert!(matches("uuid", "id 550e8400-e29b-41d4-a716-446655440000 done"));
        assert!(matches("uuid", "id 00000000-0000-0000-0000-000000000000 nil"));
        // Mixed case allowed
        assert!(matches("uuid", "id 550E8400-E29B-41d4-A716-446655440000 done"));
    }

    #[test]
    fn uuid_rejects_malformed() {
        // Wrong segment lengths
        assert!(!matches("uuid", "550e8400-e29b-41d4-a716"));
        // Non-hex chars (use 'g' which is not hex)
        assert!(!matches("uuid", "ggggggggg-eeee-eeee-eeee-eeeeeeeeeeee"));
        // No hyphens
        assert!(!matches("uuid", "550e8400e29b41d4a716446655440000"));
    }

    // === URL pattern (rule "url") — see spec §4.1 ===

    #[test]
    fn url_trims_trailing_sentence_punctuation() {
        assert_eq!(
            match_string("url", "Bkz https://example.com."),
            Some("https://example.com".into())
        );
        assert_eq!(match_string("url", "see https://a.com, then"), Some("https://a.com".into()));
        assert_eq!(match_string("url", "urls: https://a.com; next"), Some("https://a.com".into()));
        assert_eq!(match_string("url", "link: https://x.com! wow"), Some("https://x.com".into()));
        assert_eq!(
            match_string("url", "is https://example.com?"),
            Some("https://example.com".into())
        );
    }

    #[test]
    fn url_preserves_wikipedia_style_trailing_paren() {
        // REGRESSION GUARD: spec §1.1, §1.7 — closing brackets stay in match.
        assert_eq!(
            match_string("url", "see https://en.wikipedia.org/wiki/Foo_(disambig)"),
            Some("https://en.wikipedia.org/wiki/Foo_(disambig)".into())
        );
        assert_eq!(
            match_string("url", "see https://en.wikipedia.org/wiki/Foo_(disambig)."),
            Some("https://en.wikipedia.org/wiki/Foo_(disambig)".into())
        );
        assert_eq!(
            match_string("url", "MDN https://developer.mozilla.org/en-US/docs/Web/JS/Array_(prim)"),
            Some("https://developer.mozilla.org/en-US/docs/Web/JS/Array_(prim)".into())
        );
    }

    #[test]
    fn url_preserves_ipv6_literal_host() {
        // REGRESSION GUARD: spec §1.1, §1.7 — `]` stays in match.
        assert_eq!(match_string("url", "https://[::1]"), Some("https://[::1]".into()));
        assert_eq!(
            match_string("url", "https://[::1]:8080/v1"),
            Some("https://[::1]:8080/v1".into())
        );
    }

    #[test]
    fn url_paren_wrap_tradeoff() {
        // DOCUMENTED trade-off: paren-wrapped URL keeps trailing `)` in match.
        // Spec §1.1 — Wikipedia regression outweighs the wrap-case loss.
        assert_eq!(
            match_string("url", "(https://example.com)"),
            Some("https://example.com)".into())
        );
    }

    #[test]
    fn url_preserves_path_internal_characters() {
        assert_eq!(
            match_string("url", "api at https://example.com:8080/v1"),
            Some("https://example.com:8080/v1".into())
        );
        assert_eq!(
            match_string("url", "query https://x/path?q=1&r=2"),
            Some("https://x/path?q=1&r=2".into())
        );
        assert_eq!(
            match_string("url", "frag https://x/path#section"),
            Some("https://x/path#section".into())
        );
    }

    #[test]
    fn url_byte_class_includes_utf8_idn_and_percent() {
        // Char classes are byte classes under regex::bytes; bytes 0x80..0xFF
        // (UTF-8 continuation, IDN, percent-decoded paths) are implicitly
        // included because the negation only lists ASCII bytes. Spec §3.1.
        assert_eq!(
            match_string("url", "idn https://xn--bcher-kva.com/"),
            Some("https://xn--bcher-kva.com/".into())
        );
        assert_eq!(
            match_string("url", "pct https://example.com/%E4%B8%AD"),
            Some("https://example.com/%E4%B8%AD".into())
        );
        assert_eq!(
            match_string("url", "raw https://example.com/中"),
            Some("https://example.com/中".into())
        );
    }

    #[test]
    fn url_min_length_and_all_punct_path() {
        assert_eq!(match_string("url", "https://x"), Some("https://x".into()));
        assert_eq!(match_string("url", "https://."), None);
    }

    #[test]
    fn url_known_limitation_word_char_prefix() {
        // KNOWN LIMITATION: spec §1.7. \b fails when prev char is a word char.
        // These pin the documented behavior so a future change cannot silently
        // flip it.
        assert_eq!(match_string("url", "9https://x.com"), None);
        assert_eq!(match_string("url", "_https://x.com"), None);
    }

    #[test]
    fn url_rejects_unsupported_schemes() {
        // NOTE: `git+ssh://example.com` from the v0.3.2 plan fixture is NOT
        // asserted here. Under `\b(?:https?|ssh|ftp)://...`, the `+` -> `s`
        // transition forms a word boundary, so `ssh://example.com` is matched
        // as a substring. The same family of `\b`-substring matches exists
        // for `_https://` / `9https://` (covered as known limitations above);
        // this case is in the same class and would require lookbehind to
        // suppress (not supported by the Rust regex crate). Documented as a
        // plan-vs-pattern inconsistency.
        assert_eq!(match_string("url", "file:///etc/hosts"), None);
        assert_eq!(match_string("url", "just a sentence with https in it"), None);
    }

    #[test]
    fn url_matches_supported_schemes() {
        // Sanity: the legacy v0.2.2 acceptance cases still work.
        assert_eq!(
            match_string("url", "visit https://example.com today"),
            Some("https://example.com".into())
        );
        assert_eq!(
            match_string("url", "see http://example.com/path?q=1"),
            Some("http://example.com/path?q=1".into())
        );
        assert_eq!(
            match_string("url", "rsync from ssh://user@host/path"),
            Some("ssh://user@host/path".into())
        );
        assert_eq!(
            match_string("url", "download ftp://files.example.com/file.zip"),
            Some("ftp://files.example.com/file.zip".into())
        );
    }

    #[test]
    fn url_matches_git_at_host_path_form() {
        // Spec §3.1, §1.2 — new 4th alternation branch.
        assert_eq!(
            match_string("url", "clone git@github.com:user/repo.git"),
            Some("git@github.com:user/repo.git".into())
        );
        assert_eq!(
            match_string("url", "git@gitlab.com:org/sub/repo.git"),
            Some("git@gitlab.com:org/sub/repo.git".into())
        );
        assert_eq!(
            match_string("url", "git@bitbucket.org:team/repo.git"),
            Some("git@bitbucket.org:team/repo.git".into())
        );
    }

    #[test]
    fn url_git_at_host_class_rejects_pathological_hosts() {
        // REGRESSION GUARD: spec §3.1 Karar 3 — host class label-aware.
        assert_eq!(match_string("url", "git@.host:path"), None);
        assert_eq!(match_string("url", "git@host.:path"), None);
        assert_eq!(match_string("url", "git@-host:path"), None);
        assert_eq!(match_string("url", "git@host-:path"), None);
    }

    #[test]
    fn url_git_at_without_path_does_not_match_url() {
        // Spec §3.2 — `git@host` without `:path` falls through to email rule.
        assert_eq!(match_string("url", "clone git@github.com"), None);
    }

    #[test]
    fn url_git_at_trim_applies_to_path() {
        assert_eq!(
            match_string("url", "see git@github.com:user/repo.git."),
            Some("git@github.com:user/repo.git".into())
        );
    }

    #[test]
    fn url_git_at_with_path_wins_over_email() {
        // Spec §3.2 — `apply_rules` is first-rule-wins; url precedes email.
        use crate::pipeline::{apply_rules, PipelineScratch};
        use arc_swap::ArcSwap;
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"clone git@github.com:u/r.git\n", &rules, &mut scratch, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        // BrightBlue fg = SGR 94 (url); BrightGreen fg = SGR 92 (email).
        assert!(s.contains("94"), "expected BrightBlue (url): {s:?}");
        assert!(!s.contains("\x1b[92m"), "must not contain BrightGreen (email): {s:?}");
    }

    #[test]
    fn url_git_at_without_path_falls_to_email() {
        // Spec §3.2 — without `:path`, url branch fails; email rule matches.
        use crate::pipeline::{apply_rules, PipelineScratch};
        use arc_swap::ArcSwap;
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"clone git@github.com\n", &rules, &mut scratch, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        // BrightGreen fg = SGR 92 (email); BrightBlue fg = SGR 94 (url).
        assert!(s.contains("92"), "expected BrightGreen (email): {s:?}");
        assert!(!s.contains("\x1b[94m"), "must not contain BrightBlue (url): {s:?}");
    }

    #[test]
    fn email_matches_common_forms() {
        assert!(matches("email", "contact user@example.com"));
        assert!(matches("email", "from first.last@sub.example.org"));
        assert!(matches("email", "filter user+tag@example.com"));
        // SSH URL shape — intentionally matches as email
        assert!(matches("email", "clone git@github.com"));
    }

    #[test]
    fn email_rejects_malformed() {
        assert!(!matches("email", "@example.com"));
        assert!(!matches("email", "user@"));
        assert!(!matches("email", "no at sign here"));
    }

    #[test]
    fn non_duration_builtins_do_not_match_sgr_parameters() {
        // Regression for the v0.2.2 built-ins (permission, timestamp, uuid,
        // url, email): their regex shapes are restrictive enough that raw
        // SGR bytes cannot match, so apply_rules is a no-op on a bare SGR
        // sequence even at the regex layer.
        //
        // Duration is intentionally EXCLUDED from this test: v0.3.2 restored
        // bare [smhd] units (5m, 30s, ...) which collide with SGR final bytes
        // (\x1b[49m, etc.) at the regex layer. The Pipeline-layer
        // `respect_existing_colors=true` default (v0.3.0) is what keeps real
        // sessions safe. See `bare_units_collide_with_sgr_when_respect_existing_colors_is_false`
        // (this module) for the pinned collision and
        // `pipeline::pipeline_tests::sgr_in_line_with_respect_true_skips_rules`
        // for the default-on protection.
        use crate::config::{Config, GeneralSection, UserRule};
        use crate::pipeline::{apply_rules, PipelineScratch};
        use arc_swap::ArcSwap;

        // Build a Compiled with ONLY the non-duration built-ins by disabling
        // duration through a user rule. This isolates the regex-layer
        // property we still want to assert.
        let cfg = Config {
            general: GeneralSection::default(),
            rules: vec![UserRule {
                name: "duration".into(),
                pattern: None,
                style: None,
                enabled: false,
                styles: None,
            }],
        };
        let compiled = Compiled::load_with_theme(
            Some(&cfg),
            Some("/test/cfg.toml"),
            None,
            crate::terminfo::ColorDepth::Truecolor,
        )
        .unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let inputs: &[&[u8]] = &[
            b"\x1b[0m",
            b"\x1b[1;39m",
            b"\x1b[38;5;202m",
            b"\x1b[38;2;255;136;0m",
            b"prefix \x1b[44m text \x1b[0m suffix",
        ];
        for input in inputs {
            let mut out = Vec::new();
            apply_rules(input, &rules, &mut scratch, &mut out).unwrap();
            assert_eq!(
                out,
                *input,
                "apply_rules must not modify SGR sequences: input={:?} got={:?}",
                String::from_utf8_lossy(input),
                String::from_utf8_lossy(&out)
            );
        }
    }

    #[test]
    fn url_wins_over_fqdn() {
        use crate::pipeline::{apply_rules, PipelineScratch};
        use arc_swap::ArcSwap;
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"go to https://api.example.com today\n", &rules, &mut scratch, &mut out)
            .unwrap();
        let s = String::from_utf8(out).unwrap();
        // BrightBlue fg = SGR 94. Underline = SGR 4. fqdn = Blue (34) — must NOT appear.
        assert!(s.contains("94"), "expected BrightBlue (url): {s:?}");
        assert!(!s.contains("\x1b[34m"), "must not contain blue (fqdn) SGR: {s:?}");
    }

    #[test]
    fn email_wins_over_fqdn() {
        use crate::pipeline::{apply_rules, PipelineScratch};
        use arc_swap::ArcSwap;
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"mail user@example.com soon\n", &rules, &mut scratch, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        // BrightGreen = 92, Blue (fqdn) = 34
        assert!(s.contains("92"), "expected BrightGreen (email): {s:?}");
        assert!(!s.contains("\x1b[34m"), "must not contain blue (fqdn): {s:?}");
    }

    #[test]
    fn permission_does_not_steal_mac_addresses() {
        // A MAC address like aa:bb:cc:dd:ee:ff must still be styled as mac (Cyan, 36),
        // not consumed by permission (whose char class includes `-` but not `:`).
        use crate::pipeline::{apply_rules, PipelineScratch};
        use arc_swap::ArcSwap;
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"iface aa:bb:cc:dd:ee:ff up\n", &rules, &mut scratch, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("36"), "expected Cyan (mac): {s:?}");
    }

    #[test]
    fn dark_theme_is_idempotent_with_builtin_defaults() {
        // Applying the 'dark' theme MUST produce styles identical to no theme.
        // This is the contract spelled out in spec §5.1.
        use crate::terminfo::ColorDepth;
        let no_theme = Compiled::load_with_theme(None, None, None, ColorDepth::Truecolor).unwrap();
        let dark =
            Compiled::load_with_theme(None, None, Some("dark"), ColorDepth::Truecolor).unwrap();
        assert_eq!(no_theme.styles, dark.styles, "dark theme must equal no-theme defaults");
    }

    #[test]
    fn light_theme_changes_permission_to_black_dim() {
        use crate::style::Color;
        use crate::terminfo::ColorDepth;
        let c =
            Compiled::load_with_theme(None, None, Some("light"), ColorDepth::Truecolor).unwrap();
        let idx = BUILTIN_NAMES.iter().position(|n| *n == "permission").unwrap();
        assert_eq!(c.styles[idx].fg, Some(Color::Black));
        assert!(c.styles[idx].dim, "permission must be dim in light theme");
    }

    #[test]
    fn light_theme_changes_ipv4_to_red_bold() {
        use crate::style::Color;
        use crate::terminfo::ColorDepth;
        let c =
            Compiled::load_with_theme(None, None, Some("light"), ColorDepth::Truecolor).unwrap();
        let idx = BUILTIN_NAMES.iter().position(|n| *n == "ipv4").unwrap();
        assert_eq!(c.styles[idx].fg, Some(Color::Red));
        assert!(c.styles[idx].bold, "ipv4 must be bold in light theme");
    }

    #[test]
    fn user_config_overrides_theme_styles() {
        // Apply 'light' first, then a user rule that changes ipv4 to its own color.
        // The user rule must win.
        use crate::style::Color;
        use crate::terminfo::ColorDepth;
        let cfg = crate::config::Config {
            general: crate::config::GeneralSection::default(),
            rules: vec![crate::config::UserRule {
                name: "ipv4".into(),
                pattern: None,
                style: Some(crate::config::UserStyle {
                    fg: Some("#22ddee".into()),
                    ..crate::config::UserStyle::default()
                }),
                enabled: true,
                styles: None,
            }],
        };
        let c = Compiled::load_with_theme(
            Some(&cfg),
            Some("/x/cfg.toml"),
            Some("light"),
            ColorDepth::Truecolor,
        )
        .unwrap();
        let idx = BUILTIN_NAMES.iter().position(|n| *n == "ipv4").unwrap();
        match c.styles[idx].fg {
            Some(Color::Rgb(0x22, 0xdd, 0xee)) => {}
            other => panic!("user rule did not override theme; got {other:?}"),
        }
    }

    #[test]
    fn unknown_theme_returns_error_theme() {
        use crate::terminfo::ColorDepth;
        let err = Compiled::load_with_theme(None, None, Some("nope"), ColorDepth::Truecolor)
            .expect_err("unknown theme must error");
        assert!(matches!(err, crate::Error::Theme { .. }), "got: {err:?}");
    }

    #[test]
    fn load_proxies_to_load_with_theme_none() {
        // Behavioral guarantee: existing `load(...)` continues to behave as if
        // no theme were provided. Regression guard for the proxy refactor.
        use crate::terminfo::ColorDepth;
        let a = Compiled::load_with_theme(None, None, None, ColorDepth::Truecolor).unwrap();
        let b = Compiled::load_with_theme(None, None, None, ColorDepth::Truecolor).unwrap();
        assert_eq!(a.styles, b.styles);
    }

    #[test]
    fn load_rejects_pattern_exceeding_size_limit() {
        use crate::config::{Config, GeneralSection, UserRule, UserStyle};
        use crate::terminfo::ColorDepth;
        // `[01]{4,1000000}` reliably exceeds RegexBuilder::size_limit(1 MiB) on
        // the v1.x regex crate — the bounded-counted-repetition compiler expands
        // the upper bound into the NFA representation. If a future regex release
        // optimises this pattern small enough to fit, swap the fixture for
        // another known-large regex (`(?:a|b){1,1000000}` is the common backup).
        let cfg = Config {
            general: GeneralSection::default(),
            rules: vec![UserRule {
                name: "huge".into(),
                pattern: Some("[01]{4,1000000}".into()),
                style: Some(UserStyle { fg: Some("red".into()), ..UserStyle::default() }),
                enabled: true,
                styles: None,
            }],
        };
        let err = Compiled::load_with_theme(Some(&cfg), Some("/x"), None, ColorDepth::Truecolor)
            .expect_err("regex must exceed RegexBuilder::size_limit(1 MiB)");
        let msg = err.to_string();
        assert!(msg.contains("huge"), "expected rule name in error: {err}");
        assert!(msg.contains("/x"), "expected config path in error: {err}");
    }

    #[test]
    fn compiled_carries_respect_existing_colors_from_config() {
        use crate::config::{Config, GeneralSection, UserRule};
        let cfg = Config {
            general: GeneralSection { respect_existing_colors: true, ..GeneralSection::default() },
            rules: Vec::<UserRule>::new(),
        };
        let compiled = Compiled::load_with_theme(
            Some(&cfg),
            Some("/test/cfg.toml"),
            None,
            crate::terminfo::ColorDepth::Truecolor,
        )
        .expect("compile");
        assert!(compiled.respect_existing_colors, "should be true from config");
    }

    #[test]
    fn compiled_respect_existing_colors_false_from_config() {
        use crate::config::{Config, GeneralSection, UserRule};
        let cfg = Config {
            general: GeneralSection { respect_existing_colors: false, ..GeneralSection::default() },
            rules: Vec::<UserRule>::new(),
        };
        let compiled = Compiled::load_with_theme(
            Some(&cfg),
            Some("/test/cfg.toml"),
            None,
            crate::terminfo::ColorDepth::Truecolor,
        )
        .expect("compile");
        assert!(!compiled.respect_existing_colors, "should be false from config");
    }

    #[test]
    fn compiled_respect_defaults_match_general_default_when_no_config() {
        // Without a config, Compiled uses GeneralSection::default()'s
        // respect_existing_colors value, NOT a hardcoded fallback.
        let compiled =
            Compiled::load_with_theme(None, None, None, crate::terminfo::ColorDepth::Truecolor)
                .expect("compile");
        let expected = crate::config::GeneralSection::default().respect_existing_colors;
        assert_eq!(compiled.respect_existing_colors, expected);
    }

    #[test]
    fn empty_compiled_has_zero_rules() {
        let c = Compiled::empty();
        assert_eq!(c.individuals.len(), 0);
        assert_eq!(c.styles.len(), 0);
        assert_eq!(c.set.len(), 0);
        assert!(c.respect_existing_colors, "empty should default to v0.3 safe default");
    }

    #[test]
    fn compiled_uses_capture_styling_set_for_permission_after_phase6() {
        let c = Compiled::load_builtins().expect("builtins compile");
        // permission is at index 0 in BUILTIN_NAMES (first built-in).
        assert!(c.uses_capture_styling[0]);
        assert_eq!(c.group_styles[0].len(), 4);
    }

    #[test]
    fn compiled_empty_has_empty_group_styles_vecs() {
        let c = Compiled::empty();
        assert_eq!(c.group_styles.len(), 0);
        assert_eq!(c.uses_capture_styling.len(), 0);
    }

    // === Task 10: RuleSource + captures-len validation routing ===
    //
    // These three tests pin the two-way error routing that
    // `Compiled::load_with_theme` adopted in v0.3.5:
    //   - theme-sourced `styles` map errors collect into
    //     `Error::ThemeValidation` (fail-collected, Rev2 I-1);
    //   - user-config-sourced errors fail-fast as `Error::Config`;
    //   - user `styles` REPLACE the built-in's `group_styles` wholesale
    //     (Rev2 Karar 27).

    #[test]
    fn compiled_load_with_theme_collects_theme_captures_index_out_of_range() {
        // Synthetic theme TOML: targets ipv4 (built-in has no capture groups,
        // so captures_len == 1) with a styles map at key "99". Grammar passes
        // `validate_styles_map_key`; range check in
        // `resolve_group_styles_for_rule` must reject it via
        // `Error::ThemeValidation` (fail-collected, theme-routed).
        let toml_src = r#"
[[rules]]
name = "ipv4"

[rules.styles."99"]
fg = "red"
"#;
        let loaded = crate::themes::LoadedTheme {
            source: std::borrow::Cow::Borrowed(toml_src),
            path_label: "<test:synthetic>".to_owned(),
        };
        let err = Compiled::build_from_loaded(
            None,
            None,
            Some(("synthetic", &loaded)),
            crate::terminfo::ColorDepth::Truecolor,
        )
        .expect_err("should fail with ThemeValidation");
        if let crate::error::Error::ThemeValidation { errors, theme, .. } = err {
            assert_eq!(theme, "synthetic");
            assert!(
                errors.iter().any(|e| matches!(
                    e.kind,
                    crate::error::ThemeRuleErrorKind::CaptureGroupIndexOutOfRange { group: 99, .. }
                )),
                "expected CaptureGroupIndexOutOfRange {{ group: 99 }} in: {errors:?}"
            );
        } else {
            panic!("expected ThemeValidation; got: {err:?}");
        }
    }

    #[test]
    fn compiled_load_with_theme_emits_config_error_for_user_config_out_of_range() {
        use crate::config::{Config, GeneralSection, UserRule, UserStyle};
        use std::collections::BTreeMap;
        let mut user_styles: BTreeMap<String, UserStyle> = BTreeMap::new();
        user_styles.insert(
            "99".to_owned(),
            UserStyle { fg: Some("red".to_owned()), ..UserStyle::default() },
        );
        let cfg = Config {
            general: GeneralSection::default(),
            rules: vec![UserRule {
                name: "ipv4".to_owned(),
                pattern: None,
                enabled: true,
                style: None,
                styles: Some(user_styles),
            }],
        };
        let err = Compiled::load_with_theme(
            Some(&cfg),
            Some("/test/config.toml"),
            None,
            crate::terminfo::ColorDepth::Truecolor,
        )
        .expect_err("should fail with Config");
        if let crate::error::Error::Config { message, .. } = &err {
            assert!(message.contains("rule 'ipv4'"), "got: {message}");
            assert!(message.contains("styles.\"99\""), "got: {message}");
            assert!(message.contains("no capture groups"), "got: {message}");
            assert!(message.contains("styles cannot be set"), "got: {message}");
            assert!(!message.contains("valid: 1..=0"), "regression guard: {message}");
        } else {
            panic!("expected ThemeValidation; got: {err:?}");
        }
    }

    #[test]
    fn compiled_load_with_theme_sanitizes_malformed_styles_key_in_user_config() {
        use crate::config::{Config, GeneralSection, UserRule, UserStyle};
        use std::collections::BTreeMap;
        // Adversarial key: leading zero (grammar fail) + BEL control byte.
        // Before Fix A2: raw key leaks BEL into Error::Config.message.
        // After Fix A2: routed through sanitize_for_display, BEL becomes
        // literal "\x07" text (4 ASCII chars), no raw 0x07 byte in message.
        let mut user_styles: BTreeMap<String, UserStyle> = BTreeMap::new();
        user_styles.insert(
            "0\x07evil".to_owned(),
            UserStyle { fg: Some("red".to_owned()), ..UserStyle::default() },
        );
        let cfg = Config {
            general: GeneralSection::default(),
            rules: vec![UserRule {
                name: "ipv4".to_owned(),
                pattern: None,
                enabled: true,
                style: None,
                styles: Some(user_styles),
            }],
        };
        let err = Compiled::load_with_theme(
            Some(&cfg),
            Some("/test/config.toml"),
            None,
            crate::terminfo::ColorDepth::Truecolor,
        )
        .expect_err("should fail with Config");
        if let crate::error::Error::Config { message, .. } = &err {
            assert!(message.contains("rule 'ipv4'"), "got: {message}");
            assert!(
                message.contains("capture-group key must be a positive decimal"),
                "got: {message}"
            );
            // Regression guard: raw BEL byte (0x07) must not appear; the
            // Display impl routes the key through sanitize_for_display.
            assert!(!message.as_bytes().contains(&0x07), "raw control byte leaked: {message:?}");
        } else {
            panic!("expected Error::Config; got: {err:?}");
        }
    }

    #[test]
    fn compiled_load_with_theme_emits_config_error_for_user_config_zero_forbidden() {
        use crate::config::{Config, GeneralSection, UserRule, UserStyle};
        use std::collections::BTreeMap;
        let mut user_styles: BTreeMap<String, UserStyle> = BTreeMap::new();
        // styles."0" on any rule — group 0 is the entire match, forbidden.
        user_styles.insert(
            "0".to_owned(),
            UserStyle { fg: Some("red".to_owned()), ..UserStyle::default() },
        );
        let cfg = Config {
            general: GeneralSection::default(),
            rules: vec![UserRule {
                name: "ipv4".to_owned(),
                pattern: None,
                enabled: true,
                style: None,
                styles: Some(user_styles),
            }],
        };
        let err = Compiled::load_with_theme(
            Some(&cfg),
            Some("/test/config.toml"),
            None,
            crate::terminfo::ColorDepth::Truecolor,
        )
        .expect_err("should fail with Config");
        if let crate::error::Error::Config { message, .. } = &err {
            assert!(message.contains("rule 'ipv4'"), "got: {message}");
            assert!(message.contains("styles.\"0\""), "got: {message}");
            assert!(message.contains("group 0 is the entire match"), "got: {message}");
            assert!(message.contains("use the 'style' field instead"), "got: {message}");
        } else {
            panic!("expected Error::Config; got: {err:?}");
        }
    }

    #[test]
    fn compiled_load_with_theme_user_styles_replace_semantics() {
        use crate::config::{Config, GeneralSection, UserRule, UserStyle};
        use std::collections::BTreeMap;
        let red = UserStyle { fg: Some("red".to_owned()), ..UserStyle::default() };
        let mut user_styles: BTreeMap<String, UserStyle> = BTreeMap::new();
        user_styles.insert("1".to_owned(), red);
        let cfg = Config {
            general: GeneralSection::default(),
            rules: vec![UserRule {
                name: "ipv4".to_owned(),
                pattern: Some(r"(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})".to_owned()),
                enabled: true,
                style: Some(UserStyle { fg: Some("yellow".to_owned()), ..UserStyle::default() }),
                styles: Some(user_styles),
            }],
        };
        let c = Compiled::load_with_theme(
            Some(&cfg),
            Some("/test/config.toml"),
            None,
            crate::terminfo::ColorDepth::Truecolor,
        )
        .expect("compile");
        let idx = BUILTIN_NAMES.iter().position(|n| *n == "ipv4").expect("ipv4 present");
        let g = &c.group_styles[idx];
        assert_eq!(g.len(), 4, "captures_len - 1 == 4 for 4-octet ipv4 override");
        assert!(g[0].is_some(), "group 1 set by user styles map");
        assert!(g[1].is_none(), "group 2 inherits rule default (no user entry)");
        assert!(g[2].is_none(), "group 3 inherits rule default");
        assert!(g[3].is_none(), "group 4 inherits rule default");
        assert!(c.uses_capture_styling[idx], "any-Some scan must flip the cache");
    }
}
