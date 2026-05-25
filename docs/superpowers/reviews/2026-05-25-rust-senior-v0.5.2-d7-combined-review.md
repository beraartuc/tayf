# v0.5.2 D7 Combined Code Review

**Date:** 2026-05-25
**Reviewer:** opus 4.7 Rust senior (D7 — combined D2-D6 review, full diff `v0.5.1..HEAD`)
**Scope:** 23 commits, +7,965/-249 lines, mostly docs (plan = 4203 lines, spec = 918 lines) + 6 src files touched + 1 new test file.
**Verification run:** `cargo fmt --check` clean; `cargo clippy --all-targets -- -D warnings` clean; `cargo test --lib` 453/453 pass; `cargo test --test integration_profiles --test integration_capture_groups --test integration_themes` 15+15+9 pass. `assets/profiles/` contains only `.gitkeep`. `grep -n 'format!.*rule .{}' src/rules.rs` returns 5 sites (baseline preserved). `git diff v0.5.1..HEAD -- src/lib.rs | grep '^-pub' | wc -l` returns 1 — see verdict note below.

**Verdict:** SHIP_AS_IS

---

## 🔴 Critical

None.

## 🟡 Important

- **D5 concern #3 (clap wording divergence) needs a forward-pointer.** The spec §8.1 #9 wording (`"the argument '--profile' cannot be used multiple times"`) is approximate; clap actually emits `"the argument '--profile <NAME>' cannot be used multiple times"` because `value_name = "NAME"` is set. `src/cli.rs:222` pins the actual string and `src/cli.rs:214-217` documents the rationale inline. Acceptable as shipped — the implementer made the right call. **Action:** record a one-line note in the v0.5.2-shipped memory or in the D8 final review so v0.5.3 specs that copy this pattern know to use the placeholder form. No code change.

## 🔵 Nits

- **`Compiled::load_with_theme` signature is now six positional params** (config, config_path, theme, profile, profile_path, depth). Defensible against the "should be a builder" critique because all six are precedence-chain inputs and the call sites are exactly two (`lib.rs` initial load + `reload.rs reload_once`). If a seventh ever lands, escalate to `LoadOptions { ... }`. Worth a `v0.6` note.
- **`src/rules.rs:836-849` `profile_name_from_path_label` derives the user-facing name from the path label** (file stem or `<embedded:profile/{name}>` strip). Works, but it would have been more robust to thread `profile_name: Option<&str>` through `compile_merged_rules` directly. The current approach is fine because `path_label` is constructed by `profiles::load` and never user-supplied — defensive only. Document the assumption stays in the doc-comment (already present).
- **`apply_user_rules_with_source` silent-skip for whitelist-filtered built-ins (`src/config.rs:501-523`)** is the right semantic but the `continue` branch silently swallows ALL "rule named X not found" cases for non-UserConfig sources. The doc-comment correctly notes the only two reach paths and why both are no-ops by spec. Add a debug-only `crate::log::debug_msg!` when this path fires under `TAYF_LOG=debug`? Defer to v0.5.3+ — not blocking.
- **`hot_path_unchanged_when_no_profile` (rules.rs:3187) asserts `builtin_rules()` directly** rather than `Compiled::load_with_theme(..., None, None, depth).individuals` re-deriving names. The defensive `for r in builtin_rules()` loop checking `r.source == RuleSource::Builtin` is the load-bearing assertion. Acceptable — the spec's I-6 intent (no profile-active branch fires when profile is None) is satisfied by both the count check + source-tag check.
- **Public API note:** `git diff v0.5.1..HEAD -- src/lib.rs | grep '^-pub'` returns 1 line, but it's the `pub use error::{Error, Result, ThemeRuleError, ThemeRuleErrorKind};` line being EXPANDED to include the four new profile types — additive on the same `pub use` statement, not a removal. The spec §9.2 acceptance "no removed/modified existing items" is satisfied; the 1-line "removal" is line-level diff noise.

---

## D2-D6 concern resolutions

