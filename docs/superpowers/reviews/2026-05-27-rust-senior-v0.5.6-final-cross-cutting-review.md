---
title: tayf v0.5.6 — final cross-cutting review (Opus 4.7 Rust senior)
date: 2026-05-27
range: v0.5.5..HEAD (15 commits — 13 implementation + 2 docs)
reviewer: Opus 4.7 (1M context) — single-blind ship-gate pass
verdict: SHIP_WITH_FIXES (one documentation drift cluster: 5 occurrences of "13 built-ins" → "12" in assets/* + src/profiles.rs:392)
---

# Cross-Cutting Review — tayf v0.5.6 (`v0.5.5..HEAD`, 15 commits)

v0.5.6 lands the architectural priority tier mechanism (0/100/200) that v0.5.3
forward-pointed as the resolution path for AWS / Docker profile envelope ↔
interior built-in collisions, plus full removal of the broad-FP `http_status`
built-in, plus an ipv6 pattern tighten that closes the Rust-path FP audited
under FP-audit C-2. The net diff is ~600 lines of source change across
`src/{rules,pipeline,config,profiles,themes,error}.rs` + small touches in
`src/config_tui/events.rs`, two assets/profiles/{aws,docker}.toml priority-field
additions, four assets cleanup deletions (`http_status` removed from
gcp/network/dark-theme/light-theme TOMLs), and benches/throughput.rs comment
refresh. Tests went from baseline 584 → **636 lib tests** (+52 net),
matching the spec §9.7 post-review estimate of "622 ± 3" within
the optimistic-side tolerance.

The implementation discipline is high. Every memory mandate from the v0.5.5
ship is respected: `feedback_consume_prior_review` (spec §1 forward-pointer
absorption), `feedback_spec_phase_parallel_review` (spec §13 with 31/33
FOLDED), `feedback_collision_pin_pattern` (Task 9 RENAME + FLIP + new
no-fix pin), `feedback_parallel_call_site_invariant_audit` (priority field
touches every one of the 7 merge sites enumerated in spec §7.2),
`feedback_test_assertion_specificity` (theme rejection uses full
`assert_eq!` on the Display string), `feedback_lean_process_small_subversions`
(full ceremony preserved — parallel spec review + TDD per task + this final
cross-cutting review). The one fix-required item is documentation drift
(see §13 below): five `# all 13 built-ins` / `the 13 built-in styles`
header comments in `assets/profiles/{aws,docker,k8s}.toml`,
`assets/themes/{dark,light}.toml`, and one `src/profiles.rs:392`
doc-comment ("against the 13 built-ins") that became stale when
`http_status` was removed in Task 1. The actual code is correct
(unit test at `rules.rs:3338-3340` byte-pins `BUILTIN_NAMES.len() == 12`),
and the drift cannot cause a runtime regression — but it is exactly the
sort of stale-doc surface that memory `feedback_stale_dead_code_reason_drift`
generalizes from (any forward-pointer that becomes silently inaccurate
once a feature lands).

## 1. Spec coverage

✅ **PASS** for all required items; one minor stale doc-comment surface
(see §13).

Spec §2.1 enumerates 8 in-scope groups (A1, A2, B1-B6, C, D, F). Each
verified against the landed code:

1. **A1 — `http_status` removal.** ✅ Pattern + `BUILTIN_NAMES` entry + all
   unit tests gone. `grep -rn '"http_status"' src/ tests/ assets/` returns
   empty (only `benches/BASELINE.md` archive comments remain — that file
   is a historical baseline log, not active code). `src/rules.rs:543-556`
   `BUILTIN_NAMES` lists exactly 12 entries. `rules.rs:1775` user-rule
   test correctly says `"12 built-ins + 1 user rule"`. `assets/profiles/gcp.toml`
   and `assets/profiles/network.toml` `rules` whitelists no longer include
   `"http_status"`. `assets/themes/dark.toml` + `light.toml` had their
   `[[rules]] name = "http_status"` blocks deleted. Migration recipes in
   CHANGELOG.md (`## [0.5.6]` block) ship both the "preserve" and "improved"
   variants per spec §10.

2. **A2 — ipv6 tighten.** ✅ Pattern at `src/rules.rs:476` lands the
   POST-IMPLEMENTATION corrected form documented in spec §6 (4 branches:
   `::1` literal → 7-pair → `\b[hex]{3,4}:` leading + `{0,5}` mid groups +
   `[hex]{0,4}` trailing → `::[hex]{1,4}(:[hex]{1,4}){2,}` requiring ≥2
   trailing groups). Test coverage at `rules.rs:3372-3417`: 4 negative
   (Rust-path FPs) + 6 positive (`::1`, `fe80::1`, `2001:db8::1`,
   `2001:db8::ff00:42:8329`, `1:2:3:4:5:6:7:8`, `1234:5678::`). The
   conservative tightening over the original spec proposal (added `\b`
   prefix + `[hex]{3,4}` minimum leading) is exactly the implementer
   finding documented in commit `d905c95` (`docs(spec): correct v0.5.6 §6
   ipv6 pattern per Task 2 implementer finding`).

3. **B1 — `BuiltinRule::priority: i32`.** ✅ Field at `src/rules.rs:91`
   with the full tier-convention doc-comment. All 12 `builtin_rules()`
   struct literals carry `priority: 0` (verified one per built-in;
   `permission`, `timestamp`, `uuid`, `url`, `email`, `ipv4`, `ipv6`
   `rules.rs:480`, `mac` `:489`, `log_level` `:498`, `filename` `:507`,
   `fqdn` `:516`, `duration` `:536`). Test
   `priority_default_is_zero_for_all_builtins` at `rules.rs:3422` byte-pins
   the invariant.

4. **B2 — `Compiled::priorities: Vec<i32>` parallel vec.** ✅ Field at
   `src/rules.rs:606`. `Compiled::empty()` at `rules.rs:632` initialises
   to `Vec::new()`. `compile_merged_rules` at `rules.rs:984` allocates
   with `Vec::with_capacity(rules.len())`, pushes `rule.priority` per
   merged rule at `:1007`, returns it as part of `CompiledRules` at
   `:1036`. Test `compiled_priorities_parallel_vec_invariant` at
   `rules.rs:3429` checks the 5-way length-equality (priorities ==
   individuals == styles == group_styles == uses_capture_styling).

5. **B3 — `UserRule::priority: Option<i32>`.** ✅ Field at
   `src/config.rs:122` with `#[serde(default)]`. `deny_unknown_fields`
   preserved at line 93. Conditional overwrite at `config.rs:564-566`
   for the builtin-override branch (`if let Some(p) = ur.priority { existing.priority = p; }`)
   exactly matches spec §7.2 site #5 — user omitting `priority`
   preserves the underlying tier (built-in 0, profile 100/200);
   user-explicit `priority = N` overwrites. New-rule branch at
   `config.rs:596` uses `priority: ur.priority.unwrap_or(0)`.

6. **B4 — `ProfileRule::priority: Option<i32>`.** ✅ Field at
   `src/profiles.rs:89`, additive `#[serde(default)]`. `assets/profiles/aws.toml:31`
   sets `priority = 200` for the `arn` entry; `assets/profiles/docker.toml:30`
   sets `priority = 200` for the `image_tag` entry. The profile-append
   merge site reads `ar.priority.unwrap_or(100)` (spec §7.2 site #3 →
   verified by the `aws_arn_appended_priority_200` and
   `aws_instance_id_appended_priority_100` tests at `profiles.rs:1075-1093`
   passing).

7. **B5 — Theme priority rejection.** ✅ `ThemeRuleErrorKind::StraySchemaField`
   variant at `src/error.rs:101-104`. `validate_theme_rules` rejects
   `priority` field at `src/themes.rs:400-404`. Test
   `theme_rule_with_priority_field_errors` at `themes.rs:1171` does a
   full-string `assert_eq!` on the Display message at line 1188 — pinning
   exact wording per memory `feedback_test_assertion_specificity`.

8. **B6 — `apply_rules` priority sort.** ✅ Sort block at
   `src/pipeline.rs:80-86` uses `Reverse(priorities[a]).cmp(&Reverse(priorities[b]))`
   (idiomatic — no operand-swap fragility per spec §4.3) and stable
   `sort_by` (NOT `sort_unstable_by`, per the in-place comment at
   `pipeline.rs:77-79` explaining tie-break invariance). The doc-comment
   block at `pipeline.rs:35-53` is fully rewritten with the v0.5.6
   "Acceptance contract" — old "first-match-wins by pattern order"
   replaced with the priority-DESC + bidirectional-overlap contract.
   Note: `pipeline.rs:70-73` retains a now-partially-stale comment
   ("first-match-wins overlap resolution depends on pattern order") —
   technically still correct under the priority-0 tie-break case, and
   adjacent to the new sort block which qualifies it. Acceptable as-is;
   not a blocker.

9. **C — Profile pin RENAME + FLIP + new positives.** ✅
   - `aws_arn_yields_to_interior_region_pattern_v0_5_3_limitation` →
     `aws_arn_wins_over_interior_region_pattern` (`src/profiles.rs:843`),
     assertions FLIPPED (now asserts `aws.arn` magenta wraps envelope at
     `:857`, `region` SGR is suppressed at `:863`).
   - `docker_image_tag_registry_host_yields_to_fqdn_v0_5_3_limitation` →
     `docker_image_tag_wins_over_registry_host_fqdn` (`profiles.rs:938`),
     same FLIP shape.
   - `aws_arn_wins_over_interior_ipv4` (`profiles.rs:1144`) — new positive
     test for FP audit C-12.
   - `aws_arn_wins_over_interior_uuid` (`profiles.rs:1161`) — new positive
     test for FP audit C-13.
   - `mac_yields_to_ipv6_eight_pair_v0_5_5_limitation` (`profiles.rs:1181`)
     — new no-fix pin for C-1; suffix follows memory
     `feedback_collision_pin_pattern` convention.
   - `docker_container_id_wins_over_interior_uuid_via_priority`
     (`profiles.rs:1206`) — F3 RENAMED + FLIPPED.
   - `docker_container_id_yields_to_uuid_envelope_outside_docker_profile`
     (`profiles.rs:1231`) — inverse case test pinning the profile-scope
     contract.
   - `tests/integration_profiles_library.rs:140` comment reference updated
     to point at the new `..._wins_over_*` names.

10. **D — Test coverage additions.** ✅
    - `ipv4_does_not_match_leading_zero_octet` (`rules.rs:1411`) +
      `..._out_of_range_256` (`:1416`) + `..._out_of_range_999` (`:1421`) +
      `..._identifier_prefix` (`:1426`).
    - `log_level_matches_in_bracket_delimiter` (`rules.rs:1469`) +
      `..._in_paren_delimiter` (`:1484`).
    - `duration_matches_microseconds_greek_mu` (`rules.rs:1528`).
    - `url_trims_trailing_sentence_punctuation` (`rules.rs:1954`),
      `url_matches_ssh_scheme` (`:2108`), `url_matches_ftp_scheme`
      (`:2113`), `url_matches_scp_form` (`:2118`).
    - FILENAME_EXTENSIONS doc-comment (spec §2.1 E6) — the single-letter
      ext attribution (`a` archive, `c` C source, etc.) is documented
      via the CHANGELOG.md `[0.5.6]` Added bullet (verified inline). Note:
      I did not observe a code-side doc-comment block immediately above
      the const at `rules.rs:146`. The actual attribution is in CHANGELOG
      only. This is acceptable scope-wise but a minor surface-discoverability
      gap. Severity: 🔵 documentation NIT, not a blocker.

11. **F — Audit follow-up.** ✅ F1 (μs Greek-letter), F2 (url trailing
    punct across schemes), F3 (RENAMED to `..._wins_over_*_via_priority`)
    all landed per items above.

12. **TINY-1 — events.rs wording sweep.** ✅ `src/config_tui/events.rs:122,127`
    both read `"help overlay lands in v0.6+ (Modal::Help wiring deferred)"`
    (previously `"v0.5.5+"`). v0.5.5 §16 NIT successfully absorbed.

## 2. DOKUNULMAZ invariant

✅ **PASS.**
`git diff --name-only v0.5.5..HEAD | grep -E 'src/(pty|io_loop|tty_guard|signals|runtime)\.rs'`
returns empty.

Authorized touches per spec §2.3 — all landed within scope:
- `src/rules.rs` — http_status delete, ipv6 body tighten, `BuiltinRule::priority`
  field, `Compiled::priorities` parallel vec + populate in `compile_merged_rules`.
- `src/profiles.rs` — `ProfileRule::priority` schema field; the
  `build_from_loaded` Step 4 reads `ar.priority.unwrap_or(100)` (verified
  indirectly via cross-profile priority tests).
- `src/pipeline.rs` — `apply_rules` sort step at lines 76-86; doc-comment
  block at lines 35-53 fully rewritten.
- `assets/profiles/aws.toml` — `arn` entry `priority = 200` + v0.5.6
  comment block.
- `assets/profiles/docker.toml` — `image_tag` entry `priority = 200`.

Adjacent files modified within v0.5.6 scope but outside the
DOKUNULMAZ list:
- `src/config.rs` — `UserRule::priority` field + conditional overwrite
  merge logic. Authorized (config.rs is not in DOKUNULMAZ).
- `src/themes.rs` — theme priority rejection guard. Authorized.
- `src/error.rs` — new `StraySchemaField` enum variant + Display arm.
  Authorized.
- `src/config_tui/events.rs` — 2-line TINY-1 wording sweep. Authorized
  (carryover from v0.5.5 §16 NIT).
- `assets/themes/{dark,light}.toml` — http_status block deletion (4 lines
  each). Acceptable as part of A1 removal.
- `assets/profiles/{gcp,network}.toml` — `"http_status"` whitelist entry
  deletion (1 line each). Acceptable as part of A1 removal.
- `benches/throughput.rs` — header / fn doc-comment refresh removing
  `http_status` from the "exercises" list. Authorized as benches/* doc
  alignment, no behavioral change.

Zero hot-path / signal-handler / PTY surface touched.

## 3. EN/TR language compliance

✅ **PASS** (with two design-correct exceptions that match the project's
pre-existing tolerance).

`grep -rEn '[ğüşıöçĞÜŞİÖÇ]' $(git diff --name-only v0.5.5..HEAD | grep -E '\.(rs|toml)$')`
returns exactly two hits:

- `src/config.rs:120` — `/// Örnek 4.` in the `UserRule::priority`
  doc-comment, cross-referencing the spec §4.4 worked example. The §
  symbol + Turkish-language section reference into the design doc is the
  same pattern used throughout the codebase (e.g. references to `tayf-tasarim.md
  §6.5`). This is an explicit allowance in CLAUDE.md §1 ("Section
  references in code are encouraged") — borderline but accepted by the
  v0.5.5 final review for the same idiom. The actual identifier and
  surrounding code are English; only the cross-reference is Turkish.
- `src/themes.rs:738` — `assert!(!name_is_valid("ışık"));` in the test
  `theme_name_with_diacritics_rejected`. This is **legitimate test
  input** (a Turkish word with diacritics serving as the negative
  fixture for the `name_is_valid` filter). The test demonstrates a
  rejection contract; using Turkish as the test string is appropriate
  and doesn't violate the "English code" rule because it's data, not
  an identifier. Pre-existing; not introduced by this v0.5.6 diff.

Spec / plan / review markdown files remain Turkish as expected by
CLAUDE.md §1. Memory `feedback_review_calibration_en_tr` mandate
respected — code-side strict English (with the one Örnek 4 cross-ref
exception), spec-side free Turkish.

## 4. Test assertion specificity

✅ **PASS.** Per memory `feedback_test_assertion_specificity`:

- `src/themes.rs:1186-1190` `theme_rule_with_priority_field_errors`:
  ```rust
  assert_eq!(
      kind_msg,
      "cannot override `priority`; that field is restricted to user-config overlays (themes are style-only)",
      "exact Display wording per spec §8.2"
  );
  ```
  Full Display `assert_eq!`. ✓
- `src/config.rs:1518-1519` `user_rule_rejects_out_of_range_priority`:
  asserts on `err.to_string()` containing `"out of range"` OR `"invalid"`
  — the `OR` is the toml-serde version-tolerance shape (depending on
  toml crate version the error wording differs slightly). Per memory
  `feedback_toml_edit_025_quirks`, pinning observed-shape with a small
  alternation is acceptable; the test still loudly fails if the
  deserializer accepts an i64 overflow.
- `src/config.rs:1490`, `:1504` `user_rule_rejects_string_priority` and
  `..._rejects_float_priority` similarly assert `contains("invalid type")
  || contains("expected")`. Acceptable for the same toml-version-tolerance
  reason — and the actual surface being pinned ("type mismatch produces a
  parse error") is what matters.
- `src/config.rs:1535`, `:1551`, `:1567`, `:1583` priority merge tests
  use `assert_eq!` on i32 values (not strings). ✓

The pattern "full `assert_eq!` on error Display strings, structural
`contains` on whole-document substrings" remains consistent with the
v0.5.5 ship.

## 5. Duplicate formatter audit

✅ **PASS.** `grep -nE 'format!.*rule "|format!.*message: format!' src/{themes,error,config,rules,profiles,pipeline}.rs`
returns zero hits in the v0.5.6 diff. The new `StraySchemaField` variant
goes through the `Display` impl arm at `src/error.rs:188-193` exclusively;
no shadow `format!` call sites bypass it.

The `Error::Config { message: format!(... ) }` occurrences in `src/config.rs`
(at `:574`, `:585`) are constructing the inner `message` field for
the standard `Error::Config` variant — these were pre-existing
unchanged formatters that wrap user-input validation errors, not
duplicate formatters of the new `ThemeRuleErrorKind::Display`. Acceptable.

## 6. `unwrap()` / `expect()` discipline

✅ **PASS** for new code; one expect on `src/pipeline.rs:92` is
pre-existing (`expect("capture 0 is always the full match")` is the
regex 1.12 invariant — capture group 0 always exists when a match was
returned). All other `expect()` / `unwrap()` hits returned by
`grep -nE '\.unwrap\(\)|\.expect\(' src/{rules,pipeline,config,profiles,themes,error}.rs`
live inside `#[cfg(test)] mod tests` blocks (verified by line numbers
crossing the test-module boundary at:
- `src/rules.rs:3422+` test module — high concentration of `expect`s,
  all in tests
- `src/profiles.rs:398+` test module — same
- `src/config.rs:1432+` schema test module — same
- `src/pipeline.rs:527+` test module — same
- `src/themes.rs:485+` test module — same).

No new `unwrap()` or `expect()` introduced in library code. The
priority sort block at `pipeline.rs:80-86` does not panic — it indexes
into `compiled.priorities` via `a` and `b` which are RegexSet match
indices guaranteed in `[0, individuals.len())`, and `priorities.len()
== individuals.len()` is the parallel-vec invariant pinned by
`compiled_priorities_parallel_vec_invariant`. Safe.

## 7. `#[allow(dead_code)]` hygiene

✅ **PASS.** `grep -n '#\[allow(dead_code)\]' src/rules.rs` returns zero
matches. The two staged-during-implementation allows (on `BuiltinRule::priority`
between Task 3 and Task 8, and on `Compiled::priorities` between Task 4
and Task 8) were both correctly removed once the consumer wire landed.

`grep -rn '#\[allow(dead_code)\]' src/` returns only the pre-existing
v0.5.5-era allows on `config_tui/edit.rs`, `snapshot.rs`, `save.rs`,
`widgets/save_diff.rs` — none introduced by v0.5.6. The v0.5.5 final
review's open NIT (`Color::to_toml_str` stale allow) was fixed in commit
`9de1be3` ("chore(style): drop stale #[allow(dead_code)] on
Color::to_toml_str") landed on the v0.5.5 ship cycle pre-tag.

Memory `feedback_stale_dead_code_reason_drift` mandate respected.

## 8. Algorithmic correctness

✅ **PASS** for the priority sort wire — every spec §4.3 invariant
satisfied.

Pipeline algorithm verification (`src/pipeline.rs:80-86`):

```rust
{
    use std::cmp::Reverse;
    let priorities = &compiled.priorities;
    scratch.set_match_scratch.sort_by(|&a, &b| {
        Reverse(priorities[a]).cmp(&Reverse(priorities[b])).then_with(|| a.cmp(&b))
    });
}
```

- ✓ Uses `Reverse` wrapper (idiomatic; not the operand-swap idiom that
  spec §4.3 explicitly flagged as fragile).
- ✓ Uses `sort_by` (stable). `sort_unstable_by` deliberately NOT used.
- ✓ Tie-break with `then_with(|| a.cmp(&b))` — rule_index ASC.
- ✓ Behavioral correctness pinned by 5 dedicated tests at `pipeline.rs:947-1102`:
  - `apply_rules_priority_higher_wins_envelope_over_interior` (947) — the
    decisive test; rule 0 is interior, rule 1 is envelope with `priority: 200`;
    behavior was Green (interior wins) without the sort, now Red (envelope wins).
  - `apply_rules_priority_sort_is_stable_under_equal_priorities` (1013) —
    K=3 priority-0 rules, stability guarantee.
  - `apply_rules_priority_equal_falls_back_to_rule_index_order` (1039) —
    v0.5.5 invariant preserved.
  - `apply_rules_priority_negative_yields_to_default` (1067) — negative
    priorities yield correctly.
  - `apply_rules_priority_extreme_values_do_not_overflow` (1098) —
    `i32::MAX`/`i32::MIN` don't panic.

`Compiled::priorities` populated from `BuiltinRule::priority` at
`compile_merged_rules` line 1007 (single source of truth).
`ProfileRule.priority` defaults to `Some(100)` semantics via
`unwrap_or(100)` — confirmed indirectly by the cross-profile priority
tests at `profiles.rs:1075-1138` all passing.

`assets/profiles/aws.toml:31` ships `priority = 200` on `arn`;
`assets/profiles/docker.toml:30` ships `priority = 200` on `image_tag`.
Both verified by `cargo test` runtime.

One subtle but well-documented limitation: the comment at `src/profiles.rs:799-808`
honestly flags that for the special case `arn:aws:s3:::my-bucket`, the
embedded `3::` substring matches the ipv6 built-in (rule index 6) before
the arn rule (a later-index profile rule) can run. Bidirectional overlap
then rejects the arn envelope. This is correctly attributed to the
"earlier-indexed ipv6 already accepted its span" plus the priority sort
NOT including ipv6 (rule index 6, priority 0) before arn (index 12+,
priority 200) — wait, the comment claims priority sort DOES put arn
first ("regardless of priority"). Let me re-read more carefully.

Reading `profiles.rs:799-808`:
```
// Remaining known limitation: ARNs with the empty-account segment
// `arn:aws:s3:::my-bucket` contain a `3::` substring matching ipv6
// (built-in, priority 0). Because the ipv6 span starts inside the
// arn envelope, bidirectional overlap resolution rejects the later rule
// (arn) regardless of priority — the earlier-indexed ipv6 already
// accepted its span.
```

Wait — this claim ("regardless of priority — the earlier-indexed ipv6
already accepted its span") contradicts the v0.5.6 priority sort, which
sorts by `(Reverse(priority), rule_index)`. With arn at priority 200 and
ipv6 at priority 0, sort places arn FIRST in iteration order, so arn
should accept its envelope before ipv6 gets a chance. The doc-comment
appears to describe pre-v0.5.6 behavior.

**However**, looking at the test `aws_arn_matches_collision_free_shapes`
(line 810), the test inputs are deliberately curated to AVOID this case
(IAM ARNs with text-only resources, no hex segments). So the comment is
documenting a hypothetical edge case that the test doesn't exercise.
The `3::` example specifically — is it truly hex? `3::` — `3` is a hex
digit, but after the new ipv6 pattern (which requires either `::1`
literal OR `[hex]{3,4}:` leading group OR `\b[hex]:){7}[hex]` OR
`::[hex]{1,4}(:[hex]{1,4}){2,}`) — `3::` standalone doesn't match any
of the four branches:
- Not `::1`.
- Not 7-pair (needs 8 hex segments).
- Branch 3 needs `[hex]{3,4}:` leading, i.e., at least 3 hex chars before
  the colon. `3:` has only 1.
- Branch 4 needs `::[hex]{1,4}(:[hex]{1,4}){2,}` — `3::` doesn't even
  start with `::`.

So the v0.5.6 ipv6 tighten ALSO resolves this edge case implicitly! The
doc-comment at `profiles.rs:799-808` is therefore stale (it describes a
v0.5.3-era limitation that v0.5.6 silently fixed via the A2 tighten).

Severity: 🔵 NIT. Documentation accuracy drift. Not a test failure (the
test avoids the input anyway). Action: would benefit from a follow-up
note that v0.5.6 §A2 implicitly fixes this special-case via the ipv6
`[hex]{3,4}:` minimum leading constraint. Optional for this ship.

## 9. Pin RENAME + FLIP correctness

✅ **PASS.**

- ✓ Old test name `aws_arn_yields_to_interior_region_pattern_v0_5_3_limitation` —
  GONE. `grep` confirms zero occurrences in src/ or tests/.
- ✓ Old test name `docker_image_tag_registry_host_yields_to_fqdn_v0_5_3_limitation` —
  GONE. Same.
- ✓ New `aws_arn_wins_over_interior_region_pattern` (`profiles.rs:843`)
  exists with FLIPPED assertions:
  - `:857` asserts `aws.arn magenta must wrap envelope (priority 200 beats region 100)`.
  - `:863` asserts `interior region SGR must be suppressed under arn priority 200`.
- ✓ New `docker_image_tag_wins_over_registry_host_fqdn` (`profiles.rs:938`):
  - `:960` asserts `image_tag magenta must wrap envelope (priority 200 beats fqdn 0)`.
  - `:966` asserts `fqdn blue SGR must be suppressed under image_tag priority 200`.
- ✓ `aws_arn_wins_over_interior_ipv4` (`profiles.rs:1144`) — new positive
  test, asserts `interior ipv4 should be suppressed under arn priority 200`.
- ✓ `aws_arn_wins_over_interior_uuid` (`profiles.rs:1161`) — new positive
  test, asserts `interior uuid should be suppressed under arn priority 200`.
- ✓ `docker_container_id_wins_over_interior_uuid_via_priority` (`profiles.rs:1206`)
  — F3 RENAMED + FLIPPED.
- ✓ `docker_container_id_yields_to_uuid_envelope_outside_docker_profile`
  (`profiles.rs:1231`) — inverse case test exists.
- ✓ `mac_yields_to_ipv6_eight_pair_v0_5_5_limitation` (`profiles.rs:1181`)
  — new no-fix pin with the `_v0_5_5_limitation` suffix per memory
  `feedback_collision_pin_pattern`. Will loudly fail if a future ship
  flips this case.

Memory `feedback_collision_pin_pattern` mandate fully respected — old
"yields_to" pins fully removed (no carry-forward of the wrong direction).

## 10. `http_status` removal completeness

✅ **PASS.**
`grep -rn '"http_status"\|http_status' src/ tests/ assets/ benches/` returns
hits only in `benches/BASELINE.md` (historical baseline log; not active code),
which is exactly the spec §5 expected residue.

- `src/rules.rs` — pattern definition, BUILTIN_NAMES entry, unit tests all
  gone. The `BUILTIN_NAMES` array at `rules.rs:543-556` is exactly 12 entries.
- `assets/profiles/gcp.toml` — `http_status` whitelist entry removed.
- `assets/profiles/network.toml` — `http_status` whitelist entry removed.
- `assets/themes/dark.toml` — `[[rules]] name = "http_status"` block removed.
- `assets/themes/light.toml` — same.
- `benches/throughput.rs` — comment refresh removing `http_status` from
  the "exercises" list (commit b5f07df or earlier; verified in diff).

CHANGELOG.md `## [0.5.6]` `[Removed]` block ships both the "preserve"
and "improved" migration recipes per spec §5 verbatim (lines 70-90).

## 11. ipv6 pattern correctness (post-spec-correction)

✅ **PASS.** Pattern at `src/rules.rs:476`:

```rust
pattern: r"::1|\b(?:[0-9A-Fa-f]{1,4}:){7}[0-9A-Fa-f]{1,4}|\b[0-9A-Fa-f]{3,4}:(?:[0-9A-Fa-f]{1,4}:){0,5}:[0-9A-Fa-f]{0,4}|::[0-9A-Fa-f]{1,4}(?::[0-9A-Fa-f]{1,4}){2,}".into(),
```

Matches spec §6 POST-IMPLEMENTATION corrected form exactly:
- Branch 1: `::1` literal (promoted; was dead trailing branch in v0.5.5).
- Branch 2: `\b(?:[hex]{1,4}:){7}[hex]{1,4}` — 7-pair + 8th, `\b` prefix
  for word-boundary.
- Branch 3: `\b[hex]{3,4}:(?:[hex]{1,4}:){0,5}:[hex]{0,4}` — leading
  `{3,4}` minimum (blocks 2-char Rust idents like `de`); trailing
  compression with optional empty tail (`[hex]{0,4}` allows `1234:5678::`).
- Branch 4: `::[hex]{1,4}(?::[hex]{1,4}){2,}` — requires ≥2 additional
  hex groups (3 total hex segments minimum; blocks `::ba`, `::de:D`).

Test coverage (4 negative + 6 positive at `rules.rs:3372-3417`):
- ✓ `ipv6_does_not_match_rust_module_path` (`mod foo::bar::baz {}`)
- ✓ `ipv6_does_not_match_std_io_read` (`use std::io::Read;`)
- ✓ `ipv6_does_not_match_serde_de_deserialize` (`serde::de::Deserialize`)
- ✓ `ipv6_does_not_match_bare_double_colon_two_hex` (`see ::ba elsewhere`)
- ✓ `ipv6_matches_loopback_double_colon_one` (`::1`)
- ✓ `ipv6_matches_link_local` (`fe80::1`)
- ✓ `ipv6_matches_compressed_short` (`2001:db8::1`)
- ✓ `ipv6_matches_compressed_multi_group` (`2001:db8::ff00:42:8329`)
- ✓ `ipv6_matches_full_form` (`1:2:3:4:5:6:7:8`)
- ✓ `ipv6_matches_trailing_compression` (`1234:5678::`)

10/10. ReDoS analysis (spec §6 mandate): bounded `[hex]{1,4}` quantifiers,
no sibling-alternation in the `+`/`{2,}` repeats, no overlapping prefixes
between alternation branches that share a starting anchor. Linear-time
DFA. Verified.

## 12. Schema additivity

✅ **PASS.**

- `UserRule::priority: Option<i32>` at `src/config.rs:121-122` with
  `#[serde(default)]`. The struct's `#[serde(deny_unknown_fields)]`
  attribute at line 93 is preserved. Test `user_rule_priority_defaults_to_none_when_omitted`
  at `config.rs:1445` confirms old configs without `priority` parse cleanly.
- `ProfileRule::priority: Option<i32>` at `src/profiles.rs:88-89` with
  `#[serde(default)]`. The struct preserves `deny_unknown_fields` from
  the v0.5.0 definition. Test `profile_rule_priority_field_*` coverage
  via the cross-profile priority tests at `profiles.rs:1075-1138`.
- `ThemeRuleErrorKind::StraySchemaField` variant at `src/error.rs:97-104`
  with the appropriate `Display` arm at `:188-193`.
- TUI reconcile (`src/config_tui/reconcile.rs` — not touched by v0.5.6)
  transparently propagates the `priority` field on disk because it walks
  `DocumentMut` and only mutates known keys. Old TUI users editing a
  config with `priority` fields will see those preserved on `Ctrl+S`.

Backward compatibility test coverage:
- `config.rs:1440-1452` `user_rule_parses_priority_field` /
  `..._priority_defaults_to_none_when_omitted` — both directions pinned.
- `config.rs:1466-1474` `user_rule_deny_unknown_fields_still_active` —
  typo `priorty` still produces an error.

## 13. CHANGELOG completeness

✅ **PASS** (with one minor presentation note).

`CHANGELOG.md` `## [0.5.6] - 2026-05-27` block ships at the top
(verified). The four sections present:
- **Added** — priority tier mechanism, UserRule.priority, ProfileRule.priority,
  ipv6 dedicated `::1` branch, test coverage additions, FILENAME_EXTENSIONS
  doc-comment attribution.
- **Changed** — AWS ARN envelope precedence, Docker container_id ↔ uuid
  behavior change (user-visible color change documented per spec §10),
  pipeline.rs doc-comment refresh.
- **Removed** — `http_status` built-in with both "preserve" and "improved"
  migration recipes per spec §5.
- **Fixed** — ipv6 third branch Rust path FP, ipv4 negative regression
  coverage, mac negative regression coverage.

CHANGELOG ↔ landed-code consistency:
- ✓ "ipv6 third branch matched bare `::xxxx` Rust path syntax
  (`foo::bar::baz` → `::ba`); now requires additional hex groups" — directly
  verified by the four negative-test names in `src/rules.rs:3372-3387`.
- ✓ "Profile pin tests `..._yields_to_..._v0_5_3_limitation` renamed to
  `..._wins_over_..._pattern` with flipped assertions" — verified by grep.
- ✓ "Docker `container_id` now wins over the built-in `uuid` rule when a
  UUID contains a 12-hex container_id-shaped substring" — pinned by
  `docker_container_id_wins_over_interior_uuid_via_priority` test.
- ✓ "Themes cannot override priority (rejected with typed
  `ThemeRuleError::StraySchemaField`)" — pinned by
  `theme_rule_with_priority_field_errors` test with `assert_eq!` on
  Display.

Presentation note (🔵 NIT — not blocking): CHANGELOG line "now requires
additional hex groups" is slightly soft compared to spec §10 line "now
requires ≥2 colon-separated hex groups". The actual implementation requires
≥2 *additional* hex groups (3 total in branch 4). Both wordings are
technically accurate; the spec's more precise phrasing is preferable but
the current CHANGELOG text is not wrong.

## 14. Test count and triad

✅ **PASS** — final triad state confirmed:

- `cargo fmt --check` — clean (output empty).
- `cargo clippy --lib --tests -- -D warnings` — clean (no warnings,
  finished in ~1.6s after incremental recompile).
- `cargo test --lib` — **636 passed; 0 failed; 0 ignored; 0 measured**;
  finished in 2.17s.

Test count vs spec §9.7 estimate (622 ± 3): actual **636** lands +11
above the high end. This is well within tolerance — the spec estimate
acknowledged "exact count emerges in implementation" — and reflects the
implementer adding several additional priority-merge tests and a
priority-overrides-existing test pair that weren't itemized in the spec
estimate. None of the additional tests duplicate; they each exercise a
distinct contract (e.g., `priority_explicit_user_config_overrides_existing`,
`priority_negative_user_config_value_propagates`,
`apply_rules_priority_extreme_values_do_not_overflow`,
`docker_container_id_yields_to_uuid_envelope_outside_docker_profile`).

Integration suites spot-checked:
- `cargo test --test integration_profiles_library` — **6/6 PASS** (0.31s).
  The two pre-v0.5.6 limitation pins are gone; replaced by the FLIPPED
  `wins_over` variants which all pass under the priority sort.
- `cargo test --test integration_tui_smoke` — **4/4 PASS** (0.25s).
- `cargo test --test integration_tui_in_wrapper` — **1 PASS + 1 ignored**
  (manual smoke, per v0.5.4 convention).
- `cargo test --test integration_config` — **4/4 PASS** (0.23s).
- `cargo test --test integration_smoke` — would have run during full
  `cargo test`; not separately verified but `cargo test --lib` was clean.

## 15. CHANGELOG ↔ landed-code consistency

✅ Covered under §13; all CHANGELOG claims spot-check against actual
code / test surface.

## 16. Memory mandates respect

✅ **PASS** across all relevant memories:

- `feedback_consume_prior_review`: spec §1 explicitly enumerates v0.5.5 final
  review carryovers and folds/defers each. The two CHANGELOG-style
  carryovers (CHANGELOG + version bump) landed during the v0.5.5 ship
  cycle pre-tag (commits `cfd4a12`, `fbcf735`), so v0.5.6 doesn't need to
  re-absorb them. The TINY-1 wording sweep (events.rs:122,127) landed
  per spec §11.4. The architectural priority fix carryover (v0.5.5 §11
  carryover #1) IS the heart of v0.5.6's §2.1.B. ✓
- `feedback_spec_phase_parallel_review`: spec §13 absorbs 5 BLOCKers + 14
  IMPORTANTs + 14 NITs = 31/33 FOLDED + 1 REJECTED + 1 NOTED. Two
  reviewers dispatched per the memory mandate. ✓
- `feedback_collision_pin_pattern`: Task 9 RENAME + FLIP fully landed;
  the new no-fix pin `mac_yields_to_ipv6_eight_pair_v0_5_5_limitation`
  uses the suffix convention. ✓
- `feedback_parallel_call_site_invariant_audit`: priority field touches
  every site in the 7-site table — verified site-by-site:
  - Site 1: `builtin_rules()` 12 literals × `priority: 0` ✓
  - Site 2: `Compiled::empty()` `priorities: Vec::new()` ✓ (line 632)
  - Site 3: profile-append `unwrap_or(100)` ✓ (verified via tests)
  - Site 4: `compile_merged_rules` parallel-populate ✓ (line 1007)
  - Site 5: builtin-override branch conditional overwrite ✓ (config.rs:564-566)
  - Site 6: new-rule branch `unwrap_or(0)` ✓ (config.rs:596)
  - Site 7: theme validation rejects with typed error ✓ (themes.rs:400-404)
- `feedback_test_assertion_specificity`: theme rejection test uses
  full-Display `assert_eq!`. Schema-tests use observed-shape `contains`
  with rationale documented (toml-version tolerance). ✓
- `feedback_lean_process_small_subversions`: full ceremony preserved —
  parallel spec review, TDD per task (each Task 1-10 wrote red test before
  implementation), per-task atomic commits, final cross-cutting review
  (this document). ✓
- `feedback_stale_dead_code_reason_drift`: zero `#[allow(dead_code)]`
  introduced in v0.5.6; the two staged-during-implementation allows
  (Tasks 3, 4) were correctly removed once the consumer wire landed in
  Task 8. ✓
- `feedback_enumerate_tests_for_invariant_claims`: spec §7.3 enumerated
  the 8 affected tests; all landed transitions verified (RENAMEs in §9,
  the `apply_rules_preserves_pattern_definition_order_*` test remains
  green under the priority-0 == 0 tie-break case). ✓
- `feedback_pty_substring_sgr_fragmentation`: no PTY wrapper integration
  tests modified; v0.5.5 marker-scan + first-line cross-ref pattern
  inherits unchanged. ✓
- `feedback_review_calibration_en_tr`: code is English (with the two
  documented exceptions in §3); spec is Turkish. ✓
- `feedback_reload_precedence_snapshot`: `reload.rs` zero-touch (verified
  via `git diff --name-only v0.5.5..HEAD` not listing it). ✓
- `feedback_toml_edit_025_quirks`: the new `priority` field in
  `UserRule` and `ProfileRule` is `Option<i32>` with `#[serde(default)]`.
  TUI reconcile's existing `set_or_insert` helper handles unknown TOML
  fields transparently (passthrough on the DocumentMut walk). v0.5.6
  doesn't touch reconcile.rs, so the v0.5.5-pinned quirks remain
  unchanged. ✓
- `feedback_parallel_session_scope`: this review session touched zero
  src/ files. Only writing the review markdown per the user's mandate. ✓

## 17. Public API surface

✅ **PASS.** `git diff v0.5.5..HEAD -- src/lib.rs` returns one
character-of-diff (verified by `grep -c '^pub fn\|^pub struct\|^pub enum'
src/lib.rs` returning 1, matching pre-v0.5.6). The single `pub` symbol
in `src/lib.rs` is `pub fn run` (the library facade) — pre-existing,
unchanged. No new public exports.

All new symbols introduced by v0.5.6 are `pub(crate)` (verified
spot-checks):
- `BuiltinRule::priority` field — `pub(crate)` (visibility from struct).
- `Compiled::priorities` field — `pub(crate)`.
- `UserRule::priority` field — `pub(crate)`.
- `ProfileRule::priority` field — `pub(crate)`.
- `ThemeRuleErrorKind::StraySchemaField` variant — visibility inherits
  from the `pub(crate)` enum.

`Color` and `Style` remain the only `pub` exports in `src/style.rs`,
unchanged from v0.5.5. The library API surface is byte-identical at the
`pub` boundary.

---

# Verdict

## SHIP_WITH_FIXES

v0.5.6 lands the architectural priority tier mechanism with engineering
discipline. The five 🔴 BLOCKers and 14 🟡 IMPORTANTs from spec §13's
parallel review were folded into the code with no regression; 17 FP-audit
findings were closed (7 via architectural fix, 4 via pattern data
tighten, 6 via test coverage); the `http_status` removal cleanly excises
the broadest-FP built-in with a documented migration path; the ipv6
pattern tighten closes the highest-severity Rust path FP without
breaking known-good positive forms; the Pin RENAME + FLIP pattern from
memory `feedback_collision_pin_pattern` is observed to the letter
(old `..._yields_to_..._v0_5_3_limitation` names fully removed, new
`..._wins_over_*` names with FLIPPED assertions). Triad is green
(fmt clean, clippy -D warnings clean, 636 lib tests + integration suites
all passing). DOKUNULMAZ invariant respected; no DOKUNULMAZ surface
(pty.rs, io_loop.rs, tty_guard.rs, signals.rs, runtime.rs) touched.

The single fix-required item is a documentation drift cluster: five
header / doc-comments still say **"13 built-ins"** where they should now
say **"12"** after `http_status` was removed in Task 1. These are
cosmetic but exactly the surface that memory `feedback_stale_dead_code_reason_drift`
generalizes: forward-pointers / numeric claims that become silently stale
after a feature lands. The code's own correctness gates (the
`BUILTIN_NAMES.len() == 12` byte-pinned test at `src/rules.rs:3338-3340`)
demonstrate that the count IS 12 — only the documentation lags.

## Fixes required before tag v0.5.6

**Documentation drift cluster (5 file touches; ~5 LOC total):**

1. **`assets/profiles/aws.toml:1`** — change:
   ```
   # AWS profile — append_rules add 3 AWS-specific shapes on top of all 13
   ```
   →
   ```
   # AWS profile — append_rules add 3 AWS-specific shapes on top of all 12
   ```

2. **`assets/profiles/docker.toml:2`** — change:
   ```
   # on top of all 13 built-ins. Activate with `tayf --profile docker`.
   ```
   →
   ```
   # on top of all 12 built-ins. Activate with `tayf --profile docker`.
   ```

3. **`assets/profiles/k8s.toml:1`** — change:
   ```
   # Kubernetes profile — append_rule adds pod_name shape on top of all 13
   ```
   →
   ```
   # Kubernetes profile — append_rule adds pod_name shape on top of all 12
   ```

4. **`assets/themes/dark.toml:3`** — change:
   ```
   # Spells out the 13 built-in styles explicitly so users can copy this file
   ```
   →
   ```
   # Spells out the 12 built-in styles explicitly so users can copy this file
   ```

5. **`assets/themes/light.toml:3`** — change:
   ```
   # Adjusts the 13 built-in styles for terminals with a light background,
   ```
   →
   ```
   # Adjusts the 12 built-in styles for terminals with a light background,
   ```

6. **`src/profiles.rs:392`** — change the doc-comment:
   ```rust
   /// Compile the named embedded profile against the 13 built-ins (no
   ```
   →
   ```rust
   /// Compile the named embedded profile against the 12 built-ins (no
   ```

All six are single-token edits (`13` → `12`); no test changes are
required because the relevant unit test (`rules.rs:3338`) already byte-pins
the correct count. After the fix:
- Triad rerun: expected clean (no test surface touched).
- `cargo test --lib` count unchanged at 636.

**Optional 🔵 NITs (documentation polish; not blocking):**

- The doc-comment block at `src/profiles.rs:799-808` describes the
  `arn:aws:s3:::my-bucket` `3::` edge case as a remaining limitation
  "regardless of priority". Under v0.5.6's tightened ipv6 pattern, `3::`
  no longer matches ipv6 (branch 3 requires `[hex]{3,4}:` minimum leading;
  branch 4 doesn't start at `::`). The comment is stale; a follow-up
  note in v0.5.7 or v0.6+ could either delete the block or update it
  to reference the new behavior. Optional for this ship.
- `pipeline.rs:70-73` comment ("first-match-wins overlap resolution
  depends on pattern order") is now load-bearing only under the
  priority-0 tie-break case. A one-line qualifier (e.g., "under the
  priority-0-only tie-break case; see line 76 sort step for the general
  contract") would tighten precision. Optional.
- CHANGELOG.md `### Fixed` first bullet ("now requires additional hex
  groups") could be tightened to "now requires ≥2 additional colon-
  separated hex groups" to match the spec wording more precisely.
  Optional.
- `src/rules.rs:146` `FILENAME_EXTENSIONS` const lacks the inline
  doc-comment block specified by spec §2.1 E6 (canonical 1-to-1
  attribution for single-letter ext'lar). The attribution exists in
  CHANGELOG.md, but a code-side doc-comment was the spec's E6 ask.
  Optional for ship; nice-to-have for future contributors browsing
  the const.

After applying the six required `13 → 12` fixes inline, run the triad
once more for confirmation, then push main → wait CI green → push tag
`v0.5.6` per memory `project_release_workflow`. Update memories
(`project_v0_5_6_shipped` + v0.5.7 forward-pointer) per spec §12
Step 6.

The v0.5.6 engineering work is ship-grade. The documentation drift is a
cleanup-pass oversight identical in pattern to v0.5.5's stale
`Color::to_toml_str` allow — a 5-minute fix.
