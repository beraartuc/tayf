# tayf

Terminal-agnostic, PTY-based, regex-driven output colorizer in Rust.

`tayf` wraps your shell inside a pseudo-terminal and applies a small set of
regular expressions to the byte stream, so common patterns (IP addresses,
log levels, HTTP status codes, durations, FQDNs, file extensions, UUIDs,
URLs, emails, timestamps, file permissions) appear colorized — in any
terminal emulator, with any shell, with no aliases or per-command wrapping.

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

## Built-in rules

`tayf` ships with thirteen built-in patterns, listed in priority order
(most-specific first; first match wins):

| Name        | Color           | Example                                       |
|-------------|-----------------|-----------------------------------------------|
| Permission  | dim white       | `-rw-r--r--`, `drwxr-xr-x`                    |
| Timestamp   | bright black    | `2026-05-22T10:30:00Z`, `[22/May/2026:10:30:00 +0000]` |
| UUID        | bright magenta  | `550e8400-e29b-41d4-a716-446655440000`        |
| URL         | bright blue, underlined | `https://example.com/path`, `ssh://host` |
| Email       | bright green    | `user@example.com`                            |
| IPv4        | bold yellow     | `192.168.1.1`                                 |
| IPv6        | bright yellow   | `fe80::1`, `2001:db8::1`                      |
| MAC         | cyan            | `aa:bb:cc:dd:ee:ff`                           |
| Log level   | bold bright-red | `ERROR`, `WARN`, `INFO`, ...                  |
| HTTP status | magenta         | ` 200 `, `/404`, `:500`                       |
| Filename    | bright cyan     | `claude.md`, `archive.tar.gz`, `config.json`  |
| FQDN        | blue            | `api.example.com`                             |
| Duration    | green           | `20.291 ms`, `1.5 ms`, `100ms`                |

Pattern notes:

- **Permission** matches POSIX `ls -l` file mode strings.
- **Timestamp** spans multiple common formats: ISO-8601, syslog (`May 22
  10:30:00`), Apache/nginx (`[22/May/2026:10:30:00 +0000]`), and RFC 2822.
- **UUID** matches the canonical 8-4-4-4-12 hex form.
- **URL** matches `https?://`, `ssh://`, and `ftp://` URLs.
- **Email** matches an RFC 5322 simplified shape.
- The **filename** rule covers a curated catalog of common extensions —
  archives (`zip`, `tar.gz`, `7z`, ...), source code (`rs`, `py`, `ts`,
  `go`, ...), configuration (`json`, `yaml`, `toml`, ...), documents
  (`pdf`, `md`, ...), media, and binary formats. See `src/rules.rs` for
  the full list.

## Configuration (v0.2)

`tayf` reads an optional TOML config from
`$XDG_CONFIG_HOME/tayf/config.toml` (falling back to
`~/.config/tayf/config.toml`). Pass `--config <path>` to use a different
file. Without a config file, all thirteen built-in rules described above
are active with their default styles.

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
name = "kubernetes-pod"
pattern = '\b[a-z][a-z0-9-]+-[a-z0-9]{5}-[a-z0-9]{5}\b'
style = { fg = "magenta", italic = true }

[[rules]]
name = "git-sha"
pattern = '\b[0-9a-f]{7,40}\b'
style = { fg = "#888888" }
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

The thirteen names you can override or disable, in priority order:
`permission`, `timestamp`, `uuid`, `url`, `email`, `ipv4`, `ipv6`, `mac`,
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

## Themes

tayf ships two opt-in color themes baked into the binary:

- **`dark`** — explicit defaults, identical to running tayf without a theme. Useful as a starting template if you want to copy and tweak.
- **`light`** — re-tuned palette for light-background terminals (where the bright_* color family renders washed out and `BrightYellow` / `White` are effectively invisible).

Pick a theme from the CLI:

```sh
tayf --theme light
```

Or set a default in your config:

```toml
# ~/.config/tayf/config.toml
[general]
theme = "light"
```

CLI `--theme` overrides the config field. Your own `[[rules]]` blocks still win over the theme: theme styles are pre-loaded on top of built-in defaults, and your user rules override either layer.

Unknown theme names exit with code 64 (`EX_USAGE`) and list the known themes on stderr.

The theme selection is fixed at startup: changing `[general] theme` in your config does **not** take effect on hot reload, so restart tayf to switch themes. Your `[[rules]]` edits still hot-reload as usual.

Automatic background detection is planned for v0.3; for now, pick a theme manually.

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
