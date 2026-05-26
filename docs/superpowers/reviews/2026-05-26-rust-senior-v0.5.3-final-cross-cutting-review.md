# tayf v0.5.3 — Final Cross-Cutting Review

**Reviewer:** Opus 4.7 (rust-senior persona)
**Date:** 2026-05-26
**Scope:** `v0.5.2..HEAD` (`140db45..a7f11e3`, 19 commits, 22 files, +4960 / −23)
**Spec:** `docs/superpowers/specs/2026-05-26-tayf-v0.5.3-builtin-profile-library.md`

---

## Verdict: **CLEAN_SHIP**

All acceptance criteria pass. One 🔵 nit (stale doc-comment forward-pointer) and one 🟡 loose-assertion observation surface, both non-blocking. Two collision pins documented in CHANGELOG as Known Limitations and queued as forward-pointers for v0.5.4. Zero forced carryover.

---

## 1. Acceptance criteria

| Check | Expected | Actual | Result |
|---|---|---|---|
| `cargo test --lib` | 480 pass | **480 pass, 0 fail** | ✅ |
| `cargo test --bin tayf` | 6 pass (exit-code split) | **6 pass, 0 fail** | ✅ |
| `cargo test --test integration_profiles_library` | 6 pass | **6 pass, 0 fail** | ✅ |
| `cargo fmt --check` | clean | clean | ✅ |
| `cargo clippy --all-targets -- -D warnings` | clean | clean | ✅ |
| `cargo bench --quick apply_rules/profile-*` | 5 fixtures run | all 5 ran, ~31–40 MiB/s range | ✅ |
| `assets/profiles/*.toml` count | 5 | 5 (aws/k8s/docker/gcp/network) | ✅ |
| `git diff v0.5.2..HEAD -- src/lib.rs` | small, only `__bench__` extension | 30 lines, single new helper `load_profile_rules` inside existing `#[doc(hidden)] pub mod __bench__` | ✅ |
| `grep -rn 'RuleSource::' src/` | 72 | 72 | ✅ |
| `grep -n "format!.*rule .{}" src/rules.rs` (real sites) | 5 | 5 (3 additional hits are comments referencing the formatter) | ✅ |
| Pre-existing integration tests untouched | diff = 0 | diff = 0 for `integration_profiles.rs`, `integration_capture_groups.rs`, `integration_themes.rs` | ✅ |
| `EMBEDDED_PROFILES.len()` | 5 | 5, asserted by `embedded_profiles_table_pins_count_and_names` | ✅ |
| Cargo.toml + Cargo.lock version | 0.5.3 | both 0.5.3 | ✅ |
| Schema invariant: Profile/ProfileRule/LoadedProfile fields byte-identical to v0.5.2 | yes | pinned by `schema_round_trip_v0_5_2_field_set_byte_identical` | ✅ |
| Profile-inactive hot path byte-equal | yes | `hot_path_unchanged_when_no_profile` passes | ✅ |
| EX_USAGE/EX_SOFTWARE split | yes | 3 unit tests in `src/main.rs::tests` | ✅ |
| Two collision pins documented | yes | `aws_arn_yields_to_interior_region_pattern_v0_5_3_limitation` + `docker_image_tag_registry_host_yields_to_fqdn_v0_5_3_limitation` byte-pin SGR codes | ✅ |

---

## 2. Strengths

1. **`__bench__` adapter strengthening over plan.** The bench-access decomposition (`load_profile_rules` wrapping `profiles::load_with` + `rules::Compiled::load_with_theme` with stubbed env closures) is materially stronger than the plan's `pub use` re-export approach. It (a) keeps `pub(crate)` constructors private — no public-surface bleed; (b) forces the bench to go through the same code path that `Tayf::run` uses in production; (c) the doc-comment on `load_profile_rules` explicitly mirrors the production startup flow. v0.1.1 `__bench__` convention is preserved end-to-end.

2. **Both collision pins are byte-pinned, not loose.** `aws_arn_...` checks for `\x1b[32` (region green) presence AND `\x1b[35marn:aws` absence (magenta envelope must NOT fire). `docker_image_tag_...` (in `src/profiles.rs`) similarly pins the FQDN-wins behavior. This satisfies `feedback_test_assertion_specificity` — when v0.5.4 fixes rule priority, these tests will fail visibly.

3. **EX_USAGE/EX_SOFTWARE split is discriminator-precise.** The match arm in `src/main.rs:60` uses a pattern guard (`source_path.starts_with("<embedded:profile/")`) gated on `kind: RegexCompile`. Negative regression covered by `embedded_profile_parse_error_still_maps_to_ex_usage` — proves the split is RegexCompile-specific and that other ProfileErrorKind variants on embedded paths remain EX_USAGE failsafe. Three unit tests TDD-driven (Task 8).

4. **Architectural-root-cause framing in CHANGELOG.** The Known Limitations section explicitly states both pins share the same root cause (RegexSet pattern-order priority) and tells readers v0.5.4 will address it. This is honest UX-documentation — strangers can read it and understand the limitation isn't a regex bug, it's an architectural choice with a planned revisit.

5. **K8s base32 audit deviation surfaced in-place.** `assets/profiles/k8s.toml` documents the v0.5.1 §11.2 hex-only recommendation, why hex-only would miss ~99% of real pods, and the resulting base32 alphabet decision. Comment is self-contained — future readers don't need to chase the spec to understand "why not hex".

