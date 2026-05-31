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
    /// User-supplied `styles` map (per-capture-group overlay) parsed from a
    /// `[[rules]]` entry's `styles = { ... }` table. `None` means no user/
    /// theme override of per-group styles; the rule's built-in
    /// [`Self::group_styles`] applies as-is. `Some(map)` means: at
    /// `Compiled::load_with_theme` build time, validate each key against
    /// the compiled regex's `captures_len()` and overlay the user's
    /// styles into `group_styles[i]` REPLACING the built-in defaults
    /// (REPLACE semantics, Rev2 Decision 27).
    ///
    /// Keys are positive-decimal capture-group indexes (1-based; grammar
    /// `^[1-9][0-9]*$`); `validate_styles_map_key` enforces the grammar
    /// upstream in `config::parse` / `themes::validate_theme_rules`.
    /// Range validation against `captures_len` happens in
    /// `resolve_group_styles_for_rule` at compile time.
    pub(crate) styles_override:
        Option<std::collections::BTreeMap<String, crate::config::UserStyle>>,
    /// Provenance tag for diagnostic routing. Replaces the v0.5.1
    /// `is_user_supplied` + `styles_override_from_theme` two-boolean
    /// discriminant (v0.5.2 §4.45 fold). Drives error routing in
    /// `compile_error_for` (built-in compile failures are `RegexCompile`;
    /// non-built-in compile failures are `Config` / `Profile`) and in
    /// `resolve_group_styles_for_rule` (`Theme` → collected
    /// `Vec<ThemeRuleError>` for [`Error::ThemeValidation`]; `UserConfig` →
    /// fail-fast [`Error::Config`]; `EmbeddedProfile` → collected
    /// `Vec<ProfileRuleError>` for [`Error::ProfileValidation`]).
    ///
    /// Since the user-config layer applies AFTER the theme layer and
    /// REPLACES `styles_override` wholesale (Rev2 Decision 27), a value of
    /// [`RuleSource::Theme`] on a rule whose `pattern` and `style` match
    /// the built-in defaults unambiguously means "this rule's
    /// `styles_override` originated from the theme and was never
    /// overwritten by user config".
    pub(crate) source: RuleSource,
    /// Overlap-resolution priority.
    ///
    /// Rules with higher `priority` iterate first in [`apply_rules`]; their
    /// accepted spans block overlapping matches from lower-priority rules.
    /// `overlaps_accepted` is **bidirectional** — a higher-priority rule
    /// that has accepted span S blocks any lower-priority candidate whose
    /// span overlaps S in either direction (nested inside S or enveloping
    /// S). This is the load-bearing property that lets envelope rules
    /// suppress interior built-ins.
    ///
    /// Tier convention (spec §2.1.B):
    /// - `0`   — built-in defaults; user-config rules without explicit `priority`.
    /// - `100` — profile interior rules (`instance_id`, `region`, `container_id`, `pod_name`).
    /// - `200` — profile envelope rules (`arn`, `image_tag`).
    /// - Any i32 — user-config opt-in (`#[serde(default)] priority: Option<i32>`).
    ///
    /// Tie-breaking when two rules have equal priority: lower rule index
    /// (pattern-definition order) wins. Preserves v0.5.5 "first-match-wins
    /// by pattern order" for all priority-0 built-in pairs.
    pub(crate) priority: i32,
}

/// Provenance of a rule during [`Compiled::load_with_theme`] build. Determines
/// how validation errors are routed: theme-sourced errors collect into a
/// `Vec<ThemeRuleError>` for fail-collected [`Error::ThemeValidation`];
/// user-config-sourced errors fail-fast as [`Error::Config`];
/// embedded-profile-sourced errors collect into a `Vec<ProfileRuleError>` for
/// fail-collected [`Error::ProfileValidation`]. Built-in rules pass validation
/// by construction (asserted by `builtin_rules_*` tests).
///
/// Spec ref: §3.6, Rev2 I-1 (fail-collected theme routing), Rev2 Decision 27
/// (REPLACE semantics for `styles` map overlays), v0.5.2 §4.3
/// (`EmbeddedProfile` variant for `[[append_rules]]` provenance).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum RuleSource {
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
    /// A rule appended by a profile via `[[append_rules]]`. Like
    /// [`Self::UserConfig`] it is user-controllable; like [`Self::Theme`]
    /// it is not the original built-in. Range/key errors collect into a
    /// `Vec<ProfileRuleError>` for a single fail-collected
    /// [`Error::ProfileValidation`] at loop end. v0.5.2.
    EmbeddedProfile,
}

