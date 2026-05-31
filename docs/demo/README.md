# Demo recording

A short asciinema cast that shows tayf colorizing live shell output, embedded in
the project README.

> **Note:** record the cast **against the released build** so the colors match
> what users get. (If the default palette changes in a release, re-record.)

## Prerequisites

- [asciinema](https://asciinema.org/) — `brew install asciinema` /
  `pipx install asciinema`.
- A `tayf` binary on your `PATH` (installed, or `cargo build --release` and use
  `target/release/tayf`).
- Optional: [`svg-term`](https://github.com/marionebl/svg-term-cli) to convert
  the cast to an inline SVG for the README.

## Record

```bash
# 1. Start recording.
asciinema rec docs/demo/tayf.cast --title "tayf — live colorized shell"

# 2. Inside the recording, launch tayf and run the scripted content:
tayf
bash docs/demo/sample-session.sh
exit        # leave tayf
exit        # stop the asciinema recording (Ctrl-D)
```

The scripted content in [`sample-session.sh`](./sample-session.sh) is crafted to
exercise the built-in patterns (IPs, log levels, timestamps, durations, file
permissions, URLs, UUIDs, FQDNs) so the colorization is visible and
deterministic. Feel free to ad-lib a few real commands too.

## Convert (optional)

```bash
# Inline SVG for the README:
svg-term --in docs/demo/tayf.cast --out docs/demo/tayf.svg --window

# Or upload and link:
asciinema upload docs/demo/tayf.cast
```

Then reference the SVG (or the asciinema link) from the README's demo section.
