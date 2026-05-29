# tayf v0.8.0 — Final cross-cutting review

- **Reviewer:** Claude Opus 4.8 (1M context), mandatory final cross-cutting gate
- **Date:** 2026-05-30
- **Range:** `v0.7.1..8fa0ed8` (branch `v0.8.0-e2e-harness`)
- **Cycle:** measurement-first perf cycle (Group A: e2e PTY-vs-`cat` harness; Group B: EN-cleanup). Zero `src/` changes.

---

## Verdict: **SHIP-READY — yes, after one IMPORTANT fix**

The harness is correct and fair, the §7 finding is honest and well-caveated, the EN-cleanup truth-chain is internally consistent, and every gate is green. There are **no CRITICAL** issues.

One **IMPORTANT** issue must be fixed before tagging: the cycle's own stated scope item ("Anglicize the released `[0.7.0]` CHANGELOG Turkish leaks") was executed only partially, and this cycle's `check_karar_mandate → check_decision_mandate` rename orphaned a now-dangling reference **inside that same released `[0.7.0]` block**. Both defects live in `CHANGELOG.md` lines 127/130/132. The fix is three words in a doc file (no code, no behavior, no `src/`), so it does not disturb any other gate. Once applied, ship.

---

## CRITICAL

**None.**

---

## IMPORTANT

### I-1 — Released `[0.7.0]` CHANGELOG block still contains Turkish + a now-stale code cross-ref this cycle created

The EN-cleanup commit `fa4e5ec` Anglicized two Turkish words in the `[0.7.0]` **header paragraph** (`paralel`→`parallel`, `DOKUNULMAZ`→`Off-limits hot-path modules`) — correct and verified. But the **`### Added` section of the same `[0.7.0]` block** still carries Turkish, and the Group-B function rename left a dangling identifier reference there:

`CHANGELOG.md`:
- **L127:** `...declares Measurement mode (PIPELINE for **karar** measurement, RULE for debugging)...` — Turkish leak (`karar`).
- **L130:** `` `check_karar_mandate` machine-enforces... `` — **stale code cross-reference.** This cycle (commit `b06d7c6`) renamed that function to `check_decision_mandate`; `check_karar_mandate` no longer exists anywhere in `*.rs` (verified by grep). The released changelog now points at a non-existent identifier — a defect this cycle *introduced*, not pre-existing debt.
- **L132:** `> 5% FP rate forbids **KALSIN** — TIGHTEN or DEMOTE is required.` — Turkish/decision-token leak (`KALSIN`), and now also semantically stale: the live token is `KEEP`.

**Why this is a real miss, not "leave-history-alone":**
- The spec §8.2 / plan Task B3 explicitly took fixing the released `[0.7.0]` Turkish leaks into scope. The plan's *verification* step greps the `[0.7.0]` block for only `paralel\|DOKUNULMAZ` — too narrow — so it returned a false "clean" signal while `karar`/`KALSIN` sat untouched two paragraphs down. The plan even has a "Scope note" correctly excluding Turkish in *other* ([0.5.x]/[0.6.x]) blocks, but missed that the very block it edited was not clean.
- The `check_karar_mandate` reference is the more concrete defect: it is a code identifier this cycle renamed, leaving a broken reference. Per CLAUDE.md §4 (no stale cross-refs) + the `feedback_stale_dead_code_reason_drift` lesson, a rename must update its references in one commit.

**Recommended fix (three words, `CHANGELOG.md` only):**
- L127 `karar` → `decision`
- L130 `` `check_karar_mandate` `` → `` `check_decision_mandate` ``
- L132 `KALSIN` → `KEEP`

This is the same "sanctioned language-policy fix, substance/date unchanged" rationale the spec already approved for `paralel`/`DOKUNULMAZ`; it simply completes the job on the same block and repairs the rename's collateral. It touches no code and no other gate. **Recommend applying before the version bump + tag.**

