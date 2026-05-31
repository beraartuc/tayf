# Architecture

This document is a tour of tayf's internals for contributors. For usage, see the
[README](./README.md); for the security posture, see [SECURITY.md](./SECURITY.md).

## What tayf does

tayf wraps your shell inside a pseudo-terminal (PTY) and applies a small set of
linear-time regular expressions to the byte stream flowing back from the child,
so common patterns (IP addresses, log levels, timestamps, durations, file
permissions, URLs, …) appear colorized — in any terminal emulator, with any
shell, without aliases or per-command wrapping.

The design priorities, in order: **never corrupt the user's terminal**, **be
safe against adversarial output**, **stay out of the way of full-screen / already
-colored programs**, and **add minimal overhead** to the I/O path.

## Process and thread model

`Tayf::run` ([`src/lib.rs`](./src/lib.rs)) is the single entry point. It wires,
in order: logging → bypass resolution → shell discovery → TTY raw-mode guard →
PTY spawn → signal handler → the I/O loop. There is also a "bypass" branch
(`--bypass` / `TAYF_DISABLE`) that wraps the PTY and forwards signals but applies
no colorization.

At steady state tayf runs a handful of threads:

- **I/O loop** ([`runtime.rs`](./src/runtime.rs)) — a `poll(2)`-driven loop
  moving bytes between the user's stdin/stdout and the PTY master.
- **Signal thread** ([`signals.rs`](./src/signals.rs)) — forwards
  `SIGINT`/`SIGTERM`/`SIGHUP`/`SIGWINCH` to the child **process group**.
- **Hot-reload watcher + orchestrator** ([`watch.rs`](./src/watch.rs),
  [`reload.rs`](./src/reload.rs)) — optional; spawned only when a config file is
  in play and `--no-hot-reload` is not set. Swaps the in-process rule set without
  restarting the shell.

The active rule set is shared as an `ArcSwap`, so a hot reload publishes a new
ruleset atomically without locking the hot path.

**Backpressure and interruption.** The output thread writes to stdout with a
blocking `write_all` + `flush`, so a slow terminal applies natural back-pressure:
the write blocks, the PTY master stops being drained, the kernel PTY buffer
fills, and the child blocks on its next write — nothing buffers unboundedly.
`poll(2)` and `read(2)` retry on `EINTR` (e.g. a `SIGWINCH` delivered to an I/O
thread); a closed child surfaces as `EIO` on Linux or `Ok(0)` on macOS, and both
are treated as end-of-stream.

**Exit status.** `runtime::run` returns the child shell's exit code, which
`main.rs` propagates as tayf's own exit status (its low byte). tayf-internal
failures map to BSD `sysexits.h` codes instead: `64` (`EX_USAGE`) for CLI/config
errors, `70` (`EX_SOFTWARE`) for internal bugs (e.g. a shipped profile that fails
to compile), and `71` (`EX_OSERR`) for PTY/TTY/signal/shell-discovery failures.
`--help` and `--version` exit `0`.

## Data flow

```
        user stdin ──────────────────────────────► PTY master ──► child shell
                                                                       │
   terminal ◄── stdout ◄── pipeline ◄── reader ◄── PTY master ◄────────┘
                              │
                              ├─ line_buffer  (UTF-8-safe accumulation, 64 KB cap)
                              ├─ ansi         (SGR / escape-sequence state machine)
                              ├─ rules        (compiled built-in + user patterns)
                              └─ style        (single audited SGR emission)
```

Child output is accumulated into lines (bounded), classified by the ANSI state
machine (so escape sequences and already-colored / full-screen output pass
through untouched), matched against the compiled rules, and emitted with exactly
one audited SGR wrap per match. Input flows the other way unmodified.

Passthrough is two-layered. When the ANSI state machine sees the alternate
screen, bracketed-paste, or mouse-tracking mode being **enabled**, the pipeline
switches to verbatim passthrough for the duration — so `vim`, `less`, `htop`,
`tmux`, and `fzf` render exactly as they would without tayf; the matching disable
sequence clears the flag and line-buffered colorization resumes. Independently,
at the line level, a line that already carries an SGR sequence passes through
unmodified when the rule set is configured to respect existing colors, so
already-colored output (`ls --color`, `grep --color`, …) is never re-styled.

