# tayf — Project Guide for Claude

PTY-based, terminal-agnostic, regex-driven output colorizer in Rust. Open-source from day one — every decision should hold up to public scrutiny.

The full design document is in `tayf-tasarim.md` (Turkish). v0.1 scope and decisions are in `docs/superpowers/specs/`.

---

## Non-Negotiable Project Rules

These four rules override convenience and speed. Do not relax them without explicit user approval.

### 1. Language: English in code, Turkish in conversation

- **All identifiers, code comments, doc-comments, commit messages, PR titles/bodies, error messages shown to users, log strings, CLI help text, and CHANGELOG entries MUST be English.** This is an open-source project; future contributors and users read these.
- **Design documents and the conversation with the user are Turkish.** `tayf-tasarim.md` and specs under `docs/superpowers/specs/` stay in Turkish until the project is public-ready.
- A mixed-language identifier (e.g., `read_satir`) is a bug. Catch and fix on sight.
- When refactoring, do not leave half-translated files. Rename the whole module in one commit.

### 2. Architecture and Code Standards Before Code

Treat tayf like a library that someone else will read, fork, and depend on. Decide standards once, follow them everywhere.

**Before writing any non-trivial code:**
- The module's purpose, public API, dependencies, and invariants are documented (in the module's doc-comment).
- It's clear which other modules it interacts with and how (channels, function calls, shared state).
- Error types and `Result` boundaries are decided.

**Rust naming and style conventions (enforced):**
- Modules and files: `snake_case` (e.g., `io_loop.rs`, `tty_guard.rs`).
- Types, traits, enums: `UpperCamelCase` (`PtySession`, `ColorRule`).
- Functions, methods, variables, fields: `snake_case` (`spawn_shell`, `raw_mode_guard`).
- Constants and statics: `SCREAMING_SNAKE_CASE` (`DEFAULT_BUFFER_SIZE`).
- Lifetimes: short lowercase (`'a`, `'src`) — descriptive only when more than two.
- Acronyms in identifiers: treat as words (`PtyMaster`, not `PTYMaster`; `HttpStatus`, not `HTTPStatus`).
- Error enums: end with `Error` (`ConfigError`, `IoError`). Variants are nouns/noun phrases (`InvalidPattern`, not `FailedToParse`).
- Boolean flags and predicates: positive form (`is_tty`, not `not_tty`; `has_color`, not `no_color` — exception: CLI flags like `--no-color` follow CLI convention).
- File-per-concept: one logical responsibility per file. When a file passes ~400 lines, split.

**Style enforcement:**
- `cargo fmt` and `cargo clippy -- -D warnings` MUST pass before any commit.
- `clippy::pedantic` group is enabled; explicit `#[allow]` requires a one-line `// reason: ...` comment.
- No `unwrap()` or `expect()` in library code. Allowed only in: tests, `main.rs` top-level setup, and proven-unreachable paths (with `unreachable!("reason")`).
- No `panic!` in hot paths.
- Public items in `lib.rs` and module roots MUST have doc-comments (`///`) including at least a one-line summary and, for non-trivial APIs, an `# Examples` block.
- Avoid `unsafe`. If unavoidable, isolate to a small module with a `// SAFETY: ...` comment explaining every invariant.

### 3. Security Is a First-Class Concern

tayf runs in the user's terminal, spawns their shell, manipulates their TTY, and sits in the I/O path of every command. A bug here can corrupt their session, expose secrets, or be exploited by adversarial output.

**Mandatory threat model considerations for every PR:**
- **Adversarial input:** PTY output is attacker-controlled in many real scenarios (`tail` of attacker-influenced logs, `cat` of untrusted files, SSH to malicious servers). Every byte path must be safe against:
  - ReDoS — regex patterns must be linear-time. No catastrophic backtracking; benchmark on adversarial input.
  - Memory exhaustion — line buffer has a hard cap (default 64 KB, configurable). Exceeding the cap flushes, never grows unbounded.
  - Escape-sequence injection — we must never *introduce* a sequence (e.g., OSC) that the original program did not. Our SGR injections must be matched with a precise reset, never `\x1b[2J` (clear screen) or similar wide-effect codes.
  - Terminal state corruption — on any exit path (panic, signal, drop), the `termios` state MUST be restored to pre-tayf snapshot. RAII guard with `Drop`, plus `std::panic::set_hook` fallback.
- **Credentials and PII in output:** tayf sees the user's entire shell stream. We MUST NOT log raw output to disk by default. Debug logging via `tracing` is gated behind `TAYF_LOG=...` env var and writes to stderr only.
- **File-system access:** Config file path must be canonicalized; reject symlink traversal outside `~/.config/tayf/`. No code paths execute strings from config (no shell-out, no `eval`-equivalent).
- **Process spawning:** Child shell is spawned with `execvp`-style direct invocation, never `sh -c "$user_input"`. The `--shell` flag value is passed as `argv[0]`, not concatenated into a command string.
- **Signal forwarding:** SIGINT/SIGTERM go to the child *process group*, not just the child PID. Wrong target = orphan processes.
- **Dependencies:** Run `cargo audit` and `cargo deny` in CI (when CI lands). Pin transitive deps via `Cargo.lock`. Review every new direct dependency for: maintenance status, surface area, `unsafe` usage. Prefer fewer, well-audited crates.
- **Releases:** Binaries signed (cosign or sigstore) before publishing. SHA256 sums published with every release.

