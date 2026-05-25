# Final cross-cutting review: tayf v0.5.0

**Tag:** `v0.5.0` at SHA `2267fc2` (annotated, pushed 2026-05-25).
**Reviewer:** Rust + CI/DevOps senior (opus 4.7, cold).
**Date:** 2026-05-25.
**Scope:** 10-commit chain `v0.4.1..v0.5.0` + 3 post-tag housekeeping commits (`f5a5902`, `2f84a3c`, `d94be78`). Spec: `docs/superpowers/specs/2026-05-25-tayf-v0.5.0-gha-bump-and-capture-group-naming.md`. CI run 26408704910 — 5/5 jobs green.

---

## Verdict

**CARRYOVER_FINDINGS.** Tag stands. One 🟡 (theme-path Phase-1 grammar gate makes named-key TOML support effectively dead-code on the Theme branch — only exercised through `RuleSource::UserConfig`) folds into v0.5.1. No 🔴.

Two-track scope was executed cleanly: Task 1 = single-line atomic commit (separately revertible per spec §7); Tasks 2–5 each map 1:1 to a planned commit; post-tag ceremony commits correctly held until after the tag. Every spec §8 acceptance criterion is verifiable against the diff. Byte-pin test discipline is markedly tighter than v0.4.1 — five `assert_eq!` byte-exact Display tests plus a dedicated duplicate-formatter cross-path identity test ([src/rules.rs:2601-2667](src/rules.rs#L2601)).

---

## Findings

### 🔴 Critical

None.

### 🟡 Important (carryover to v0.5.1)

**I-1. Theme-path Phase-1 grammar gate rejects all named keys.** `themes::validate_theme_rules` ([src/themes.rs:410-435](src/themes.rs#L410)) enforces the v0.3.5 grammar via `crate::config::validate_styles_map_key`, which returns `None` for any non-all-ASCII-digit key ([src/config.rs:175-189](src/config.rs#L175)). So `"scheme"`, `"perm_owner"`, `"date"` are emitted as `CaptureGroupKeyMalformed` **before** the load reaches `resolve_group_styles_for_rule`'s Step-3 named-resolution path ([src/rules.rs:1054-1095](src/rules.rs#L1054)). Net effect: the `RuleSource::Theme` match arms at lines 1067-1076 and 1116-1124 are unreachable through the production theme-load pipeline.

Why the test suite missed it: `duplicate_formatter_theme_and_user_paths_byte_identical_diagnostic` ([src/rules.rs:2601](src/rules.rs#L2601)) calls `resolve_group_styles_for_rule` **directly** with a pre-built `BuiltinRule`, bypassing `validate_theme_rules`. The 3 new integration tests at [tests/integration_capture_groups.rs:431-518](tests/integration_capture_groups.rs#L431) all exercise `[[rules]]` blocks in user-config TOML, not theme TOML. Spec §5.4 was correctly labelled "user-config"; theme-TOML coverage was not promised.

Remediation for v0.5.1: relax Phase-1 to accept the same grammar as the dispatch loop (literal `"0"` → ZeroForbidden; all-digit → `validate_styles_map_key`; non-digit → defer to Phase-2 in `Compiled::load_with_theme` where the compiled regex is in hand). Add a theme-TOML integration test with `styles.scheme = {...}` symmetric to the user-config one. **🟡 not 🔴** because v0.5.0 user-config named-key support ships fully functional and `assets/themes/{dark,light}.toml` use no `styles` maps — end-user impact is limited to advanced users authoring custom theme TOML.

**I-2. Defense-in-depth `unreachable!` arms.** [src/rules.rs:1109-1113](src/rules.rs#L1109): the `(true, true)` / `(false, false)` arms of the positional/named match are `unreachable!`'d. Reasoning is sound (TOML rejects duplicate keys; regex crate rejects duplicate named groups). Calling this out only because the I-1 fix should add an end-to-end duplicate-target regression test through `validate_theme_rules` + `Compiled::load_with_theme`, not via direct dispatch.

### 🔵 Nits + closing-loop observations

**N-1. Spec §10 disposes of all 5 v0.4.1 Recommendations.**
- Rec #1 (upload-artifact bump): FOLD_WITH_CORRECTION → `@v7` (commit `61fa8b3`); v0.4.1 review §N-1 retroactively corrected (`2f84a3c`). The v5→v7 discovery at spec phase (v5 still Node 20; v6 Node 24 transition; v7.0.1 current) is exactly the kind of late research the `feedback_consume_prior_review` discipline rewards.
- Rec #2 (open spec by reading prior review): FOLD via spec §10 disposition table.
- Rec #3 (CI infra canonical): FOLD as constraint — `git diff v0.4.1..v0.5.0 -- benches/baselines/ | wc -l` → 0; workflow diff is the single `@v4`→`@v7` line.
- Rec #4 (EN/TR 🟡 calibration): FOLD — permission rename `perm_type/perm_owner/perm_group/perm_other` is the first task-level application. Rationale (`type` Rust keyword; `group` triple-overloaded; `user` Unix-incorrect for owner) absorbed pre-implementation via AskUserQuestion. Consistent across [src/rules.rs:354-360](src/rules.rs#L354).
- Rec #5 (per-bench `std_dev_ns` thresholds): DEFER to demand. Fixed thresholds held green.

**N-2. Public API additive only.** `git diff v0.4.1..v0.5.0 -- src/` shows zero `-pub ` lines. Two new variants land on `#[non_exhaustive] ThemeRuleErrorKind` — additive. No signature changes. No MSRV bump. No `Cargo.toml` dep delta.

**N-3. Duplicate-formatter audit: clean.** `grep -n 'format!.*rule .{}' src/rules.rs` → 5 sites ([977, 1010, 1043, 1085, 1134](src/rules.rs#L977)) — all 5 delegate via `format!("rule '{}': {kind}", rule.name)`. No inline reconstruction. The cross-path identity test ([src/rules.rs:2601](src/rules.rs#L2601)) institutionalises this — future variants must pass it by construction.

**N-4. Zero-regression invariant proved byte-by-byte.** `git show v0.5.0:tests/integration_capture_groups.rs | head -421` is byte-identical to v0.4.1. New tests strictly appended (lines 422-518). Spec §5.1 criterion met without ambiguity.

**N-5. Dispatch regression-guarded with negative assertions.** `dispatch_malformed_digit_key_emits_key_malformed_not_name_unknown` ([src/rules.rs:2435-2452](src/rules.rs#L2435)) pins both `"capture-group key must be a positive decimal"` (positive) AND `!message.contains("no capture group named")` (negative). Textbook `feedback_test_assertion_specificity` — broken and fixed states have distinct truth values. `cargo test --lib ... dispatch_malformed_digit_key_emits_key_malformed_not_name_unknown` → pass.

**N-6. Pre-existing test re-pin is the correct call.** `compiled_load_with_theme_sanitizes_malformed_styles_key_in_user_config` ([src/rules.rs:2252-2302](src/rules.rs#L2252)) now asserts the empty-available specialization. Adversarial `"0\x07evil"` routes through Step 3 (non-all-digit); BEL-leak invariant preserved via `sanitize_for_display` on the new Display arm. `validate_theme_rules_collects_capture_group_key_malformed` ([src/themes.rs:1063](src/themes.rs#L1063)) correctly keeps `"01"` (all-digit) unchanged — KeyMalformed regardless of dispatch path. No other tests needed updating.

**N-7. `available` ordering byte-pinned across 3 representative rules.** `available_order_is_positional_left_to_right_for_url`, `..._for_timestamp_skips_capture_free_branches`, `..._for_permission_uses_perm_prefix_names` ([src/rules.rs:2519-2565](src/rules.rs#L2519)) — all three patterns pinned with explicit anti-alphabetical assertion. Rust senior I-2 absorbed end-to-end.

**N-8. `gh run download` flow verified post-bump.** Spec §5.5 C-2 acceptance: `criterion-{ubuntu,macos}-latest.zip` preserved; `gh run download -n criterion-{os}-latest` works; `jq '.mean.point_estimate'` returns clean f64 (ubuntu 2482795.10, macos 2254007.84). v7 zip semantics drop-in. Linux/CI senior C-2 absorbed.

**N-9. CHANGELOG entry pedagogical.** Enumerates named-group inventory per rule, explains why `@v7` (not `@v5`) is the bump target, surfaces dual-key error semantics. A reader has enough to author named-key user-config from CHANGELOG alone.

---

## What v0.5.0 did right

1. **Atomic two-track scope.** CI bump (Task 1, `61fa8b3`) independently revertible from capture-group work. Spec §7 risk table called this out.
2. **Spec-phase parallel review.** Two opus 4.7 reviews (Rust + Linux/CI) ran before implementation. Surfaced the v5→v7 correction (C-1) that single-reviewer iteration would have shipped wrong.
3. **Byte-pin discipline extended to negative regression guards.** Loose-contains() failure mode (memory `feedback_test_assertion_specificity`) closed at the test level, not the convention level.
4. **Compile-time name→index resolution.** Zero runtime delta verified by clean bench-CI run on both OS. Captures-heavy bench did not regress further from v0.4.0's accepted +7.93%.
5. **Duplicate-formatter audit institutionalised.** Cross-path identity test is now a permanent forcing function, not a one-shot memory pattern.

---

## Process observations

10-commit chain ordering is textbook: spec → plan → CI bump (atomic) → patterns (data-only) → dispatch (consumes patterns) → variants (consumed by dispatch) → tests → CHANGELOG → version → tag. Each commit references task numbers from spec/plan.

What got tightened from v0.4.1:
- **Pre-implementation parallel review** documented as a spec-phase step (v0.4.1 did this implicitly).
- **Disposition table in spec §10** is now the standard fold/defer mechanism. `feedback_consume_prior_review` discipline structurally enforced.
- **Wrong forward-pointer correction precedent.** Frozen review artifact got ONE retroactive correction commit (`2f84a3c`), signed and scoped. Append-only with bounded exceptions — the right pattern.

---

## v0.4.1 carryover retrospective — 5 Recs

| Rec | Disposition | Status |
|---|---|---|
| #1 upload-artifact bump | FOLD_WITH_CORRECTION → `@v7` | ✅ shipped |
| #2 open spec by reading prior review | FOLD via §10 table | ✅ shipped |
| #3 CI infra canonical | FOLD as constraint | ✅ shipped |
| #4 EN/TR 🟡 calibration | FOLD via `perm_*` rename | ✅ shipped |
| #5 per-bench dynamic thresholds | DEFER to demand | ⏸ deferred |

4/5 folded, 1/5 deferred. No silent omissions.

---

## Memory recommendations

1. **`feedback_phase1_grammar_gate_blind_spot.md`** — when a feature extends grammar (numeric→named keys), audit ALL grammar gates in the load pipeline, not just the dispatch site. Pattern: `themes::validate_theme_rules` predates v0.3.5 dispatch by 4 sub-versions; spec §5.4 tests landed entirely on UserConfig because Theme was upstream-gated by older grammar. Fix-forward: `grep -rn 'validate_styles_map_key' src/` when extending grammar.
2. **`feedback_spec_phase_parallel_review.md`** — two opus 4.7 reviewers (Rust + domain-specific) in parallel during spec phase catches wrong forward-pointers (v5→v7). Cost: one agent. Value: avoids wrong-target hotfix.
3. Update `project_v0_5_0_shipped.md` (already planned per spec §9) — add I-1 as v0.5.1 forcing function.

---

## Recommendations for v0.5.1 cycle

1. **🟡 I-1 (theme Phase-1 gate). FOLD as Task 1.** Relax `validate_theme_rules` Phase-1; add theme-TOML integration test with `styles.scheme = {...}`.
2. **🟡 I-2 (defense-in-depth coverage). FOLD as part of I-1.** End-to-end duplicate-target test through theme load pipeline, not direct dispatch.
3. **Per-bench dynamic thresholds (v0.4.1 Rec #5). DEFER to demand.** No false positives observed.
4. **`@v7.0.1` exact pin vs major-track `@v7`. DEFER to security review.** Spec §11 #1.
5. **Theme TOML coverage matrix. FOLD as v0.5.1 test stratum.** 3 patterns × theme-TOML symmetric to user-config.
6. **`tayf config dump` named-form (v0.5.3 scope). NO ACTION** in v0.5.1; grammar inherited.

---

## Verdict — CARRYOVER_FINDINGS

Tag `v0.5.0` (`2267fc2`) stands. Two interrelated 🟡 (theme Phase-1 gate + dependent E2E coverage), both in `themes.rs` + `tests/integration_capture_groups.rs`, fold into v0.5.1 as Task 1. No 🔴. v0.4.1 carryover queue cleared (4/5 folded, 1/5 deferred to demand). v0.5 minor opens with a tight, interrelated carryover queue.
