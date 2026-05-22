# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
