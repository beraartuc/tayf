# Final cross-cutting review: tayf v0.5.2

**Tag:** v0.5.2 at SHA `140db45` (annotated, pushed 2026-05-26).
**Reviewer:** opus 4.7 senior (cold, re-dispatch after prior stall).
**Date:** 2026-05-26.
**Scope:** 32-commit chain `v0.5.1..v0.5.2` (`224fa68` … `140db45`). +8,134 / −251 lines, mostly docs (spec 918 + plan 4203 + reviews 252). Production touch: 6 `src/` files + 1 new test file (`tests/integration_profiles.rs`, 677 lines).

---

## Verdict

**CLEAN_SHIP.** Zero 🔴, zero 🟡. D1a (2🔴+7🟡) + D1b (2🔴+4🟡) + D7 (1🟡) all closed; the v0.5.2-specific 🟡 from D7 was a forward-pointer-only note, recorded below. Two v0.5.1 deferred Recs (`@v7.0.1` exact-pin, per-bench thresholds) remain on the queue with no new urgency. Tag stands.

---

## Findings

### 🔴 Critical

None.

### 🟡 Important (carryover to v0.5.3)

None — the lone D7 🟡 (clap `<NAME>` placeholder forward-pointer) is documentation-only and already encoded in v0.5.2 source comments (`src/cli.rs:214-225`); no v0.5.3 spec text depends on it.

### 🔵 Nits + closing-loop observations

- **Public-API delta:** `git diff v0.5.1..v0.5.2 -- src/lib.rs` shows one `^-pub` line (`pub use error::{Error, Result, ThemeRuleError, ThemeRuleErrorKind};`) being EXPANDED to add `ProfileErrorKind`, `ProfileRuleError`, `ProfileRuleErrorKind`. Strictly additive; the line-level "removal" is diff noise — same `pub use` statement, four added symbols. Acceptance §9.2 "no removed/modified items" satisfied.
- **Duplicate-formatter audit clean:** `grep -n 'format!.*rule .{}' src/rules.rs` returns 5 sites (lines 1086, 1128, 1172, 1226, 1287) — baseline preserved from v0.5.0's §N-3 invariant. The shape changed (post-D3 dispatch reroute through `RuleSource`) but the byte-equal Display contract holds (three-way identity test at `src/rules.rs:3147-3177` pins `Theme == UserConfig == EmbeddedProfile`).
- **`is_user_supplied` / `styles_override_from_theme` swap clean:** `grep -rn 'is_user_supplied\|styles_override_from_theme' src/` returns one hit, a comment at `src/rules.rs:56` documenting the schema change. Old booleans fully gone, replaced by `source: RuleSource` on `BuiltinRule`.
- **`RuleSource::` match-site density 72** across `src/` — every dispatch arm enumerated explicitly, no `_ =>` catch-alls hiding profile-source bugs. D3 grep audit operationalised.
- **Append-only invariant verified mechanically:** `git diff v0.5.1..v0.5.2 -- tests/integration_capture_groups.rs tests/integration_themes.rs` returns 0/0 lines. v0.5.0 §N-4 byte-equal contract holds across v0.5.1 → v0.5.2.
- **`assets/profiles/.gitkeep`-only:** verified empty profile directory ships. v0.5.2 acceptance §9.2 "no embedded profiles" enforced by filesystem.
- **`ReloadOrchestrator::spawn` is 8-positional including `banner_sink`** (`#[allow(clippy::too_many_arguments)]` documented in-source at `src/reload.rs:200-204`). D5 deviation #2 (`bg_default` as 7th precedence input) doc-comment names every dimension as load-bearing. Forward-pointer for v0.6: builder pattern if a ninth ever lands.

---

## v0.5.1 carryover retrospective

The v0.5.1 final review (CLEAN_SHIP) carried **zero forced** carryovers. Two deferred Recs persist:

| v0.5.1 item | Disposition in v0.5.2 | Status |
|---|---|---|
| Rec #1 — `@v7.0.1` exact-pin on GHA actions vs `@v7` major-track | Out of scope (security review track) | ⏸ deferred unchanged |
| Rec #3 — Per-bench dynamic thresholds | No false positives in v0.5.2 bench-CI | ⏸ deferred (still no demand) |

§N-1..N-9 constraint table from v0.5.1 final review: all 9 invariants verified post-v0.5.2 (see Nits above + spec §3.2). Zero regressions.

---

## D1 + D7 fold verification

