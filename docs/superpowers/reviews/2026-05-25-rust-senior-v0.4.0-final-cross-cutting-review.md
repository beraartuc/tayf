# Final cross-cutting review: tayf v0.4.0

**Tag:** v0.4.0 (`c0eb945`)
**Reviewer:** Senior Rust (opus 4.7, post-ship retrospective)
**Date:** 2026-05-25

## Verdict

**CARRYOVER_FINDINGS.** v0.4.0 ships cleanly — the three orthogonal pieces
of work (RegexSet pre-filter, Pipeline-owned scratch, ZeroForbidden
delegation) compose without latent correctness bugs, the zero-regression
invariant on `tests/integration_capture_groups.rs` holds (0-line diff vs
v0.3.7, 9/9 tests green), `cargo fmt`/`clippy -D warnings`/`cargo test
--lib` all pass on 410 tests, and the public API surface
(`src/lib.rs`'s top-level pub re-exports, `Error`, `Args`, `Pipeline::new`,
`Compiled` field shape) is byte-stable. The CHANGELOG + BASELINE.md +
spec + test-name surfaces all converge on the same honest "per-line
allocation in `PipelineScratch`'s surface is zero; `RegexSet::matches()`
upstream `PatternSet::new()` is opaque per-call cost" framing — rev2's
disclaimed claim does not leak back in. The captures-heavy +7.93%
regression is documented as a tradeoff in CHANGELOG and BASELINE per
spec §6.3 review-gate disposition.

