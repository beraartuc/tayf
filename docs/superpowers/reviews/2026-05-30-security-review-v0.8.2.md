# Security review — tayf v0.8.2 (H4 chunk-level pipeline)

- **Gate:** CLAUDE.md §3 mandatory `security-review` (DOKUNULMAZ `src/pipeline.rs` touched).
- **Date:** 2026-05-30
- **Range:** `main` (v0.8.1 `fc02a43`) .. `v0.8.2-h4-chunk-pipeline` HEAD.
- **Method:** security-review skill — identify → parallel false-positive filter → confidence ≥8 cutoff.

## Verdict

**No security vulnerabilities identified (HIGH/MEDIUM, confidence ≥8: none).** Behavior-preserving byte-routing refactor in memory-safe Rust; no new `unsafe`; `src/runtime.rs` / `src/pty.rs` (termios / raw-mode / signal-forwarding) untouched. The security-review gate is CLEAN.

## Categories traced (against the CLAUDE.md §3 threat model + spec §5 C-1..I-5)

### 1. Escape-sequence injection / output integrity — SAFE
- Fast path (`pipeline.rs` `feed`) fires only when `AnsiSm::is_ground()`; the run is `memchr(0x1b, rest)`-delimited so it never contains ESC, and every byte in it is a non-ESC Ground byte that `AnsiSm::step` classifies as `Data` while mutating no SM field. Skipping per-byte `step` over the run is a true no-op (spec §4). The ESC byte always falls to the unchanged slow path.
- The slow path — TUI-active verbatim branch, OSC/DCS payload drain ordering, **both** `ForceStringTerminate` arms (synthetic ST `\x1b\\` + re-step), and `SEQUENCE_BYTES_CAP` counting — is byte-for-byte identical to v0.8.1. The PR only prepends the Ground-run batch.
- tayf introduces no sequence the input lacked: injected bytes remain `apply_rules_with`'s SGR + matching `Style::reset_sgr()` and the re-emitted raw `\n`. No wide-effect codes; no introduced/dropped/reordered bytes.
- C-1 (TUI flag re-read at each run head, never cached across the slow path) and C-3 (fast path is a pure feeder — no buffer drain / flag reset) preserve mid-chunk alt-screen toggling and OSC byte-ordering. Pinned by `alt_screen_toggle_reroutes_following_data_run`, `data_run_before_osc_preserves_stdout_byte_order`.

### 2. Buffer-cap (64 KB) bypass — SAFE
`LineBuffer::feed_data_run`'s `until_overflow = MAX_BUFFER_BYTES + 1 - inner.len()` arithmetic was traced against the old per-byte `feed_byte_with_overflow`. The invariant `inner.len() <= MAX` holds on every entry, so `until_overflow >= 1`; overflow blobs are exactly `MAX+1` bytes and can never end in `\n`; the simultaneous overflow+newline edge matches the old path; cross-call accumulation crossing the cap matches; the overflow `warn_msg!` fires on every flush (spec I-2, no silent drop). Memory cannot grow past `MAX+1` for any adversarial run shape. (Pure DOS/resource-exhaustion is out of scope per the skill; this is a *bypass* check and it passes.)

### 3. Path traversal / command injection / deserialization / auth / crypto — N/A
Pure in-memory byte routing. No `unsafe` added (grep-confirmed across the three changed files). The H2 single-snapshot change strictly *tightens* reload consistency (one `Compiled` snapshot drives both the skip gate and styling); its source is the config-file `ArcSwap`, not attacker-controlled PTY bytes.

## Empirical confirmation
- `cargo test --lib` = 789 passed / 0 failed, including `feed_output_independent_of_chunk_boundaries` (mixed Data/IP/SGR/OSC fed whole vs byte-by-byte → identical output), the `feed_data_run` oracle + static cap-edge cases, and the I-1/I-3/C-1/C-3 named tests.
- `cargo clippy --all-targets -- -D warnings` clean; `cargo fmt --check` clean.

## Disposition
Behavior-preserving, byte-identity proven, documented invariants preserved, no new attack surface, no `unsafe`, DOKUNULMAZ I/O boundary (runtime/pty) untouched. **Cleared for the final cross-cutting review + release.**