| Concern | Status |
|---|---|
| D2 `#![allow(dead_code)]` on `src/profiles.rs` | RESOLVED — D5 removed it (verified absent from `src/profiles.rs`). |
| D2 `load_with(name, xdg, home)` env-closure shape | ACCEPTABLE — consistent with `themes::load_with`. Pattern reuse is correct. |
| D3 BuiltinRule schema swap (two booleans → `source: RuleSource`) | LANDED — `is_user_supplied`/`styles_override_from_theme` removed (`grep -rn` returns only one comment reference in `src/rules.rs:56` documenting the change). All 13 `BuiltinRule { ... }` literals in `builtin_rules()` use `source: RuleSource::Builtin`. |
| D3 `apply_user_rules_with_source` signature (bool → `RuleSource`) | LANDED — `src/config.rs:448-453`. Three callers pass UserConfig/Theme/EmbeddedProfile correctly. |
| D3 `compile_error_for(&BuiltinRule, ...)` refactor | LANDED — `src/rules.rs:869-892` reads `rule.source` directly, four-arm match including `EmbeddedProfile`. |
| D5 concern #1 (whitelist+theme false-positive) — FIXED by 680c794 | VERIFIED CORRECT — `src/config.rs:501-523` short-circuits the missing-rule lookup; doc-comment cites spec §5.4. Test 17 (`profile_whitelist_plus_theme_referencing_filtered_builtin_loads_cleanly`) exercises the failure case with a negative regression guard against the `appears twice with conflicting` substring. The fix is surgical (silently skip; not error) and the rationale matches spec semantics. |
| D5 deviation #2 (`bg_default` 7th param on `ReloadOrchestrator::spawn`) | ACCEPTABLE — `src/reload.rs:194-210`. Rationale is sound: bg-detect requires OSC 11 round-trip latency-sensitive at startup; persisting the snapshot through reloads matches spec §7.2's "CLI never mutates during session" invariant. The spec sketched theme+profile params but did not address how bg-detect's last-resort role survives reload — D5 closed the gap correctly. 7 params is at the edge; doc-comment explains each. |
| D5 concern #3 (clap wording) | ACCEPTABLE — actual `<NAME>` placeholder. See 🟡 above. |
| D5 pre-existing OSC-11 flake | OUT OF SCOPE — not introduced by v0.5.2. |
| D6 concern #1 (EX_USAGE mapping) — FIXED by 6c83f2f | VERIFIED CORRECT — `src/main.rs:48-50` adds `Error::Profile { .. } => 64` + `Error::ProfileValidation { .. } => 64`. Two new unit tests (`profile_maps_to_ex_usage` + `profile_validation_maps_to_ex_usage`). Tests 11+12 in `integration_profiles.rs` assert `Some(64)` exactly. |
| Test 15 hot-reload flake | NOT FLAKY locally (4/4 stable). A.11 MARK_PRE/MARK_POST sync markers + 1500ms sleep landed verbatim. macOS CI flake-rerun fallback per memory if it surfaces. |

---

## Appendix A delta verification

| Delta | Status | Evidence |
|---|---|---|
| A.1 — ProfileErrorKind shape (no Clone, String-message fields, no `#[from]`) | ✅ | `src/error.rs:193-230`. `#[derive(Debug)]` only; all four variants carry `String` messages. |
| A.2 — `format_profile_*` helper bodies match revised shape | ✅ | `src/error.rs:530-588`. |
| A.3 — Byte-pin tests cover all 4 ProfileErrorKind variants | ✅ | `src/error.rs` tests pass (lib 453/453). |
| A.4 — `profiles::load` constructs new shape (`message: e.to_string()`) | ✅ | `src/profiles.rs:172-230`. All three sites (canonicalize, parse, regex) use the string-form. |
| A.5 — `themes::name_is_valid` `pub(crate)` bump | ✅ | `src/themes.rs:78` reads `pub(crate) fn name_is_valid`. |
| A.6 — BuiltinRule `source: RuleSource` + apply_user_rules_with_source(RuleSource) | ✅ | `src/rules.rs:71` + `src/config.rs:448-453`. Old booleans gone. |
| A.7 — `Compiled::load_with_theme` six-param shape | ✅ | `src/rules.rs:631-638`. Two callers (lib.rs + reload.rs) updated. |
| A.8 — I-6 test six-arg + source-tag check | ✅ | `src/rules.rs:3187-3231`. Asserts `r.source == RuleSource::Builtin` for all rules. |
| A.9 — clap test three sub-assertions + byte-pin | ✅ | `src/cli.rs:202-225`. Pinned actual `<NAME>` placeholder (see 🟡). |
| A.10 — 8 explicit theme precedence functions | ✅ | `tests/integration_profiles.rs:184-306`. All 8 cells named per spec table. |
| A.11 — MARK_PRE/MARK_POST + 1500ms + region-split | ✅ | `tests/integration_profiles.rs:546-677`. Pre-region asserts cyan+dim; post-region asserts cyan present, dim absent. |
| A.12 — Three-way identity (Theme=UserConfig=EmbeddedProfile byte-equal Display) | ✅ | `src/rules.rs:3147-3177`. Pins KeyMalformed wording across all three sources. |

