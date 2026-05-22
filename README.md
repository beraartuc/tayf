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

## Configuration (v0.2)

`tayf` reads an optional TOML config from
`$XDG_CONFIG_HOME/tayf/config.toml` (falling back to
`~/.config/tayf/config.toml`). Pass `--config <path>` to use a different
file. Without a config file, `tayf` behaves exactly as v0.1: the eight
built-in rules described above are active.

```toml
# ~/.config/tayf/config.toml

# Override a built-in: change the log_level color to yellow (loses the
# built-in's bold attribute — style overrides REPLACE wholesale).
[[rules]]
name = "log_level"
style = { fg = "yellow", bold = true }

# Disable a built-in by name.
[[rules]]
name = "fqdn"
enabled = false

# Append a new custom rule.
[[rules]]
name = "uuid"
pattern = '\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b'
style = { fg = "#888888" }

[[rules]]
name = "kubernetes-pod"
pattern = '\b[a-z][a-z0-9-]+-[a-z0-9]{5}-[a-z0-9]{5}\b'
style = { fg = "magenta", italic = true }
```

### Color values

- ANSI names: `"red"`, `"bright_cyan"`, etc. (case-insensitive).
- 256-color palette: `"color(178)"`.
- 24-bit hex: `"#ff8800"` (six digits).
- 24-bit functional: `"rgb(255, 136, 0)"`.

When the terminal cannot display a requested color depth (`TERM=dumb`,
no `COLORTERM=truecolor`, etc.) `tayf` automatically downgrades — Rgb
values collapse to the closest 256-indexed or ANSI 16 entry, attributes
like `bold` and `italic` are preserved.

### Style fields

`style = { fg, bg, bold, italic, underline, dim }`. Every field is
optional, but a rule whose style would produce no visible effect is
rejected at load time — use `enabled = false` to disable a rule instead.

### Built-in rule names

The eight names you can override or disable: `ipv4`, `ipv6`, `mac`,
`log_level`, `http_status`, `filename`, `fqdn`, `duration`.

### Errors

Malformed configs exit with code `64` (`EX_USAGE`) and print a friendly
diagnostic to stderr that includes the file path and the offending line
number when available.

### Hot reload (v0.2.1)

`tayf` watches your config file and reloads it whenever you save:

- Editing `~/.config/tayf/config.toml` (or the file passed to `--config`)
  takes effect within ~250 ms — no restart, no shell respawn.
- `pkill -HUP tayf` (or `kill -HUP <pid>`) forces an immediate reload that
  bypasses the file-watcher debounce window. Useful from scripts.
- If your edit produces invalid TOML or a bad regex, `tayf` keeps the
  **previous** rule set in effect and logs a warning to stderr
  (`TAYF_LOG=warn`, the default). Your terminal session is never disrupted
  by a typo in the config.
- The child shell is never restarted. `tayf` only swaps its in-process
  rule set; the PTY, signal handlers, and the running shell are untouched.

If `tayf` was launched without a config file, the file watcher is not
active — but `SIGHUP` still works as a manual reload trigger. If you
create a config later, send `SIGHUP` to pick it up.

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
- **Bare-unit duration matching (`1m`, `1h`, `1s`) disabled in v0.1.**
  These would collide with SGR escape sequence final bytes (`\x1b[49m`,
  etc.) producing visible garbage in colorized prompts. Re-enabled in
  v0.3 once VTE awareness can answer "is this match inside an escape
  sequence?" Currently `ns`, `us`, `μs`, `ms` are supported and cover
  modern logging conventions.
- **No capture-group colorization.** Each match is wrapped in one style;
  per-group styling lands in v0.5.
- **First match wins.** Overlapping rule matches resolve by definition order.
- **Partial-line prompts colorize on a 50ms idle tick.** The 50ms delay is
  the cost of avoiding a per-byte flush; finer-grained flush-on-prompt
  arrives in v0.3 with VTE awareness.
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
