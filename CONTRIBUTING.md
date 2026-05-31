# Contributing to tayf

Thanks for your interest in tayf — a terminal-agnostic, PTY-based, regex-driven
output colorizer written in Rust. This guide covers how to build, test, and
submit changes.

## Prerequisites

- **Rust** — stable toolchain, MSRV **1.88** (the project's declared
  `rust-version`). Install via [rustup](https://rustup.rs/).
- **Platform** — Unix only: Linux or macOS. tayf uses PTYs, `termios`, and
  POSIX signals; Windows support is out of scope (see [SECURITY.md](./SECURITY.md)).

## Building

```bash
git clone https://github.com/beraartuc/tayf
cd tayf
cargo build            # debug
cargo build --release  # optimized
```

### Optional: faster link times on Linux

Link steps dominate the edit/build cycle. On Linux x86_64 you can opt into the
[mold](https://github.com/rui314/mold) linker locally — it is a drop-in
replacement and parallelizes linking. tayf does **not** commit a linker config
(so the default toolchain works out of the box for everyone); to enable mold for
yourself, install `mold` and `clang`, then create an **untracked**
`.cargo/config.toml`:

```toml
# .cargo/config.toml — local only, do not commit
[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=mold"]
```

## Testing

```bash
cargo test                       # lib + integration + doctests
cargo bench --bench throughput   # performance benches (criterion)
```

Some integration tests spawn a real PTY. In a headless or CI environment where
`/dev/tty` is a pseudo-terminal slave that cannot answer an OSC 11 background
query, set `TAYF_DISABLE_BG_DETECT=1` so startup background detection is skipped:

```bash
TAYF_DISABLE_BG_DETECT=1 cargo test
```

New behavior is developed test-first (TDD): write a failing test, make it pass,
then refactor. Pure logic lands with unit tests; PTY/signal code lands with
integration or smoke tests committed alongside the change.

## Code standards

Before submitting, your change must pass the local pre-commit gate:

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

- **Formatting:** `cargo fmt` is mandatory; CI enforces `cargo fmt --check`.
- **Lints:** `clippy::pedantic` is enabled and warnings are denied. An explicit
  `#[allow(...)]` requires a one-line `// reason: ...` comment justifying it.
- **No `unwrap()` / `expect()` in library code.** Allowed only in tests, the
  `main.rs` top-level setup, and proven-unreachable paths (with
  `unreachable!("reason")`).
- **English everywhere in code** — identifiers, comments, doc-comments, commit
  messages, PR titles/bodies, error messages, log strings, and CLI help text.
- **Public items** carry doc-comments (`///`) with at least a one-line summary;
  non-trivial APIs include an `# Examples` block.
- **`unsafe` is avoided.** Where unavoidable, isolate it and document every
  invariant with a `// SAFETY: ...` comment.
- **File-per-concept** — one logical responsibility per file. When a file grows
  past ~400 lines, that is a signal it is doing too much; consider splitting.

## Security

tayf sits in the I/O path of every command in your shell, and treats PTY output
as attacker-controlled (e.g. `tail` of untrusted logs, `ssh` to a malicious
host). Security is a first-class concern for every change — adversarial input,
memory bounds, escape-sequence handling, and terminal-state restoration.

**Do not open a public issue for a suspected vulnerability.** Report it
privately per [SECURITY.md](./SECURITY.md).

## Commits and pull requests

- Keep commits **small and atomic** — one logical change each.
- Title in **imperative present tense**, under ~70 characters
  (e.g. `fix: restore termios on panic-abort path`).
- The body explains **why**, not just what.
- PRs describe the change and reference the relevant module or area. Include the
  test that proves the behavior.

## Architecture

For a tour of the modules and data flow, see [ARCHITECTURE.md](./ARCHITECTURE.md).

## License

By contributing, you agree that your contributions are dual-licensed under the
[MIT](./LICENSE-MIT) and [Apache-2.0](./LICENSE-APACHE) licenses, at the user's
option — the same terms as the project.
