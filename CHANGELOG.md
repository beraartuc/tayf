# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.2] — 2026-05-23

### Added

- `url` built-in now matches `git@host:path` SSH-form Git URLs alongside the existing `https://`, `ssh://`, and `ftp://` schemes. Host class is label-aware (start and end alphanumeric; `.` and `-` allowed in the middle). Path segment shares the URL trim semantics below. Resolves the v0.2.2 deferral.
- `duration` built-in now matches bare-suffix durations (`5s`, `30m`, `2h`, `7d`) AND compound forms (`2d3h`, `1h30m20s`) as single spans. Compound coverage picks up `kubectl get pods` AGE columns and `docker ps` STATUS columns directly. Bare-unit forms require no whitespace between digit and unit (`5m` matches, `5 m` does not — false-positive guard for prose).
- `TAYF_DISABLE_BG_DETECT` environment variable. When set to `1`, `true`, or `yes` (case-insensitive), `bg_detect::resolve()` short-circuits to `BgTheme::Dark` before any `/dev/tty` I/O. Documented as **test-only** — production users should rely on automatic detection or pin a theme via `--theme` / `[general] theme`. Exists for CI environments where `/dev/tty` is a `portable-pty` slave that cannot respond to OSC 11.

### Changed

- `url` built-in no longer includes trailing sentence punctuation (`.`, `,`, `;`, `:`, `!`, `?`) at the end of a match. Closing brackets (`)`, `]`) **stay** in matches to preserve Wikipedia/MDN URLs ending in `)` (e.g. `Foo_(disambig)`) and IPv6 literal hosts (e.g. `https://[::1]`). Trade-off: a URL wrapped in parens (`(https://example.com)`) keeps the trailing `)` in the match — most click-to-open terminals tolerate this; users can override via `[[rules]]`.
- `respect_existing_colors=true` (the v0.3.0 default) remains the recommended configuration. Users who set `respect_existing_colors=false` opt into v0.1-class SGR collision with bare-unit duration matches (e.g. `49m` inside `\x1b[49m`). The default is structurally safe; the opt-out trade-off is explicit. Pinned by the new `bare_units_collide_with_sgr_when_respect_existing_colors_is_false` unit test.

### Fixed