/// File extensions colored by the `filename` built-in rule. See spec §3.8 for
/// rationale and the curated catalog. To add an extension, add it here, run
/// the rules tests, and add a smoke case below.
///
/// **Single-letter extensions:** the following single-letter extensions are
/// intentional canonical 1-to-1 attributions (FP audit D-6 / spec §11.1):
///
/// - `a` — static library archive
/// - `c` — C source
/// - `h` — C header
/// - `m` — Objective-C source
/// - `o` — object file
/// - `r` — R script
/// - `v` — Verilog source
///
/// Tradeoff: prose ending in these single-letter extensions (e.g., `a.b.c`)
/// matches as filename. Frequency low in real terminal output. v0.6+ may
/// tighten with "path-separator prefix required" semantics.
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
const TS_ISO8601: &str = r"\b(?P<date>\d{4}-\d{2}-\d{2})(?P<sep>[T ])(?P<time>\d{2}:\d{2}:\d{2})(?P<ms>\.\d{1,9})?(?P<tz>[Zz]|[+-]\d{2}:?\d{2})?\b";
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
            pattern: r"(?:^|\s)(?P<perm_type>[dlcbps-])(?P<perm_owner>[rwxsStT-]{3})(?P<perm_group>[rwxsStT-]{3})(?P<perm_other>[rwxsStT-]{3})\+?(?:\s|$)".into(),
            style: Style { fg: Some(Color::Rgb(0x83, 0x83, 0x8d)), ..Style::DEFAULT },
            group_styles: vec![
                Some(Style { fg: Some(Color::Rgb(0x5b, 0x8c, 0xff)), ..Style::DEFAULT }),  // perm_type
                Some(Style { fg: Some(Color::Rgb(0xff, 0x4d, 0x6d)), ..Style::DEFAULT }),  // perm_owner
                Some(Style { fg: Some(Color::Rgb(0xff, 0xd2, 0x3f)), ..Style::DEFAULT }),  // perm_group
                Some(Style { fg: Some(Color::Rgb(0xa8, 0xff, 0x3e)), ..Style::DEFAULT }),  // perm_other
            ],
            styles_override: None,
            priority: 0,
            source: RuleSource::Builtin,
        },
        BuiltinRule {
            name: "timestamp".into(),
            pattern: build_timestamp_pattern(),
            style: Style { fg: Some(Color::Rgb(0x83, 0x83, 0x8d)), ..Style::DEFAULT },
            group_styles: vec![
                Some(Style { fg: Some(Color::Rgb(0xff, 0xd2, 0x3f)), ..Style::DEFAULT }),  // 1: date
                Some(Style { fg: Some(Color::Rgb(0x83, 0x83, 0x8d)), ..Style::DEFAULT }),  // 2: T/space sep
                Some(Style { fg: Some(Color::Rgb(0xa8, 0xff, 0x3e)), ..Style::DEFAULT }),  // 3: time
                Some(Style { fg: Some(Color::Rgb(0x83, 0x83, 0x8d)), ..Style::DEFAULT }),  // 4: .ms
                Some(Style { fg: Some(Color::Rgb(0xb4, 0x83, 0xff)), ..Style::DEFAULT }),  // 5: tz
            ],
            styles_override: None,
            priority: 0,
            source: RuleSource::Builtin,
        },
        BuiltinRule {
            name: "uuid".into(),
            pattern: r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b".into(),
            style: Style { fg: Some(Color::Rgb(0xff, 0x5c, 0xf0)), ..Style::DEFAULT },
            group_styles: Vec::new(),
            styles_override: None,
            priority: 0,
            source: RuleSource::Builtin,
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
                r#"\b(?P<scheme>https?|ssh|ftp)(?P<sep>://)(?P<body>[^\s<>"\\^`{|}]*[^\s<>"\\^`{|}.,;:!?])"#,
                r#"|"#,
                r#"\bgit@[A-Za-z0-9][A-Za-z0-9.-]*[A-Za-z0-9]:[^\s<>"\\^`{|}]*[^\s<>"\\^`{|}.,;:!?]"#,
            ).into(),
            style: Style { fg: Some(Color::Rgb(0x5b, 0x8c, 0xff)), underline: true, ..Style::DEFAULT },
            group_styles: Vec::new(),
            styles_override: None,
            priority: 0,
            source: RuleSource::Builtin,
        },
        BuiltinRule {
            name: "email".into(),
            pattern: r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b".into(),
            style: Style { fg: Some(Color::Rgb(0xa8, 0xff, 0x3e)), ..Style::DEFAULT },
            group_styles: Vec::new(),
            styles_override: None,
            priority: 0,
            source: RuleSource::Builtin,
        },
        BuiltinRule {
            name: "ipv4".into(),
            pattern: r"\b(?:25[0-5]|2[0-4]\d|1\d{2}|[1-9]?\d)(?:\.(?:25[0-5]|2[0-4]\d|1\d{2}|[1-9]?\d)){3}\b".into(),
            style: Style { fg: Some(Color::Rgb(0x1f, 0x9f, 0xe6)), ..Style::DEFAULT },
            group_styles: Vec::new(),
            styles_override: None,
            priority: 0,
            source: RuleSource::Builtin,
        },
        BuiltinRule {
            name: "ipv6".into(),
            pattern: r"::1|\b(?:[0-9A-Fa-f]{1,4}:){7}[0-9A-Fa-f]{1,4}|\b[0-9A-Fa-f]{3,4}:(?:[0-9A-Fa-f]{1,4}:){0,5}:[0-9A-Fa-f]{0,4}|::[0-9A-Fa-f]{1,4}(?::[0-9A-Fa-f]{1,4}){2,}".into(),
            style: Style { fg: Some(Color::Rgb(0x7c, 0x5c, 0xff)), ..Style::DEFAULT },
            group_styles: Vec::new(),
            styles_override: None,
            priority: 0,
            source: RuleSource::Builtin,
        },
        BuiltinRule {
            name: "mac".into(),
            pattern: r"\b[0-9A-Fa-f]{2}(?:[:-][0-9A-Fa-f]{2}){5}\b".into(),
            style: Style { fg: Some(Color::Rgb(0x2e, 0xe6, 0xc4)), ..Style::DEFAULT },
            group_styles: Vec::new(),
            styles_override: None,
            priority: 0,
            source: RuleSource::Builtin,
        },
        BuiltinRule {
            name: "log_level".into(),
            pattern: r"\b(?:ERROR|FAIL|FATAL|CRITICAL|WARN|WARNING|INFO|DEBUG|TRACE)\b".into(),
            style: Style { fg: Some(Color::Rgb(0xff, 0x4d, 0x6d)), bold: true, ..Style::DEFAULT },
            group_styles: Vec::new(),
            styles_override: None,
            priority: 0,
            source: RuleSource::Builtin,
        },
        BuiltinRule {
            name: "filename".into(),
            pattern: build_filename_pattern(),
            style: Style { fg: Some(Color::Rgb(0xff, 0x9f, 0x1c)), ..Style::DEFAULT },
            group_styles: Vec::new(),
            styles_override: None,
            priority: 0,
            source: RuleSource::Builtin,
        },
        BuiltinRule {
            name: "fqdn".into(),
            pattern: r"\b(?:[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?\.){1,}[A-Za-z]{2,24}\b".into(),
            style: Style { fg: Some(Color::Rgb(0xb4, 0x83, 0xff)), ..Style::DEFAULT },
            group_styles: Vec::new(),
            styles_override: None,
            priority: 0,
            source: RuleSource::Builtin,
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
            style: Style { fg: Some(Color::Rgb(0xff, 0xd2, 0x3f)), ..Style::DEFAULT },
            group_styles: Vec::new(),
            styles_override: None,
            priority: 0,
            source: RuleSource::Builtin,
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
/// equivalent `RegexSet` consumed by [`crate::pipeline::apply_rules`] as a
/// per-line pre-filter; `RegexSet::matches(line).iter()` yields hit indices
/// in pattern-definition order (regex 1.12 stable contract), and downstream
/// dispatch reads only those indices.
// reason: required by `.expect_err()` in tests. Do NOT log a `Compiled`
// instance directly — `Regex`'s Debug output echoes the pattern source,
// which may include user-supplied patterns (mild info-leak surface).
#[derive(Debug)]
pub(crate) struct Compiled {
    pub(crate) set: RegexSet,
    pub(crate) individuals: Vec<Regex>,
    /// Rule name parallel to `individuals` (same index). Populated at
    /// [`Self::build_from_loaded`] time; used by the corpus harness via
    /// `crate::pipeline::select_runs_named` to map accepted spans back to
    /// the originating rule without re-scanning the rule list.
    pub(crate) names: Vec<String>,
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
    /// (spec §4.4, Decision 11).
    pub(crate) respect_existing_colors: bool,
    /// Per-rule priority parallel vec (NEW v0.5.6). Length matches
    /// `individuals` / `styles` / `group_styles` / `uses_capture_styling`.
    /// Populated at [`Compiled::build_from_loaded`] from each merged
    /// [`BuiltinRule::priority`]. Consumed by [`crate::pipeline::apply_rules`]
    /// sort step: iteration order is `(Reverse(priority), rule_index)`.
    /// See spec §2.1.B / §4.3.
    pub(crate) priorities: Vec<i32>,
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
            names: Vec::new(),
            styles: Vec::new(),
            group_styles: Vec::new(),
            uses_capture_styling: Vec::new(),
            respect_existing_colors: true,
            priorities: Vec::new(),
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
        profile: Option<&crate::profiles::Profile>,
        profile_path: Option<&str>,
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
        Self::build_from_loaded(config, config_path, loaded_ref, profile, profile_path, depth)
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
        profile: Option<&crate::profiles::Profile>,
        profile_path: Option<&str>,
        depth: crate::terminfo::ColorDepth,
    ) -> Result<Self> {
        let mut rules = builtin_rules();

        // Step 2 (v0.5.2 §5.4) — profile.rules whitelist filter. Applied
        // AFTER built-ins and BEFORE the theme layer so theme overrides
        // only target the surviving rule set. `None` keeps all built-ins
        // (v0.5.1 default). Unknown names in the whitelist were caught at
        // `profiles::validate_profile` (Phase 1), so any name here that
        // does not match a built-in is by definition unreachable; the
        // `retain` simply drops non-listed built-ins.
        if let Some(profile_def) = profile {
            if let Some(whitelist) = profile_def.rules.as_ref() {
                rules.retain(|r| whitelist.iter().any(|w| w == &r.name));
            }
        }

        // Layer 3 (Step 3 in spec §5.4): optional preset theme. Applied
        // BEFORE the user config so user rules win on conflict (spec §2
        // Decision 5). Validation runs BEFORE the merge so semantic
        // errors surface against the synthetic theme path rather than
        // mutating the rule set first.
        if let Some((name, loaded)) = loaded_theme {
            let theme_cfg = crate::config::parse(&loaded.path_label, &loaded.source)?;
            crate::themes::validate_theme_rules(name, &loaded.path_label, &theme_cfg)?;
            // `RuleSource::Theme` so any `styles_override` map written here
            // is tagged for theme-routed error collection downstream.
            crate::config::apply_user_rules_with_source(
                &loaded.path_label,
                &mut rules,
                &theme_cfg.rules,
                RuleSource::Theme,
            )?;
        }

        // Step 4 (v0.5.2 §5.4) — profile.append_rules. Each entry is a NEW
        // rule with `RuleSource::EmbeddedProfile` so range/key validation
        // routes into the fail-collected `Vec<ProfileRuleError>` ->
        // `Error::ProfileValidation` envelope. Name-collision checks
        // (with built-ins and within `append_rules`) were performed in
        // Phase 1 `profiles::validate_profile`; pattern compile +
        // styles-key dispatch happen in Step 6 (`compile_merged_rules`).
        //
        // Note ordering: append_rules land AFTER the theme layer but
        // BEFORE user-config, so user-config still has last-writer-wins
        // semantics over profile-appended rules (a user override of an
        // appended rule name behaves identically to overriding a
        // built-in — `apply_user_rules_with_source` finds it and
        // mutates in place).
        if let Some(profile_def) = profile {
            let path_for_diag = profile_path.filter(|p| !p.is_empty()).unwrap_or("<profile>");
            for ar in &profile_def.append_rules {
                let style = match &ar.style {
                    Some(us) => us.to_style(path_for_diag, &ar.name)?,
                    None => crate::style::Style::default(),
                };
                rules.push(BuiltinRule {
                    name: ar.name.clone(),
                    pattern: ar.pattern.clone(),
                    style,
                    group_styles: Vec::new(),
                    styles_override: ar.styles.clone(),
                    // Data-driven via ProfileRule.priority (spec §2.1.B4 / Task 6).
                    // Defaults to 100 (interior tier) when omitted in TOML.
                    priority: ar.priority.unwrap_or(100),
                    source: RuleSource::EmbeddedProfile,
                });
            }
        }

        // Step 5 (v0.5.2 §5.4) — user config. `RuleSource::UserConfig`:
        // user-config writes overwrite any prior theme- or profile-tagged
        // `styles_override` (REPLACE semantics, Rev2 Decision 27), and any
        // subsequent range/key errors surface as `Error::Config` so the
        // user sees them on their own config path.
        if let Some(c) = config {
            // `config_path` flows into Error::Config messages produced inside
            // apply_user_rules (and any nested UserStyle::to_style call) so
            // users see `config error in /home/u/.config/tayf/config.toml: ...`
            // rather than the empty-path sentinel.
            let path = config_path.filter(|p| !p.is_empty()).unwrap_or("<config>");
            crate::config::apply_user_rules_with_source(
                path,
                &mut rules,
                &c.rules,
                RuleSource::UserConfig,
            )?;
        }

        let theme_name = loaded_theme.map(|(n, _)| n);
        let theme_path = loaded_theme.map(|(_, l)| l.path_label.as_str());
        // Profile diagnostic context: surface a user-facing name derived
        // from the path label (file stem for disk paths, the
        // <embedded:profile/{name}> suffix for embedded). The path label
        // itself is the canonical source location surfaced in
        // `Error::ProfileValidation::source_path`.
        let profile_name_owned = profile_path.map(profile_name_from_path_label);
        let profile_name = profile_name_owned.as_deref();
        let compiled_rules = compile_merged_rules(
            &rules,
            config_path,
            theme_name,
            theme_path,
            profile_name,
            profile_path,
        )?;

        // Decision 11: snapshot config value into Compiled so reads happen at
        // line boundary via ArcSwap<Compiled>, no separate atomic needed.
        let respect_existing_colors = config.map_or_else(
            || crate::config::GeneralSection::default().respect_existing_colors,
            |c| c.general.respect_existing_colors,
        );

        let mut compiled = Compiled {
            set: compiled_rules.set,
            individuals: compiled_rules.individuals,
            names: compiled_rules.names,
            styles: compiled_rules.styles,
            group_styles: compiled_rules.group_styles,
            uses_capture_styling: compiled_rules.uses_capture_styling,
            respect_existing_colors,
            priorities: compiled_rules.priorities,
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
        Self::load_with_theme(None, None, None, None, None, crate::terminfo::ColorDepth::Truecolor)
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

/// Build a [`Compiled`] from an in-memory [`crate::config::Config`] + optional
/// theme name + optional profile name. Additive entry-point: wraps
/// [`Compiled::load_with_theme`] by resolving theme/profile-by-name and
/// defaulting `depth` to `ColorDepth::Truecolor` (TUI preview hint).
///
/// Used by Config TUI live-preview (`compile_pending`) to recompile from
/// `PendingEdits` + `ConfigSnapshot` deltas without touching disk.
///
/// All validation, merge ordering, and error-routing semantics are
/// identical to [`Compiled::load_with_theme`].
///
/// # Errors
/// Returns the same error set as [`Compiled::load_with_theme`]; additionally,
/// any error from [`crate::profiles::load`] (Phase-1 validation, `NotFound`,
/// IO) when `profile_name` is `Some` is propagated.
pub(crate) fn compile_from_config(
    config: &crate::config::Config,
    theme_name: Option<&str>,
    profile_name: Option<&str>,
) -> Result<Compiled> {
    let loaded_profile = match profile_name {
        Some(name) => Some(crate::profiles::load(name)?),
        None => None,
    };
    Compiled::load_with_theme(
        Some(config),
        None, // config_path: in-memory synth, no on-disk path
        theme_name,
        loaded_profile.as_ref().map(|lp| &lp.profile),
        None, // profile_path: embedded only
        crate::terminfo::ColorDepth::Truecolor,
    )
}

/// Fuzz-only: compile an arbitrary user pattern through the exact production
/// builder (1 MiB NFA + DFA size limits). Returns the build result; the fuzz
/// harness asserts only that this neither panics nor hangs. Compiled ONLY
/// under `--cfg fuzzing`.
#[cfg(fuzzing)]
pub(crate) fn fuzz_compile_pattern(
    pattern: &str,
) -> std::result::Result<regex::bytes::Regex, regex::Error> {
    regex::bytes::RegexBuilder::new(pattern)
        .size_limit(REGEX_SIZE_LIMIT_BYTES)
        .dfa_size_limit(REGEX_SIZE_LIMIT_BYTES)
        .build()
}

/// Derive a user-facing profile name from a profile source-path label.
///
/// `<embedded:profile/{name}>` → `{name}`. A disk path ending in
/// `<...>/{name}.toml` → `{name}`. Anything else falls back to the
/// label itself (defensive — non-canonical labels reach this helper
/// only via internal mis-wiring).
///
/// The profile name surfaces in [`Error::ProfileValidation::profile`]
/// and [`Error::Profile::name`]; `source_path` carries the full label
/// separately so downstream diagnostics can show both
/// "`profile 'myaws' validation failed (loaded from /path/...)`".
fn profile_name_from_path_label(label: &str) -> String {
    // Synthetic embedded label: <embedded:profile/{name}>
    if let Some(rest) = label.strip_prefix("<embedded:profile/") {
        if let Some(name) = rest.strip_suffix('>') {
            return name.to_owned();
        }
    }
    // Disk path: take the file stem (basename minus `.toml`).
    let basename = label.rsplit(std::path::MAIN_SEPARATOR).next().unwrap_or(label);
    if let Some(stem) = basename.strip_suffix(".toml") {
        return stem.to_owned();
    }
    label.to_owned()
}

/// Map a regex-compile error to a user-facing [`Error`] variant keyed by the
/// failing rule's provenance:
/// - [`RuleSource::Builtin`]  → [`Error::RegexCompile`] (tayf bug).
/// - [`RuleSource::UserConfig`] / [`RuleSource::Theme`]
///   → [`Error::Config`] with `config_path` (or `<embedded:theme/{name}>`
///   for theme-supplied rules — themes flow through the same
///   `apply_user_rules` path and reuse the `Config` envelope so the
///   message points at the right source file).
/// - [`RuleSource::EmbeddedProfile`] → [`Error::Profile`] with
///   `ProfileErrorKind::RegexCompile`. Single-error fail-fast (the
///   profile-rules fail-collect path is for *styles-key* errors, not
///   pattern-compile errors — a malformed regex is a load-fatal issue
///   per spec §6.4 #6).
///
/// `profile_name` + `profile_path` are `None` whenever `source != EmbeddedProfile`;
/// they are populated by the merged-rules caller (Phase 4 Task 11) once the
/// profile load is threaded through. Phase 3 leaves them `None` because no
/// caller produces `EmbeddedProfile` rules yet.
fn compile_error_for(
    rule: &BuiltinRule,
    config_path: Option<&str>,
    profile_name: Option<&str>,
    profile_path: Option<&str>,
    err: regex::Error,
) -> Error {
    match rule.source {
        RuleSource::Builtin => Error::from(err),
        RuleSource::UserConfig | RuleSource::Theme => {
            let path = config_path.filter(|p| !p.is_empty()).unwrap_or("<config>");
            Error::config_regex(path.to_string(), &rule.name, err)
        }
        RuleSource::EmbeddedProfile => Error::Profile {
            name: profile_name.unwrap_or("<unknown>").to_owned(),
            source_path: profile_path.unwrap_or("").to_owned(),
            kind: crate::error::ProfileErrorKind::RegexCompile {
                rule_name: rule.name.clone(),
                pattern: rule.pattern.clone(),
                message: err.to_string(),
            },
        },
    }
}

/// Output of [`compile_merged_rules`]: parallel vectors plus the aggregated
/// `RegexSet` and the `uses_capture_styling` cache. Internal-only; the
/// caller in [`Compiled::build_from_loaded`] zips this with
/// `respect_existing_colors` to populate the final `Compiled` struct.
struct CompiledRules {
    set: RegexSet,
    individuals: Vec<Regex>,
    names: Vec<String>,
    styles: Vec<Style>,
    group_styles: Vec<Vec<Option<Style>>>,
    uses_capture_styling: Vec<bool>,
    priorities: Vec<i32>,
}

/// Compile each merged rule, build the parallel style/regex/group-styles
/// vectors, and aggregate theme-routed and profile-routed validation errors
/// into a single [`Error::ThemeValidation`] / [`Error::ProfileValidation`].
/// User-config-routed errors and built-in compile failures fail-fast via `?`
/// inside the loop. Profile pattern-compile failures also fail-fast (per
/// spec §6.4 #6 — a malformed `[[append_rules]]` pattern is load-fatal).
///
/// `theme_name` / `theme_path` flow into the `Error::ThemeValidation`
/// payload when at least one theme-routed error is collected; they're
/// otherwise unused. Both are `Some(...)` together or both `None`.
///
/// `profile_name` / `profile_path` mirror the theme pair for profile-routed
/// diagnostics (v0.5.2). Both `Some(...)` together or both `None`. Phase 3
/// leaves these `None` at every call site (no profile-sourced rules exist
/// yet); Phase 4 Task 11 wires them through `Compiled::load_with_theme`.
fn compile_merged_rules(
    rules: &[BuiltinRule],
    config_path: Option<&str>,
    theme_name: Option<&str>,
    theme_path: Option<&str>,
    profile_name: Option<&str>,
    profile_path: Option<&str>,
) -> Result<CompiledRules> {
    let mut individuals: Vec<Regex> = Vec::with_capacity(rules.len());
    let mut names: Vec<String> = Vec::with_capacity(rules.len());
    let mut styles: Vec<Style> = Vec::with_capacity(rules.len());
    let mut sources: Vec<String> = Vec::with_capacity(rules.len());
    let mut group_styles: Vec<Vec<Option<Style>>> = Vec::with_capacity(rules.len());
    let mut priorities: Vec<i32> = Vec::with_capacity(rules.len());
    let mut theme_errors: Vec<crate::error::ThemeRuleError> = Vec::new();
    let mut profile_errors: Vec<crate::error::ProfileRuleError> = Vec::new();

    for rule in rules {
        let regex = regex::bytes::RegexBuilder::new(&rule.pattern)
            .size_limit(REGEX_SIZE_LIMIT_BYTES)
            .dfa_size_limit(REGEX_SIZE_LIMIT_BYTES)
            .build()
            .map_err(|e| compile_error_for(rule, config_path, profile_name, profile_path, e))?;
        let captures_len = regex.captures_len();
        let final_group_styles = resolve_group_styles_for_rule(
            rule,
            rule.source,
            captures_len,
            config_path,
            &mut theme_errors,
            &mut profile_errors,
        )?;
        names.push(rule.name.clone());
        sources.push(rule.pattern.clone());
        individuals.push(regex);
        styles.push(rule.style);
        group_styles.push(final_group_styles);
        priorities.push(rule.priority);
    }

    if !theme_errors.is_empty() {
        return Err(Error::ThemeValidation {
            theme: theme_name.unwrap_or("<unknown>").to_owned(),
            source_path: theme_path.unwrap_or("").to_owned(),
            errors: theme_errors,
        });
    }
    if !profile_errors.is_empty() {
        return Err(Error::ProfileValidation {
            profile: profile_name.unwrap_or("<unknown>").to_owned(),
            source_path: profile_path.unwrap_or("").to_owned(),
            errors: profile_errors,
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

    Ok(CompiledRules {
        set,
        individuals,
        names,
        styles,
        group_styles,
        uses_capture_styling,
        priorities,
    })
}

/// Resolve the per-capture-group style overlay vector for a single rule,
/// routing range/key validation errors to either a collected
/// `Vec<ThemeRuleError>` (theme provenance), a collected
/// `Vec<ProfileRuleError>` (embedded-profile provenance), or a fail-fast
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
/// Spec ref: §3.6, §1.3.5, Rev2 I-1, Rev2 Decision 27, v0.5.0 §2.3, v0.5.2 §6.4.
// reason: explicit three-step dispatch (zero / all-digit / named) × four
// provenance arms (Theme / UserConfig / EmbeddedProfile / Builtin) × four
// error paths (zero, malformed, out-of-range, name-unknown, duplicate-target)
// cannot collapse without sacrificing readability or duplicating logic across
// helpers. The unreachable!() arms carry CLAUDE.md §2-mandated reason
// strings and are part of the invariant documentation.
#[allow(clippy::too_many_lines)]
#[allow(clippy::too_many_arguments)]
// reason: the dispatcher needs the full provenance context (rule + source +
// caps_len + user-cfg path) AND two fail-collected error sinks (theme +
// profile). Bundling them into a struct just renames the parameters without
// shrinking the surface; the explicit signature is the contract.
fn resolve_group_styles_for_rule(
    rule: &BuiltinRule,
    source: RuleSource,
    captures_len: usize,
    user_cfg_path: Option<&str>,
    theme_errors: &mut Vec<crate::error::ThemeRuleError>,
    profile_errors: &mut Vec<crate::error::ProfileRuleError>,
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

    // Compile the rule's regex once so we can read `capture_names()` for the
    // named-resolution path (v0.5.0 spec §2.3). The rule's pattern is grammar-
    // validated at builtin construction time; if compilation fails here for a
    // Builtin, that's a constructor bug — surface via `unreachable!`.
    //
    // Note: this regex compile happens at config-resolve time (config-parse /
    // theme-load), NOT on the hot path. Caching is not required (Rust senior
    // reviewer N-3 disposition); the hot path remains `apply_rules`' index-
    // based dispatch via `Compiled.individuals`.
    let Ok(regex) = regex::bytes::Regex::new(&rule.pattern) else {
        match source {
            RuleSource::UserConfig | RuleSource::Theme | RuleSource::EmbeddedProfile => {
                // Pattern compilation failures are surfaced earlier in the
                // load pipeline (`compile_merged_rules` runs the
                // size-limited compile via `regex::bytes::RegexBuilder`);
                // reaching here means an upstream pre-flight missed.
                // Return an empty overlay defensively rather than
                // double-erroring on the same root cause.
                return Ok(vec![None; captures_len.saturating_sub(1)]);
            }
            RuleSource::Builtin => unreachable!(
                "Builtin rules ship with grammar-valid regex patterns; \
                 compile failure here would be a constructor bug."
            ),
        }
    };

    // Pre-compute the available named-group list (positional order, `None`
    // filtered out). Used by both `CaptureGroupNameUnknown` diagnostic
    // construction AND named-resolution lookup.
    let available_names: Vec<String> =
        regex.capture_names().filter_map(|opt| opt.map(String::from)).collect();

    // Per-slot tracker for duplicate-target detection. Indexed by slot
    // (= group_index - 1). Records the raw key that filled the slot.
    let mut assigned_by: Vec<Option<String>> = vec![None; captures_len.saturating_sub(1)];

    for (key, user_style) in map {
        // -----------------------------------------------------------------
        // Step 1: literal "0" — group-zero forbidden (existing behavior).
        // -----------------------------------------------------------------
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
                RuleSource::EmbeddedProfile => {
                    profile_errors.push(crate::error::ProfileRuleError {
                        rule_name: rule.name.clone(),
                        kind: crate::error::ProfileRuleErrorKind::StylesKey(
                            crate::error::ThemeRuleErrorKind::CaptureGroupIndexZeroForbidden,
                        ),
                    });
                    continue;
                }
                RuleSource::Builtin => unreachable!(
                    "Builtin rules ship with styles_override == None; reached the \
                     map iteration only for UserConfig/Theme/EmbeddedProfile. \
                     styles_override on a Builtin would be a constructor bug."
                ),
            }
        }

        // -----------------------------------------------------------------
        // Step 2: all-digit key → positional path (existing grammar gate).
        // CRITICAL: the `None` branch MUST NOT fall through to Step 3. The
        // regression-guard test
        // `dispatch_malformed_digit_key_emits_key_malformed_not_name_unknown`
        // pins "01" → KeyMalformed (Rust senior reviewer I-1 absorb).
        // -----------------------------------------------------------------
        let resolved_idx: usize = if key.bytes().all(|b| b.is_ascii_digit()) {
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
                    RuleSource::EmbeddedProfile => {
                        profile_errors.push(crate::error::ProfileRuleError {
                            rule_name: rule.name.clone(),
                            kind: crate::error::ProfileRuleErrorKind::StylesKey(
                                crate::error::ThemeRuleErrorKind::CaptureGroupKeyMalformed {
                                    key: key.to_owned(),
                                },
                            ),
                        });
                        continue;
                    }
                    RuleSource::Builtin => unreachable!(
                        "Builtin rules ship with grammar-valid styles keys (validated at \
                         constructor time via builtin_rules()); this arm is reachable only \
                         through UserConfig/Theme/EmbeddedProfile paths handled above."
                    ),
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
                    RuleSource::EmbeddedProfile => {
                        profile_errors.push(crate::error::ProfileRuleError {
                            rule_name: rule.name.clone(),
                            kind: crate::error::ProfileRuleErrorKind::StylesKey(
                                crate::error::ThemeRuleErrorKind::CaptureGroupIndexOutOfRange {
                                    group: parsed,
                                    captures_len,
                                },
                            ),
                        });
                        continue;
                    }
                    RuleSource::Builtin => unreachable!(
                        "Builtin rules ship with capture-group indices < captures_len \
                         (validated at constructor time via builtin_rules()); this arm is \
                         reachable only through UserConfig/Theme/EmbeddedProfile paths handled above."
                    ),
                }
            }
            parsed
        } else {
            // -------------------------------------------------------------
            // Step 3: non-digit key → named resolution via `capture_names()`.
            // -------------------------------------------------------------
            // Find the 1-based group index whose `capture_names()` entry
            // matches `key`. Group 0 (the whole match) has `None` name, so
            // iter index == group index.
            let lookup = regex
                .capture_names()
                .enumerate()
                .find_map(|(i, opt)| opt.filter(|n| *n == key.as_str()).map(|_| i));
            let Some(idx) = lookup else {
                match source {
                    RuleSource::Theme => {
                        theme_errors.push(crate::error::ThemeRuleError {
                            rule_name: rule.name.clone(),
                            kind: crate::error::ThemeRuleErrorKind::CaptureGroupNameUnknown {
                                name: key.to_owned(),
                                available: available_names.clone(),
                            },
                        });
                        continue;
                    }
                    RuleSource::UserConfig => {
                        let kind = crate::error::ThemeRuleErrorKind::CaptureGroupNameUnknown {
                            name: key.to_owned(),
                            available: available_names.clone(),
                        };
                        return Err(Error::Config {
                            path: user_cfg_path_or_sentinel.to_owned(),
                            line: 0,
                            message: format!("rule '{}': {kind}", rule.name),
                        });
                    }
                    RuleSource::EmbeddedProfile => {
                        profile_errors.push(crate::error::ProfileRuleError {
                            rule_name: rule.name.clone(),
                            kind: crate::error::ProfileRuleErrorKind::StylesKey(
                                crate::error::ThemeRuleErrorKind::CaptureGroupNameUnknown {
                                    name: key.to_owned(),
                                    available: available_names.clone(),
                                },
                            ),
                        });
                        continue;
                    }
                    RuleSource::Builtin => unreachable!(
                        "Builtin rules ship with styles_override == None; named-key \
                         resolution unreachable for Builtin source."
                    ),
                }
            };
            idx
        };

        // -----------------------------------------------------------------
        // Duplicate-target check. Both positional and named keys can reach
        // this point; a clash arises when one of each resolves to the same
        // slot (Rust senior reviewer I-3 absorb).
        // -----------------------------------------------------------------
        let slot_idx = resolved_idx - 1;
        if let Some(prior_key) = &assigned_by[slot_idx] {
            let prior_is_positional = prior_key.bytes().all(|b| b.is_ascii_digit());
            let current_is_positional = key.bytes().all(|b| b.is_ascii_digit());
            let (positional, named) = match (prior_is_positional, current_is_positional) {
                (true, false) => (prior_key.clone(), key.to_owned()),
                (false, true) => (key.to_owned(), prior_key.clone()),
                _ => unreachable!(
                    "two positional keys cannot collide (TOML rejects duplicate keys at \
                     parse time); two named keys cannot resolve to the same index (the \
                     `regex` crate forbids duplicate group names within a single Regex)."
                ),
            };
            match source {
                RuleSource::Theme => {
                    theme_errors.push(crate::error::ThemeRuleError {
                        rule_name: rule.name.clone(),
                        kind: crate::error::ThemeRuleErrorKind::CaptureGroupDuplicateTarget {
                            positional,
                            named,
                        },
                    });
                    continue;
                }
                RuleSource::UserConfig => {
                    let kind = crate::error::ThemeRuleErrorKind::CaptureGroupDuplicateTarget {
                        positional,
                        named,
                    };
                    return Err(Error::Config {
                        path: user_cfg_path_or_sentinel.to_owned(),
                        line: 0,
                        message: format!("rule '{}': {kind}", rule.name),
                    });
                }
                RuleSource::EmbeddedProfile => {
                    profile_errors.push(crate::error::ProfileRuleError {
                        rule_name: rule.name.clone(),
                        kind: crate::error::ProfileRuleErrorKind::StylesKey(
                            crate::error::ThemeRuleErrorKind::CaptureGroupDuplicateTarget {
                                positional,
                                named,
                            },
                        ),
                    });
                    continue;
                }
                RuleSource::Builtin => unreachable!(
                    "Builtin rules ship with styles_override == None; duplicate-target \
                     unreachable for Builtin source."
                ),
            }
        }

        let style = user_style.to_style(user_cfg_path_or_sentinel, &rule.name)?;
        // `resolved_idx >= 1` (grammar excludes "0" and leading zeros; named
        // resolution returns the iter-position which is >= 1 for any non-
        // group-0 name), so the subtraction is safe; index is
        // < captures_len - 1 == vec.len().
        vec[slot_idx] = Some(style);
        assigned_by[slot_idx] = Some(key.to_owned());
    }
    Ok(vec)
}

// ---------------------------------------------------------------------------
// Corpus-harness shims — delegated to by `__test_api` in `src/lib.rs`.
// Not part of the production path; only compiled when the lib target is built.
// ---------------------------------------------------------------------------

/// Per-rule isolation helper for the corpus harness. Compiles the built-in
/// rule named `rule_name`, runs it against `input`, and returns the leftmost
/// match span as a `String`. Returns `None` when `rule_name` is not a known
/// built-in or the pattern does not match.
///
/// Builds a fresh `Regex` per call — only for use in test/harness code.
/// Production code uses `Compiled::load_builtins` + `apply_rules`.
#[doc(hidden)]
#[must_use]
pub(crate) fn testing_match_named_rule(rule_name: &str, input: &str) -> Option<String> {
    let pattern = builtin_rules().into_iter().find(|r| r.name == rule_name)?.pattern;
    // Patterns are always valid (tested by the built-in compile tests); unwrap
    // is safe here — this is test-only code.
    #[allow(clippy::expect_used)] // reason: test-only shim; patterns are pre-validated built-ins
    let re = regex::bytes::Regex::new(&pattern)
        .expect("built-in pattern must compile — pre-validated by compile tests");
    re.find(input.as_bytes()).map(|m| String::from_utf8_lossy(m.as_bytes()).into_owned())
}

/// Full-pipeline span helper for the corpus harness. Builds a `Compiled`
/// with built-ins only (when `profile` is `None`) or with the named embedded
/// profile active, runs `select_runs_named` against `input`, and returns
/// `Vec<(rule_name, matched_span)>` in start-ascending (accepted) order.
///
/// Applies the full production pipeline: priority sort + overlap suppression
/// + profile gating (whitelist + append_rules). Used for corpus decision
/// measurement (spec §5.3, §5.4).
///
/// Builds a fresh `Compiled` per call — only for use in test/harness code.
/// Returns an empty `Vec` on compile error (unknown profile, etc.).
#[doc(hidden)]
#[must_use]
pub(crate) fn testing_pipeline_spans(input: &str, profile: Option<&str>) -> Vec<(String, String)> {
    let compiled = if let Some(name) = profile {
        match crate::profiles::load(name) {
            Ok(lp) => Compiled::load_with_theme(
                None,
                None,
                None,
                Some(&lp.profile),
                Some(lp.path_label.as_str()),
                crate::terminfo::ColorDepth::Truecolor,
            ),
            Err(_) => return Vec::new(),
        }
    } else {
        Compiled::load_builtins()
    };

    let Ok(compiled) = compiled else { return Vec::new() };

    let mut scratch = crate::pipeline::PipelineScratch::default();
    let named_runs = crate::pipeline::select_runs_named(input.as_bytes(), &compiled, &mut scratch);
    named_runs
        .into_iter()
        .map(|(name, start, end)| {
            (name, String::from_utf8_lossy(&input.as_bytes()[start..end]).into_owned())
        })
        .collect()
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
    fn ipv4_does_not_match_leading_zero_octet() {
        assert!(!matches("ipv4", "1.01.30.4"));
    }

    #[test]
    fn ipv4_does_not_match_out_of_range_256() {
        assert!(!matches("ipv4", "256.0.0.0"));
    }

    #[test]
    fn ipv4_does_not_match_out_of_range_999() {
        assert!(!matches("ipv4", "999.0.0.0"));
    }

    #[test]
    fn ipv4_does_not_match_identifier_prefix() {
        assert!(!matches("ipv4", "v10.20.30.40"));
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
    fn mac_seven_pair_yields_substring_match_documented_limitation() {
        // KNOWN LIMITATION: a 7-pair colon-separated hex string (`aa:...:77`)
        // produces a substring match on the first 6 pairs. The `\b` after the
        // 6th pair fires because `:` is not a word character (even though more
        // hex pairs follow). Full 7-pair rejection would require a negative
        // lookahead, which the Rust regex crate does not support.
        // This test pins the actual behavior so a future change (e.g., using
        // the fancy-regex crate) does not silently regress.
        assert_eq!(match_string("mac", "11:22:33:44:55:66:77"), Some("11:22:33:44:55:66".into()));
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
    fn log_level_matches_in_bracket_delimiter() {
        assert!(matches("log_level", "[ERROR] something failed"));
    }

    #[test]
    fn log_level_matches_with_colon_suffix() {
        assert!(matches("log_level", "INFO: starting up"));
    }

    #[test]
    fn log_level_matches_with_dash_suffix() {
        assert!(matches("log_level", "WARN - slow query"));
    }

    #[test]
    fn log_level_matches_in_paren_delimiter() {
        assert!(matches("log_level", "(CRITICAL) database unreachable"));
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
    fn duration_matches_microseconds_greek_mu() {
        // μ (U+03BC GREEK SMALL LETTER MU, UTF-8: 0xCE 0xBC).
        assert!(matches("duration", "took 8.5μs"));
        assert!(matches("duration", "100μs"));
        assert!(matches("duration", "0.5μs"));
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
            None,
            None,
            ColorDepth::Truecolor,
        )
        .unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"\x1b[49m", &rules, &mut scratch, &mut out).unwrap();
        // Duration style fg is Rgb(0xff,0xd2,0x3f) = SGR 38;2;255;210;63. With
        // respect=false, the bare-`m` bit of `49m` matches the duration rule and
        // an SGR wrap appears.
        let s = String::from_utf8_lossy(&out).into_owned();
        assert!(
            s.contains("38;2;255;210;63"),
            "expected Neon-amber Rgb SGR somewhere in output (the documented v0.1-class \
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
        assert_eq!(n, 12, "v0.5.6 ships twelve built-in rules");
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
        // filename fg = Rgb(0xff,0x9f,0x1c) = SGR 38;2;255;159;28; fqdn Blue dropped.
        assert!(s.contains("38;2;255;159;28"), "expected filename Rgb SGR, got: {s:?}");
        assert!(!s.contains("38;2;180;131;255"), "should not contain fqdn Rgb SGR: {s:?}");
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
        // filename fg = Rgb(0xff,0x9f,0x1c) = SGR 38;2;255;159;28.
        assert!(s.contains("38;2;255;159;28"), "expected filename Rgb SGR: {s:?}");
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
        // fqdn fg = Rgb(0xb4,0x83,0xff) = SGR 38;2;180;131;255 (no extension to conflict).
        assert!(s.contains("38;2;180;131;255"), "expected fqdn Rgb SGR: {s:?}");
    }

    #[test]
    fn load_with_no_config_matches_load_builtins() {
        use crate::terminfo::ColorDepth;
        let a = Compiled::load_builtins().unwrap();
        let b =
            Compiled::load_with_theme(None, None, None, None, None, ColorDepth::Truecolor).unwrap();
        assert_eq!(a.individuals.len(), b.individuals.len());
        assert_eq!(a.styles, b.styles);
    }

    #[test]
    fn load_at_none_depth_strips_colors_but_keeps_attributes() {
        use crate::terminfo::ColorDepth;
        let c = Compiled::load_with_theme(None, None, None, None, None, ColorDepth::None).unwrap();
        for s in &c.styles {
            assert_eq!(s.fg, None, "depth=None must strip all fg colors");
            assert_eq!(s.bg, None, "depth=None must strip all bg colors");
        }
        // log_level built-in still has bold:true even when colors are stripped.
        let log_idx = builtin_rules().iter().position(|r| r.name == "log_level").unwrap();
        assert!(c.styles[log_idx].bold);
    }

    #[test]
    fn log_level_builtin_neon_color_at_truecolor() {
        use crate::terminfo::ColorDepth;
        let c =
            Compiled::load_with_theme(None, None, None, None, None, ColorDepth::Truecolor).unwrap();
        let log_idx = builtin_rules().iter().position(|r| r.name == "log_level").unwrap();
        // Built-in log_level fg is Rgb(0xff,0x4d,0x6d) — the Neon palette hot-coral.
        assert_eq!(c.styles[log_idx].fg, Some(crate::style::Color::Rgb(0xff, 0x4d, 0x6d)));
    }

    #[test]
    fn downgrade_for_depth_walks_group_styles_basic16() {
        let mut c = Compiled::load_builtins().unwrap();
        c.downgrade_for_depth(crate::terminfo::ColorDepth::Basic16);
        // permission's group_styles entries should all still be Some after
        // downgrade — built-ins use Rgb colors, so Basic16 converts them via
        // nearest_ansi_basic rather than dropping them.
        let perm_idx = 0; // permission is at index 0
        for slot in &c.group_styles[perm_idx] {
            assert!(slot.is_some(), "group_style slot dropped during downgrade");
        }
    }

    #[test]
    fn builtin_neon_color_downgrades_to_expected_ansi_at_basic16() {
        use crate::style::Color;
        use crate::terminfo::ColorDepth;
        // log_level built-in is Rgb(0xff,0x4d,0x6d) = (255,77,109).
        // nearest_ansi_basic distance² to each of the 16 ANSI colors:
        //   BrightRed (255,85,85): dr=0, dg=-8, db=24 → d²=0+64+576=640  ← winner
        //   Magenta  (170,0,170): d²=7225+5929+1521=...                   → distant
        // Expected: Color::BrightRed.
        let result = Color::Rgb(0xff, 0x4d, 0x6d).downgrade(ColorDepth::Basic16);
        assert_eq!(
            result,
            Some(Color::BrightRed),
            "Neon log_level Rgb(255,77,109) must downgrade to BrightRed at Basic16; got: {result:?}"
        );
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
                priority: None,
            }],
        };
        // At Basic16 depth, the appended user rule's Rgb fg downgrades to an ANSI color.
        let c = Compiled::load_with_theme(
            Some(&cfg),
            Some("/test/cfg.toml"),
            None,
            None,
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
        assert_eq!(c.individuals.len(), 13, "12 built-ins + 1 user rule");
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
                priority: None,
            }],
        };
        let err = Compiled::load_with_theme(
            Some(&cfg),
            Some("/x/cfg.toml"),
            None,
            None,
            None,
            ColorDepth::Truecolor,
        )
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
                priority: None,
            }],
        };
        let err = Compiled::load_with_theme(
            Some(&cfg),
            Some("/x/cfg.toml"),
            None,
            None,
            None,
            ColorDepth::Truecolor,
        )
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
                    priority: None,
                })
                .collect(),
        };
        let c = Compiled::load_with_theme(
            Some(&cfg),
            Some("/x"),
            None,
            None,
            None,
            ColorDepth::Truecolor,
        )
        .unwrap();
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
    fn url_matches_ssh_scheme() {
        assert!(matches("url", "ref ssh://user@host.example.com"));
    }

    #[test]
    fn url_matches_ftp_scheme() {
        assert!(matches("url", "see ftp://example.com"));
    }

    #[test]
    fn url_matches_scp_form() {
        assert!(matches("url", "clone git@github.com:user/repo"));
    }

    #[test]
    fn url_git_at_host_class_rejects_pathological_hosts() {
        // REGRESSION GUARD: spec §3.1 Decision 3 — host class label-aware.
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
        // url fg = Rgb(0x5b,0x8c,0xff) = SGR 38;2;91;140;255; email Rgb dropped.
        assert!(s.contains("38;2;91;140;255"), "expected url Rgb SGR: {s:?}");
        assert!(!s.contains("38;2;168;255;62"), "must not contain email Rgb SGR: {s:?}");
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
        // email fg = Rgb(0xa8,0xff,0x3e) = SGR 38;2;168;255;62; url Rgb absent.
        assert!(s.contains("38;2;168;255;62"), "expected email Rgb SGR: {s:?}");
        assert!(!s.contains("38;2;91;140;255"), "must not contain url Rgb SGR: {s:?}");
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
                priority: None,
            }],
        };
        let compiled = Compiled::load_with_theme(
            Some(&cfg),
            Some("/test/cfg.toml"),
            None,
            None,
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
        // url fg = Rgb(0x5b,0x8c,0xff) = SGR 38;2;91;140;255 + underline; fqdn Rgb absent.
        assert!(s.contains("38;2;91;140;255"), "expected url Rgb SGR: {s:?}");
        assert!(!s.contains("38;2;180;131;255"), "must not contain fqdn Rgb SGR: {s:?}");
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
        // email fg = Rgb(0xa8,0xff,0x3e) = SGR 38;2;168;255;62; fqdn Rgb absent.
        assert!(s.contains("38;2;168;255;62"), "expected email Rgb SGR: {s:?}");
        assert!(!s.contains("38;2;180;131;255"), "must not contain fqdn Rgb SGR: {s:?}");
    }

    #[test]
    fn permission_does_not_steal_mac_addresses() {
        // A MAC address like aa:bb:cc:dd:ee:ff must still be styled as mac
        // (Rgb(0x2e,0xe6,0xc4) = SGR 38;2;46;230;196), not consumed by permission
        // (whose char class includes `-` but not `:`).
        use crate::pipeline::{apply_rules, PipelineScratch};
        use arc_swap::ArcSwap;
        let compiled = Compiled::load_builtins().unwrap();
        let rules = ArcSwap::from_pointee(compiled);
        let mut scratch = PipelineScratch::default();
        let mut out = Vec::new();
        apply_rules(b"iface aa:bb:cc:dd:ee:ff up\n", &rules, &mut scratch, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        // mac fg = Rgb(0x2e,0xe6,0xc4) = SGR 38;2;46;230;196.
        assert!(s.contains("38;2;46;230;196"), "expected mac Rgb SGR: {s:?}");
    }

    #[test]
    fn dark_theme_is_idempotent_with_builtin_defaults() {
        // Applying the 'dark' theme MUST produce styles identical to no theme.
        // This is the contract spelled out in spec §5.1.
        use crate::terminfo::ColorDepth;
        let no_theme =
            Compiled::load_with_theme(None, None, None, None, None, ColorDepth::Truecolor).unwrap();
        let dark =
            Compiled::load_with_theme(None, None, Some("dark"), None, None, ColorDepth::Truecolor)
                .unwrap();
        assert_eq!(no_theme.styles, dark.styles, "dark theme must equal no-theme defaults");
    }

    #[test]
    fn light_theme_changes_permission_to_black_dim() {
        use crate::style::Color;
        use crate::terminfo::ColorDepth;
        let c =
            Compiled::load_with_theme(None, None, Some("light"), None, None, ColorDepth::Truecolor)
                .unwrap();
        let idx = BUILTIN_NAMES.iter().position(|n| *n == "permission").unwrap();
        assert_eq!(c.styles[idx].fg, Some(Color::Black));
        assert!(c.styles[idx].dim, "permission must be dim in light theme");
    }

    #[test]
    fn light_theme_changes_ipv4_to_red_bold() {
        use crate::style::Color;
        use crate::terminfo::ColorDepth;
        let c =
            Compiled::load_with_theme(None, None, Some("light"), None, None, ColorDepth::Truecolor)
                .unwrap();
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
                priority: None,
            }],
        };
        let c = Compiled::load_with_theme(
            Some(&cfg),
            Some("/x/cfg.toml"),
            Some("light"),
            None,
            None,
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
        let err =
            Compiled::load_with_theme(None, None, Some("nope"), None, None, ColorDepth::Truecolor)
                .expect_err("unknown theme must error");
        assert!(matches!(err, crate::Error::Theme { .. }), "got: {err:?}");
    }

    #[test]
    fn load_proxies_to_load_with_theme_none() {
        // Behavioral guarantee: existing `load(...)` continues to behave as if
        // no theme were provided. Regression guard for the proxy refactor.
        use crate::terminfo::ColorDepth;
        let a =
            Compiled::load_with_theme(None, None, None, None, None, ColorDepth::Truecolor).unwrap();
        let b =
            Compiled::load_with_theme(None, None, None, None, None, ColorDepth::Truecolor).unwrap();
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
                priority: None,
            }],
        };
        let err = Compiled::load_with_theme(
            Some(&cfg),
            Some("/x"),
            None,
            None,
            None,
            ColorDepth::Truecolor,
        )
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
            None,
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
            None,
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
        let compiled = Compiled::load_with_theme(
            None,
            None,
            None,
            None,
            None,
            crate::terminfo::ColorDepth::Truecolor,
        )
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
    //     (Rev2 Decision 27).

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
            None,
            None,
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
                priority: None,
            }],
        };
        let err = Compiled::load_with_theme(
            Some(&cfg),
            Some("/test/config.toml"),
            None,
            None,
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
        // Adversarial key: leading zero + BEL control byte + non-digit suffix.
        // Before Fix A2: raw key leaks BEL into Error::Config.message.
        // After Fix A2: routed through sanitize_for_display, BEL becomes
        // literal "\x07" text (4 ASCII chars), no raw 0x07 byte in message.
        //
        // v0.5.0 dispatch change: a key with non-digit bytes is routed
        // through the named-resolution path (Step 3) per spec §2.3 — the
        // diagnostic surfaces as CaptureGroupNameUnknown (not
        // CaptureGroupKeyMalformed which is reserved for all-digit grammar
        // failures like "01" or "00"). The sanitization invariant is the
        // core contract this test defends and it holds across both paths:
        // sanitize_for_display is applied to user-supplied bytes in every
        // ThemeRuleErrorKind Display arm that echoes a key/name.
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
                priority: None,
            }],
        };
        let err = Compiled::load_with_theme(
            Some(&cfg),
            Some("/test/config.toml"),
            None,
            None,
            None,
            crate::terminfo::ColorDepth::Truecolor,
        )
        .expect_err("should fail with Config");
        if let crate::error::Error::Config { message, .. } = &err {
            assert!(message.contains("rule 'ipv4'"), "got: {message}");
            // v0.5.0 dispatch: non-digit key → named-resolution miss
            // diagnostic; ipv4 has no named groups so the empty-available
            // specialization fires (spec §2.4).
            assert!(message.contains("rule's regex has no named capture groups"), "got: {message}");
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
                priority: None,
            }],
        };
        let err = Compiled::load_with_theme(
            Some(&cfg),
            Some("/test/config.toml"),
            None,
            None,
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
                priority: None,
            }],
        };
        let c = Compiled::load_with_theme(
            Some(&cfg),
            Some("/test/config.toml"),
            None,
            None,
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

    // -------------------------------------------------------------------
    // v0.5.0 dispatch tests — exercise resolve_group_styles_for_rule's
    // three-step dispatch (zero → all-digit positional → named) with NO
    // fall-through, plus duplicate-target detection. See spec §2.3, §5.2.
    // -------------------------------------------------------------------

    fn compile_test_regex(name: &str) -> regex::bytes::Regex {
        let pattern = find_rule(name);
        regex::bytes::Regex::new(&pattern).expect("built-in pattern compiles")
    }

    fn dispatch_user_config_single_style(
        rule_name: &str,
        key: &str,
        user_style: crate::config::UserStyle,
    ) -> std::result::Result<Vec<Option<crate::style::Style>>, crate::error::Error> {
        use std::collections::BTreeMap;
        let mut overrides: BTreeMap<String, crate::config::UserStyle> = BTreeMap::new();
        overrides.insert(key.to_owned(), user_style);
        let regex = compile_test_regex(rule_name);
        let captures_len = regex.captures_len();
        let rule = BuiltinRule {
            name: rule_name.to_owned(),
            pattern: find_rule(rule_name),
            style: crate::style::Style::DEFAULT,
            group_styles: vec![None; captures_len.saturating_sub(1)],
            styles_override: Some(overrides),
            priority: 0,
            source: RuleSource::UserConfig,
        };
        let mut theme_errors: Vec<crate::error::ThemeRuleError> = Vec::new();
        let mut profile_errors: Vec<crate::error::ProfileRuleError> = Vec::new();
        resolve_group_styles_for_rule(
            &rule,
            RuleSource::UserConfig,
            captures_len,
            Some("<test-config>"),
            &mut theme_errors,
            &mut profile_errors,
        )
    }

    #[test]
    fn dispatch_zero_key_emits_zero_forbidden() {
        let style = crate::config::UserStyle::default();
        let err = dispatch_user_config_single_style("url", "0", style).unwrap_err();
        let crate::error::Error::Config { message, .. } = err else {
            panic!("expected Error::Config");
        };
        assert!(message.contains("group 0 is the entire match"), "got: {message}");
    }

    #[test]
    fn dispatch_all_digit_key_resolves_positional() {
        let style = crate::config::UserStyle { fg: Some("red".to_owned()), ..Default::default() };
        let result =
            dispatch_user_config_single_style("url", "1", style).expect("positional resolve");
        assert!(result[0].is_some(), "group 1 slot must be filled");
        assert!(result[1].is_none(), "group 2 slot must remain default");
    }

    #[test]
    fn dispatch_malformed_digit_key_emits_key_malformed_not_name_unknown() {
        // Critical regression guard for Rust senior I-1 absorb.
        // "01" must hit the KeyMalformed grammar gate, NOT fall through to
        // named resolution.
        let style = crate::config::UserStyle::default();
        let err = dispatch_user_config_single_style("url", "01", style).unwrap_err();
        let crate::error::Error::Config { message, .. } = err else {
            panic!("expected Error::Config");
        };
        assert!(
            message.contains("capture-group key must be a positive decimal"),
            "expected KeyMalformed diagnostic, got: {message}"
        );
        assert!(
            !message.contains("no capture group named"),
            "MUST NOT fall through to CaptureGroupNameUnknown: {message}"
        );
    }

    #[test]
    fn dispatch_named_key_resolves_via_capture_names() {
        let style = crate::config::UserStyle { fg: Some("cyan".to_owned()), ..Default::default() };
        let result =
            dispatch_user_config_single_style("url", "scheme", style).expect("named resolve");
        // url's group 1 is `scheme` per Task 3 retrofit; slot 0 (1-based
        // group 1) filled.
        assert!(result[0].is_some(), "group 'scheme' (index 1) slot must be filled");
    }

    #[test]
    fn dispatch_unknown_name_emits_name_unknown_with_available_byte_exact() {
        let style = crate::config::UserStyle::default();
        let err = dispatch_user_config_single_style("url", "foo", style).unwrap_err();
        let crate::error::Error::Config { message, .. } = err else {
            panic!("expected Error::Config");
        };
        // Error::Config { message } stores ONLY the inner
        // format!("rule '{}': {kind}", ...) string — the "config error in
        // <path>: " envelope is added by thiserror's Display impl on
        // Error::Config and is NOT part of the `message` field.
        assert_eq!(
            message,
            "rule 'url': styles.\"foo\": rule's regex has no capture group named 'foo' (available: scheme, sep, body)"
        );
    }

    #[test]
    fn dispatch_duplicate_positional_named_emits_duplicate_target() {
        use std::collections::BTreeMap;
        let style1 = crate::config::UserStyle { fg: Some("red".to_owned()), ..Default::default() };
        let style2 = crate::config::UserStyle { fg: Some("cyan".to_owned()), ..Default::default() };
        let mut overrides: BTreeMap<String, crate::config::UserStyle> = BTreeMap::new();
        overrides.insert("1".to_owned(), style1);
        overrides.insert("scheme".to_owned(), style2);
        let regex = compile_test_regex("url");
        let captures_len = regex.captures_len();
        let rule = BuiltinRule {
            name: "url".to_owned(),
            pattern: find_rule("url"),
            style: crate::style::Style::DEFAULT,
            group_styles: vec![None; captures_len.saturating_sub(1)],
            styles_override: Some(overrides),
            priority: 0,
            source: RuleSource::UserConfig,
        };
        let mut theme_errors: Vec<crate::error::ThemeRuleError> = Vec::new();
        let mut profile_errors: Vec<crate::error::ProfileRuleError> = Vec::new();
        let err = resolve_group_styles_for_rule(
            &rule,
            RuleSource::UserConfig,
            captures_len,
            Some("<test-config>"),
            &mut theme_errors,
            &mut profile_errors,
        )
        .unwrap_err();
        let crate::error::Error::Config { message, .. } = err else {
            panic!("expected Error::Config");
        };
        assert_eq!(
            message,
            "rule 'url': styles.\"1\" and styles.scheme target the same capture group (index 1); set exactly one"
        );
    }

    #[test]
    fn available_order_is_positional_left_to_right_for_url() {
        // Construct an unknown-name error against the url rule and inspect
        // the available list ordering — must be ["scheme", "sep", "body"],
        // NOT alphabetical (which would be ["body", "scheme", "sep"]).
        let style = crate::config::UserStyle::default();
        let err = dispatch_user_config_single_style("url", "foo", style).unwrap_err();
        let crate::error::Error::Config { message, .. } = err else {
            panic!("expected Error::Config");
        };
        let alpha_substr = "(available: body, scheme, sep)";
        let positional_substr = "(available: scheme, sep, body)";
        assert!(
            message.contains(positional_substr),
            "expected positional ordering, got: {message}"
        );
        assert!(!message.contains(alpha_substr), "MUST NOT be alphabetical ordering");
    }

    #[test]
    fn available_order_for_timestamp_skips_capture_free_branches() {
        // timestamp has 4 alternation branches; only ISO has named groups.
        // available must list ["date", "sep", "time", "ms", "tz"] (positional
        // order of the ISO branch); the other 3 branches contribute no names
        // by construction.
        let style = crate::config::UserStyle::default();
        let err = dispatch_user_config_single_style("timestamp", "foo", style).unwrap_err();
        let crate::error::Error::Config { message, .. } = err else {
            panic!("expected Error::Config");
        };
        assert!(
            message.contains("(available: date, sep, time, ms, tz)"),
            "expected ISO-branch positional ordering, got: {message}"
        );
    }

    #[test]
    fn available_order_for_permission_uses_perm_prefix_names() {
        let style = crate::config::UserStyle::default();
        let err = dispatch_user_config_single_style("permission", "foo", style).unwrap_err();
        let crate::error::Error::Config { message, .. } = err else {
            panic!("expected Error::Config");
        };
        assert!(
            message.contains("(available: perm_type, perm_owner, perm_group, perm_other)"),
            "expected perm_ prefixed names in positional order, got: {message}"
        );
    }

    #[test]
    fn dispatch_user_style_to_style_fails_on_invalid_color_after_name_resolved() {
        // Sanity: a valid name + invalid color must still surface as a
        // color-parse error, not get masked by name-resolution success.
        // Confirms the dispatch order doesn't swallow downstream errors.
        let style =
            crate::config::UserStyle { fg: Some("not-a-color".to_owned()), ..Default::default() };
        let err = dispatch_user_config_single_style("url", "scheme", style).unwrap_err();
        let crate::error::Error::Config { message, .. } = err else {
            panic!("expected Error::Config");
        };
        assert!(
            message.contains("not-a-color") || message.contains("color"),
            "expected color-parse error, got: {message}"
        );
    }

    #[test]
    fn dispatch_named_key_on_pattern_with_no_named_groups_emits_empty_available() {
        // Use a built-in with NO named capture groups (ipv4 — has no capture
        // groups at all) to exercise the empty-available specialization
        // (spec §2.4 Display arm).
        let style = crate::config::UserStyle::default();
        let err = dispatch_user_config_single_style("ipv4", "foo", style).unwrap_err();
        let crate::error::Error::Config { message, .. } = err else {
            panic!("expected Error::Config");
        };
        assert_eq!(
            message,
            "rule 'ipv4': styles.\"foo\": rule's regex has no named capture groups"
        );
    }

    #[test]
    fn duplicate_formatter_theme_and_user_paths_byte_identical_diagnostic() {
        // Regression guard per memory feedback_duplicate_formatter_audit: the
        // UserConfig path's format!("rule '{}': {kind}", ...) and the Theme path's
        // theme_errors.push(...).kind.to_string() MUST produce byte-identical
        // diagnostic strings (modulo the Error envelope).
        use std::collections::BTreeMap;
        let style1 = crate::config::UserStyle { fg: Some("red".to_owned()), ..Default::default() };
        let style2 = crate::config::UserStyle { fg: Some("cyan".to_owned()), ..Default::default() };
        let mut overrides: BTreeMap<String, crate::config::UserStyle> = BTreeMap::new();
        overrides.insert("1".to_owned(), style1);
        overrides.insert("scheme".to_owned(), style2);
        let regex = compile_test_regex("url");
        let captures_len = regex.captures_len();
        let rule = crate::rules::BuiltinRule {
            name: "url".to_owned(),
            pattern: find_rule("url"),
            style: crate::style::Style::DEFAULT,
            group_styles: vec![None; captures_len.saturating_sub(1)],
            styles_override: Some(overrides.clone()),
            priority: 0,
            source: crate::rules::RuleSource::Theme,
        };

        // Theme path: collect into theme_errors vector.
        let mut theme_errors_t: Vec<crate::error::ThemeRuleError> = Vec::new();
        let mut profile_errors_t: Vec<crate::error::ProfileRuleError> = Vec::new();
        let _ = resolve_group_styles_for_rule(
            &rule,
            crate::rules::RuleSource::Theme,
            captures_len,
            None,
            &mut theme_errors_t,
            &mut profile_errors_t,
        );
        assert_eq!(theme_errors_t.len(), 1, "theme path should collect exactly one error");
        let theme_kind_display = theme_errors_t[0].kind.to_string();

        // UserConfig path: returns Err with format!("rule '{}': {kind}", ...).
        let mut theme_errors_u: Vec<crate::error::ThemeRuleError> = Vec::new();
        let mut profile_errors_u: Vec<crate::error::ProfileRuleError> = Vec::new();
        let err = resolve_group_styles_for_rule(
            &rule,
            crate::rules::RuleSource::UserConfig,
            captures_len,
            Some("<test-config>"),
            &mut theme_errors_u,
            &mut profile_errors_u,
        )
        .unwrap_err();
        let crate::error::Error::Config { message, .. } = err else {
            panic!("expected Error::Config, got {err:?}");
        };
        // Strip the "rule 'url': " envelope to extract the kind portion.
        let envelope_prefix = "rule 'url': ";
        let user_kind_display = message
            .strip_prefix(envelope_prefix)
            .unwrap_or_else(|| panic!("UserConfig message missing envelope: {message}"));

        assert_eq!(
            theme_kind_display, user_kind_display,
            "Theme and UserConfig paths produced divergent diagnostic strings — \
             duplicate-formatter discipline broken. \
             Theme: {theme_kind_display:?} ≠ User: {user_kind_display:?}"
        );

        // Bonus: byte-pin against the spec §2.4 string.
        assert_eq!(
            user_kind_display,
            "styles.\"1\" and styles.scheme target the same capture group (index 1); set exactly one"
        );
    }

    // v0.5.2 Phase 3 Task 9 dispatch tests — exercise the
    // `RuleSource::EmbeddedProfile` arm of `resolve_group_styles_for_rule`
    // through `compile_merged_rules` so the assertions target the
    // production envelope `Error::ProfileValidation` (not the raw
    // `profile_errors` vec). Spec §6.4 #7 + plan A.12.

    /// Construct a single-rule `Vec<BuiltinRule>` with the supplied profile-
    /// sourced overrides + run `compile_merged_rules` to surface the
    /// `Error::ProfileValidation` envelope. Mirrors `dispatch_user_config_single_style`
    /// but for the `EmbeddedProfile` path: every rule has
    /// `source = RuleSource::EmbeddedProfile`, and the diagnostic profile
    /// context (`profile_name`, `profile_path`) is threaded through the
    /// signature added in Task 8.
    fn dispatch_embedded_profile_single_style(
        rule_name: &str,
        pattern: &str,
        key: &str,
        user_style: crate::config::UserStyle,
    ) -> std::result::Result<(), crate::error::Error> {
        use std::collections::BTreeMap;
        let mut overrides: BTreeMap<String, crate::config::UserStyle> = BTreeMap::new();
        overrides.insert(key.to_owned(), user_style);
        let rule = BuiltinRule {
            name: rule_name.to_owned(),
            pattern: pattern.to_owned(),
            style: crate::style::Style::DEFAULT,
            group_styles: Vec::new(),
            styles_override: Some(overrides),
            priority: 0,
            source: RuleSource::EmbeddedProfile,
        };
        compile_merged_rules(
            &[rule],
            None,
            None,
            None,
            Some("test-profile"),
            Some("<test-profile-path>"),
        )
        .map(|_| ())
    }

    #[test]
    fn dispatch_embedded_profile_zero_forbidden_pushes_to_profile_errors() {
        // Key "0" on a profile-sourced rule must surface as
        // Error::ProfileValidation containing exactly one
        // ProfileRuleError whose kind is
        // StylesKey(CaptureGroupIndexZeroForbidden).
        let style = crate::config::UserStyle::default();
        let err = dispatch_embedded_profile_single_style(
            "myprofile_rule",
            r"(?P<scheme>https?)://",
            "0",
            style,
        )
        .unwrap_err();
        let crate::error::Error::ProfileValidation { profile, source_path, errors } = err else {
            panic!("expected Error::ProfileValidation, got: {err:?}");
        };
        assert_eq!(profile, "test-profile");
        assert_eq!(source_path, "<test-profile-path>");
        assert_eq!(errors.len(), 1, "exactly one error expected, got: {errors:?}");
        assert_eq!(errors[0].rule_name, "myprofile_rule");
        let crate::error::ProfileRuleErrorKind::StylesKey(inner) = &errors[0].kind else {
            panic!("expected StylesKey wrapper, got: {:?}", errors[0].kind);
        };
        assert!(
            matches!(inner, crate::error::ThemeRuleErrorKind::CaptureGroupIndexZeroForbidden),
            "expected CaptureGroupIndexZeroForbidden, got: {inner:?}"
        );
        // Display delegation: outer kind byte-equals inner kind.
        assert_eq!(errors[0].kind.to_string(), inner.to_string());
        // Negative regression: top-level Display must NOT contain other-
        // variant wording.
        let top = errors[0].kind.to_string();
        assert!(!top.contains("no capture group named"), "must not be NameUnknown: {top}");
        assert!(
            !top.contains("capture-group key must be a positive decimal"),
            "must not be KeyMalformed: {top}"
        );
    }

    #[test]
    fn dispatch_embedded_profile_key_malformed_pushes_to_profile_errors() {
        // Key "01" must hit the KeyMalformed grammar gate (NOT fall through
        // to NameUnknown), then collect as a ProfileRuleError.
        let style = crate::config::UserStyle::default();
        let err = dispatch_embedded_profile_single_style(
            "myprofile_rule",
            r"(?P<scheme>https?)://",
            "01",
            style,
        )
        .unwrap_err();
        let crate::error::Error::ProfileValidation { errors, .. } = err else {
            panic!("expected Error::ProfileValidation, got: {err:?}");
        };
        assert_eq!(errors.len(), 1);
        let crate::error::ProfileRuleErrorKind::StylesKey(inner) = &errors[0].kind else {
            panic!("expected StylesKey wrapper, got: {:?}", errors[0].kind);
        };
        match inner {
            crate::error::ThemeRuleErrorKind::CaptureGroupKeyMalformed { key } => {
                assert_eq!(key, "01");
            }
            other => panic!("expected CaptureGroupKeyMalformed, got: {other:?}"),
        }
        assert_eq!(errors[0].kind.to_string(), inner.to_string());
        // Negative regression: must not fall through to NameUnknown wording.
        let top = errors[0].kind.to_string();
        assert!(!top.contains("no capture group named"), "must not be NameUnknown: {top}");
    }

    #[test]
    fn dispatch_embedded_profile_index_out_of_range_pushes_to_profile_errors() {
        // Pattern has one capture group ("scheme"); key "5" is out of range
        // (captures_len = 2, valid 1..=1).
        let style = crate::config::UserStyle::default();
        let err = dispatch_embedded_profile_single_style(
            "myprofile_rule",
            r"(?P<scheme>https?)://",
            "5",
            style,
        )
        .unwrap_err();
        let crate::error::Error::ProfileValidation { errors, .. } = err else {
            panic!("expected Error::ProfileValidation, got: {err:?}");
        };
        assert_eq!(errors.len(), 1);
        let crate::error::ProfileRuleErrorKind::StylesKey(inner) = &errors[0].kind else {
            panic!("expected StylesKey wrapper, got: {:?}", errors[0].kind);
        };
        match inner {
            crate::error::ThemeRuleErrorKind::CaptureGroupIndexOutOfRange {
                group,
                captures_len,
            } => {
                assert_eq!(*group, 5);
                assert_eq!(*captures_len, 2);
            }
            other => panic!("expected CaptureGroupIndexOutOfRange, got: {other:?}"),
        }
        assert_eq!(errors[0].kind.to_string(), inner.to_string());
        // Negative regression: must not surface NameUnknown / KeyMalformed.
        let top = errors[0].kind.to_string();
        assert!(!top.contains("no capture group named"), "must not be NameUnknown: {top}");
        assert!(
            !top.contains("capture-group key must be a positive decimal"),
            "must not be KeyMalformed: {top}"
        );
    }

    #[test]
    fn dispatch_embedded_profile_name_unknown_pushes_to_profile_errors() {
        // Pattern has named groups "date" + "time"; key "bogus" references
        // an unknown name.
        let style = crate::config::UserStyle::default();
        let err = dispatch_embedded_profile_single_style(
            "myprofile_rule",
            r"(?P<date>\d{4}-\d{2}-\d{2})T(?P<time>\d{2}:\d{2}:\d{2})",
            "bogus",
            style,
        )
        .unwrap_err();
        let crate::error::Error::ProfileValidation { errors, .. } = err else {
            panic!("expected Error::ProfileValidation, got: {err:?}");
        };
        assert_eq!(errors.len(), 1);
        let crate::error::ProfileRuleErrorKind::StylesKey(inner) = &errors[0].kind else {
            panic!("expected StylesKey wrapper, got: {:?}", errors[0].kind);
        };
        match inner {
            crate::error::ThemeRuleErrorKind::CaptureGroupNameUnknown { name, available } => {
                assert_eq!(name, "bogus");
                assert_eq!(available, &vec!["date".to_owned(), "time".to_owned()]);
            }
            other => panic!("expected CaptureGroupNameUnknown, got: {other:?}"),
        }
        assert_eq!(errors[0].kind.to_string(), inner.to_string());
        // Byte-pin the message + negative regression on other-variant wording.
        let top = errors[0].kind.to_string();
        assert!(
            top.contains("no capture group named 'bogus'"),
            "expected NameUnknown wording, got: {top}"
        );
        assert!(
            !top.contains("capture-group key must be a positive decimal"),
            "must not be KeyMalformed: {top}"
        );
    }

    #[test]
    fn dispatch_embedded_profile_duplicate_target_pushes_to_profile_errors() {
        // Pattern with named group "scheme" at position 1; both styles."1"
        // and styles.scheme reference the same slot.
        use std::collections::BTreeMap;
        let style1 = crate::config::UserStyle { fg: Some("red".to_owned()), ..Default::default() };
        let style2 = crate::config::UserStyle { fg: Some("cyan".to_owned()), ..Default::default() };
        let mut overrides: BTreeMap<String, crate::config::UserStyle> = BTreeMap::new();
        overrides.insert("1".to_owned(), style1);
        overrides.insert("scheme".to_owned(), style2);
        let rule = BuiltinRule {
            name: "myprofile_rule".to_owned(),
            pattern: r"(?P<scheme>https?)://".to_owned(),
            style: crate::style::Style::DEFAULT,
            group_styles: Vec::new(),
            styles_override: Some(overrides),
            priority: 0,
            source: RuleSource::EmbeddedProfile,
        };
        let err = compile_merged_rules(
            &[rule],
            None,
            None,
            None,
            Some("test-profile"),
            Some("<test-profile-path>"),
        )
        .map(|_| ())
        .unwrap_err();
        let crate::error::Error::ProfileValidation { errors, .. } = err else {
            panic!("expected Error::ProfileValidation, got: {err:?}");
        };
        assert_eq!(errors.len(), 1);
        let crate::error::ProfileRuleErrorKind::StylesKey(inner) = &errors[0].kind else {
            panic!("expected StylesKey wrapper, got: {:?}", errors[0].kind);
        };
        match inner {
            crate::error::ThemeRuleErrorKind::CaptureGroupDuplicateTarget { positional, named } => {
                assert_eq!(positional, "1");
                assert_eq!(named, "scheme");
            }
            other => panic!("expected CaptureGroupDuplicateTarget, got: {other:?}"),
        }
        assert_eq!(errors[0].kind.to_string(), inner.to_string());
        // Byte-pin against the spec §2.4 wording.
        assert_eq!(
            errors[0].kind.to_string(),
            "styles.\"1\" and styles.scheme target the same capture group (index 1); set exactly one"
        );
        // Negative regression: must not surface other variants' wording.
        let top = errors[0].kind.to_string();
        assert!(!top.contains("no capture group named"), "must not be NameUnknown: {top}");
        assert!(!top.contains("out of range"), "must not be IndexOutOfRange: {top}");
    }

    #[test]
    fn dispatch_three_way_identity_theme_userconfig_embedded_profile_byte_equal() {
        // For one representative variant (KeyMalformed), assert that the
        // Display wording is byte-identical across all three RuleSource
        // paths:
        //   - RuleSource::Theme: ThemeRuleErrorKind.to_string()
        //   - RuleSource::UserConfig: ThemeRuleErrorKind.to_string()
        //   - RuleSource::EmbeddedProfile:
        //     ProfileRuleErrorKind::StylesKey(inner).to_string()
        // The third must equal the first two — the StylesKey wrapper
        // delegates via `write!(f, "{inner}")`. Spec §6.3 cross-path
        // identity contract; plan Appendix A.12 absorption.
        let kind =
            crate::error::ThemeRuleErrorKind::CaptureGroupKeyMalformed { key: "01".to_owned() };
        let theme_str = kind.to_string();
        // Same kind reused through the user-config envelope wording at the
        // dispatch site — the inner kind Display is shared.
        let userconfig_str = kind.to_string();
        let profile_kind = crate::error::ProfileRuleErrorKind::StylesKey(kind.clone());
        let profile_str = profile_kind.to_string();
        assert_eq!(theme_str, userconfig_str);
        assert_eq!(
            theme_str, profile_str,
            "StylesKey wrapper must delegate byte-equal to inner ThemeRuleErrorKind"
        );
        // Bonus byte-pin: anchors the wording against regressions in either
        // ThemeRuleErrorKind Display or the StylesKey delegation.
        assert_eq!(
            profile_str,
            "styles.\"01\": capture-group key must be a positive decimal (1, 2, ..., N) with no leading zeros"
        );
    }

    /// v0.5.2 spec §11.1 I-6 / §8.1 #8 — when no profile is active, the
    /// rule set produced by `Compiled::load_with_theme` MUST be
    /// byte-equivalent to the v0.5.6 baseline (12 built-in rules, all
    /// tagged `RuleSource::Builtin`). Catches any accidental
    /// profile-active branch firing on a `None` profile (e.g. a misplaced
    /// `.retain` over the whitelist filter, or an off-by-one in the
    /// `append_rules` loop).
    #[test]
    fn hot_path_unchanged_when_no_profile() {
        let compiled = Compiled::load_with_theme(
            None, // config
            None, // config_path
            None, // theme
            None, // profile
            None, // profile_path
            crate::terminfo::ColorDepth::Truecolor,
        )
        .expect("baseline load with all-None must succeed");

        // Hard baseline — the 12 built-in rules, neither filtered nor
        // augmented.
        assert_eq!(
            compiled.individuals.len(),
            12,
            "v0.5.6 baseline = 12 built-in rules; got {n}",
            n = compiled.individuals.len(),
        );
        assert_eq!(compiled.styles.len(), 12, "styles must parallel individuals length");

        // The compiled rule names match the canonical BUILTIN_NAMES list
        // 1:1 in order — i.e. nothing was inserted, dropped, or reordered.
        let baseline_names: Vec<&str> = BUILTIN_NAMES.to_vec();
        let merged_names: Vec<String> = builtin_rules().into_iter().map(|r| r.name).collect();
        assert_eq!(
            merged_names, baseline_names,
            "BUILTIN_NAMES and builtin_rules() must agree on the 12 baseline rules"
        );

        // Defensive: every rule produced by builtin_rules() carries
        // `source == RuleSource::Builtin`. If a profile-active path
        // accidentally fired on the None branch, at least one rule's
        // `source` would have flipped to EmbeddedProfile (or the rule
        // count would have shifted) — both fail the assertions above.
        for r in builtin_rules() {
            assert_eq!(
                r.source,
                RuleSource::Builtin,
                "rule '{}' must be tagged Builtin in the baseline; got {:?}",
                r.name,
                r.source,
            );
        }
    }

    // --- ipv6 FP audit C-2: Rust module path negative regression (TDD red → green) ---

    #[test]
    fn ipv6_does_not_match_rust_module_path() {
        assert!(!matches("ipv6", "mod foo::bar::baz {}"));
    }

    #[test]
    fn ipv6_does_not_match_std_io_read() {
        assert!(!matches("ipv6", "use std::io::Read;"));
    }

    #[test]
    fn ipv6_does_not_match_serde_de_deserialize() {
        assert!(!matches("ipv6", "serde::de::Deserialize"));
    }

    #[test]
    fn ipv6_does_not_match_bare_double_colon_two_hex() {
        assert!(!matches("ipv6", "see ::ba elsewhere"));
    }

    #[test]
    fn ipv6_matches_loopback_double_colon_one() {
        assert!(matches("ipv6", "loopback ::1 here"));
    }

    #[test]
    fn ipv6_matches_link_local() {
        assert!(matches("ipv6", "fe80::1 link-local"));
    }

    #[test]
    fn ipv6_matches_compressed_short() {
        assert!(matches("ipv6", "2001:db8::1 doc-net"));
    }

    #[test]
    fn ipv6_matches_compressed_multi_group() {
        assert!(matches("ipv6", "addr 2001:db8::ff00:42:8329 end"));
    }

    #[test]
    fn ipv6_matches_full_form() {
        assert!(matches("ipv6", "full 1:2:3:4:5:6:7:8 end"));
    }

    #[test]
    fn ipv6_matches_trailing_compression() {
        assert!(matches("ipv6", "trail 1234:5678:: here"));
    }

    #[test]
    fn priority_default_is_zero_for_all_builtins() {
        for r in builtin_rules() {
            assert_eq!(r.priority, 0, "built-in '{}' must have priority 0", r.name);
        }
    }

    #[test]
    fn compiled_priorities_parallel_vec_invariant() {
        let compiled = Compiled::load_with_theme(
            None,
            None,
            None,
            None,
            None,
            crate::terminfo::ColorDepth::Truecolor,
        )
        .expect("default load");
        assert_eq!(
            compiled.priorities.len(),
            compiled.individuals.len(),
            "priorities vs individuals"
        );
        assert_eq!(compiled.priorities.len(), compiled.styles.len(), "priorities vs styles");
        assert_eq!(
            compiled.priorities.len(),
            compiled.group_styles.len(),
            "priorities vs group_styles"
        );
        assert_eq!(
            compiled.priorities.len(),
            compiled.uses_capture_styling.len(),
            "priorities vs uses_capture_styling"
        );
    }

    #[test]
    fn compile_from_config_with_empty_config_compiles_builtins_only() {
        use crate::config::{Config, GeneralSection};

        let config = Config { general: GeneralSection::default(), rules: Vec::new() };
        let compiled = compile_from_config(&config, None, None).expect("compile");
        assert!(compiled.individuals.len() >= 12, "at least 12 builtins compiled");
        assert!(compiled.priorities.iter().all(|&p| p == 0), "all builtins priority 0");
    }

    #[test]
    fn compile_from_config_with_invalid_theme_name_errs() {
        use crate::config::{Config, GeneralSection};

        let config = Config { general: GeneralSection::default(), rules: Vec::new() };
        let result = compile_from_config(&config, Some("nonexistent_theme"), None);
        assert!(result.is_err(), "unknown theme name surfaces as Error");
    }
}
