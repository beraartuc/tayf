# Rust-senior spec review — tayf v0.8.2 (H4 chunk-level pipeline)

- **Reviewer:** senior Rust engineer (Opus 4.8, 1M), parallel spec-phase review
- **Date:** 2026-05-30
- **Target:** `docs/superpowers/specs/2026-05-30-tayf-v0.8.2-chunk-level-pipeline.md`
- **Method:** adversarial; every cited file:line verified against the real code (the prior cycle's review confabulated 3/5 findings — explicitly guarded against here).

## VERDICT

**Sound to proceed.** The equivalence proof holds, the reuse claim is correct, all cited tokens verify, and the §1 confabulation purge is honest. One IMPORTANT correctness subtlety in the multi-newline/SGR reuse path needs an explicit invariant; the rest are NITs. (Note: the parallel terminal/security reviewer escalated the reuse subtlety + an overflow-framing case to CRITICAL — see that doc; both folded into spec §5.)

## IMPORTANT

### I-1. Multi-newline reuse rests on an UNSTATED `line_has_sgr` batch-ordering invariant — pin it in §5.
The slice-path equivalence depends on `feed_with_overflow` emitting lines in order and `apply_or_passthrough` being called once per line in that order (the SGR dispatch at pipeline.rs:574-576 already does this). §5 named only mid-line SGR, not batch ordering across a multi-newline Data run — the NEW path H4a introduces. `respect_existing_colors = true` (rules.rs:636) makes it load-bearing. **Fix:** add the batch-order invariant + a concrete `data_run_with_leading_sgr_then_multiple_newlines_*` test.

### I-2. §2.1/§5 omit `line_has_string_payload` interaction — a Data run can START with the flag set.
After a mid-line OSC payload, the SM returns to Ground (ansi.rs:385/404) but `line_has_string_payload` stays true until the next `\n`. The chunk path must route that first line through `apply_or_passthrough` unchanged. Works by construction (flag on `self`), but unstated and exactly the cross-path carry a testless spike could break. **Fix:** extend §5's OSC bullet; add a test.

## NIT
- **N-1.** §4 "skipping per-byte step is a no-op" is airtight (verified ansi.rs:172-231, every field) — but state the load-bearing precondition explicitly: `Ground ⟹ sequence_bytes_seen == 0` is an SM invariant (zeroed at every return-to-Ground: ansi.rs:220/264/352/385/404/497/541), not enforced by the spike.
- **N-2.** Dead-code deletion of `feed_byte_with_overflow` is the right call — sole production caller is pipeline.rs:498; all other refs are its own tests + BASELINE.md:656/685 prose (which become stale → fold into the `[0.8.2]` delta). Minor cite drift: the `debug_assert` is at line_buffer.rs:122-125 (not 119-125).
- **N-3.** H2 single-snapshot: both `load_full()` sites verified (pipeline.rs:75 + 607); the split-snapshot Karar-11 hazard is real. But changing `apply_rules`'s signature `&ArcSwap<Compiled>` → `&Compiled` ripples to ~12+ `rule_tests` call sites + `apply_rules_spans`/`select_runs_named`. Cheaper alternative: keep the signature, make `apply_or_passthrough` the single loader and inline `select_runs` + emit loop. Pick one at checkpoint.
- **N-4.** Spike methodology sound — `BenchPipeline::feed` forwards verbatim to `Pipeline::feed` (lib.rs:1118-1119), so the spike IS measured; `with_builtins()` is outside `b.iter` (pipeline_feed.rs:36, confirming §1's N-3 rebuttal). Confounders to name: Vec sink excludes `write_all` (TUI batch-write gain only shows in e2e); `log` is apply_rules-dominated, so read the H4a ceiling off `prose`/`ansi`.
- **N-5.** `786 lib` test count is EXACT (`cargo test --lib`).
- **N-6.** §6's `BASELINE.md:493` is mis-cited — that line is a valid English data row (`apply_rules/ipv4-heavy: -3.57%`), not a leak. Drop it.

## Claims verified CORRECT
Ground arm zero-mutation no-op (ansi.rs:231); `0x1b` diverts to Escape (ansi.rs:177-196); cap dead in Ground (ansi.rs:203 + counter always 0); `feed_with_overflow` keeps `\n` (line_buffer.rs:73-76) vs `feed_byte_with_overflow` strips it (126-128); SGR/OtherCsi already use the slice path (pipeline.rs:568-590); `feed_byte_with_overflow` sole prod caller pipeline.rs:498; H2 double-load real (pipeline.rs:75+607); `BenchPipeline::feed` forwards verbatim (lib.rs:1118-1119); all §8-cited test names exist; `respect_existing_colors` defaults true (rules.rs:636); 786 lib tests; `tui_mode_active()` = `flags != 0` (ansi.rs:159-161), orthogonal to state.

**Bottom line:** land I-1 and I-2 as explicit §5 invariants + concrete §8 tests before promoting the spike; note N-3's signature ripple at the checkpoint; fix the N-6 mis-pointer. (All folded into the hardened spec.)
