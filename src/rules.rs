//! Compiled rule set and built-in patterns.
//!
//! `Compiled` holds a parallel collection of regexes and styles. v0.1 walks
//! `individuals` for every line; the `set` field is populated for the v0.4
//! `RegexSet` fast-path so that switching does not break the public shape.
//! See spec §3.6 and §3.8.

use regex::bytes::{Regex, RegexSet};

use crate::error::{Error, Result};
use crate::style::{Color, Style};

/// One built-in rule: a name (for diagnostics), a regex pattern source, and
/// the style applied to each match. The pattern is owned `String` (not
/// `&'static str`) because the filename rule is built dynamically; the cost
/// of eight heap allocations at startup is negligible.
pub(crate) struct BuiltinRule {
    // reason: read by tests (`find_rule` selects by name) and reserved for
    // diagnostic logging once user-defined rules land; the live compile
    // path indexes by position so the field is otherwise unread.
    #[allow(dead_code)]
    pub(crate) name: &'static str,
    pub(crate) pattern: String,
    pub(crate) style: Style,
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

/// Construct the eight built-in rules. Returns a fresh `Vec` because the
/// filename rule contains a dynamically built pattern string. See spec §3.6
/// and §3.8.
pub(crate) fn builtin_rules() -> Vec<BuiltinRule> {
    vec![
        BuiltinRule {
            name: "ipv4",
            pattern: r"\b(?:25[0-5]|2[0-4]\d|1\d{2}|[1-9]?\d)(?:\.(?:25[0-5]|2[0-4]\d|1\d{2}|[1-9]?\d)){3}\b".into(),
            style: Style { fg: Some(Color::Yellow), bold: true, ..Style::DEFAULT },
        },
        BuiltinRule {
            name: "ipv6",
            pattern: r"(?:[0-9A-Fa-f]{1,4}:){7}[0-9A-Fa-f]{1,4}|(?:[0-9A-Fa-f]{1,4}:){1,6}:[0-9A-Fa-f]{0,4}|::[0-9A-Fa-f]{1,4}|::1".into(),
            style: Style { fg: Some(Color::BrightYellow), ..Style::DEFAULT },
        },
        BuiltinRule {
            name: "mac",
            pattern: r"\b[0-9A-Fa-f]{2}(?:[:-][0-9A-Fa-f]{2}){5}\b".into(),
            style: Style { fg: Some(Color::Cyan), ..Style::DEFAULT },
        },
        BuiltinRule {
            name: "log_level",
            pattern: r"\b(?:ERROR|FAIL|FATAL|CRITICAL|WARN|WARNING|INFO|DEBUG|TRACE)\b".into(),
            style: Style { fg: Some(Color::BrightRed), bold: true, ..Style::DEFAULT },
        },
        BuiltinRule {
            name: "http_status",
            pattern: r"(?:^|[\s/:])([1-5]\d{2})\b".into(),
            style: Style { fg: Some(Color::Magenta), ..Style::DEFAULT },
        },
        BuiltinRule {
            name: "filename",
            pattern: build_filename_pattern(),
            style: Style { fg: Some(Color::BrightCyan), ..Style::DEFAULT },
        },
        BuiltinRule {
            name: "fqdn",
            pattern: r"\b(?:[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?\.){1,}[A-Za-z]{2,24}\b".into(),
            style: Style { fg: Some(Color::Blue), ..Style::DEFAULT },
        },
        BuiltinRule {
            name: "duration",
            // reason: dropping bare `s`, `m`, `h` units that collide with SGR final
            // bytes (\x1b[49m, etc.) and produce false-positive duration matches inside
            // escape sequences. Multi-character units cover the modern use cases
            // (nanosec, microsec, millisec). Tracked in spec §6.2 — full ANSI awareness
            // arrives in v0.3 to allow the bare units back safely.
            pattern: r"\b\d+(?:\.\d+)?\s?(?:ns|us|μs|ms)\b".into(),
            style: Style { fg: Some(Color::Green), ..Style::DEFAULT },
        },
    ]
}

/// Compiled rule set ready for application against output lines.
///
/// `individuals` and `styles` are parallel — index `i` of `individuals` carries
/// the regex; index `i` of `styles` carries the style to apply. `set` is the
/// equivalent `RegexSet` populated for v0.4's planned fast-path; v0.1 ignores
/// it but the storage shape stays stable.
pub(crate) struct Compiled {
    #[allow(dead_code)]
    // reason: reserved for v0.4 RegexSet fast-path; populated now to keep the shape stable
    pub(crate) set: RegexSet,
    pub(crate) individuals: Vec<Regex>,
    pub(crate) styles: Vec<Style>,
}

impl Compiled {
    /// Compile the eight built-in rules.
    ///
    /// # Errors
    /// Returns `Error::RegexCompile` if any built-in pattern fails to compile.
    /// In practice this never happens — the patterns are tested.
    pub(crate) fn load_builtins() -> Result<Self> {
        let rules = builtin_rules();
        let mut individuals = Vec::with_capacity(rules.len());
        let mut styles = Vec::with_capacity(rules.len());
        let mut sources = Vec::with_capacity(rules.len());

        for rule in &rules {
            individuals.push(Regex::new(&rule.pattern).map_err(Error::from)?);
            styles.push(rule.style);
            sources.push(rule.pattern.clone());
        }

        let set = RegexSet::new(&sources).map_err(Error::from)?;

        Ok(Compiled { set, individuals, styles })
    }
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