**Security review gate:** Use the `security-review` skill on the diff before declaring any milestone complete.

### 4. Zero Technical Debt Tolerance

This project will be read by strangers. Make it look like it was always meant to be public.

- **No "we'll fix it later" comments** unless tied to a tracked issue (`// TODO(#42): ...`). Untracked TODOs fail review.
- **No dead code.** Delete it. Git history preserves it.
- **No commented-out code.** Delete or move to a documented experiment branch.
- **Public API stability:** Any public item (anything `pub` outside `pub(crate)`) is contract. Breaking changes require a major version bump and CHANGELOG entry.
- **Tests with the feature, not after.** Pure logic uses TDD (test first). PTY/signal code uses smoke + integration tests committed alongside.
- **Documentation with the feature.** A new module without a module-level doc-comment is incomplete. A new CLI flag without `--help` text is incomplete.
- **CHANGELOG.md** maintained from the first release. Use Keep a Changelog format.
- **Error messages are user-facing UX.** Every `Display` impl on an error type must produce a sentence that tells the user (a) what failed, (b) why, (c) what to do about it. No `"error: invalid"` lines.
- **No half-features behind feature flags** as a way to merge incomplete work. A feature is merged when it works end-to-end or it stays in a branch.
- **Deprecation policy:** when removing a public API, deprecate for one minor version with `#[deprecated(note = "...")]` first.

---

## Project Layout (v0.1)

```
tayf/
├── Cargo.toml
├── Cargo.lock
├── build.rs
├── clippy.toml
├── rustfmt.toml
├── deny.toml                          # cargo-deny policy (licenses, advisories)
├── LICENSE-MIT
├── LICENSE-APACHE
├── README.md
├── CHANGELOG.md
├── CLAUDE.md                          # this file
├── tayf-tasarim.md                    # master design (Turkish)
├── .github/
│   └── workflows/
│       └── ci.yml                     # fmt + clippy + test + audit + deny
├── docs/
│   └── superpowers/
│       ├── specs/
│       │   ├── 2026-05-21-tayf-v0.1-design.md
│       │   └── 2026-05-21-tayf-v0.2.0-design.md
│       ├── plans/
│       │   ├── 2026-05-21-tayf-v0.1.md
│       │   └── 2026-05-21-tayf-v0.1.1-cleanup.md
│       └── reviews/
│           └── 2026-05-21-rust-senior-architecture-review.md
├── src/
│   ├── main.rs                        # CLI entry, ExitCode mapping
│   ├── lib.rs                         # Tayf::run facade
│   ├── cli.rs                         # clap derive Args
│   ├── error.rs                       # tayf::Error enum (thiserror)
│   ├── shell.rs                       # ShellSpec discovery
│   ├── pty.rs                         # PtySession + into_parts decomposition
│   ├── tty_guard.rs                   # RAII raw mode + panic hook
│   ├── signals.rs                     # signal_hook thread
│   ├── runtime.rs                     # two-thread I/O loop + shutdown
│   ├── pipeline.rs                    # TUI mode SM + line buffer + apply_rules
│   ├── line_buffer.rs                 # UTF-8 safe accumulator
│   ├── rules.rs                       # Compiled struct + builtin patterns + filename catalog
│   ├── style.rs                       # Color + Style + SGR audit gate
│   ├── terminfo.rs                    # TTY detection + winsize helper + color depth
│   ├── logging.rs                     # tracing init from TAYF_LOG
│   └── version.rs                     # build-time SHA + rustc info
├── benches/
│   ├── throughput.rs                  # criterion throughput benches
│   └── BASELINE.md                    # recorded baseline numbers
└── tests/
    ├── integration_smoke.rs           # spawn shell, send command, assert exit
    └── common/
        └── mod.rs                     # shared test helpers
```

Each file MUST have a module-level doc-comment explaining its purpose, public API, and invariants.

---

## Working Conventions for Claude

- **Always read `tayf-tasarim.md` decisions before designing new modules.** Section references in code are encouraged (e.g., `// See tayf-tasarim.md §6.5 — Drop guard requirement`).
- **Use the `superpowers:writing-plans`, `superpowers:test-driven-development`, `superpowers:systematic-debugging`, and `superpowers:security-review` skills** at the appropriate points. They are not optional.
- **`cargo fmt && cargo clippy -- -D warnings && cargo test`** is the local pre-commit check. Run it before claiming work is complete.
- **Commits are small and atomic.** One logical change per commit. Title in imperative present tense, under 70 chars. Body explains *why*, not *what*.
- **PR descriptions reference the design doc section(s) and the spec.**
- **Do not introduce dependencies casually.** New crates require a one-line justification in the commit message and an audit note (maintainer activity, license, `unsafe` lines).