All 12 A-deltas shipped. Zero ❌, zero 🟡.

## What v0.5.2 did right

- **D1 fold discipline.** Spec §3.1 fold-or-defer table is exhaustive (8/8 fold) and every absorbed concern has a traceable artifact (test name, line ref, or commit). The `feedback_consume_prior_review` mandate is structurally honoured — no silent omissions.
- **Pre-implementation match-site enumeration.** D3 ran `grep -n 'RuleSource::' src/` BEFORE adding `EmbeddedProfile`. Every dispatch site (Step 1 / Step 2 / Step 3 / duplicate-target × 4 source arms) carries an explicit arm with reachability commentary. No dead arms; no unreachable!() panics under any of the 15 integration tests. The `feedback_phase1_grammar_gate_blind_spot` lesson is operationalised.
- **Public API hygiene.** `ProfileErrorKind` carries `String` messages instead of leaking `toml::de::Error` / `std::io::Error` / `regex::Error`. The future `toml`-crate major bump cannot break tayf's public surface. All new types `#[non_exhaustive]` where applicable.
- **Byte-pin + negative regression discipline.** Every new diagnostic test (15 integration + 9 unit + 8 dispatch + 1 three-way + 1 hot-path) carries both a positive substring assert and a negative regression guard (`!contains("validation error")`, `!contains("collides with built-in")`, etc.). The `feedback_test_assertion_specificity` mandate is honoured throughout.
- **Whitelist+theme silent-skip fix.** D5-fix 680c794 is the right call — themes don't know about user-runtime whitelists, so their references to filtered built-ins must be no-ops, not errors. Test 17 pins both the success case AND the negative regression guard against the false-positive diagnostic.
- **EX_USAGE mapping.** D6-fix restored the contract that user-input errors exit 64; tightened tests 11+12 from accept-either (64||70) to exact-match Some(64).
- **Hot-path-unchanged proof.** I-6 test asserts 13 built-ins + all `RuleSource::Builtin` when profile is None. The profile-inactive byte-equivalence to v0.5.1 is structurally enforced.

## Recommendations for v0.5.3 cycle

1. **Memory write:** record the clap `<NAME>` placeholder convention (v0.5.2 D5 concern #3) so future `--flag NAME` tests pin the actual wording. One sentence in `feedback_test_assertion_specificity` follow-up or a new `feedback_clap_value_name_in_dup_error` entry.
2. **v0.5.3 spec prerequisites.** Library content (`assets/profiles/aws.toml`, `k8s.toml`, etc.) only — no schema modifications. `#[serde(deny_unknown_fields)]` is the contract. Domain-audit matrix from v0.5.1 spec §11.2 is the mandatory input.
3. **`bg_default` 7th param disclosure.** When the v0.5.3 spec touches `ReloadOrchestrator::spawn`, note the 7-param shape so the spec sketch matches reality. (D5 closed a latent v0.5.1 gap — propagate the fix forward in documentation.)
4. **Profile-active bench baseline.** v0.5.2 ships zero embedded profiles, so a representative bench fixture (two `append_rules`) was deferred. v0.5.3's first embedded profile is the right inflection point to record canonical baselines.
5. **CHANGELOG note** for v0.5.2: schema additivity, public API additivity (4 new error types, 2 new variants, all `#[non_exhaustive]`), and the bg-detect-survives-reload behavior fix.
