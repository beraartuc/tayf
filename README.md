# tayf

Terminal-agnostic, PTY-based, regex-driven output colorizer in Rust.

`tayf` wraps your shell inside a pseudo-terminal and applies a small set of
regular expressions to the byte stream, so common patterns (IP addresses,
log levels, HTTP status codes, durations, FQDNs, file extensions, UUIDs,
URLs, emails, timestamps, file permissions, MAC addresses) appear colorized —
in any terminal emulator, with any shell, with no aliases or per-command
wrapping.

[![CI](https://github.com/beraartuc/tayf/actions/workflows/ci.yml/badge.svg)](https://github.com/beraartuc/tayf/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/tayf)](https://crates.io/crates/tayf)
[![MSRV 1.88](https://img.shields.io/badge/MSRV-1.88-blue)](https://doc.rust-lang.org/stable/)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](./LICENSE-MIT)

## Install

### Quick install (Linux & macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/beraartuc/tayf/main/install.sh | sh
```

Detects your OS/architecture, downloads the matching signed binary from the
latest [release](https://github.com/beraartuc/tayf/releases), verifies its
SHA256 (and its Sigstore provenance if an authenticated `gh` CLI is present),
and installs it to `~/.local/bin` — no sudo. Linux binaries are static (musl),
so they run on any distribution (glibc or musl, old or new) and on x86-64 or
ARM64.

Prefer to read it first? `curl -fsSL https://raw.githubusercontent.com/beraartuc/tayf/main/install.sh | less`

Knobs:

- `TAYF_INSTALL_DIR=/usr/local/bin` — install elsewhere.
- `TAYF_VERSION=v0.11.0` — pin a specific release (also the escape hatch if the
  GitHub API rate-limits the latest-version lookup).

### Homebrew (macOS / Linux)

```bash
brew install beraartuc/tayf/tayf
```

Homebrew places the binary in `/opt/homebrew/bin` (Apple Silicon) or
`/usr/local/bin` (Intel). Both are typically on `PATH` after a standard
Homebrew install.

### Cargo

```bash
cargo install tayf
```

Installs to `~/.cargo/bin`. Ensure that directory is on your `PATH`
(`echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.zshrc`).

Minimum supported Rust version: **1.88**.

### Pre-built binary

Download the binary for your platform from the
[GitHub releases page](https://github.com/beraartuc/tayf/releases), then
verify and install:

```bash
# Verify provenance (GitHub-signed SLSA attestation)
gh attestation verify ./tayf --repo beraartuc/tayf

# Verify checksum
shasum -a 256 -c SHA256SUMS

# Install
chmod +x ./tayf
sudo mv ./tayf /usr/local/bin/tayf
```

## Use

```bash
# Replace your shell for one session:
tayf

# Or pass a specific shell:
tayf --shell /bin/bash

# Use a light-background theme:
tayf --theme light

# Use the classic named-ANSI palette (lighter SGR output):
tayf --theme classic
```

`tayf` automatically discovers your shell from `$SHELL`, falling back to
`/etc/passwd` and then `/bin/sh`. Override with `--shell /path/to/shell` or
request a login shell with `--login`.

When stdout is not a TTY (e.g. `tayf | tee log.txt`), colorization is
disabled automatically.

## Get started (`tayf init`)

After installing, run once:

```bash
tayf init
```

This writes a default config to `~/.config/tayf/config.toml` (a starting point
you can customize) and, for **bash/zsh**, adds an always-on guard to your
`~/.zshrc` / `~/.bashrc` so tayf wraps every new interactive terminal
automatically. It backs up your rc first and is safe to re-run (idempotent).
For **fish** and other shells it prints the snippet to add manually.

tayf takes effect in new terminals — open one, or run `exec tayf` to start now.
Built-in colors work immediately; use `tayf config` (below) to customize.

Useful flags: `--print` (show the snippet without editing anything),
`--no-shell-hook` (create the config only), `--uninstall` (remove the guard),
`--force` (overwrite the config with defaults).

## Configure interactively (`tayf config`)

`tayf` ships a full-screen TUI for browsing and editing your colorization
rules live — no need to hand-edit TOML:

```bash
tayf config
```

Four tabs (switch with `Tab` / `Shift+Tab` or the number keys `1`–`4`):

- **Patterns** — toggle, override, reset, or delete any of the eighteen built-in
  rules, add your own regex patterns, and pick colors interactively (ANSI 16,
  256-color, or 24-bit hex, plus bold / italic / underline). Press Space on a
  rule to enable or disable it.
- **Themes** — browse and select a built-in or on-disk theme.
- **Profiles** — browse and activate personal disk profiles.
- **Status** — the resolved config state and recent hot-reload events.

A live preview strip shows how your changes colorize a sample line as you
edit. `Ctrl+S` saves back to `config.toml` (atomic write with a timestamped
backup; if the file changed on disk in the meantime, a three-way merge view
lets you resolve each conflict). Press `?` or `F1` for the full keybinding
cheat-sheet, and `Ctrl+C` or `q` to quit.

Two non-interactive helpers share the same subcommand:

```bash
tayf config dump      # print the 18 built-in patterns + theme catalog + profiles note as TOML
tayf config status    # print the resolved config + recent reload events
```

## Always-on (run tayf automatically)

### Terminal emulator startup command (recommended)

Set your terminal's shell/command to `tayf` so every new window uses it:

- **iTerm2**: Preferences → Profiles → General → Command →
  select "Custom Shell" → `/opt/homebrew/bin/tayf`
- **Terminal.app**: Preferences → Profiles → Shell → "Run command" → `tayf`
- **Alacritty** (`~/.config/alacritty/alacritty.toml`):
  ```toml
  [shell]
  program = "/opt/homebrew/bin/tayf"
  ```
- **kitty** (`~/.config/kitty/kitty.conf`):
  ```
  shell /opt/homebrew/bin/tayf
  ```
- **GNOME Terminal / Konsole**: Edit the profile and set the custom command to
  `tayf` (or its full path).

### Shell rc (works for all terminals at once)

`tayf init` adds this for you (bash/zsh). To do it by hand instead:

Add this guard to `~/.zshrc` or `~/.bashrc`:

```sh
# ~/.zshrc or ~/.bashrc
[[ $- == *i* && -t 1 && -z $TAYF_SESSION ]] && exec tayf
```

How it works: tayf sets `TAYF_SESSION=1` in the child shell it spawns. The
guard checks for an interactive shell (`*i*`), a TTY on stdout (`-t 1`), and
the absence of `TAYF_SESSION` — preventing an infinite re-exec loop.

## Built-in rules

`tayf` ships with **eighteen** built-in patterns, listed in priority order
(most-specific first; first match wins). The default `dark` theme uses a
curated 24-bit "Neon" palette:

| Name        | Default | Color                      | Example                                              |
|-------------|---------|----------------------------|------------------------------------------------------|
| permission  | on      | dim gray (per-group colors) | `-rw-r--r--`, `drwxr-xr-x`                         |
| timestamp   | on      | dim gray (date=amber, time=green, tz=violet) | `2026-05-22T10:30:00Z`, `[22/May/2026:10:30:00 +0000]` |
| uuid        | on      | magenta                    | `550e8400-e29b-41d4-a716-446655440000`               |
| url         | on      | blue, underlined           | `https://example.com/path`, `ssh://host`             |
| email       | on      | lime green                 | `user@example.com`                                   |
| ipv4        | on      | azure (not bold)           | `192.168.1.1`                                        |
| ipv6        | on      | indigo                     | `fe80::1`, `2001:db8::1`                             |
| mac         | on      | teal                       | `aa:bb:cc:dd:ee:ff`                                  |
| log_level   | on      | hot-coral, **bold**        | `ERROR`, `WARN`, `INFO`, `DEBUG`, `TRACE`, ...       |
| filename    | on      | orange                     | `claude.md`, `archive.tar.gz`, `config.json`         |
| fqdn        | on      | violet                     | `api.example.com`                                    |
| duration    | on      | amber                      | `20.291 ms`, `1.5 ms`, `100ms`, `2d3h`              |
| arn         | on      | cyan                       | `arn:aws:iam::123456789012:role/MyRole`              |
| instance_id | on      | orange-red                 | `i-0abcd1234567890ef`                                |
| region      | **off** | green                      | `us-east-1`, `eu-west-2`                             |
| container_id | **off** | peach                     | `7c79c4bf9712` (12-hex Docker short hash)            |
| image_tag   | **off** | pink                       | `gcr.io/google/nginx:1.21`, `nginx:latest`           |
| pod_name    | **off** | periwinkle                 | `nginx-deployment-7c79c4bf97-9hk6r`                 |

Pattern notes:

- **permission** uses per-capture-group styling: type character (blue), owner
  bits (red), group bits (amber), other bits (green). The overall style is dim
  gray for the padding spaces.
- **timestamp** uses per-capture-group styling: date (amber), separator
  (dim), time (green), timezone (violet). Spans ISO-8601, syslog, Apache/nginx,
  and RFC 2822 formats.
- **uuid** matches the canonical 8-4-4-4-12 hex form.
- **url** matches `https?://`, `ssh://`, `ftp://`, and `git@host:path` URLs.
  The entire span is rendered as a single underlined blue span.
- **email** matches an RFC 5322 simplified shape.
- **log_level** is the one bold "alert" affordance in the Neon palette —
  hot-coral and bold for immediate visibility.
- **ipv4** is azure (a medium blue), without bold (bold was removed in v0.9.1
  as part of the Neon palette tuning).
- The **filename** rule covers a curated catalog of common extensions —
  archives (`zip`, `tar.gz`, `7z`, ...), source code (`rs`, `py`, `ts`,
  `go`, ...), configuration (`json`, `yaml`, `toml`, ...), documents
  (`pdf`, `md`, ...), media, and binary formats. See `src/rules.rs` for
  the full list.
- **arn** matches the `arn:aws[…]` prefix and is prose-immune (`warn:aws`,
  `alarm:aws` are not matched). Priority 200 so it envelopes interior rules.
- **instance_id** matches `i-` followed by exactly 17 lowercase hex digits —
  `i-0abcd1234567890ef`. Multi-hex words (`multi-…`, `wifi-…`) are rejected.
- **region** is a 34-entry exhaustive enumeration of AWS region strings
  (dated snapshot 2026-05-26). New regions are added in patch releases.
- **container_id** matches a bare 12-hex token. Collides with git short
  hashes, so it ships default-off.
- **image_tag** matches `registry.host/image:tag` and `name:latest` forms.
  Single-word `name:version` without a dot-host (e.g. `nginx:1.21`) is not
  matched to keep false-positive rate low.
- **pod_name** matches Kubernetes pod suffixes: `<deploy>-<10-base32>-<5-base32>`.

### Default-off built-ins

Four built-in rules ship **disabled** by default because they have
higher false-positive risk. Enable them individually in your config:

```toml
# ~/.config/tayf/config.toml

# Enable Docker container-ID coloring (default off — collides with git hashes).
[[rules]]
name = "container_id"
enabled = true

# Enable AWS region highlighting (default off — dated enum snapshot).
[[rules]]
name = "region"
enabled = true

# Enable Docker/OCI image-tag highlighting (default off).
[[rules]]
name = "image_tag"
enabled = true

# Enable Kubernetes pod-name highlighting (default off).
[[rules]]
name = "pod_name"
enabled = true
```

The same `enabled = true` override works inside a named profile file (see
[Named profiles](#named-profiles) below).

### Themes

tayf ships three built-in themes:

- **`dark`** — the default Neon 24-bit palette described above. Best for
  dark-background terminals.
- **`light`** — hand-authored adaptation for light-background terminals,
  where the bright ANSI family renders washed out.
- **`classic`** — the pre-v0.9 named-ANSI palette, terminal-adaptive. Emits
  shorter SGR sequences (a byte-count advantage on heavily-matched streams
  compared to the 24-bit Neon palette).

**24-bit palette note:** The default `dark` (Neon) theme emits 24-bit
(`\x1b[38;2;R;G;Bm`) SGR sequences. On streams with many matches this
produces slightly more bytes than the `classic` named-ANSI palette. Use
`--theme classic` if byte overhead matters (e.g. very high-throughput log
tailing on slow connections).

Pick a theme from the CLI:

```sh
tayf --theme light
tayf --theme classic
```

Or set a default in your config:

```toml
# ~/.config/tayf/config.toml
[general]
theme = "light"
```

CLI `--theme` overrides the config field. Your own `[[rules]]` blocks still
win over the theme: theme styles are pre-loaded on top of built-in defaults,
and your user rules override either layer.

Unknown theme names exit with code 64 (`EX_USAGE`) and list the known themes
on stderr.

The theme selection is fixed at startup: changing `[general] theme` in your
config does **not** take effect on hot reload, so restart tayf to switch
themes. Your `[[rules]]` edits still hot-reload as usual.

### Custom themes

In addition to the three built-in presets, you can drop your own theme files
into `~/.config/tayf/themes/<name>.toml` (or
`$XDG_CONFIG_HOME/tayf/themes/<name>.toml`) and reference them by name:

```toml
# ~/.config/tayf/themes/my-dark.toml
[[rules]]
name = "log_level"
style = { fg = "cyan", bold = true }

[[rules]]
name = "ipv4"
style = { fg = "#ffaa00" }
```

```sh
tayf --theme my-dark
```

A disk theme follows the same schema as a built-in preset — it is a list of
`[[rules]]` blocks that override the style of an existing built-in pattern.
The following constraints apply:

- The file name **must not** match a built-in (`dark`, `light`, `classic`); use
  a different name (`my-dark`, `solarized-dark`) and reference it with
  `--theme <name>`. The check is case-insensitive.
- Theme rules **must** name an existing built-in pattern; they cannot introduce
  new patterns. Use the user config (`config.toml`) to add patterns.
- Theme rules **must not** set `pattern` or `enabled = false`.
- The theme file **must not** carry a `[general]` section.

Validation surfaces every violation in one pass — fix all of them in a single
editor cycle. Invalid theme files exit `EX_USAGE` (64).

### Automatic background detection (best-effort)

When you have not pinned a theme via `--theme <name>` or `[general] theme`,
tayf tries to detect your terminal's background color at startup and apply the
matching preset:

1. `COLORFGBG` env var (rxvt/urxvt; some xterm configs).
2. OSC 11 query against `/dev/tty` with a 100 ms timeout (xterm, iTerm2,
   kitty, Alacritty, foot, WezTerm, modern tmux with
   `set -g allow-passthrough on`).
3. Fallback: `dark`.

**Limitations.** Modern terminals have dozens of light/dark variants —
Solarized, Nord, Gruvbox, Dracula, Tokyo Night, Catppuccin, and many more.
tayf's built-in `dark` and `light` presets give reasonable contrast for
"default-ish" terminals; they do not claim to match every specific palette.
**This detection is a starting point, not an authoritative choice.**

**Opt-out.** Pin a theme explicitly with `--theme dark` (or `light`,
`classic`) on the CLI, or set `[general] theme = "dark"` in your config.
Explicit choices always win over detection.

**Custom palettes.** For fine-grained matching to a specific terminal palette,
override rule styles in your config under `[[rules]]`, or drop a full theme
file under `~/.config/tayf/themes/<name>.toml` (see
[Custom themes](#custom-themes) above).

**Multiplexers.** tmux is supported when passthrough is enabled — tmux ≤3.2
enables it by default; tmux ≥3.3 requires `set -g allow-passthrough on`. GNU
screen is not supported (the OSC 11 path is skipped and tayf falls back to
`dark`). Detection is also skipped when stdout is not a TTY, when `--no-color`
is set, or when `TERM=dumb`.

## Configuration

`tayf` reads an optional TOML config from
`$XDG_CONFIG_HOME/tayf/config.toml` (falling back to
`~/.config/tayf/config.toml`). Pass `--config <path>` to use a different
file. Without a config file, the fourteen default-on built-in rules described
above are active with their default styles.

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
no `COLORTERM=truecolor`, etc.) `tayf` automatically downgrades — Rgb values
collapse to the closest 256-indexed or ANSI 16 entry, attributes like `bold`
and `italic` are preserved.

### Style fields

`style = { fg, bg, bold, italic, underline, dim }`. Every field is optional,
but a rule whose style would produce no visible effect is rejected at load
time — use `enabled = false` to disable a rule instead.

### Built-in rule names

All eighteen built-in names, in priority order (override or disable any of
them with a `[[rules]]` entry in your config):

`permission`, `timestamp`, `uuid`, `url`, `email`, `ipv4`, `ipv6`, `mac`,
`log_level`, `filename`, `fqdn`, `duration`,
`instance_id`, `region`, `arn`, `container_id`, `image_tag`, `pod_name`.

The last six are domain-specific patterns; `region`, `container_id`,
`image_tag`, and `pod_name` are default-off (see above).

### Errors

Malformed configs exit with code `64` (`EX_USAGE`) and print a friendly
diagnostic to stderr that includes the file path and the offending line number
when available.

### Named profiles

A **profile** is a personal, switchable preset — a `[[rules]]` list plus an
optional `theme`. Create a profile file at
`~/.config/tayf/profiles/<name>.toml`:

```toml
# ~/.config/tayf/profiles/work.toml

# Enable container-ID coloring in this profile.
[[rules]]
name = "container_id"
enabled = true

# Recolor log_level to your taste.
[[rules]]
name = "log_level"
style = { fg = "#ffaa00", bold = true }

# Add a custom Jira-ticket rule.
[[rules]]
name = "ticket"
pattern = 'PROJ-\d+'
style = { fg = "#5ad7e0" }
```

Switch to it with `--profile work` or by setting `[general] profile = "work"`
in `config.toml`. When a profile is active its `[[rules]]` **replace**
`config.toml`'s `[[rules]]`; the built-in rule catalog (18 rules) remains the
substrate — built-ins not overridden in the profile retain their default
colors and enabled state. `[general]` settings (theme, banner, profile pointer)
always come from `config.toml`.

```sh
# One-off:
tayf --profile work

# Permanent (add to config.toml):
# [general]
# profile = "work"
```

Profiles are stored at `~/.config/tayf/profiles/*.toml` (or
`$XDG_CONFIG_HOME/tayf/profiles/*.toml`). The `tayf config` TUI lists and
activates them from the **Profiles** tab.

### Hot reload

`tayf` watches your config file and reloads it whenever you save:

- Editing `~/.config/tayf/config.toml` (or the file passed to `--config`)
  takes effect within ~250 ms — no restart, no shell respawn.
- `pkill -HUP tayf` (or `kill -HUP <pid>`) forces an immediate reload that
  bypasses the file-watcher debounce window. Useful from scripts.
- If your edit produces invalid TOML or a bad regex, `tayf` keeps the
  **previous** rule set in effect and logs a warning to stderr
  (`TAYF_LOG=warn`, the default). Your terminal session is never disrupted by
  a typo in the config.
- The child shell is never restarted. `tayf` only swaps its in-process rule
  set; the PTY, signal handlers, and the running shell are untouched.

If `tayf` was launched without a config file, the file watcher is not active
— but `SIGHUP` still works as a manual reload trigger. If you create a config
later, send `SIGHUP` to pick it up.

**Note:** `SIGHUP` is forwarded to the child process group in every
configuration, mirroring `SIGINT` / `SIGTERM`. If you have a script that
relies on `tayf` swallowing `SIGHUP`, install your own trap
(`trap '' HUP`) in the child shell.

### Respecting existing colors

If your input already contains ANSI color sequences (`git log --color=always`,
`journalctl` with colors, colored compiler output piped through tayf), you can
tell tayf to leave those lines alone:

```toml
# ~/.config/tayf/config.toml
[general]
respect_existing_colors = true   # default
```

When `true`, any line containing at least one SGR sequence (`\e[…m`) is
written to stdout byte-for-byte; tayf does not apply its own rules to it.
Lines without SGR are colored normally per your config.

Caveat: line-level only. A multi-line color block (e.g. `git log` with
`\e[31m` on one line and `\e[0m` later) is honored per SGR-bearing line, but
rules may still run on intermediate plain lines.

## Escape hatches

### `--bypass` / `TAYF_DISABLE`

When you need tayf to *get out of the way* — a noisy program you are
debugging, an output stream whose patterns conflict with tayf's built-ins, or
a CI invocation where you want raw passthrough — set either the CLI flag or
the env var. CLI takes precedence.

```sh
# One-shot:
TAYF_DISABLE=1 my-tool

# Conditional wrap in ~/.zshrc:
[[ -n "$TAYF_DISABLE" ]] || exec tayf
```

`TAYF_DISABLE` accepts `1`, `true`, or `yes` (case-insensitive). Any other
value (including unset) means bypass is off.

In bypass mode tayf still:
- Wraps the PTY (your child shell still has a controlling terminal).
- Forwards `SIGWINCH`, `SIGINT`, `SIGTERM`, `SIGHUP` to the child process
  group (so `Ctrl-C` and tmux detach still work).
- Protects the terminal via its raw-mode RAII guard (Ctrl-C / panic cleanup
  is still safe).

Bypass mode does NOT:
- Match patterns or inject SGR.
- Detect the terminal background (`bg_detect` is skipped — zero startup
  latency).
- Spawn the file watcher or reload orchestrator threads.
- Read your config file (so `--config <path>` is silently ignored under
  `--bypass`).

**Note:** `TAYF_DISABLE` is **different** from `TAYF_DISABLE_BG_DETECT`
(test-only OSC 11 bypass — do not use in production).

### `--no-hot-reload`

Skip the file watcher and the reload orchestrator. Config is still loaded at
startup; only *re*-loading is off.

```sh
tayf --no-hot-reload
```

With no config file present, `--no-hot-reload` is a no-op — no watcher would
have spawned anyway. `SIGHUP` is forwarded to the child process group
regardless of this flag.

### `[general] show_reload_banner` (opt-in)

When set, tayf writes a one-line dim banner to `/dev/tty` after each
successful hot reload:

```
tayf: config reloaded
```

```toml
[general]
show_reload_banner = true
```

Default: `false` (opt-in). The banner is wrapped in cursor save/restore
(DECSC/DECRC) so most multi-line shell prompts (zsh ZLE, `RPROMPT`) keep
their visual cursor position. Exotic prompt frameworks (Powerlevel10k
transient prompt) may show small redraw artifacts; press `Ctrl-L` to clean up.

When you are inside `vim` / `less` / any program using the alt-screen, the
banner is drawn into the alt-screen buffer and vanishes when the program
exits.

Banner is suppressed on reload **failures** — those still surface via the
existing `TAYF_LOG=warn` stderr line.

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

## Known limitations

A few built-in patterns can mis-fire on inputs that are shape-identical to
their real targets. tayf's regex engine is linear-time (no backtracking, no
look-around — a deliberate ReDoS-safety choice), so it cannot use the
surrounding context that would disambiguate these cases. They are rare in real
terminal output, but if one bothers you, disable the built-in (recipe below):

- **`filename` on dotted prose.** Prose ending in a single-letter source
  extension — `a.b.c`, `u.v.w` — can be styled as a filename, because real
  files like `main.c` / `top.v` are suffix-identical and there is no
  path/command context to tell them apart.
- **`fqdn` on JWT / base64url tokens.** A 3-segment dotted base64url token
  (e.g. a JWT, `eyJhbGc.eyJzdWI.signature`) matches `fqdn`. Telling it from a
  real domain would need a 4000+ entry TLD allowlist.
- **`ipv4` on 5+-segment versions.** A bare 5-segment dotted version such as
  `1.2.3.4.5` styles its leading `1.2.3.4` as an IPv4 address.

**Recipe — disable a noisy built-in.** Add an `enabled = false` rule to your
config:

```toml
# ~/.config/tayf/config.toml
[[rules]]
name = "fqdn"
enabled = false
```

Use `name = "filename"` or `name = "ipv4"` to disable those instead.

## Diagnostics

Set `TAYF_LOG=debug` to send diagnostic logs to stderr.

`TAYF_DISABLE_BG_DETECT=1` (or `true`/`yes`, case-insensitive) is a
**test-only escape hatch** that skips the startup background-color detection
and assumes a dark theme. Use it in CI environments where `/dev/tty` is a
pseudo-terminal slave that cannot respond to OSC 11 queries. For production
use, prefer `--theme dark` / `--theme light` or `[general] theme = "..."`
in your config.

## Security posture

`tayf` sits in the I/O path of every command in your shell. Its built-in
patterns are hand-tuned to be linear-time (no catastrophic backtracking), its
line buffer is hard-capped at 64 KB, and its only ANSI emission is through a
single audited SGR sequence. See [`SECURITY.md`](./SECURITY.md) for the
vulnerability reporting policy and the full threat model.

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

- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — internal design and module map
- [`CONTRIBUTING.md`](./CONTRIBUTING.md) — how to contribute
- [`SECURITY.md`](./SECURITY.md) — vulnerability reporting and threat model
