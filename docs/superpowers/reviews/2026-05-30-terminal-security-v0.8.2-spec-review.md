# Terminal/ANSI + security + perf-methodology spec review — tayf v0.8.2 (H4 chunk-level pipeline)

- **Reviewer:** senior terminal-protocol + security + measurement-methodology reviewer (Opus 4.8, 1M), parallel spec-phase review
- **Date:** 2026-05-30
- **Target:** `docs/superpowers/specs/2026-05-30-tayf-v0.8.2-chunk-level-pipeline.md`
- **Method:** adversarial, trace-driven; every cited file:line verified against real code (prior cycle confabulated 3/5 findings — guarded against).

## VERDICT

**CONDITIONAL-APPROVE pending §5 additions.** The spike-first methodology and §7 disposition are honest, but §5's invariant list was INCOMPLETE in security-critical, byte-fidelity-breaking ways: the mid-chunk TUI-flag flip, the SGR/string-payload mid-run flush ordering, the `\n`-strip rule-input asymmetry, and the overflow-vs-newline framing for >64KB runs. Three CRITICAL gaps + supporting IMPORTANTs must be added to §5 before Phase 2. (All folded into the hardened spec as binding invariants C-1..I-5.)

## CRITICAL

### C-1. Mid-chunk `tui_mode_active()` flip not addressed; fast/slow alternation can misroute the post-toggle Data run.
Per-byte loop re-evaluates `tui_mode_active()` at the top of every iteration (pipeline.rs:453); the flag flips only in `finalize_csi` (ansi.rs:523/532), which returns to Ground (ansi.rs:541). §2.1 decided "TUI active ? verbatim : line buffer" once per run. **Hazard:** a "read the flag once before the loop" optimization would misroute. Trace toggle-OFF `<alt>\e[?1049l<plain>\n` in one chunk: a cached flag sends `<plain>` verbatim, silently bypassing all rules. **Fix:** §5 must require re-reading the flag at the head of every Ground run; never cache across a slow-path segment. Add `chunk_with_alt_screen_toggle_on_then_off_routes_each_segment_correctly`.

### C-2. The §4 "provably equivalent" proof omits the `\n`-strip / re-add asymmetry — the only reason the per-byte path is currently correct.
Per-byte path strips trailing `\n` (line_buffer.rs:126-128) → `apply_rules` sees `"foo"`; then re-adds the raw `\n` (pipeline.rs:505-507). §2.1's naive `feed_with_overflow` keeps the `\n` (line_buffer.rs:73-74 inclusive drain) → `apply_rules` would see `"foo\n"`, a real change to the regex match surface (`$`, `\b`, `.`, negated classes interact with a trailing `\n`). §2.1 framed dropping the re-add as a *benefit* — it is not. **Fix:** the shipping path must strip the trailing `\n` before `apply_or_passthrough` and re-add it (mirroring the per-byte contract); §5 must pin "rule-input newline contract"; §8 must add an anchored/`$`-sensitive test.

### C-3. Fast path must be a pure buffer-feeder; §5 must forbid independent drain/flag-reset.
Trace `abc\e]0;title\x07def\n`: equivalence holds only if the fast path, when consuming a Ground run, does NOT drain the partial buffer and does NOT reset `line_has_sgr`/`line_has_string_payload` except via `apply_or_passthrough` at a real `\n`. A "tidy up" impl that drains at run boundaries (mirroring the slow-path toggle drain at pipeline.rs:561) would split `"def"` out of line context and re-apply rules. **Fix:** §5 states the fast path is a pure buffer-feeder; sole drain/flag-reset sites are the slow-path string/toggle arms (pipeline.rs:513-535, :556) and `apply_or_passthrough`. Add a pipeline-level exact-stdout-byte test.