- `watch::drop_stops_debounce_thread` no longer flakes on macOS CI under load. The test now drains in-flight events from the initial write before asserting, then polls for `mpsc::TryRecvError::Disconnected` with a 5-second overall budget. Tolerates the 100–500 ms FSEvents runloop shutdown that previously blew through the prior 500 ms `recv_timeout` budget.
- OSC 11 background-detection hang on macOS `portable-pty` subprocesses not isolated to a single root cause within the v0.3.2 investigation budget. The diagnostic example (`examples/repro_osc11_hang`, mirrors `detect_from_osc11`'s 10 phases) when spawned under `portable-pty` on macOS with `COLORFGBG` scrubbed never flushes any stderr to the master within 15 seconds — total silence prevents phase-level localisation, which ruled out responsible Senaryo 1 (per-phase fix) and Senaryo 2 (kqueue) paths. Added `TAYF_DISABLE_BG_DETECT` env-var bypass (see Added) and replaced the v0.3.1 CI `COLORFGBG=15;15` workaround with `TAYF_DISABLE_BG_DETECT=1`. Production behavior unchanged. Further investigation deferred.

### Notes

- No new dependencies. No public API changes; no config schema changes; no CLI flag changes. v0.3.1 (and v0.2.x) config files remain shimless backward-compatible.
- New repo artifact: `examples/repro_osc11_hang.rs` — standalone diagnostic tool for the `bg_detect` OSC 11 path, mirroring the production function's 10 phases with per-phase wall-clock timing on stderr. Reusable for future regression triage.
- `tests/integration_bg_detect.rs` pins the `TAYF_DISABLE_BG_DETECT` bypass against future regression (scrubs `COLORFGBG` to prove the bypass — not the env-var fast path — is what completes startup within budget).

[0.3.2]: https://github.com/beraartuc/tayf/releases/tag/v0.3.2

## [0.3.1] — 2026-05-23

### Added

- Automatic terminal background detection at startup (best-effort). When you haven't pinned a theme via `--theme` or `[general] theme`, tayf tries `COLORFGBG` first, then an OSC 11 query against `/dev/tty` with a 100 ms timeout, then falls back to `dark`. The matching preset (`light` or `dark`) is applied automatically. tmux is supported; tmux ≥3.3 requires `set -g allow-passthrough on`. GNU screen is not supported. Detection is also skipped when stdout is not a TTY, when `--no-color` is set, or when `TERM=dumb`. **Detection is a starting point, not authoritative** — modern terminals have dozens of light/dark variants and binary detection can't match all of them. README "Themes" section documents limitations and the custom-palette path.

### Fixed

- **I4 (deferred from v0.3.0):** when the ANSI state machine's 4 KiB sequence cap fires while in a string state (OSC / DCS / PM / APC), `Pipeline` now emits a synthetic 7-bit ST (`\e\\`) to stdout to close the unterminated string sequence on the terminal side. Prevents the terminal from absorbing subsequent shell output as part of the never-terminated sequence when fed an adversarial unterminated string payload (>4 KiB OSC, etc.).

### Notes

- No public API changes; no new dependencies. v0.3.0 config files are shimless backward-compatible.
- The new `src/bg_detect.rs` module manages its own short-lived termios snapshot and process-wide panic hook (parallel to `tty_guard`), necessary because release builds use `panic = "abort"` and `Drop` does not run on panic.
- I4 cap-fire-in-string-state coverage is pinned by the `pipeline_writes_st_on_cap_fire_in_string_state` unit test in `src/pipeline.rs`, which drives the same 5 KiB unterminated-OSC input through `Pipeline.feed` deterministically. The binary-level integration test originally planned for `tests/integration_ansi.rs` was deferred until a non-blocking PTY-read primitive lands — `portable-pty`'s blocking reader doesn't honor a deadline between shell exit and EOF propagation on macOS CI runners, so the test could hang past the runner budget.

[0.3.1]: https://github.com/beraartuc/tayf/releases/tag/v0.3.1

## [0.3.0] — 2026-05-23

### Changed

- **`respect_existing_colors` is honored by default.** The config field defaults to `true` and was parsed but ignored in v0.2.0–v0.2.4. Starting with v0.3.0 it is wired into the hot path: any line that already contains an ANSI SGR sequence (`\e[…m`) bypasses tayf's rules and is written to stdout byte-for-byte. Users whose input was already colored (e.g. piped `git log --color=always`, `journalctl` with `SYSTEMD_COLORS=true`) will see tayf stop overlaying its own rules on those lines. Migration: set `[general] respect_existing_colors = false` to restore the v0.2 effective behavior of running rules on every line.

### Added

- OSC, DCS, PM, and APC sequence handling. `\e]…`, `\eP…`, `\e^…`, `\e_…` are now classified as terminal-control payloads and pass through to stdout verbatim. Lines containing such sequences are written byte-for-byte and skip rule application, so OSC 8 hyperlinks (`\e]8;;URL\aLABEL\e]8;;\a`) render correctly without the URL being matched by tayf's `url` rule.
- Non-CSI ESC sequences (`\e=`, `\eM`, `\e7`, `\e8`, `\ec` RIS) and multi-byte ESC sequences (`\e(B` G0 designate, `\e#8` DEC alignment test) are now parsed as control sequences rather than leaking their payload bytes into the rule engine.
- Trigger sequence bytes (`\e[?1049h` alt-screen entry, `\e[31m` SGR, etc.) used to land in the line buffer alongside surrounding text. They are now collected in a per-pipeline scratch buffer and routed by sequence type: stdout for TUI toggles, line_buffer for SGR/other CSI/ESC completions.

### Internal

- New module `src/ansi.rs` (47 unit tests) implementing a 16-state subset of the Paul Williams VT500 ANSI parser (https://vt100.net/emu/dec_ansi_parser). Replaces the manual `TuiModeSm` that lived in `src/pipeline.rs` since v0.1.
- `Pipeline::feed` rewritten with a three-path architecture (TUI passthrough / sequence accumulation in scratch / OSC-payload direct-to-stdout). Existing callers unchanged.
- New 4 KiB internal cap on unterminated CSI/ESC byte accumulation. Defense against malicious input keeping the parser in a non-Ground state forever.
- `Compiled` struct gains a `respect_existing_colors: bool` field. Hot-reload-aware via the existing `ArcSwap<Compiled>` (snapshotted at every line boundary).

### Notes

- No new dependency. No public CLI / config schema change.
- Cross-line SGR state is not tracked: a multi-line color block (e.g. `git log --color=always` with prompts that span lines) is honored on each SGR-bearing line, but rules may still run on intermediate lines that have no SGR themselves. Segment-level semantics are planned for v0.4.
- Known limitation: when the 4 KiB unterminated-sequence cap fires while the state machine is mid-OSC / DCS / PM / APC, the partial payload bytes already on stdout are unterminated. Modern terminals will keep absorbing subsequent bytes as payload until they see their own terminator (or hit their own cap). Mitigation lands in v0.3.1 — the state machine will emit a synthetic ST (`\e\\`) on cap-fire-in-string-state so the terminal closes the sequence promptly.

[0.3.0]: https://github.com/beraartuc/tayf/releases/tag/v0.3.0

## [0.2.4] — 2026-05-23

### Changed

- README "Themes" section moved under `## Configuration (v0.2)` as a subsection. Themes are a configuration source, not a sibling concept; readers find them while looking for how to configure colors.
- `Compiled::load` proxy removed. The v0.2.3 series kept a thin `load(config, path, depth)` wrapper around `load_with_theme(config, path, None, depth)` to spare existing callsites. With the layering settled, the proxy is dead surface — production callers use `load_with_theme` directly and the remaining test callers have been inlined. `Compiled::load_builtins` is the only convenience shim left.
- CI workflow upgraded `actions/checkout` from v4 to v5 ahead of the Node.js 20 deprecation enforced June 2026.

### Documentation

- `src/rules.rs::compile_error_for` doc-comment now spells out that theme TOML rules ride the same error-routing path as user `[[rules]]` after `themes::validate_theme_rules` runs.

### Notes

- No new dependencies. No public API change. Patch release containing only the polish items flagged in the senior Rust review of v0.2.3.

[0.2.4]: https://github.com/beraartuc/tayf/releases/tag/v0.2.4

## [0.2.3] — 2026-05-23

### Added

- Preset color themes embedded in the binary. `dark` ships as the explicit defaults — identical to running tayf without a theme, useful as a copy-and-edit template. `light` ships as a re-tuned palette for light-background terminals.
- `--theme <NAME>` CLI flag. CLI value overrides the config-file value.
- `[general] theme = "..."` in `~/.config/tayf/config.toml`. Defaults to `None` (no theme).
- New `Error::Theme { name, available }` variant; unknown theme names exit with code 64 (`EX_USAGE`) and list the available themes on stderr.

### Fixed

- Light-background terminal portability. Built-in styles `permission` (`White + dim`), `timestamp` (`BrightBlack`), `ipv6` (`BrightYellow`), and `ipv4` (`Yellow + bold`) were invisible or low-contrast on light backgrounds. The new `light` theme provides a readable palette without requiring a user config.

### Notes

- No new dependencies. No public API change beyond the additive `--theme` CLI flag and the additive `[general] theme` config field.
- Theme styles apply on top of built-in defaults; your `[[rules]]` in `~/.config/tayf/config.toml` still win over the theme. Same precedence as before, with the theme as a new middle layer.
- Theme files are baked into the binary at build time. Users wanting a custom theme should still use the user-config layer.
- Automatic background detection (`COLORFGBG` / OSC 11) is deferred to v0.3.

[0.2.3]: https://github.com/beraartuc/tayf/releases/tag/v0.2.3

## [0.2.2] — 2026-05-22

### Added

- Five new built-in patterns, growing the default rule set from 8 to 13:
  - `permission` — POSIX `ls -l` file mode strings (`-rwxr-xr-x`, `drwxr-xr-x`, ACL `+` suffix). Styled `White` + `dim`.
  - `timestamp` — multi-format (ISO-8601, syslog, Apache/nginx, RFC 2822 incl. obsolete US zones EST/EDT/CST/CDT/MST/MDT/PST/PDT). Styled `BrightBlack`.
  - `uuid` — canonical 8-4-4-4-12 hex form, case-insensitive. Styled `BrightMagenta`.
  - `url` — `https?://`, `ssh://`, and `ftp://` URLs. Styled `BrightBlue` + `underline`. `git@host:path` SSH URL alt-form deferred to v0.3.
  - `email` — RFC 5322 simplified shape. Styled `BrightGreen`.
- All five new patterns are subject to the existing user-config override / disable / restyling mechanism shipped in v0.2.0 — no config schema changes required.

[0.2.2]: https://github.com/beraartuc/tayf/releases/tag/v0.2.2

## [0.2.1] — 2026-05-22

### Added

- **Config hot reload.** Editing `~/.config/tayf/config.toml` (or the path passed to `--config`) takes effect in the running tayf within ~250 ms — no restart, no shell respawn. The file watcher uses `notify` 8 (inotify on Linux, FSEvents on macOS) with a 200 ms manual debounce window.
- **`SIGHUP` triggers a reload.** `pkill -HUP tayf` or `kill -HUP <pid>` forces an immediate reload that bypasses the watcher debounce — useful from scripts. SIGHUP also re-resolves the config path, so if you didn't have a config at startup and create one later, SIGHUP picks it up.
- **Fail-safe semantics.** If your edit produces invalid TOML or a bad regex, tayf keeps the **previous** rule set in effect and logs a warning to stderr (`TAYF_LOG=warn`, the default). The terminal session is never disrupted by a typo in the config.
- Integration test for `SIGWINCH` delivery (filling a v0.2.0 coverage gap surfaced during the signal-hook 0.4 review). Three integration tests for hot reload covering file edit, parse failure, and SIGHUP.

### Changed

- `signal-hook` upgraded from 0.3 to 0.4. No behavioral change in tayf's signal path; 0.4.2 includes a bug-fix in the `Handle::close` codepath that `SignalGuard::drop` exercises.
- `Pipeline.rules` now lives behind `Arc<ArcSwap<Compiled>>`. `apply_rules` snapshots the handle once per line via `ArcSwap::load_full`, so reloads landing mid-line take effect on the next line — never split the current one.
- New direct dependencies: `arc-swap 1.9` (wait-free atomic Arc swap) and `notify 8.2` (cross-platform filesystem watcher, `default-features = false`).
- `signals::spawn_handler` now takes a third argument `Option<Sender<ReloadRequest>>`; non-`None` enables SIGHUP forwarding to the reload orchestrator.
- `deny.toml` allow list widened to permit `CC0-1.0` (notify itself) and `ISC` (the inotify family on Linux). Both are OSI-recognized permissive licenses; neither imposes copyleft contagion.

### Internal

- New modules `src/reload.rs` (orchestrator + `reload_once` function) and `src/watch.rs` (notify wrapper + manual debounce loop).
- New threads at runtime: `tayf-debounce` (notify event coalescing) and `tayf-reload` (parse + compile + atomic swap). Total in v0.2.1: 6 threads (main, `tayf-output`, `tayf-input`, `tayf-signals`, `tayf-debounce`, `tayf-reload`).
- `Tayf::run` shutdown sequence carefully ordered: `_orchestrator` is declared last among the threading scaffolding so any `?` failure on earlier setup never drops it with live `reload_tx` clones (which would deadlock the join). After `runtime::run` returns, watcher and signal guard are explicitly dropped *before* the orchestrator's implicit Drop, so the reload thread sees the channel close cleanly.
- `info_msg!` macro added to `src/log.rs` mirroring the existing `warn_msg!` shape. Successful reloads emit at `TAYF_LOG=info`; default behavior is silent.

[0.2.1]: https://github.com/beraartuc/tayf/releases/tag/v0.2.1

## [0.2.0] — 2026-05-22

### Added

- TOML config (`~/.config/tayf/config.toml`, fallback to `$XDG_CONFIG_HOME/tayf/config.toml`, override via `--config <PATH>`). Override built-in rule styles by name, disable built-ins via `enabled = false`, or append custom regex rules. Without a config file, behavior is byte-identical to v0.1.
- Color string parser for TOML values: ANSI names (`"red"`, `"bright_cyan"`), 256-indexed (`"color(178)"`), 24-bit hex (`"#ff8800"`), and functional rgb (`"rgb(255, 136, 0)"`).
- Color depth downgrade: `Style::downgrade(depth)` collapses Rgb / Indexed values into whatever the terminal supports (detected from `$COLORTERM` and `$TERM`). Pre-baked into the compiled rule set at startup so the hot path is unchanged.
- New `Error::Config { path, line, message }` variant routed to exit code 64 (`EX_USAGE`) with friendly stderr diagnostics.
- `serde = "1"` and `toml = "0.9"` direct deps; `tempfile = "3.27"` dev-dep (used by integration tests).

### Changed

- `rules::Compiled::load` now takes `(config: Option<&Config>, config_path: Option<&str>, depth: ColorDepth)`. The `config_path` is threaded into user-rule validation/regex errors so diagnostics carry the real file path. The previous `Compiled::load_builtins()` is preserved as a thin wrapper so the `__bench__` shim and existing call sites compile unchanged.
- `config::load` now returns `Option<(Config, PathBuf)>` so callers know which file was loaded without re-resolving the XDG/home cascade.
- `BuiltinRule::name` migrated from `&'static str` to `String` to hold user-supplied rule names. Cost: eight heap allocations at startup.
- Every regex (built-in and user) now compiles with both `RegexBuilder::size_limit(1 MiB)` (NFA program cap) and `dfa_size_limit(1 MiB)` (DFA lazy-cache cap) to bound the memory a single user regex can consume.
- **Breaking:** `Error` enum gained the `Config { path, line, message }` variant and is now marked `#[non_exhaustive]`. External callers using non-exhaustive matches need a `_ => ...` arm. The `#[non_exhaustive]` attribute prevents future variant additions from being silent breaks.

### Security

- Default-path resolution canonicalizes both the candidate config file and the configured base directory; the file is rejected if it resolves outside the base. Protects against `~/.config/tayf/config.toml` being a symlink to `/etc/shadow` or a hostile shared mount. `--config <PATH>` is the documented opt-out for project-local configs.
- 1 MiB cap on the size of the loaded config file.
- `--config <PATH>` verifies the target is a regular file before reading.

[0.2.0]: https://github.com/beraartuc/tayf/releases/tag/v0.2.0

## [0.1.3] — 2026-05-22

### Changed

- Bumped `thiserror` 1.0 → 2.0. Mechanical migration; the `tayf::Error` enum's derive surface is source-compatible. Done ahead of v0.2.0 so upcoming TOML config error variants can be written in modern syntax from the start.
- Bumped `criterion` 0.5 → 0.8 (dev-dep). `criterion::black_box` is now deprecated in favour of `std::hint::black_box`; updated the import in `benches/throughput.rs`. Bench semantics unchanged.

[0.1.3]: https://github.com/beraartuc/tayf/releases/tag/v0.1.3

## [0.1.2] — 2026-05-22

### Changed

- Bumped `portable-pty` 0.8 → 0.9; transitively drops unmaintained `serial 0.4` (RUSTSEC-2017-0008) and `bitflags 1.x` (on Unix).
- Bumped `nix` 0.27 → 0.28 to match portable-pty 0.9's internal nix; collapses the duplicate nix version in the tree.
- Refreshed requirement lines: `clap` 4.5 → 4.6, `regex` 1.10 → 1.12, `tempfile` 3.10 → 3.27 (resolved versions unchanged; hygiene only).
- `nix::unistd::pipe()` returns `(OwnedFd, OwnedFd)` directly in 0.28, so one `unsafe { OwnedFd::from_raw_fd(...) }` block in `runtime.rs` is gone (3 unsafe sites in `src/` now, down from 4).

### Removed

- `built` (build-dep) — replaced with a 60-line `build.rs` calling `git rev-parse` and `rustc --version` via stdlib `Command`. Drops ~25 transitive crates: git2, libgit2-sys, libz-sys, url, idna, and the entire ICU stack (icu_collections, icu_normalizer, icu_properties, icu_provider, yoke, zerovec, tinystr, displaydoc, etc.).
- `tracing` + `tracing-subscriber` — replaced with a 30-line `src/log.rs` (env-gated `eprintln!` wrapper via `AtomicU8` level + `Once`-guarded init from `TAYF_LOG`). Drops 11 transitive crates: tracing-core, tracing-attributes, tracing-log, matchers, sharded-slab, thread_local, nu-ansi-term, smallvec, valuable, plus a second `regex-automata` copy via `env-filter`. The single `warn_msg!` call site preserved at `pipeline.rs` line-buffer overflow.

### Fixed

- `RUSTSEC-2017-0008` (unmaintained `serial 0.4` via portable-pty) is gone; `deny.toml` ignore entry removed.

[0.1.2]: https://github.com/beraartuc/tayf/releases/tag/v0.1.2

## [0.1.1] — 2026-05-22

### Added

- GitHub Actions CI matrix (Linux + macOS) with fmt, clippy, build, test.
- `cargo audit` and `cargo deny` in CI for dependency hygiene.
- `criterion` throughput benchmarks (`benches/throughput.rs`).
- Pipeline tick-flush: interactive prompts now colorize within 50 ms of idle.
- Input thread self-pipe wakeup: clean shutdown without OS-reap.

### Changed

- CLI parse errors exit with 64 (`EX_USAGE`) per BSD sysexits.
- `TIOCGWINSZ` ioctl consolidated into a single `terminfo::winsize` helper.
- Public API trimmed to spec §3.2: `Args`, `Error`, `Result`, `Tayf::run`.
- `cli` and `error` modules hidden behind crate-root re-exports.

[0.1.1]: https://github.com/beraartuc/tayf/releases/tag/v0.1.1

## [0.1.0] — 2026-05-21

### Added

- PTY-based shell wrapper that spawns the user's shell inside a pseudo-terminal
  and pipes output through a regex-driven rule engine.
- Eight built-in patterns: IPv4, IPv6, MAC, log level, HTTP status, FQDN,
  duration metric, and filename extension (curated catalog covering archives,
  source code, configuration, documents, media, and binaries).
- Shell discovery cascade: `$SHELL` → `/etc/passwd` (`getpwuid`) → `/bin/sh`,
  with `--shell <path>` and `--login` overrides.
- CLI flags: `--shell`, `--login`, `--no-color`, `--help`, `--version`.
- Build-time SHA and rustc version surfaced via `--version`.
- `TAYF_LOG` environment variable activates `tracing-subscriber` diagnostics on
  stderr (off by default).
- 5-state DEC private mode parser detects TUI modes (alt-screen
  `\x1b[?1049h` and legacy variants `47`/`1047`, bracketed paste `2004`,
  mouse tracking `1000`/`1002`/`1003`/`1006`) and switches the pipeline to
  passthrough so vim, less, htop, neovim, Claude Code, lazygit, k9s, gum,
  bubbletea-based tools, and similar TUIs render unaltered.
- 64 KB hard-capped, UTF-8-safe line buffer.
- Raw-mode termios RAII guard with panic-hook fallback so the terminal is
  restored on any exit path.
- Signal handling thread (signal_hook): `SIGWINCH` resizes the PTY, `SIGINT`
  and `SIGTERM` are forwarded to the child process group with `killpg`.
- Exit-code propagation from the child shell; OS-error and software-error
  codes per BSD sysexits when tayf itself fails.
- Automatic colorization bypass when stdout is not a TTY.

### Security

- All built-in patterns are linear-time (verified by inspection); no
  catastrophic backtracking.
- Line buffer is hard-capped to bound memory under adversarial output.
- ANSI emission is restricted to SGR sequences by a single audited function
  (`Style::to_sgr`) with a unit test asserting the output grammar.
- Shell is spawned via `argv`-style `CommandBuilder`, never via `sh -c`.

[0.1.0]: https://github.com/beraartuc/tayf/releases/tag/v0.1.0