| Source | Finding | Disposition | Status |
|---|---|---|---|
| D1a 🔴 | `BuiltinRule` needs `EmbeddedProfile` discriminator | Spec revised; `source: RuleSource` replaces two bools | ✅ shipped @ `a09a116` + Appendix A.6 |
| D1a 🔴 | `ProfileErrorKind` `Clone` derive unsound | Drop `Clone`; carry `message: String` instead of source types | ✅ shipped @ `3de71d4` + Appendix A.1 |
| D1a 🟡 | `themes::name_is_valid` private visibility | `pub(crate)` bump | ✅ shipped (`src/themes.rs:78`) + Appendix A.5 |
| D1a 🟡 | `Compiled::load_with_theme` arity (1 vs 2 new params) | Six-positional shape: `profile` + `profile_path` | ✅ shipped @ `654145b` + Appendix A.7 |
| D1a 🟡 | `#[from]` thiserror attribute mismatch | Removed; manual error wrapping | ✅ shipped (Appendix A.1) |
| D1a 🟡 | `toml::de::Error` leaks into public API | Wrapped as `message: String` | ✅ shipped (Appendix A.1) |
| D1a 🟡 | Rule merge order Steps 2 + 4 call-site | New `apply_profile_append_rules` + whitelist filter step | ✅ shipped @ `36018da` |
| D1a 🟡 | `synthetic_path` shape conflation | Pinned: always `<embedded:profile/{name}>`; disk path separate | ✅ shipped @ `073c5e6` |
| D1a 🟡 | `format_profile_*` helpers undeclared | Spec + Appendix A.2 pinned signatures | ✅ shipped @ `3de71d4` + Appendix A.2 |
| D1b 🔴 | Test 14 ships only 4 cells (handwave 4 more) | All 8 cells enumerated in spec | ✅ shipped @ `2686a55` + Appendix A.10 (`tests/integration_profiles.rs:184-306`) |
| D1b 🔴 | Test 14 byte-pin missing (TODO placeholders) | SGR bytes committed in spec table | ✅ shipped (Appendix A.10) |
| D1b 🟡 | High-risk surface enumeration too loose | Spec §8.0 extended | ✅ shipped |
| D1b 🟡 | `AppendRuleConflictsWithBuiltin` wording prefix divergence | Documented as namespace-divergent (`append_rules` vs `styles.`) | ✅ shipped (spec §6.3) |
| D1b 🟡 | Test 15 hot-reload flake risk | 1500 ms sleep + `MARK_PRE`/`MARK_POST` sync markers | ✅ shipped @ `b201d73` + Appendix A.11 |
| D1b 🟡 | Test 9 clap error wording underspecified | Exact `<NAME>` placeholder pinned in test | ✅ shipped (Appendix A.9) |
| D7 🟡 | clap `<NAME>` placeholder forward-pointer | Doc-only — recorded in v0.5.2-shipped memory | ✅ closes-loop here (recommendation 1 below) |
| D7 🔵 | 6-positional `load_with_theme` builder critique | Defensible at 6; escalate to `LoadOptions` at 7 | ✅ closes (note for v0.6) |

**Total:** 4🔴 closed, 11🟡 closed, 2🔵 noted. Silent omission: zero.

---

## Inline fix independent verdict

- **D5-fix `680c794` (whitelist + theme silent skip):** Agree. Themes are layered policy; user-runtime whitelists are an orthogonal exclusion mechanism. A theme referencing a whitelist-filtered built-in by name is a NO-OP, not a validation error — same logic by which `styles_override_from_theme` already silently dies if the target was filtered. `src/config.rs:501-523` codifies the right semantic with negative regression guard in test 17. Concur with D7 verdict.
- **D6-fix `6c83f2f` (EX_USAGE mapping):** Agree with the single-mapping choice. `RegexCompile` in `ProfileErrorKind` is reached only via `profiles::load` on a *disk-supplied or embedded* profile; v0.5.2 ships zero embedded profiles, so the only reach path today is user-disk-authored content → EX_USAGE is correct. A future embedded library (v0.5.3) makes the path bivalent (tayf bug vs user error), but the discriminator is `path_label.starts_with("<embedded:profile/")` and can be re-mapped then without breaking v0.5.2's contract. **Forward-pointer for v0.5.3:** when the first embedded profile ships, split the mapping (embedded RegexCompile → EX_SOFTWARE 70; disk RegexCompile → EX_USAGE 64). Not needed today.
- **D5 deviation #2 (`bg_default` 7th precedence input on `ReloadOrchestrator::spawn`):** Latent v0.5.1 bug fix wearing a v0.5.2-design-choice hat. Pre-v0.5.2, `effective_theme` was passed verbatim into the reload thread; if it had been bg-detect-resolved at startup, reloads would re-resolve precedence WITHOUT a bg-detect fallback (silently degrading to "no theme" mid-session). v0.5.2 surfaces it because the 4-tier precedence now has four explicit dimensions, exposing the gap. **Memory recommendation:** add `feedback_reload_precedence_snapshot.md` — "when a startup-only resolved value sits in a precedence chain that also feeds hot reload, the snapshot MUST be threaded through `spawn` separately from the resolved-at-startup composite." One sentence; high reuse potential as the precedence chain grows.
- **D5 concern #3 (clap `--profile <NAME>` placeholder):** Forward-pointer ALREADY encoded in `src/cli.rs:214-225` (inline doc-comment names the placeholder convention + cites the actual clap-emitted string). No v0.5.3 spec wording depends on it. A `feedback_clap_value_name_in_dup_error.md` memory entry would close the loop on the test-assertion-specificity discipline. Recommend one-line memory write; no spec carryover.

