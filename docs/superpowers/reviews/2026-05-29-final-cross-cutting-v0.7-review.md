# tayf v0.7 — Final cross-cutting review (opus 4.7, post-implementation)

**Reviewer:** Opus 4.7 senior, cross-cutting lens.
**Date:** 2026-05-29.
**Base / Head:** `761a57f` (v0.6.3 ship) → `e2ac1cd` (Task 21 cleanup, pre-bump).
**Diff scale:** 46 files, +7132 / -86.
**Spec under test:** `docs/superpowers/specs/2026-05-29-tayf-v0.7-design.md` rev2 + rev2.1 LCS-tie correction (`9f5aaea`).
**Plan under test:** `docs/superpowers/plans/2026-05-29-tayf-v0.7.md` (23 tasks).

---

## 1. Verdict

**🟡 ABSORB CHANGES — small, well-scoped fixes before tag.**

The five v0.7 work-items all landed at spec contract. Pin tests are exact-string where the spec required exact strings, the AoT algorithm exactly matches §3.2 (including the rev2 step-3a order-divergence extension and the Task 8 absent-AoT-key broadening), the LCS-DP fix matches rev2.1 strict-greater-then-Add-on-tie convention, the snapshot helper uses the correct ratatui-0.30 API (`buf.area` field + `buf[(x, y)]` Index), and the corpus harness runs through the production pipeline as A-C-5 demanded.

One CRITICAL finding holds up the tag, and there are five IMPORTANT items that should land in the bump commit or as a v0.7.1 hotfix declared up-front in the v0.7 spec §11 closeout. The corpus karar=TIGHTEN-without-fix shape is **acceptable as a v0.7 baseline** with the disposition this review prescribes (see CRITICAL-1).

---

## 2. CRITICAL findings (must-fix before tag)

### C-1. `MissingIntermediate` collapse into the delete-modify arm is a load-bearing apply-layer change that the spec did NOT authorize.

`src/config_tui/events.rs:811-822` (`build_final_doc`):

```rust
Err(
    crate::config_tui::merge::WriteToPathError::AotElementMissing { .. }
    | crate::config_tui::merge::WriteToPathError::MissingIntermediate { .. },
) if conflict.is_delete_modify() => { ... remove_aot_element_by_name ... }
```

Spec §3.3 explicitly pinned the apply-layer translation to **AotElementMissing only**:

> ```text
> Err(AotElementMissing { .. }) if conflict.is_delete_modify() => { ... }
> Err(e) => return Err(e),  // any other error surfaces to UI
> ```

The implementation widens the arm to also swallow `MissingIntermediate`. That variant is the historical "missing intermediate at <key>" toast that v0.6.2 cross-cutting review I3 fixed against false-firing — collapsing it into a silent `remove_aot_element_by_name` call breaks the I3 contract for any future non-AoT path. The pre-existing `path_exists` skip-guard at line 803-807 covers the common case but not all of them (e.g. `Skip` is the only short-circuit; `Ours`/`Theirs` reaching a non-existent intermediate now silently no-op-removes).

**Required action:** drop `| MissingIntermediate { .. }` from the match arm. Pin test §3.4 #11 (exact-message AotElementMissing) already covers the intended path; add a negative-regression pin that `MissingIntermediate` on a non-AoT path bubbles up. If the implementer believes the wider arm IS necessary for some delete-modify case the spec missed, the spec must be amended FIRST with the failing scenario.

This is the only finding that genuinely blocks tag.

---

## 3. IMPORTANT findings (in-cycle or v0.7.1 hotfix)

### I-1. Three corpus items shipped as `TIGHTEN` with no pattern fix and no tracked issue (spec §13 "exception clause" loophole abused).

Spec §13 A-N-4 fold introduced a narrow escape valve for incomplete TIGHTEN fixes: implementer "may add `// TODO(issue-N): see audit §C-N followup` comment — tracked-issue mandate korunur". The implementation shipped C-4 (5/15 FP, 33%), C-8 (1/10 FP, 10%), and E-1 (1/8 FP, 12.5%) with karar=TIGHTEN and the rationale "Deferred to follow-up commit" in the `tests/audit_corpus.rs` constants — **no GitHub issue, no `// TODO(#N)` annotation, no tracked file**. C-9 (60% FP) is even more egregious: the rationale says "deferred per spec §13 exception clause" but the clause requires a tracked issue.

