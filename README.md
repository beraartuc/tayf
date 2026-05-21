# tayf

Terminal-agnostic, PTY-based, regex-driven output colorizer.

> **Status:** v0.1 in development. Not yet ready for general use.

## Overview

`tayf` wraps your shell in a pseudo-terminal and applies regex rules to the
output stream, so common patterns (IP addresses, log levels, HTTP status codes,
duration metrics, etc.) appear colorized automatically — in any terminal
emulator, with any shell. Think iTerm2 Triggers as a standalone binary.

See [`tayf-tasarim.md`](./tayf-tasarim.md) for the full design (Turkish).

## License

Dual-licensed under either of:

- [Apache License, Version 2.0](./LICENSE-APACHE)
- [MIT license](./LICENSE-MIT)

at your option.