(Out-of-scope and correctly left alone: Turkish in `[0.5.x]`/`[0.6.x]` changelog blocks and in `docs/` design docs/specs/plans/reviews — those are historical/Turkish-allowed and the plan's scope note already flags the changelog ones for a future sweep. Not flagged here.)

---

## NIT

### N-1 — Plan text describes a bench-internal `#[cfg(test)]` test layout that the implementation (correctly) abandoned

Spec §7.1/§13 and plan lines 23/178 describe the pure-logic tests living in `benches/e2e_overhead.rs` under `#[cfg(test)] mod tests`, run via `cargo test --bench e2e_overhead`. The implementation instead extracted the math to `benches/common/math.rs` and put the unit tests in `tests/e2e_overhead_math.rs` (commit `09d273f`). This divergence is **correct and an improvement**: a `harness = false` bench's `#[cfg(test)]` block is never compiled under `cargo test`, so the spec's original placement would have produced tests that never actually run. The code documents this rationale clearly in both module doc-comments (`benches/common/math.rs:7-10`, `tests/e2e_overhead_math.rs:4-7`). The plan/spec text is now slightly stale relative to the as-built layout, but these are Turkish-allowed design docs and the divergence was a deliberate, reviewed engineering call. No action required; noted for accuracy.

### N-2 — `ansi-passthrough` template is only 4 lines

`benches/inputs/e2e_ansi.txt` has 4 lines (vs 8 for prose/log). This is harmless — `write_corpus` repeats the template to the ~16 MiB target regardless — but for visual symmetry with the other two shapes it could be padded. Cosmetic only.

---

## Checklist results (9 cross-cutting lenses)

| # | Lens | Result | Evidence |
|---|------|--------|----------|
| 1 | DOKUNULMAZ / zero-`src`; `ci.yml` + `throughput.rs` untouched | **PASS** | `git diff v0.7.1..HEAD -- src/` empty; `-- .github/workflows/ci.yml` empty; `-- benches/throughput.rs` empty (all three verified). |
| 2 | No new dependencies | **PASS** | `Cargo.toml` diff = only a `[[bench]] name="e2e_overhead", harness=false` entry. No `[dependencies]`/`[dev-dependencies]` change. New harness uses existing `portable-pty` + `tempfile`. No `criterion` in new bench/tests (grep: only a doc-comment saying it is *not* criterion). |
| 3 | Measurement correctness & fairness | **PASS** | `timed_run` (`benches/e2e_overhead.rs:55-103`): clock starts at `writer.write_all(stdin)` *after* spawn + 200 ms `STARTUP_GRACE`, stops at EOF — streaming phase only, symmetric. Numerator = `tayf --shell /bin/sh` (env `TAYF_DISABLE_BG_DETECT=1`); denominator = bare `/bin/sh`; **same shell, same `cat <corpus>`, same `exit`, same PTY geometry, same 64 KiB drain-to-EOF + `try_wait`-then-drain loop**. Differ only by the tayf process. tayf's stdout → real PTY master ⇒ colorize path (not bypass). Env asymmetry (`TAYF_DISABLE_BG_DETECT=1` only on tayf) is correct — the bare shell never reads it. No artifact inflating/deflating tayf unfairly; if anything, draining at memory speed is a *pessimistic* bound for tayf (documented). The smoke `bare_shell_elapsed` mirrors `timed_run`'s shape exactly (fixed in `312f6b1`). |
| 4 | BASELINE.md finding integrity | **PASS** | `## v0.8.0` section (`benches/BASELINE.md:509-584`): numbers (prose +793%, log +1461%, ansi +568%; tayf 9.8–19.6 MiB/s vs cat ~130–150) match the table; disposition honest and correctly caveated — (a) memory-speed consumer = pessimistic bound, (b) real terminal gates both sides ⇒ interactive/small-output latency unaffected, (c) even low-match prose ~8× ⇒ I/O loop implicated, not just scanner. Defers optimization to v0.8.1+ with an explicit `security-review` gate for the DOKUNULMAZ I/O loop. The old §7 deferral note (`BASELINE.md:50-53`) is updated to "validated in v0.8.0 … Result: not met on bulk streams." |
| 5 | EN-cleanup truth-chain consistency | **PASS (Group B code surface) — but see I-1 for the `[0.7.0]` block** | Per item, corpus `.txt` `# Decision:` header ↔ `DECISION_*` const agree: C-8/D-7/F-3/F-4 = `KEEP`; C-4/C-9/E-1 = `ACCEPT-DOCUMENTED` (unchanged, EXPECTED_FP 5/6/1). `check_decision_mandate` (renamed) enforces both guards (`>5% ⟹ != "KEEP"` at L135-139; `ACCEPT-DOCUMENTED ⟹ fp>5%` at L143-149). Self-test `check_decision_mandate_rejects_accept_documented_below_threshold` genuinely exercises the panic via `catch_unwind(0,20,"ACCEPT-DOCUMENTED")` ⇒ `is_err` (and the suite passes). **No residual `karar`/`KALSIN`/`Karar` anywhere under `tests/`** (the only `tests/` hits — `integration_bypass.rs` "Karar 14" — are in a file **not touched this cycle**, pre-existing, out of scope). The `[0.8.0]` CHANGELOG block is English. **However**, the released `[0.7.0]` block is NOT fully clean — see I-1. |
| 6 | EN/TR calibration (CLAUDE.md §1) | **PASS for new code; the `[0.7.0]` block fails — see I-1** | All added lines in `benches/`, `tests/`, `Cargo.toml`, and the `[0.8.0]` CHANGELOG block are English. The only Turkish on *changed* lines flagged by scan are: `benches/BASELINE.md:493` (`çoğu kez redundant`, in the **pre-existing v0.4.0** section — NOT an added/changed line this cycle, confirmed) and the `[0.7.0]` residuals in I-1. Specs/plans under `docs/` are Turkish-allowed (not flagged). |
| 7 | CHANGELOG `[0.8.0]` accuracy | **PASS** | Honestly describes: added harness; §7 not-met finding with the pessimistic-bound + interactive-unaffected nuance; EN-cleanup rename; `[0.7.0]` fix; zero `src/`; optimization deferred to v0.8.1+. "Performance" framing will not mislead a public reader (explicitly states the bound is pessimistic and interactive latency is unaffected). No over/under-claim. Correctly marked `Unreleased` (version bump is the next release-prep step, not this review). |
| 8 | Test integrity | **PASS** | 9 math unit tests in `tests/e2e_overhead_math.rs` (incl. `#[should_panic(expected="non-empty")] median_empty_panics`) run under `cargo test` (real integration target): **9 passed**. Smoke test is a real mechanism check — inert-marker (`ZZE2EMARKERZZ`) window-scan, SGR-safe, no perf threshold, plus no-hang + finite-ratio guards: **1 passed** under `TAYF_DISABLE_BG_DETECT=1`. CI runs `cargo test` with `TAYF_DISABLE_BG_DETECT=1` + `RUST_TEST_THREADS=1` (`ci.yml:37,42,50`), which covers both new integration targets cleanly. |
| 9 | Anything a per-task review would miss | **One IMPORTANT (I-1) + two NITs found** | I-1: partial `[0.7.0]` EN-cleanup + stale `check_karar_mandate` cross-ref created by this cycle's rename (the plan's own verification grep was too narrow to catch it). N-1: stale plan/spec test-layout text vs the better as-built layout. N-2: cosmetic ansi template length. No dead code, no masking `#[allow]` (the two `#[allow]`s in the diff — `cast_precision_loss`/`cast_possible_truncation` for display, and the pre-existing audit-corpus `dead_code` ones — are justified with reasons). |

