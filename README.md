# tayf

Terminal-agnostic, PTY-based, regex-driven output colorizer in Rust.

`tayf` wraps your shell inside a pseudo-terminal and applies a small set of
regular expressions to the byte stream, so common patterns (IP addresses,
log levels, HTTP status codes, durations, FQDNs, file extensions) appear
colorized — in any terminal emulator, with any shell, with no aliases or
per-command wrapping.

> **Status:** v0.1 is the working skeleton. Use it for casual sessions; expect
> rough edges around interactive programs that aren't full-screen. Full ANSI
> awareness, configuration via TOML, and the OSC 133 / shell-integration
> story land in v0.2 → v0.4.

## Install (from source)

```bash
git clone https://github.com/beraartuc/tayf
cd tayf
cargo install --path .
```

## Use

```bash
# Replace your shell:
tayf

# Or launch it from a terminal-emulator config (e.g. Kitty):
# shell tayf
```

`tayf` automatically discovers your shell from `$SHELL`, falling back to
`/etc/passwd` and then `/bin/sh`. Override with `--shell /path/to/shell` or
request a login shell with `--login`.

When stdout is not a TTY (e.g. `tayf | tee log.txt`), colorization is
disabled automatically.

## Built-in rules (v0.1)

| Name        | Color           | Example                                       |
|-------------|-----------------|-----------------------------------------------|
| IPv4        | bold yellow     | `192.168.1.1`                                 |
| IPv6        | bright yellow   | `fe80::1`, `2001:db8::1`                      |
| MAC         | cyan            | `aa:bb:cc:dd:ee:ff`                           |
| Log level   | bold bright-red | `ERROR`, `WARN`, `INFO`, ...                  |
| HTTP status | magenta         | ` 200 `, `/404`, `:500`                       |
| FQDN        | blue            | `api.example.com`                             |
| Duration    | green           | `20.291 ms`, `1.5s`, `100ms`                  |
| Filename    | bright cyan     | `claude.md`, `archive.tar.gz`, `config.json`  |

The filename rule covers a curated catalog of common extensions — archives
(`zip`, `tar.gz`, `7z`, ...), source code (`rs`, `py`, `ts`, `go`, ...),
configuration (`json`, `yaml`, `toml`, ...), documents (`pdf`, `md`, ...),
media, and binary formats. See `src/rules.rs` for the full list.

Configuration via TOML and user-defined patterns are slated for v0.2.

## TUI compatibility

`tayf` detects when a program enters a full-screen or interactive mode and
gets out of the way. Specifically, when a program activates any of:

- alt-screen (`\x1b[?1049h` and legacy `47`/`1047` variants)
- bracketed paste (`\x1b[?2004h`)
- mouse tracking (`\x1b[?1000h`, `1002`, `1003`, `1006`)

…tayf switches to passthrough until the program clears all of those modes.
This covers vim, less, htop, neovim, Claude Code, lazygit, k9s, gum,
bubbletea-based tools, and anything else that follows standard terminal
conventions. Their output is never altered by `tayf`.

## Known v0.1 limits

- **Passthrough is mode-based, not full ANSI-aware.** Programs that emit
  ANSI escapes outside of an alt-screen / paste / mouse mode (e.g. a
  one-line progress bar) are still colorized; in rare cases this can
  cause leakage into surrounding color contexts. Full ANSI-aware
  colorization arrives in v0.3.
- **No capture-group colorization.** Each match is wrapped in one style;
  per-group styling lands in v0.5.
- **First match wins.** Overlapping rule matches resolve by definition order.
- **Partial-line prompts are not colorized.** Interactive prompts that lack
  a trailing newline within 50ms are flushed without rule application;
  this is fixed in v0.2 when the tick-flush goes live.
- **Linux + Unix + macOS only.** Windows support is post-v1.0.
- **If `tayf` is killed by SIGKILL or aborts**, terminal state may be left
  in raw mode. Run `reset` to recover.

## Diagnostics

Set `TAYF_LOG=debug` to send diagnostic logs to stderr.

## Security posture

`tayf` sits in the I/O path of every command in your shell. Its built-in
patterns are hand-tuned to be linear-time (no catastrophic backtracking),
its line buffer is hard-capped at 64KB, and its only ANSI emission is
through a single audited SGR sequence. The full threat model is in
[`CLAUDE.md`](./CLAUDE.md) §3.

If you find a security issue, please open an issue with the label
`security`, or email the maintainer if it requires private disclosure.

## Performance

See [`benches/throughput.rs`](./benches/throughput.rs) and
[`benches/BASELINE.md`](./benches/BASELINE.md) for current numbers.
Target: <20% overhead vs native `cat` (spec §7).

Reproduce locally with:

```bash
cargo bench --bench throughput
```

## License

Dual-licensed under either of:

- [Apache License, Version 2.0](./LICENSE-APACHE)
- [MIT license](./LICENSE-MIT)

at your option.

## See also

- [`tayf-tasarim.md`](./tayf-tasarim.md) — full design (Turkish)
- [`docs/superpowers/specs/`](./docs/superpowers/specs/) — versioned design specs