## Module map

Grouped by responsibility (one logical concern per file —
[`src/`](./src/)):

**Entry & CLI**
- [`main.rs`](./src/main.rs) — process entry; maps results to `ExitCode`.
- [`lib.rs`](./src/lib.rs) — the `Tayf::run` facade and crate-wide policy.
- [`cli.rs`](./src/cli.rs) — `clap` argument surface (`tayf` flags + `tayf config`).

**Terminal & process**
- [`tty_guard.rs`](./src/tty_guard.rs) — RAII raw-mode guard; restores `termios`
  on every exit path (Drop + a panic hook for the `panic = "abort"` profile).
- [`pty.rs`](./src/pty.rs) — PTY allocation and child spawn (direct `argv`, never
  `sh -c`).
- [`shell.rs`](./src/shell.rs) — shell discovery (`$SHELL` → `/etc/passwd` → `/bin/sh`).
- [`signals.rs`](./src/signals.rs) — signal forwarding to the child process group.
- [`runtime.rs`](./src/runtime.rs) — the two-thread `poll(2)` I/O loop + shutdown.
- [`terminfo.rs`](./src/terminfo.rs) — TTY detection, window size, color depth.
- [`bg_detect.rs`](./src/bg_detect.rs) — best-effort startup background-color
  detection (`COLORFGBG`, then an OSC 11 query) to pick a default theme.

**Colorization pipeline**
- [`pipeline.rs`](./src/pipeline.rs) — the passthrough/colorize state machine
  (alt-screen / bracketed-paste / mouse-mode awareness) and `apply_rules`.
- [`line_buffer.rs`](./src/line_buffer.rs) — UTF-8-safe line accumulator with a
  hard 64 KB cap that flushes rather than growing unbounded.
- [`ansi.rs`](./src/ansi.rs) — the ANSI/SGR escape-sequence state machine.
- [`rules.rs`](./src/rules.rs) — compiled rule struct, the built-in pattern
  catalog, and the filename-extension catalog.
- [`style.rs`](./src/style.rs) — `Color`/`Style` types and the SGR audit gate
  (the single point that can emit an escape sequence).

**Configuration**
- [`config.rs`](./src/config.rs) — TOML config schema and loading.
- [`themes.rs`](./src/themes.rs) — built-in + on-disk color themes.
- [`profiles.rs`](./src/profiles.rs) — named, embedded + on-disk rule profiles.
- [`reload.rs`](./src/reload.rs) — hot-reload orchestration and precedence.
- [`watch.rs`](./src/watch.rs) — config file watcher.
- [`config_tui/`](./src/config_tui/) — the interactive `tayf config` TUI
  (ratatui).

**Support**
- [`error.rs`](./src/error.rs) — the `Error` enum (`thiserror`); user-facing
  `Display` messages.
- [`log.rs`](./src/log.rs) — `TAYF_LOG`-gated, stderr-only diagnostic logging.
- [`version.rs`](./src/version.rs) — build-time commit SHA + rustc banner.

## Built-in patterns

tayf ships a small, curated set of built-in rules ([`rules.rs`](./src/rules.rs))
matching patterns common in terminal output — IP addresses (v4/v6), MAC
addresses, log levels, timestamps, durations, file permissions, URLs, emails,
UUIDs, FQDNs, HTTP status codes, and a catalog of file extensions.

Design constraints that shape the catalog:

- **Linear-time only.** Every pattern is a linear-time regex — no backtracking,
  no look-around (a deliberate ReDoS-safety choice). A pattern that would need
  surrounding context to disambiguate is either kept simple (accepting rare
  shape-collisions, documented in the README) or left to user config rather than
  shipped as a built-in.
- **First match wins.** Rules are ordered most-specific-first; the first rule to
  match a span colorizes it. Overlap resolves by definition order.
- **One audited SGR wrap per match.** Styling goes through [`style.rs`](./src/style.rs)'s
  audit gate (see Key invariants), never a hand-rolled escape.