---

## What v0.5.2 did right

- **Appendix A operationalised.** D1's 4🔴+13🟡 absorbed into a 12-section pre-implementation revision pack (lines 3636-end of plan), every implementer subagent gate-checked it before dispatch. Zero D1 findings reached D7 unaddressed.
- **Match-site enumeration pre-dispatch.** D3 ran `grep -n 'RuleSource::' src/` BEFORE adding `EmbeddedProfile`; 72 post-D3 sites verified, zero dead arms, zero `unreachable!()` panics under the 15-test integration suite. `feedback_phase1_grammar_gate_blind_spot` lesson institutionalised.
- **Public API hygiene.** `ProfileErrorKind` carries `String` messages instead of leaking `toml::de::Error` / `regex::Error` / `io::Error`. Future `toml`-crate or `regex`-crate major bumps cannot break tayf's public surface.
- **Three-way identity test** (`src/rules.rs:3147-3177`) pins `Theme == UserConfig == EmbeddedProfile` byte-equal Display. The cross-path duplicate-formatter discipline scaled to the new third source without forking the wording.
- **Hot-path-unchanged structurally proven.** I-6 test `hot_path_unchanged_when_no_profile` (`src/rules.rs:3187-3231`) asserts both the count (13 built-ins) AND `source == RuleSource::Builtin` on each rule when profile is None. Profile-inactive byte-equivalence to v0.5.1 is enforced, not asserted.

---

## Recommendations for v0.5.3 cycle

1. **v0.5.3 brainstorm MUST open by reading this review + v0.5.1 spec §11.2** (domain-senior pattern audit verdict matrix: 5 DROP / 3 RESHAPE / 1 AUDIT / 1 SHIP-AS-IS — only 4 of 9 originally-proposed v0.5.3 patterns survive). Silent omission = replay of v0.4.0 failure mode (`feedback_consume_prior_review`).
2. **Memory writes:** (a) `feedback_reload_precedence_snapshot.md` (D5 deviation #2 lesson), (b) one-line note in `feedback_test_assertion_specificity` follow-up about clap `<NAME>` placeholder convention (D7 🟡 / D5 concern #3).
3. **EX_USAGE mapping split when first embedded profile lands:** v0.5.3 spec must address embedded vs disk RegexCompile — same `Error::Profile { kind: RegexCompile }` variant but the source path determines the exit code. Add the path-discriminator branch in `map_error_to_exit_code` (`src/main.rs:51`).
4. **Profile-active bench baseline.** v0.5.2 ships zero embedded profiles. v0.5.3's first embedded profile is the right inflection point to record canonical baselines (two `append_rules` + whitelist + theme override).

---

**Synopsis (final cross-cutting verdict):** CLEAN_SHIP. v0.5.2's mechanism-only profile system lands with surgical discipline: Appendix A absorbed all 4🔴+13🟡 from D1's parallel spec review pre-implementation, D5-fix and D6-fix closed the only D2-D6 concerns surfaced during code review (whitelist+theme silent-skip + EX_USAGE mapping), and the v0.5.1 §N-1..N-9 invariant set verified byte-for-byte post-tag (5 formatter sites, append-only on capture-groups + themes integration tests, public-API additive-only, empty `assets/profiles/`). Two v0.5.1 deferred Recs persist on the queue with no new urgency; the D5 deviation #2 `bg_default` snapshot is a genuine latent v0.5.1 bug fix wearing v0.5.2 clothes, worth a one-sentence memory entry. Tag `v0.5.2` at `140db45` stands.