The single finding the per-task reviews missed is **C-1 below**: the
v0.3.7 final cross-cutting review's explicit recommendation #2
("Fold N-4 — bare `unreachable!()` → `unreachable!("reason")` — into the
same v0.4.0 cleanup commit as I-1") was not executed. I-1 itself
(`CaptureGroupIndexZeroForbidden` delegation) shipped exactly as
specified at `src/rules.rs:931-936`, but the sibling `RuleSource::Builtin
=> unreachable!()` arms at lines 964 and 993 are still bare. This is a
non-shipping defect — `RuleSource::Builtin` is constructor-guaranteed
unreachable on these paths, the panic message is informational not
recoverable — but it violates CLAUDE.md §2 ("`unreachable!("reason")`")
and was an explicit carryover from the prior cross-cutting review.

## Findings

### 🔴 Critical (address before next sub-version)

none.

### 🟡 Important (carryover to v0.4.1's scope)

**C-1. v0.3.7 final review's recommendation #2 (N-4 carryover) was
silently dropped.** The v0.3.7 final cross-cutting review (`docs/
superpowers/reviews/2026-05-24-rust-senior-v0.3.7-final-cross-cutting-
review.md:239-240`) explicitly recommended:

> 2. **Fold N-4** (bare `unreachable!()` → `unreachable!("reason")` in
>    three places in `src/rules.rs`) into the same v0.4.0 cleanup commit.

v0.4.0 commit `37aa7a0` ("refactor(rules): delegate ZeroForbidden
UserConfig arm to Display") executed recommendation #1 (the I-1
delegation) faithfully at `src/rules.rs:931-936`, but did **not** touch
the sibling `RuleSource::Builtin => unreachable!(),` arms at
`src/rules.rs:964` and `src/rules.rs:993`. The third site at
`src/rules.rs:938-942` already carried the multi-line explanation; the
two bare arms remain asymmetric:

```rust
// src/rules.rs:964
RuleSource::Builtin => unreachable!(),
// src/rules.rs:993
RuleSource::Builtin => unreachable!(),
```

vs. the sibling at line 938-942:

```rust
RuleSource::Builtin => unreachable!(
    "Builtin rules ship with styles_override == None; reached the \
     map iteration only for UserConfig/Theme. styles_override on \
     a Builtin would be a constructor bug."
),
```

The `RuleSource::Builtin` arm is constructor-guaranteed unreachable on
all three paths (built-ins ship with `styles_override == None`, so the
map-iteration body never executes for them), so this is **not a shipping
defect**. But the asymmetry violates CLAUDE.md §2's `unreachable!("reason")`
rule, and the carryover discipline mattered enough that the prior
cross-cutting review called it out by recommendation number. Two of the
three review-recommended cleanups were folded into v0.4.0; the third
slipped silently.

**Disposition recommendation:** Fold into v0.4.1 cleanup scope (the
bench-CI + regression detection sub-version per umbrella §2 table). One
mechanical edit, ~3 lines, the same message text copied from line
938-942 with minor wording adaptation per arm. NOT a v0.4.0.1 hotfix
trigger — no user-visible behavior changes, and creating a hotfix
release for `unreachable!()` message strings would itself violate the
"no busywork releases" spirit codified at the end of the v0.3.7 review.

### 🔵 Nits / observations

**N-1. The `Compiled.set` doc-comment refresh is small but
load-bearing.** `src/rules.rs:538-547` now reads:

> `set` is the equivalent `RegexSet` consumed by [`crate::pipeline::
> apply_rules`] as a per-line pre-filter; `RegexSet::matches(line).iter()`
> yields hit indices in pattern-definition order (regex 1.12 stable
> contract), and downstream dispatch reads only those indices.

This is the canonical place a future contributor will read to understand
why `set` exists and what guarantees it carries. Naming the regex 1.12
stable contract explicitly (rather than the looser "yields indices in
some order") is the right call — the first-match-wins overlap kontract
depends on it, and `apply_rules_preserves_pattern_definition_order_for_
overlapping_matches` (the third new regression-guard test) makes the
dependency executable. Good.

**N-2. Allocation honesty is consistent across all four surfaces.** The
test name (`pipeline_scratch_capacity_preserved_across_apply_rules_
calls`), the test doc-comment ("NOT a zero-allocation invariant overall
— `regex::bytes::RegexSet::matches` itself allocates a small bitset…"),
the `PipelineScratch` struct doc-comment ("the five Vecs allocate at
most once per process lifetime"), the CHANGELOG entry ("Per-line
allocation in `PipelineScratch`'s surface is zero…
`regex::bytes::RegexSet::matches()` itself internally allocates a small
bitset per call"), and BASELINE.md ("`PatternSet::new(pattern_len)`;
that upstream cost is opaque and unchanged from prior baselines") all
converge on the same honest framing. The disclaimed-in-rev2 "zero
allocation overall" claim does not leak back in anywhere. Calibration
check passes.

**N-3. The captures-heavy +7.93% regression framing.** Both CHANGELOG
(lines 21-34) and BASELINE.md (lines 484-491) frame this as a tradeoff
intrinsic to a uniform RegexSet pre-filter, name the precise hit-ratio
(4.12/13 patterns per line on the fixture), name the precise cost/
savings numbers (~0.50 µs/line pre-filter, ~0.14 µs/line skipped
patterns), and explicitly invoke spec §6.3 as the review-gate authority
that authorized the ship-with-tradeoff disposition. This is exactly the
"error messages are user-facing UX" extension to release notes that
CLAUDE.md §4 implies. The geomean framing (~5.5% faster across the
three `apply_rules/*` rows, dominated by mixed-syslog) is included
without being weaponized as a hedge. Good.

**N-4. The Style by-value migration's 8 `std::ptr::eq` → `assert_eq!`
test sites all hold.** Spot-checked the inner-wins-three-runs case
(`src/pipeline.rs:759-783` in the diff): the previous test asserted
pointer identity between `runs[0].2` and `group_styles[0].as_ref().
unwrap()`, which would have failed once `runs` carried `Style` by value
(no shared storage to point at). The new `assert_eq!(runs[0].2,
group_styles[0].unwrap())` is the semantically correct assertion (Style:
PartialEq + Copy) and is strictly stronger as a test: it asserts
structural equality, not just storage identity. None of the eight
migrations lost a semantic check.

**N-5. The bench shim contract held.** `benches/throughput.rs:55, 84,
112` all hoist `BenchScratch::default()` outside the `b.iter` block in
the three `bench_apply_rules_*` functions, and `bench_passthrough` is
unchanged. Per-iter work inside each `b.iter` is exactly
`Cursor::new(Vec::with_capacity(...))` + `apply_rules(...)` + `black_
box(out)`, matching the v0.3.5 shape modulo the new `&mut scratch`
argument. v0.4.0 BASELINE numbers measure the scanner, not the
allocator, as spec §3.2 Bölüm B (b) mandated.

**N-6. The spec §10 explicit divergence from umbrella §3.2 Bölüm B is
correctly resolved per umbrella §7.** Umbrella preferred (b) Pipeline
method shape and (ii) `rule_idx` encoding for runs entry; spec rev2
chose (b) free-fn + `&mut PipelineScratch` (disjoint field borrows
work, destructure boilerplate avoided) and (ii) `Style` by-value
(branch-elimination in emit hot loop > 8-byte cache cost). The
implementation in `src/pipeline.rs:48-101` matches the spec, not the
umbrella. Umbrella §7's "spec wins on divergence" clause applied
correctly.

**N-7. Commit ordering held the rev2 spirit.** Spec §9 codified 8
pre-tag commits + atomic bundle. Actual shipped: 8 pre-tag commits +
the `d94b03b` style-cleanup follow-up (Turkish-phrasing translation in
`src/pipeline.rs:65-67` comment, promoted from a Task 3 reviewer 🟢
nice-to-have per CLAUDE.md §1 "catch and fix on sight"). This adds one
commit but does not break the bisect-friendly claim — each commit
remains independently `cargo test`-passing and the production behavior
delta is localized to commits `37aa7a0` (ZeroForbidden), `28e3784`
(scratch refactor), `43d20ee` (RegexSet wiring). A `git bisect` for
"when did `apply_rules` start using RegexSet" lands on `43d20ee` deterministically.

**N-8. The pattern-order regression-guard test
(`apply_rules_preserves_pattern_definition_order_for_overlapping_
matches`) constructs a `Compiled` directly with two synthetic rules.**
This is the only test in the bundle that bypasses `Compiled::
load_builtins()` / `Compiled::load_with_theme()`. It's a load-bearing
guard against a hypothetical future change that swaps `RegexSet::
matches()` consumption from `.iter()` (definition-order) to a
HashSet-mediated iteration (insertion-order) — the kind of "obvious
cleanup" that would silently break first-match-wins. The test's `red`
(SGR 31) / `blue` (SGR 34) assertion with negative regression guard
(`!s.contains("\x1b[34m")`) is exactly the assertion specificity the
user's memory entry on test-assertion specificity calls for.

## What v0.4.0 did right

- **Three orthogonal pieces of work in three atomic commits.** Each of
  `37aa7a0` (ZeroForbidden delegation), `28e3784` (PipelineScratch +
  Style by-value + bench shim), `43d20ee` (RegexSet wiring) is
  independently revertible and independently testable. A v0.4.0.1
  hotfix targeting any one of these three would be a one-revert
  operation.
- **The atomic-bundle scope discipline held under pressure.** Task 2
  plan listed 3 files; the implementer correctly added a 4th
  (`src/rules.rs` `#[cfg(test)]` callsites) to the same commit rather
  than splitting — splitting would have broken the pre-commit gate
  (cargo test failures between commits). The Task 2 reviewer's
  confirmation that all 4th-file additions are `#[cfg(test)]`-gated
  was the right scope check.
- **Style by-value carries through `emit_capture_runs` cleanly.** The
  function lost its `'r` lifetime, its `default_style` and
  `group_styles[...]` reads no longer need `.as_ref()`, and the
  `Vec<(usize, usize, Style)>` output Vec sorted by start position
  works without any closure-pattern adjustment beyond the diff'd
  `&(s, _, _)` pattern.
- **Public API surface is byte-stable.** `git diff v0.3.7..main --
  src/error.rs src/cli.rs src/shell.rs src/runtime.rs src/signals.rs
  src/tty_guard.rs` returns empty. `Pipeline::new` signature unchanged.
  `Compiled` field shape unchanged (only the `#[allow(dead_code)]`
  attribute on `.set` dropped). The `__bench__` module signature
  change is acknowledged as "not part of the public API" by its
  doc-comment and by `BenchScratch`'s own doc-comment.
- **Zero-regression invariant mechanically proven.** `git diff
  v0.3.7..main -- tests/integration_capture_groups.rs | wc -l` returns
  0, exactly as spec §5.1 promised. The 9-test capture-group
  integration suite passes against v0.4.0 code without a single line
  change — this is the strongest possible proof that the RegexSet
  pre-filter + Style by-value refactor preserves capture-group styling
  output byte-for-byte.
- **Three new regression-guard tests + one new spec-compliance test
  land alongside the feature, not after.** Lib test count 406 → 410,
  not via a separate "test-only" commit but bundled into the work
  commits per CLAUDE.md §4 "tests with the feature, not after". The
  capacity-preserved test (PipelineScratch contract), the no-set-hits
  byte-identical test (RegexSet pre-filter correctness), the
  pattern-order overlap test (first-match-wins invariant), and the
  ZeroForbidden delegation test (Display-impl coverage) each guard a
  distinct invariant the diff established.
- **Captures-heavy regression handled with rigor, not denial.** The
  per-group floor breach was investigated, root-caused via profile
  data, hypothesis verified, dispositioned through spec §6.3's review
  gate, documented in CHANGELOG with explicit user guidance ("Users
  running workloads dominated by capture-styled rules firing on every
  line may want to evaluate the tradeoff against their input"), and
  documented in BASELINE.md with the human-judgment-call rationale.
  This is exactly the disposition discipline the user's memory entry
  on cross-cutting review value implies — a per-task reviewer cannot
  see "is this regression acceptable for the release?", but the
  cross-cutting review + release ceremony can.
- **The release ceremony split codified in the v0.3.7 final review
  was followed.** v0.4.0 umbrella vision row was updated pre-tag
  ("ship in progress / unmarked") and the ✅ shipped marker landed
  post-tag in commit `80e9289`. v0.3.7 review recommendation #3
  ("codify the v0.3.7 release ceremony split as the standard pattern")
  was honored.

## Process observations

**Commit sequence (oldest first):**

1. `f089cf5` spec
2. `28f9d4c` spec rev2 (senior-review applied) + senior review doc
3. `7b0d2af` plan
4. `37aa7a0` Task 3 fix (ZeroForbidden delegation)
5. `28e3784` Task 1 refactor (PipelineScratch + Style by-value + bench shim)
6. `43d20ee` Task 2 perf (RegexSet wiring)
7. `d94b03b` Turkish-phrasing translation follow-up
8. `70f16af` Task 4 regression-guard tests
9. `9c985a9` Task 6 BASELINE.md numbers
10. `5c44b89` CHANGELOG entry (date `TBD`)
11. `c0eb945` version bump → tag `v0.4.0`
12. `5ed536d` CHANGELOG release date (**post-tag**)
13. `80e9289` vision shipped marker (**post-tag**)

**Two notable things this sequence got right.** First, the
implementation order (Task 3 → Task 1 → Task 2 → Task 4) ordered the
smallest, most-independent piece (ZeroForbidden delegation) first so
the bigger refactor commits had a clean base. Second, the umbrella
vision row was *already* anticipating v0.4.0 before this work began
(commit `1401464` during v0.3.7 cycle), so the standard pre-tag
"insert row as in-flight" step was a no-op for v0.4.0 — the row simply
flipped from "planned" to "✅ shipped" post-tag. This is the right
behavior when the umbrella is forward-looking enough to already carry
the row.

**One thing worth tightening for v0.4.1.** The Task 3 implementer
copied Turkish phrasing into a code comment that should have been
English per CLAUDE.md §1. The Task 3 reviewer flagged it as 🟢
nice-to-have rather than 🟡 important; this is reviewer-calibration
drift that should be tightened. CLAUDE.md §1's "catch and fix on
sight" applies whether the wrong-language text is in a doc-comment,
inline comment, or test message — all are read by future contributors.
A 🟡 flag from the per-task reviewer would have prevented the need for
the `d94b03b` cleanup commit. Recommend updating reviewer calibration:
**any English-vs-Turkish mismatch in code (not specs/conversation) is 🟡 or higher**.

## v0.3.7 carryover retrospective

v0.3.7 final review made three explicit recommendations for the v0.4.0
cycle:

1. **Fold I-1** (`CaptureGroupIndexZeroForbidden` user-config arm
   delegation) into v0.4.0 cleanup. ✅ **Done** — commit `37aa7a0`,
   `src/rules.rs:930-937`, exactly as the prior review's suggested
   patch.
2. **Fold N-4** (three bare `unreachable!()` → `unreachable!("reason")`
   in `src/rules.rs`). ❌ **Not done** — see C-1 above.
3. **Codify the release ceremony split** (pre-tag vision insert / post-
   tag shipped marker). ✅ **Done** — v0.4.0 followed the v0.3.7 pattern.

2/3 carryovers executed, 1 silently dropped. This is the kind of finding
only a cross-cutting review with explicit prior-review reference can
catch — no single per-task reviewer in the v0.4.0 cycle was
re-reading the v0.3.7 final review to track its open items, because
that's not their scope. Recommend the v0.4.1 spec phase begin with an
explicit read-out of the v0.4.0 final review's "Important" findings, so
the same drop pattern doesn't repeat at v0.4.1 → v0.4.2.

## Memory recommendations

The controller should write **one new memory entry**.

**ENTRY 1 (new): "Track prior cross-cutting review carryovers
explicitly in the next sub-version spec."** Pattern: every cross-cutting
review's "Important" findings + "Recommendations for vN.N+1 cycle"
section is an open-item list that the next sub-version's spec must
explicitly acknowledge — either by folding each item into scope, or by
explicitly deferring with a reason. Concrete prompt for future
sessions:

> When writing a new sub-version spec, the spec author should open the
> immediately prior cross-cutting review document, enumerate every 🟡
> finding and every numbered "Recommendations" item, and either (a)
> fold it into the spec's scope with a section reference, or (b)
> defer it with one-sentence rationale. v0.3.7 → v0.4.0 dropped one
> of three carryovers silently (the `unreachable!("reason")` cleanup
> from v0.3.7 N-4) because no spec section enumerated the prior
> review's recommendations. v0.4.0 → v0.4.1 should not repeat this.

This complements the existing memory entry on cross-cutting review
value (`feedback_cross_cutting_review_value`) — that entry says
"never skip the cross-cutting review"; this new entry says "also
consume the previous one's output".

## Recommendations for v0.4.1 cycle

1. **Fold C-1** (the two bare `unreachable!()` arms at
   `src/rules.rs:964` and `src/rules.rs:993` → `unreachable!("reason")`)
   into v0.4.1 cleanup scope. Three sub-versions of carryover is one
   too many; v0.4.1 closes it cleanly.
2. **Open the v0.4.1 spec by reading
   `docs/superpowers/reviews/2026-05-25-rust-senior-v0.4.0-final-cross-
   cutting-review.md` (this document)** and enumerating every 🟡 + every
   "Recommendations" item. Carryover items should be folded or
   explicitly deferred, never dropped.
3. **Update reviewer calibration: any English-vs-Turkish mismatch in
   code (not spec/conversation) is 🟡, not 🟢.** v0.4.0's Task 3
   reviewer flagged Turkish phrasing in a code comment as 🟢; this
   forced a follow-up commit. CLAUDE.md §1 implies 🟡 minimum.
4. **The v0.4.1 bench-CI work should treat the captures-heavy +7.93%
   number as an established v0.4.0 baseline**, not as a regression to
   fix. The v0.4.0 disposition (ship + document tradeoff) is final;
   v0.4.1's CI threshold should be measured against v0.4.0 numbers
   (including the +7.93%), not against an aspirational v0.3.5 floor
   that would constantly fire on the regression.
5. **Do not introduce a v0.4.0.1 hotfix.** C-1 is a non-shipping
   defect (constructor-guaranteed unreachable, informational panic
   message). Creating a hotfix for `unreachable!()` message strings
   would burn a release cycle for zero user benefit and violate the
   "no busywork releases" discipline codified at the end of the
   v0.3.7 review.

---

**End of review.** Tag `v0.4.0` is a clean ship with one carryover
finding (C-1, dropped from the v0.3.7 → v0.4.0 carryover list). All
three pieces of substantive work compose cleanly without latent
correctness bugs; the captures-heavy regression is dispositioned with
rigor; the zero-regression invariant on capture-group styling holds
mechanically; public API surface is byte-stable; the release ceremony
split (pre-tag insert / post-tag shipped marker) was followed. v0.4.1
should fold C-1 and adopt the "consume the previous cross-cutting
review's open items" discipline so the same drop pattern doesn't
repeat.