## IMPORTANT
- **I-1.** `line_has_sgr` multi-line carry (trace `\e[31m AAA\nBBB\n`): correctly designed (per-line reset at pipeline.rs:615) but the §4 one-liner is too weak for a "byte-identical" guarantee. Add `sgr_then_multiline_ground_run_skips_only_first_line` — highest-value new test (most plausible silent regression; cf. feedback-enumerate-tests-for-invariant-claims).
- **I-2.** 64KB cap: per-byte fires at byte MAX+1 (line_buffer.rs:100); slice path accumulates across calls (line 58) — coincide. BUT the per-byte warn site (line_buffer.rs:103) is deleted with `feed_byte_with_overflow`; the Ground-run feeder MUST inspect the returned `Option<Error::BufferOverflow>` and `warn_msg!` (SGR/OtherCsi arms do: pipeline.rs:571/584), else the security overflow log silently disappears. Overflow flush = verbatim, no strip, no rule application (both paths).
- **I-3.** **Outright byte-divergence:** `feed_with_overflow` checks the cap (line 58) BEFORE the `\n`-split (line 73), so a >64KB run with interior newlines flushes as ONE verbatim blob — whereas the per-byte path emitted+rule-applied each interior line first. H4a promotes this latent property onto the hot Data path. **Fix:** drain complete lines before the overflow check, or feed Ground runs in `\n`-bounded segments. Add `overflow_run_with_interior_newlines_preserves_per_line_framing`.
- **I-4.** UTF-8 split safe, but state why: ESC (0x1b) is ASCII, never a UTF-8 lead/continuation, so `memchr(0x1b)` never splits a codepoint; cross-call splits accumulate raw in `LineBuffer.inner` (regex::bytes). Existing `utf8_*_split_*_is_safe` tests already drive the slice `feed` API.
- **I-5.** `ForceStringTerminate` + `SEQUENCE_BYTES_CAP`: correctly slow-path-only (cap increments only in non-Ground arms; Ground keeps it 0). Pin "slow path stays strictly per-byte (no batching of sequence/string bytes)" as a security-load-bearing §5 invariant — both `ForceStringTerminate` arms (pipeline.rs:457-477) preserved incl. re-step.

## NIT
- **N-1.** Deleting `feed_byte_*` tests must MIGRATE their byte-strip + overflow-no-strip assertions to the new chunk-level tests, not vanish (feedback-collision-pin-pattern).
- **N-2.** Micro-bench feeds the whole 1 MiB as one slice (pipeline_feed.rs:40), newline-terminated → best case for H4a; real PTY reads arrive in ≤64 KiB chunks (e2e_overhead.rs:82) with splits. The micro-bench overstates the ceiling; set the N target off the e2e cross-check. §7 "<20% likely unreachable, report relative" is the honest, non-fabricating stance.
- **N-3.** Confirm in §5 that `tick`/`drain`/`flush_partial` are unaffected (a Ground run ending mid-line leaves the partial in `LineBuffer.inner` for `tick`).

## Invariants the spec DOES correctly cover
Ground arm state-mutation-free (ansi.rs:231); ESC always restarts slow path (ansi.rs:177-196), so memchr-on-ESC lands where the SM would leave Ground; slow path stays per-byte incl. both `ForceStringTerminate` arms; `memchr` zero new audit surface (2.8.0 transitive via regex); apply_rules/ReDoS untouched; `runtime.rs`/`pty.rs` DOKUNULMAZ; §1 confabulation purge honest (`long_lines`/`ipv4_dense`/`690.0` = 0 grep hits; shapes are prose/log/ansi; `with_builtins()` outside `b.iter`, pipeline_feed.rs:36).

**Bottom line:** §5 read as a checklist of *areas* but not *binding equalities*. Rewrite it to assert C-1 (re-read TUI flag), C-2 (`\n`-strip contract), C-3 (fast path = pure feeder), I-3 (overflow-vs-newline framing) — each with a single-`feed`-call named test, since the multi-segment-in-one-chunk property is exactly what per-byte tests never exercised and where every silent divergence hides. (All folded into the hardened spec.)
