# Security review — tayf v0.8.3 (perf-series finale)

- **Gate:** CLAUDE.md §3 ("use the `security-review` skill on the diff before declaring any milestone complete").
- **Date:** 2026-05-31
- **Range:** `main` (`58eea14`) .. `v0.8.3-perf-finale` HEAD (`e082622`). (v0.8.2 tag peels to `71d53e4`; diff vs that is identical in `src/`.)
- **Method:** security-review skill — identify (sub-task) → false-positive filter. Zero candidate findings → no filter pass needed.

## Verdict

**No security vulnerabilities. CLEAN.**

v0.8.3 introduces **zero new attack surface**. The Phase-0 spike-first measurement found both candidate optimizations (H5 no-match fast-lane, H6 apply_rules internals) not worth shipping, so **no hot-path code changed**. Verified independently: `git diff 58eea14..HEAD -- src/ tests/` shows every changed line in `src/` and `tests/` is inside a comment or doc-comment (the Turkish→English `Karar`→`Decision` rename), stripped by `rustc` before code generation. No executable code, identifier, string literal, logic, struct field, function signature, match arm, or trait impl was modified. The remainder of the diff is documentation (spec/plan/review markdown), `CHANGELOG.md`, `benches/BASELINE.md`, and a `Cargo.toml`/`Cargo.lock` version bump (`0.8.2` → `0.8.3`, no new dependencies).

## Threat model traced (CLAUDE.md §3) — all N/A this cycle (no logic change)

- **Escape-sequence injection / output integrity** — no change to any byte-routing, SGR injection, or passthrough path (`src/pipeline.rs` logic byte-identical to v0.8.2).
- **Terminal-state corruption / termios restoration** — `src/tty_guard.rs`, drop guards, panic hooks untouched.
- **Buffer-cap (64 KB) bypass** — `src/line_buffer.rs` untouched.
- **ReDoS / linear-time regex** — no new or modified patterns (`src/rules.rs` change is comment-only).
- **Process spawning / signal forwarding** — `src/shell.rs`, `src/signals.rs`, `src/pty.rs`, `src/runtime.rs` untouched.
- **Path traversal / config** — `src/config.rs` change is comment-only; no change to path canonicalization.
- **Credential / PII exposure** — no change to logging or output paths.

## Disposition

Behavior-identical to v0.8.2 in all executable code; no new attack surface; off-limits I/O boundary (`runtime.rs`/`pty.rs`/`tty_guard.rs`) untouched. **Cleared for release.**