    #[test]
    fn duration_matches() {
        assert!(matches("duration", "took 20.291 ms"));
        assert!(matches("duration", "elapsed 1500 ms"));
        assert!(matches("duration", "150ms"));
        assert!(matches("duration", "2.5 us"));
        assert!(matches("duration", "100 μs"));
        assert!(matches("duration", "50 ns"));
        // v0.1 drops bare `s` / `m` / `h` units because they collide with SGR
        // final bytes; these intentionally do NOT match anymore:
        assert!(!matches("duration", "took 1m"));
        assert!(!matches("duration", "took 1h"));
        assert!(!matches("duration", "took 1s"));
        assert!(!matches("duration", "milliseconds"));
    }

    #[test]
    fn duration_does_not_match_sgr_parameters() {
        // Regression test for v0.1 SGR-collision bug.
        // Bytes like "\x1b[0m", "\x1b[49m" must NOT contain a duration match
        // when scanned as raw bytes — otherwise apply_rules will inject an
        // escape mid-sequence and break Powerlevel10k-style prompts.
        use crate::pipeline::apply_rules;
        let compiled = Compiled::load_builtins().unwrap();
        let inputs: &[&[u8]] =
            &[b"\x1b[0m", b"\x1b[49m", b"\x1b[1;39m", b"prefix \x1b[44m text \x1b[0m suffix"];
        for input in inputs {
            let mut out = Vec::new();
            apply_rules(input, &compiled, &mut out).unwrap();
            // Output must equal input — no SGR injection inside escape sequences.
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
        assert_eq!(n, 8, "v0.1 ships exactly eight built-in rules");
    }

    #[test]
    fn filename_wins_over_fqdn_for_known_extensions() {
        use crate::pipeline::apply_rules;
        let compiled = Compiled::load_builtins().unwrap();
        let mut out = Vec::new();
        apply_rules(b"edit claude.md please\n", &compiled, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        // BrightCyan fg = SGR 96; Blue fg = SGR 34. Verify the filename style wins.
        assert!(s.contains("96"), "expected filename SGR 96 (bright cyan), got: {s:?}");
        assert!(!s.contains("\x1b[34m"), "should not contain blue SGR 34: {s:?}");
    }

    #[test]
    fn filename_wins_for_rust_source() {
        use crate::pipeline::apply_rules;
        let compiled = Compiled::load_builtins().unwrap();
        let mut out = Vec::new();
        apply_rules(b"vim src/main.rs and tests.rs\n", &compiled, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("96"), "expected bright cyan: {s:?}");
    }

    #[test]
    fn fqdn_still_matches_when_no_filename_competes() {
        use crate::pipeline::apply_rules;
        let compiled = Compiled::load_builtins().unwrap();
        let mut out = Vec::new();
        apply_rules(b"visit api.example.org today\n", &compiled, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        // Blue SGR 34 should appear (no extension to conflict).
        assert!(s.contains("34"), "expected fqdn SGR 34 (blue): {s:?}");
    }
}