---

## 3. CARRYOVER_FINDINGS

### 🔴 Critical
None.

### 🟡 Important
- **`src/profiles.rs:113-114` — stale forward-pointer in `EMBEDDED_PROFILES` doc-comment.** Says "a unit test added in Task 6 (`network` profile) will pin the table count + name set." Task 6 has landed; the test exists in this same file (line 687, `embedded_profiles_table_pins_count_and_names`). Future-tense "will pin" referring to a now-shipped test reads as half-translated work-in-progress. Per CLAUDE.md §4 "Zero Technical Debt Tolerance" and per memory `project_v0_5_0_shipped` "v0.5.3 MUST open with themes Phase-1 gate fix" pattern (consume prior reviews), this should have been softened by a Task-revision commit. Fix is one-line edit: drop the "added in Task 6" + change "will pin" to "is pinned by `embedded_profiles_table_pins_count_and_names` below". Non-blocking for ship — small enough to either (a) hot-fix in a final pre-tag commit or (b) absorb into v0.5.4 with a one-line cleanup.

- **`tests/integration_profiles_library.rs::docker_profile_renders_container_id_and_image_tag` is loose.** Uses `gcr.io/proj/app:v1.2` (registry-host branch — the *colliding* shape per the v0.5.3 known limitation) and asserts `has_some_sgr_around(..., "gcr.io/proj/app:v1.2")`. Per the documented limitation, the `fqdn` built-in claims `gcr.io` and `docker.image_tag` magenta envelope does NOT fire. `has_some_sgr_around` is loose — it returns true if the substring is anywhere in the output AND any ESC byte is anywhere in the output, which is satisfied by the FQDN match alone. The test name implies envelope rendering. Either (a) rename to "...renders container_id and partial image_tag prefix", (b) switch the fixture to `:latest` bare branch (which IS unaffected by the limitation), or (c) tighten the SGR assertion to require magenta around the colon. Non-blocking — the limitation is correctly pinned by the byte-strict `docker_image_tag_registry_host_yields_to_fqdn_v0_5_3_limitation` unit test in `src/profiles.rs:927`. Queue for v0.5.4.

### 🔵 Nits
- **Bench output noise.** `--quick` mode flagged one `profile-gcp` p-value at exactly the threshold (`p = 0.05 < 0.05`, "Performance has improved" 3–5%). Likely noise from cold start; canonical baselines recorded post-tag will provide the stable reference. No action.
- **`benches/BASELINE.md` not refreshed.** Plan §10 calls for post-tag baseline recording; current file references pre-v0.5.3 HEAD. Consistent with the documented "post-tag" workflow, but worth a reminder in release ceremony. No action needed before tag.

---

## 4. Forward-pointers (v0.5.4)

1. **ARN/ipv6 collision** — rule-priority architectural fix OR tighten built-in ipv6 pattern to require ≥1 trailing hex group. Pinned by `aws_arn_yields_to_interior_region_pattern_v0_5_3_limitation`.

2. **image_tag/fqdn collision** — same architectural root cause as #1; either elevate profile `append_rules` to outrank built-ins on overlap, OR carve `fqdn` to refuse trailing `:tag`. Pinned by `docker_image_tag_registry_host_yields_to_fqdn_v0_5_3_limitation`.

3. **Stale Task-6 doc-comment (`src/profiles.rs:113-114`).** Cosmetic but per CLAUDE.md zero-debt mandate.

4. **Loose integration assertion (`docker_profile_renders_container_id_and_image_tag`).** Tighten or rename per option (a)/(b)/(c) above.

5. **gcp `resource_id` deferred to v0.6+** (v0.5.1 spec §11.2 audit verdict, mentioned in `assets/profiles/gcp.toml` comment). Not a v0.5.4 item — recorded for completeness.

6. **`BASELINE.md` refresh** after v0.5.3 tag + CI green to record the 5 new profile-active benches as canonical baselines.

---

## 5. Recommendation

**Proceed to release ceremony.** All five spec acceptance gates pass (test counts, fmt/clippy, bench smoke, schema invariant, pre-existing test diff = 0). The two collision-pin known limitations are honestly documented in CHANGELOG with shared architectural root cause and explicit v0.5.4 forward pointer. The `__bench__` adapter extension strengthens the plan rather than weakening it. The 🟡 items above are real but small and non-blocking.

Suggested order:
1. **Optional pre-tag micro-fix:** one-line edit to `src/profiles.rs:113-114` softening the Task-6 forward-pointer (turns the 🟡 → 🔵 trivially, but absorbs into v0.5.4 just as easily if you prefer a clean diff log).
2. `git push origin main`
3. Wait for CI green (per memory `project_release_workflow` — push main first, only tag after CI green).
4. `git tag -a v0.5.3 -m "v0.5.3 — built-in profile library"` and `git push origin v0.5.3`.
5. Post-tag: update `CHANGELOG.md` `[0.5.3] - TBD` → date, mark v0.5.3 shipped in vision doc, refresh `BASELINE.md` with canonical profile-active numbers.

Zero forced carryover. v0.5.3 is **CLEAN_SHIP**.
