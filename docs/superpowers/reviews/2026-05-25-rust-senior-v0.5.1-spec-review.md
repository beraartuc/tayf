# v0.5.1 Spec Review — Rust Senior

**Reviewer:** Rust senior (opus 4.7, cold).
**Date:** 2026-05-25.
**Spec:** `docs/superpowers/specs/2026-05-25-tayf-v0.5.1-themes-phase1-gate-fix.md`.
**Carryover source:** `docs/superpowers/reviews/2026-05-25-rust-senior-v0.5.0-final-cross-cutting-review.md` (§I-1 + §I-2).
**Scope:** Rust-language correctness only. Tests/QA reviewer running in parallel.

**Verdict:** SHIP_AS_IS

---

## 🔴 Critical (blocks ship)

None.

## 🟡 Important (fold before implementation)

None.

## 🔵 Nits / observations

**N-1. §4.2 call-site enumeration uses a looser grep than what the invariant requires.** Spec asserts `grep -rn 'validate_styles_map_key' src/` returns "exactly two sites" ([spec line 134](docs/superpowers/specs/2026-05-25-tayf-v0.5.1-themes-phase1-gate-fix.md#L134)). Reproducing locally returns **22 hits** (definition + `validate_styles_map_key_*` unit tests in `src/config.rs:1322-1352` + `src/rules.rs:55` doc reference + the two production call sites). The two production sites are correctly enumerated; the looser grep just picks up tests and doc-comments. The fully-qualified `grep -rn 'crate::config::validate_styles_map_key' src/` (which the user-supplied review charter recommends) returns exactly 2 — `src/rules.rs:996` + `src/themes.rs:421`. Recommend the implementation plan (D2 acceptance criterion §9.2 line 478) use the fully-qualified form so the assertion is mechanically meaningful. Not a correctness issue, but a future-reader trap.

**N-2. §4.3 option-(c) reasoning is sound but underspecifies one branch.** Confirmed: dispatch `src/rules.rs:1001` silently `continue`s on `RuleSource::Theme` + malformed-digit because Phase-1 already collected the error. Option (c) would invalidate that comment AND would force `validate_theme_rules_collects_capture_group_key_malformed` (`src/themes.rs:1063-1083`) to retire (verified: it builds a `[[rules]]` with `"01"`, which would now pass Phase-1 and surface only at `Compiled::load_with_theme` time — outside this unit's reach). Spec §4.3 captures both, but does not call out that retiring the test would also weaken the v0.5.0 dispatch regression guard `dispatch_malformed_digit_key_emits_key_malformed_not_name_unknown` at `src/rules.rs:2435`. Option (b) is the correct choice; the spec just under-credits one additional cost of (c).

**N-3. §4.1 invariant 1 ("empty key") relies on a subtle Rust semantic.** `"".bytes().all(|b| b.is_ascii_digit())` returns **`true`** (vacuous truth — `Iterator::all` on an empty iterator returns `true`). The empty key therefore enters the all-digit branch, hits `validate_styles_map_key("")` → `None` (`src/config.rs:176-178`), and emits `KeyMalformed`. Byte-identical to current behavior. Spec §4.1 #1 asserts this correctly; flagging only because the vacuous-truth dependency is non-obvious and merits a `// reason: Iterator::all is vacuously true on empty key — preserves byte-identical KeyMalformed emission` comment next to the all-digit branch.

**N-4. §5.4 BTreeMap iteration-order assumption verified.** `UserRule.styles: Option<BTreeMap<String, UserStyle>>` (`src/config.rs:95`) — `BTreeMap` iterates in `Ord` order on the key. For `String`, `Ord` is lexicographic byte order; `"1"` (0x31) precedes `"date"` (0x64). Spec §5.4's predicted `(prior_is_positional=true, current_is_positional=false)` collision path with `positional="1"`, `named="date"` is correct. Confirmed against `src/rules.rs:1102-1114`.

## What the spec got right

1. **Carryover disposition table (§3 + §10) is byte-symmetric with v0.5.0's spec pattern.** Every v0.5.0 review finding (I-1, I-2, N-1..N-9, Rec #1..#6) is explicitly disposed. The `feedback_consume_prior_review` mandate is structurally enforced, not aspirational.
2. **Public-API additivity gate verified mechanically.** `git diff v0.5.0..HEAD -- src/lib.rs src/error.rs` returns 0 lines. `ThemeRuleErrorKind` is `#[non_exhaustive]` at `src/error.rs:34`. Acceptance criterion §9.2 line 477 ("public API delta empty") will hold by construction since no production source files are touched outside `src/themes.rs:410-435`.
3. **Dispatch flow walked correctly.** `"0"` → ZeroForbidden (Phase-1 short-circuit `src/themes.rs:414`); `"01"` → all-digit branch, `validate_styles_map_key` → `None`, KeyMalformed (Phase-1 same as today); `"5"` → all-digit, `validate_styles_map_key` → `Some(5)`, deferred to dispatch range-check (`src/rules.rs:1023`); `"date"` → non-digit, Phase-1 bypass, named resolution at `src/rules.rs:1061-1064` resolves to slot 1; `"bogus"` → non-digit, Phase-1 bypass, named-resolution `None`, dispatch arm `src/rules.rs:1067-1076` fires `CaptureGroupNameUnknown` with `available_names`. All four shapes land at the predicted diagnostic.
4. **`timestamp` named-group inventory cross-checked.** Spec §5.3 claim `available: date, sep, time, ms, tz` matches the existing pinned test `available_order_for_timestamp_skips_capture_free_branches` at `src/rules.rs:2549`. Byte-pinned wording stays stable.

---

## Verdict — SHIP_AS_IS

All five Rust-language correctness checks pass: option (b) reasoning is structurally sound, five invariants hold against the after-block (including the vacuous-truth edge), production call-sites are exactly two (under the fully-qualified grep), public API is unchanged and additive-locked, dispatch flow is correct for all five key shapes, and BTreeMap iteration order matches the duplicate-target test's predicted path. The four nits are documentation tightenings, not correctness blockers — fold them into the implementation plan's commit messages or pre-D2 spec touch-up if convenient; do not block dispatch.