The spec §11 row notes likewise say "Deferred to follow-up commit" with zero issue numbers. CLAUDE.md §4 is unambiguous: "No 'we'll fix it later' comments unless tied to a tracked issue (`// TODO(#42): ...`). Untracked TODOs fail review." This is the exact pattern CLAUDE.md §4 forbids; the §13 exception was *not* a license to bypass it.

**Verdict on the karar=TIGHTEN-without-fix question (the implementer's call-out concern):**

- Shipping the regression pins as a v0.7 baseline is **acceptable**: the corpus locks in the current FP/FN counts so the next pattern fix attempt sees a clean before/after. The machine-enforced karar threshold (the `check_karar_mandate` test in `tests/audit_corpus.rs:128`) also fires correctly if anyone tries to flip the karar to KALSIN without fixing the pattern, which prevents silent drift.
- Shipping the karar=TIGHTEN labels **without** opening one tracked issue per item is **not acceptable**. Without traceable issues, the "follow-up commit" promise has no enforcement mechanism. v0.7.1 / v0.8 won't know they need to look at these.

**Required action (in-cycle):** open four GitHub issues (or four entries in a `docs/superpowers/specs/post-v0.7-pattern-followups.md` tracker doc), reference the issue number in:
1. The corpus file headers (`# Decision: TIGHTEN (TODO #N)` replacing `TBD`).
2. The `DECISION_C4` / `DECISION_C8` / `DECISION_C9` / `DECISION_E1` const doc-comments in `tests/audit_corpus.rs`.
3. The v0.7 spec §11 karar table rows (replace "Deferred to follow-up commit" with `Issue #N`).
4. The CHANGELOG `[0.7.0]` entry under a new `### Known issues` subsection.

This costs ~30 minutes and closes the CLAUDE.md §4 + spec §13 hole.

### I-2. `# Decision: TBD` in corpus files contradicts the decisions encoded in `tests/audit_corpus.rs`.

All seven corpus files carry `# Decision: TBD` headers (see `tests/audit_corpus/c4_filename_single_letter.txt:6` and friends). The harness already encodes hard decisions in the const tables (`DECISION_C4 = "TIGHTEN"`, etc.) and machine-enforces them in `check_karar_mandate`. Two truths-of-record diverge. A future reader scanning corpus files first will read `TBD` and assume measurement hasn't run — but the assertion is locked.

**Required action:** propagate the decision into the corpus file headers (`# Decision: TIGHTEN` etc.). This is a 7-line one-commit cleanup, no code change.

### I-3. Spec §3.6 internal call-site audit table has stale line numbers.

Spec §3.6 listed `events.rs:1378 / 1414 / 1485 / 1532 / 1585 / 1593` and `merge.rs:455 / 546` as audit targets. The current implementation puts them at `events.rs:1393 / 1429 / 1500 / 1547 / 1600 / 1608 / 2083` and `merge.rs` 817 / 910 / 1068 / 1086 / 1124 / 1150 / 1164. The line numbers drifted by ~15 because the audit-time draft pre-dated the actual implementation. The spec is the contract — drifted line numbers in a fold-or-defer table are exactly the kind of stale-comment drift CLAUDE.md §4 and memory `feedback_stale_dead_code_reason_drift` rail against.

**Required action:** the cross-cutting absorb commit (or the bump commit) refreshes spec §3.6 table line numbers against `HEAD` and confirms the v0.7 fixture intent at each site holds (it does — I spot-checked `events.rs:1393` and `tests/config_tui_conflict_list.rs:155`, both correctly model the identity-fail / order-divergence fallback paths spec §3.6 demanded).

### I-4. `build_diff` 13 tests vs spec's 12 — undercounted +1 in the test budget.

Spec §4.5 enumerated 12 pin tests. The implementation has 13 in `src/config_tui/widgets/save_diff.rs` — counts confirmed by `cargo test`. The thirteenth is `render_modal_save_diff_clean_matches_snapshot` (lines 495-503) which is actually a snapshot test, not a build_diff test, but it lives in the `build_diff` mod. The §9 budget says save_diff gets +9 lib tests; actual delta is +10. Net effect on §9 budget: +1 lib test (782 actual vs 779 spec).

Together with the lib test deltas elsewhere (see Compliments §5.6), the total is **782 lib + 46 integration = 828** vs spec's 825 — 3 over, no surprises, no missing tests. This is **not a defect** but the spec §9 row arithmetic should be refreshed before the bump commit to keep the audit trail honest.

**Required action:** refresh §9 row totals to match measured delta. Trivial spec patch.

### I-5. Spec §11 row description for E-1 leaks "1.2.3.4.5 long" as the FP — actual corpus produces a DIFFERENT shape.

`tests/audit_corpus/e1_semver_vs_ipv4.txt` was not fully inspected, but the §11 row note says "the FP is `1.2.3.4.5 long`". This needs to match. If the corpus's NEG line is `1.2.3.4.5 multi-segment` or similar, the spec row prose is wrong. The const + decision are correct (TIGHTEN at 12.5%), but if a reader debugging an FP regression searches for "1.2.3.4.5 long" in the corpus they won't find it.

**Required action:** spot-verify the actual NEG line in the corpus and update the §11 row note (or the corpus line) to match the other. Pure documentation precision.

---

## 4. NIT findings (defer-OK)

### N-1. `src/rules.rs:1429` test-only `#[allow(clippy::expect_used)]` reason comment is a one-liner without `// reason:` prefix.

Line: `#[allow(clippy::expect_used)] // reason: test-only shim; patterns are pre-validated built-ins`. Reason prefix is present, just on the same line. CLAUDE.md §2.4 says "explicit `#[allow]` requires a one-line `// reason: ...` comment." Format complies. Not actually a finding — flagging because I scanned for this and want the next reviewer to know.

### N-2. The `__test_api_smoke` mod in `src/rules.rs:1021-1043` is named "smoke" but it isn't strictly smoke — it tests positive + negative behavior. Name is mildly misleading. Defer.

### N-3. `MAX_DP_CELLS = 100_000` documented as accommodating 316×316 = 99,856 cells. Math is right; commentary `dp[i][j] <= min(i, j) <= floor(sqrt(n*m))` only holds for the "all-different" worst case, not generic `dp[i][j]`. The bound `dp[i][j] <= min(i, j)` is the correct invariant — the sqrt step adds nothing and is a hand-wave. Defer to v0.7.1 doc polish.

### N-4. `src/config_tui/test_support.rs` uses `is_ok_and` (Rust 1.70+). MSRV per Cargo.toml is `1.74`. Fine, but unnecessary churn over the spec's `.map(|v| v == "true" || v == "1").unwrap_or(false)`. Equivalent semantics. Defer.

---

## 5. Compliments — what landed cleanly

### 5.1. Per-element AoT merge is faithful to spec §3.2

`src/config_tui/merge.rs:254-364` implements the exact algorithm. Identity validation, same-side duplicate guard with debug_assert on ours side, order-divergence guard with both case (a) and case (b), distinct-ordered-name list construction, and per-element delegation — all present, all in the order the spec described. The Task 8 follow-up extension (`at_least_one_aot_no_shape_clash` treating `None` as an empty AoT) is sound: it gracefully handles `[[rules]]` deletion-on-one-side without spuriously falling into the leaf-conflict arm. No v0.6.x test regresses on this (verified by `cargo test --lib`).

### 5.2. LCS-DP rev2.1 strict-greater-then-Add-on-tie is correctly implemented

`src/config_tui/widgets/save_diff.rs:311-350` `trace_back` follows rev2.1 exactly: strict-greater dp neighbour checks before equality short-circuit, Add-on-tie when cells don't match. The duplicate-line case `"a\na\nb"` vs `"a\nb"` correctly removes the SECOND `a` (test #5 passes). The interleaved-changes test #6 pins the exact `- a\n+ x\n  b\n- c\n+ y\n` output. The `#[allow(clippy::if_same_then_else)]` annotation with the reason comment (lines 305-310) correctly explains why the boundary guard and the tied-non-match arm can't be merged.

### 5.3. Snapshot helper uses the correct ratatui-0.30 API

`src/config_tui/test_support.rs:74-87` uses `buf.area` (field, not method) and `buf[(x, y)]` (Index, not `.get()`) — exactly the R-C-1 fold. CI guard works: `assert!(!(update_requested && in_ci), ...)` panics when both env vars are set. `.gitattributes` ships `*.snap text eol=lf`. All 13 snap files exist with visible content (e.g. `modal_help.snap` shows the full keybindings table). LCS-diff dogfood at line 62-71 (failure message uses `build_diff`) is exactly the spec's intent.

### 5.4. Corpus harness production-pipeline measurement is the right A-C-5 implementation

`tests/audit_corpus.rs:101-126` correctly delegates to `tayf::__test_api::pipeline_spans` in PIPELINE mode and `match_named_rule` in RULE mode. `src/rules.rs:1462` `testing_pipeline_spans` uses `Compiled::load_builtins` (when profile is None) or `Compiled::load_with_theme` with the profile rule — matches production startup. `src/pipeline.rs:214` `select_runs_named` correctly runs priority sort + overlap suppression + the capture-group sub-span fan-out — production semantics. The 2 parser self-tests (lines 142-154) are panic-catching and fire correctly.

### 5.5. Stale comment cleanup is a complete sweep

`git grep -nE 'v0\.7\+' src/` = 0 hits. `git grep -nE 'v0\.8\+' src/` = 5 line-hits across 4 sites (matches spec §7.1 #6/#7/#8/#9 — `edit.rs:10` and `edit.rs:11` are one logical block). The Task 11 rename + flip (`conflict_list_array_block_row_surfaces_array_shape_conflict_warning` in `tests/config_tui_conflict_list.rs:145`) cleanly applies the `feedback_collision_pin_pattern` memory: the test asserts BOTH the new wording AND a negative on "v0.7+" to catch a hypothetical regression.

### 5.6. Memory feedback adherence is uniformly good

- `feedback_test_assertion_specificity`: §3.4 #11 exact-string AotElementMissing format (`merge.rs:1190-1193`), §4.5 #4/#5/#6 exact build_diff outputs (`save_diff.rs:422-445`), `assert_eq!` not `contains` for the load-bearing assertions.
- `feedback_collision_pin_pattern`: see 5.5.
- `feedback_stale_dead_code_reason_drift`: applied as the rustdoc-string variant (the spec section §7 was authored anticipating exactly this memory).
- `feedback_consume_prior_review`: spec rev2 §1 enumerated all prior items (v0.6.3 carryover + paralel review items) and fold-or-defer'd each. Nothing silent.
- `feedback_parallel_call_site_invariant_audit`: spec §3.6 enumerated all is_array_block call-sites with v0.7 disposition; implementation matched the audit (modulo the I-3 line-number drift).
- `feedback_dependency_minimalism`: `git diff 761a57f..HEAD -- Cargo.toml Cargo.lock` = empty (zero new deps).
- `feedback_lean_process_small_subversions`: BIG ceremony correctly applied for this 7000-line cycle (parallel spec review, atomic commits per item, final cross-cutting). Process choice was right.

### 5.7. DOKUNULMAZ modules respected

`git diff 761a57f..HEAD -- src/pty.rs src/runtime.rs` = empty. `src/rules.rs` gained only `pub(crate)` shims + `Compiled::names` field + `testing_*` helpers. `src/pipeline.rs` gained only `pub(crate) select_runs_named`. No `unsafe` introduced (existing bg_detect.rs unsafe is pre-existing, properly annotated). Zero new `unwrap()` outside test fixtures. `cargo fmt --check` clean. `cargo clippy -- -D warnings` clean.

### 5.8. CLAUDE.md mandate compliance is exemplary

English in code, Turkish in spec. No unwrap() in library code. No unsafe. No new direct dependencies. Public API surface limited to `__test_api` extension (which is `#[doc(hidden)]` with explicit "no stability guarantees"). CHANGELOG draft in spec §12 ready to copy.

---

## 6. Summary table

| ID | Area | Finding | Severity | Disposition |
|---|---|---|---|---|
| C-1 | events.rs:811-822 | `MissingIntermediate` collapsed into delete-modify arm — unauthorized widening of spec §3.3 contract | 🔴 CRITICAL | **Fix before tag** — drop the `\| MissingIntermediate` from the match arm + add negative-regression test |
| I-1 | tests/audit_corpus.rs + spec §11 | TIGHTEN-no-fix-no-tracked-issue x 4 (C-4, C-8, C-9, E-1) violates CLAUDE.md §4 + spec §13 exception clause | 🟡 IMPORTANT | Open 4 issues (or tracker doc), reference in corpus files + const docs + spec §11 + CHANGELOG `### Known issues` |
| I-2 | tests/audit_corpus/*.txt | Corpus headers all say `# Decision: TBD`; harness encodes hard karar | 🟡 IMPORTANT | One-commit propagation: TBD → final karar per item |
| I-3 | spec §3.6 | Internal call-site audit line numbers drifted ~15 lines from current HEAD | 🟡 IMPORTANT | Refresh table line numbers in absorb commit |
| I-4 | spec §9 | Test budget +1 unaccounted (build_diff mod has 13 tests not 12) | 🟡 IMPORTANT | Refresh §9 row totals |
| I-5 | spec §11 E-1 row | "1.2.3.4.5 long" FP description may not match actual NEG line in corpus | 🟡 IMPORTANT | Spot-verify, align prose with corpus |
| N-1 | src/rules.rs:1429 | Allow-reason format compliance (false positive after re-check) | 🟢 NIT | No action |
| N-2 | src/rules.rs:1021-1043 | `__test_api_smoke` mod name slightly misleading | 🟢 NIT | Defer |
| N-3 | save_diff.rs:266-269 | DP cell-value upper-bound hand-wave with redundant sqrt step | 🟢 NIT | Defer |
| N-4 | test_support.rs | `is_ok_and` instead of spec's `.map(...).unwrap_or` | 🟢 NIT | Defer |

---

## 7. Disposition guidance for the implementer

1. Fix C-1 (drop `| MissingIntermediate` from the arm + negative-regression test). One commit.
2. Land I-1 + I-2 + I-3 + I-4 + I-5 as one absorb commit (`docs(spec)+test(corpus): v0.7 cross-cutting review absorb`). Open 4 tracking issues (or one `docs/superpowers/specs/post-v0.7-pattern-followups.md`) FIRST so the issue references are real before the commit lands. ~30 min.
3. Bump `Cargo.toml` 0.6.3 → 0.7.0 + populate `CHANGELOG.md` `[0.7.0]` block from spec §12 + add `### Known issues` subsection referencing the 4 pattern-followup issues.
4. Tag v0.7.0 after CI green (memory `project_release_workflow`).

Total: 3 commits before tag (C-1 fix, absorb, bump). Ship-ready in under an hour.

---

## 8. Closing note on the karar=TIGHTEN-without-fix question

The implementer's self-flagged concern was the right thing to raise. The shape "ship the regression pin now, defer the actual pattern fix" is **defensible** because:

1. The pins lock the current behavior — any drift fails the test.
2. The machine-enforced karar mandate (`check_karar_mandate`) prevents anyone silently flipping to KALSIN without a real fix.
3. Per-rule pattern fixes are not zero-risk; they cascade through other test-corpora (memory `feedback_consume_prior_review` + the v0.5.6 builtin audit doc). Doing all four in one cycle would balloon scope.

But "defer without a tracked issue" is the exact CLAUDE.md §4 failure mode. The deferral is fine; the tracking is non-negotiable. Once I-1 lands, v0.7 ships clean.

— *opus 4.7 senior, final cross-cutting v0.7 review, 2026-05-29*