**Default palette.** The built-in rules default to a curated 24-bit "Neon"
palette (`Color::Rgb`), with `log_level` carrying the one bold "alert"
affordance. On terminals below truecolor, colors downgrade to the nearest
256/16-indexed entry while preserving attributes. Three themes are built in:
`dark` (the default Neon palette), `light` (a hand-authored light-background
adaptation), and `classic` (the previous named-ANSI palette, terminal-adaptive,
opt-in via `--theme classic`). The 24-bit palette emits longer SGR sequences
than named-ANSI, so per-line overhead is higher on heavily-matched streams — a
byte-count effect only; `classic` is the lighter-weight option. User config and
on-disk themes/profiles override, disable, or extend any built-in
([`config.rs`](./src/config.rs), [`themes.rs`](./src/themes.rs), [`profiles.rs`](./src/profiles.rs)).

## Configuration precedence

The initial load and every hot reload resolve the same chain
([`reload.rs`](./src/reload.rs)):

- **Profile** — `--profile` flag, else `config.general.profile`.
- **Theme** — `--theme` flag, else `config.general.theme`, else the selected
  profile's `theme`, else the startup background-color detection
  ([`bg_detect.rs`](./src/bg_detect.rs)).

CLI flags are snapshotted once at startup and re-applied on every reload, so a
mid-session config edit can never silently drop an active CLI override. The
background-color probe likewise runs only at startup; reloads reuse that result
rather than re-querying the terminal.

## Key invariants

- **Terminal state is always restored.** The raw-mode guard restores `termios`
  on normal exit, error return, and panic (a `set_hook` fallback covers the
  `panic = "abort"` profile where `Drop` is skipped).
- **The line buffer is hard-capped (64 KB).** Exceeding the cap flushes; it never
  grows unbounded, bounding memory against pathological no-newline input.
- **Everything is raw bytes.** Rules run on `regex::bytes`, never `str`, so binary
  output and invalid UTF-8 pass through without panicking, and a multibyte
  character split across two reads is reassembled across feeds.
- **Only eligible bytes are colorized.** A byte is a candidate for a rule match
  only when it is outside an active escape sequence, outside passthrough mode
  (alt-screen / bracketed-paste / mouse), and within the line-buffer bounds.
  Everything else — escape sequences, full-screen UIs, over-cap blobs — is
  emitted verbatim.
- **The regex engine is linear-time.** No backtracking, no look-around — a
  deliberate ReDoS-safety choice. Every user/TUI compile path enforces regex
  size and DFA-cache limits.
- **Exactly one audited SGR emission.** `style.rs` is the only place that emits an
  escape sequence, and it can only produce a numeric SGR wrap with a precise
  `\x1b[0m` reset — never a wide-effect code. tayf never introduces an OSC or
  other sequence the child did not.
- **Signals target the process group.** Forwarding uses `killpg` against the
  child's group so signals reach the whole job, not just the leader.
- **Config paths are canonicalized and symlink-checked**, rejecting traversal
  outside the config base. No code path executes strings from config.

## Safety and security

PTY output is treated as attacker-controlled. The byte path is hardened against
ReDoS, escape-sequence injection, memory exhaustion, and terminal-state
corruption. The threat model and non-goals live in [SECURITY.md](./SECURITY.md).
The crate keeps `unsafe` to a tiny, individually-documented set of sites.

## Testing and performance

- **Unit tests** live alongside the logic they cover.
- **Integration tests** ([`tests/`](./tests/)) spawn a real PTY and assert
  end-to-end behavior (signals, colorization, escape hatches).
- **Fuzzing** ([`fuzz/`](./fuzz/)) — a separate `cargo-fuzz` workspace with
  libFuzzer targets over the ANSI state machine, line buffer, pipeline, and regex
  compilation; crashes are distilled into the stable adversarial regression
  tests.
- **Benches** ([`benches/`](./benches/)) — criterion microbenchmarks (scanner
  throughput, pipeline feed, a ReDoS time-bound check) plus an end-to-end
  PTY-vs-`cat` overhead harness, against a recorded baseline
  ([`BASELINE.md`](./benches/BASELINE.md)). The spec's <20%-overhead target is
  end-to-end and measured per input shape (rule-heavy, no-newline, passthrough);
  see BASELINE.md for the current results.