---

## Gate-run results

| Command | Result |
|---------|--------|
| `git diff v0.7.1..HEAD -- src/` | **empty** ✓ |
| `git diff v0.7.1..HEAD -- .github/workflows/ci.yml` | **empty** ✓ |
| `git diff v0.7.1..HEAD -- benches/throughput.rs` | **empty** ✓ |
| `cargo fmt --check` | **clean (exit 0)** ✓ |
| `cargo clippy --all-targets -- -D warnings` | **no warnings** ✓ |
| `cargo test --lib` | **782 passed; 0 failed** ✓ |
| `cargo test --test audit_corpus` | **10 passed; 0 failed** ✓ |
| `cargo test --test e2e_overhead_math` | **9 passed; 0 failed** ✓ |
| `TAYF_DISABLE_BG_DETECT=1 cargo test --test e2e_overhead_smoke` | **1 passed; 0 failed** ✓ |
| `cargo bench --bench e2e_overhead --no-run` | **compiles (release profile)** ✓ |

---

## Bottom line

SHIP-READY. No CRITICAL. Apply the three-word `CHANGELOG.md` fix in **I-1** (Turkish `karar`/`KALSIN` + stale `check_karar_mandate` reference inside the released `[0.7.0]` block) to finish the EN-cleanup the cycle started and to repair the rename's collateral, then proceed with the version bump (`0.7.1`→`0.8.0`) and tag per the release workflow. The two NITs are non-blocking.
