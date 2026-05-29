# tayf v0.7 spec — independent architecture review

**Spec under review:** `docs/superpowers/specs/2026-05-29-tayf-v0.7-design.md` rev1 (882 lines).
**Reviewer lens:** algorithm correctness, domain fit, integration with existing codebase
invariants, FP-audit methodology, architectural cohesion. (Rust idioms / public API surface
delegated to parallel reviewer.)
**Reviewer:** opus 4.7 senior, independent.

---

## 1. Verdict

🟡 **ABSORB CHANGES.** The spec is well-structured and the dogfood symmetry between Items 4/5
and Item 2 is genuinely elegant. But §3's per-element merge algorithm has at least three
under-specified or wrong cases, §4's LCS trace-back rule is mis-stated (will produce a
non-canonical edit script), §5's corpus methodology is missing two methodological holes the
audit doc itself warned about, and §6's snapshot coverage misses three modals that v0.6 just
shipped (NewPattern, EditRegex, ColorPicker). None of these block the ship plan — they're
fold-or-defer absorbable in a rev2 patch.

---

## 2. CRITICAL findings

### C-1. §3.2 — convergent-deletion case explicitly omitted from the enumerated table.

The §3.2 algorithm enumerates 11 (be, oe, te) cases but misses **`be present, oe absent, te
absent`** when stated cleanly. The spec has it (line 137 "be present, oe absent, te absent →
DROP"), but the WORDING merges two distinct cases under "DROP (convergent deletion)":

1. `be=X, oe=absent, te=absent` — convergent deletion.
2. `be=X, oe=absent, te=X` (line 138) — "ours deleted, theirs untouched-or-restored" → DROP.

Case 2 is **wrong**. If theirs equals base (i.e. theirs DID NOT touch the element), and ours
deleted it, the merge_table contract everywhere else in tayf says "take ours" (only ours
changed). Dropping is correct here. But the SYMMETRIC case on line 139 `be=X, oe=X,
te=absent, oe==be → DROP (theirs deleted)` is also correct — and both match the existing
top-level merge_table convention. **Fine.**

The real miss is the symmetric pair the spec did NOT enumerate: **`be=X, oe=Y (modified),
te=absent`** where `oe != be`. The spec covers this on line 152 ("ours modified what theirs
deleted → element-level conflict"), but the dual `be=X, oe=absent, te=Y` where `te != be`
appears only as line 150-151 ("theirs modified what ours deleted"). So both ARE there. **OK
on re-read.**

However, **`be=absent, oe=X, te=Y` where `oe != te`** (line 148) is enumerated but the
algorithm says "element-level conflict, shape: Block, path = [...key, name]". Question: what
goes in `base_value` for this conflict's render? It must be `"(absent)"` (matching
render_item line 305). The spec doesn't pin this. It matters because the conflict-list UI
renders `base_value` — if implementer fills it with empty-string instead, the row reads
"ours:X theirs:Y base:" which is ambiguous between "absent" and "empty string". **Pin in
spec §3.2: "absent sides render via render_item(None) = `\"(absent)\"`, identical to the
v0.6.2 contract."**

→ Recommended action: §3.2 step 4 add explicit "absent-side render contract" sub-bullet.

### C-2. §3 — ordering invariant breaks under symmetric ours-only + theirs-only with name collision NOT covered.

§3.2 step 3 says:

> order = base_names + (ours-only in ours order) + (theirs-only in theirs order)

Consider this case the spec doesn't cover:

- base = `[a]`
- ours = `[a, x, y]` (appended two)
- theirs = `[a, y, x]` (appended same two, opposite order)

By §3.2 step 3: `order = [a] + ours-only [x, y] + theirs-only []` = `[a, x, y]`. (Symmetric
swap: `order = [a, y, x]` if we run with sides flipped.) Both x and y are convergent
insertions (both sides added them with identical content presumably), so step 4 produces
`push x, push y`. **Auto-merge succeeds.**

But the ORDER differs from what theirs has on disk. After tayf writes back, the disk file
gets `[a, x, y]` even though theirs (the version that just landed on disk) was `[a, y, x]`.
That's a silent reorder of a third-party edit. Whether that matters semantically depends on
whether `[[rules]]` order matters — and it DOES (first-match-wins iteration order, rules.rs
RegexSet semantics depend on definition order, per the audit doc §0.1).

So convergent-insertions-in-different-order is a **silent semantic conflict**. The spec
should either (a) detect this and surface it as a conflict, or (b) explicitly document that
ours-order wins and the user is responsible for noticing the reorder. Option (a) is
defensible: if convergent set is identical but order differs, treat as element-level
conflict path = `["rules"]` shape = Block.

→ Recommended action: add §3.2 step 3a "**order-divergence guard**: if `ours-only` and
`theirs-only` are both empty and base_names != ours_names != theirs_names as ordered lists,
emit whole-array conflict (Block, is_array_block: true)."

### C-3. §4.2 — LCS trace-back rule produces non-canonical edit scripts.

Spec line 259: `dp[i-1][j] >= dp[i][j-1] → Remove(old[i-1]), i--`.

The `>=` here means TIES go to Remove (i.e. emit `-` lines before `+` lines on a
modification). This is a valid LCS recovery, but it's NOT what most diff tools do.
Hunt-McIlroy canonical and `diff -u` use `>=` to prefer ADD on ties (i.e. emit `+` before
`-`), giving a different but equally-minimal script. This isn't a correctness bug per se,
but it has user-visible consequences:

Test §4.5 #4 pins `"a\nb\n" → "a\nc\n"` ≡ `"  a\n- b\n+ c\n"` (Remove-then-Add). Fine. But
test #6 pins `"a\nb\nc\n" → "x\nb\ny\n"` to "LCS-trace order (b survives)". The actual
output depends on tie-handling. With Remove-on-tie: `- a\n+ x\n  b\n- c\n+ y\n`. With
Add-on-tie: `+ x\n- a\n  b\n+ y\n- c\n`. The spec doesn't pin which, but test #6's loose
phrasing satisfies both. Memory `feedback_test_assertion_specificity` mandates exact
strings here.

More importantly: there's a subtle BUG in the trace-back when **both** old[i-1] != new[j-1]
AND `dp[i-1][j] == dp[i][j-1]`. With `>=`, the algorithm always takes Remove. That's fine,
but at the end of trace-back when `i > 0, j == 0`, the loop must still emit Removes until
`i == 0`. The spec's trace-back rule (line 257-260) doesn't explicitly handle the boundary
conditions: what if `i == 0` and `j > 0`? The `dp[i-1][j]` indexing is invalid. Same for
`j == 0, i > 0`.

→ Recommended action: §4.2 add explicit boundary handling: "while i > 0 OR j > 0: if i == 0
→ Add, j--; elif j == 0 → Remove, i--; elif old[i-1] == new[j-1] → Same; else if
dp[i-1][j] >= dp[i][j-1] → Remove; else → Add". And §4.5 should pin test #6 to an exact
output string.

### C-4. §3.3 — `write_to_path` AoT extension breaks for element names equal to `(absent)`, empty string, or numeric-coercible.

Spec §3.3 says `write_to_path(doc, ["rules", "log_level", "pattern"], source)` descends by
matching the AoT element with `name == "log_level"`. Now consider:

- An element with `name = ""`. The path `["rules", "", "pattern"]` is well-defined? Yes,
  but the `key_path_display` rendering (`join(".")`) would be `rules..pattern`, ambiguous
  with a typo.
- An element with `name = "0"` (string-form). String identity works.
- An element where the conflict path was generated when `name` was `"x"` but by the time
  `write_to_path` runs, the source side has had that element renamed (because we're picking
  Theirs and theirs has `"y"` not `"x"`). Then descend by name `"x"` finds the source's
  `"x"` element which is the BASE shape, not the renamed one.

The last case is the genuine algorithm bug. Consider:

- base: `[[rules]] name="x" pattern="A"`
- ours: `[[rules]] name="x" pattern="B"` (modified pattern only)
- theirs: `[[rules]] name="y" pattern="A"` (renamed only)

By §3.2 step 1-2, base_names=[x], ours_names=[x], theirs_names=[y]. Identity collect gives
distinct names. Step 3 order: [x] + [] (ours-only — x is in base) + [y] (theirs-only) =
[x, y]. Step 4 for "x": be=Some, oe=Some, te=None. Line 139 says `te=absent, oe==be → DROP`
but `oe != be` (B vs A). So this is "ours modified what theirs deleted" → element-level
conflict at path `["rules", "x"]`. For "y": be=None, oe=None, te=Some → push te (theirs
insertion).

Now the user picks Theirs at `["rules", "x"]`. `write_to_path(doc, ["rules", "x"],
theirs_source)` descends `theirs.rules.x` — but theirs has no element named `"x"`! The
new helper `descend_aot_by_name` returns `AotElementMissing`. **This is the right error**,
but it bubbles up as a "merge apply failed" toast at events.rs:721, which is misleading: the
correct semantic is "theirs deleted this element, so picking Theirs means delete". The spec
should pin that `Theirs` choice when theirs-side does not have the path needs to be
translated into a `remove` on auto_merged.

→ Recommended action: §3.3 add explicit case: "when write_to_path's source-side descent
fails with AotElementMissing AND the merge_three_way conflict arm classified this as a
delete-modify, the apply layer (events.rs:apply_conflict_selections) must interpret
Theirs/Ours as 'side deleted → remove from dest' rather than surface AotElementMissing".
Pin test §3.4 #6 currently asserts Block conflict but doesn't follow through to the
apply-layer behavior.

### C-5. §5.3 — `tayf::testing::match_rule` harness measures wrong thing for FP audit.

The harness calls `match_rule(rule_name, input)` and asserts FP if a single-rule match
fires on a NEG. This measures **per-rule regex behavior in isolation**, ignoring the
production pipeline's:

1. **Priority resolution** — v0.5.6 §F-3 priority field means container_id (priority 100)
   wins over uuid (priority 0) inside docker profile. A NEG like
   `abc12345-1234-1234-1234-123456789012` would FP on `container_id` alone in the harness,
   but is correctly suppressed in production by uuid's envelope.
2. **First-match-wins overlap** — pipeline.rs accepts each non-overlapping match. So
   filename `pkg.go` (audit C-8) firing in isolation is technically a FP against fqdn, but
   filename's earlier match consumes the span and fqdn is suppressed. The harness reports
   filename's FP, but production sees only one styled span.
3. **Profile gating** — `aws.region` only exists in aws profile. Harness must specify
   profile context for each corpus item; spec §5.3 doesn't.

This is exactly the methodology trap the audit doc itself called out in §0.2 ("the
production pipeline iterates rules in order and accepts each non-overlapping match. So the
winner-by-lowest-index is the right call for **overlapping** spans"). The harness as
specified replicates the audit's per-rule view but the karar enum (§5.4) talks about
**production-level FP**, mixing two ontologies.

→ Recommended action: §5.3 add a second helper `tayf::testing::pipeline_spans(input,
profile) -> Vec<(rule_name, span)>` that runs the full pipeline (priority sort, overlap
resolution, profile gating). Corpus assertions then check `pipeline_spans` returns the
expected set, not a single-rule call. The per-rule helper is OK as a debugging primitive
but the karar measurement MUST use the pipeline view.

---

## 3. IMPORTANT findings

### I-1. §3.5 — same-side duplicate name fallback semantics.

Spec §3.5 says toml grammar allows duplicate `name` per AoT and merge falls back to
whole-array. Audit: `apply_user_rules` in config.rs late-validates and rejects duplicate
names (config_tui spec elsewhere documents this). So duplicate names only arise on the
THEIRS side when a malicious-or-broken on-disk config slipped past validation (e.g. hand
edit between TUI sessions). The spec's defensive "whole-array fallback" is defensible, but
should additionally surface a TOAST or error message — silently treating a malformed disk
file as "merge conflict, whole array" hides the actual problem.

Equally: what if duplicate occurs on OURS side? OURS comes from the TUI's pending-edit
projection — which goes through apply_user_rules at edit time. So OURS-side duplicates
SHOULD be impossible by construction. Pin that invariant: §3.5 add "ours-side duplicate
name is an internal invariant violation; debug_assert! it before the fallback path."

### I-2. §3.3 — `WriteToPathError::AotElementMissing` SemVer claim is suspect.

§14 risk table line 4 says `WriteToPathError` is `pub(crate)` after v0.6.3 demote, so no
SemVer break. Verify: `merge::WriteToPathError` is `pub` in current source (merge.rs:101).
Was it demoted in v0.6.3? Memory `project_v0_6_3_shipped` says `merge` module became
`pub(crate)`, but the error enum itself in the file is `pub` — its public visibility
follows the module visibility, so it IS `pub(crate)` transitively. **Confirmed safe.** But
the spec should cite the exact line, not the memory hand-wave.

### I-3. §3 — `is_array_block` semantic change audit incomplete.

Spec §3.2 says the field semantic narrows to "only true on fallback". Existing call sites:

- `widgets/conflict_list.rs:54` — renders `"  ⚠ array merge v0.7+"` suffix. Under new
  semantic, this suffix renders only on whole-array fallback. **Wording must change** —
  "v0.7+" is wrong (v0.7 is the version doing this) AND the array-merge claim is now
  misleading (per-element shipped). New copy: `"  ⚠ array-shape conflict (no name
  identity)"`. Spec §7.1 cleanup table misses this site.
- `events.rs:1378, 1414, 1485, 1532, 1585, 1593` — all test fixtures hand-constructing
  `KeyConflict` with `is_array_block: true` for synthetic ConflictList tests. Under new
  semantic, the tests now misrepresent what such conflicts look like. Audit each.
- Merge.rs:546 test `assert!(c.is_array_block)` is renamed by §3.4 #1 → flipped. OK.

→ Recommended action: §7.1 stale-comment table add row for `widgets/conflict_list.rs:54`
suffix string. Spec §3 add bullet "audit events.rs:1378+ test fixtures for new semantic
fitness."

### I-4. §6 — initial snapshot coverage misses 3 modals v0.6 just shipped.

The 7 snapshots in §6.3 cover: tabs (themes/rules/profiles), Edit modal, ConflictList,
SaveDiff Clean, Help. The `Modal` enum (app.rs:82-118) has these variants the spec does NOT
snapshot:

- `Modal::NewPattern { phase: Name | Regex | Style, ... }` — v0.6.2 3-phase wizard, shipped
  ~700 LOC. Render regressions here are exactly what snapshots catch.
- `Modal::EditRegex { rule_id, buffer, error }` — v0.6.2 D3, error-displaying modal.
- `Modal::ColorPicker(...)` — v0.6.1 binding extension. High visual surface area (grid of
  color swatches).
- `Modal::FullPreview` — v0.6 spec §12.4.
- `Modal::Confirm { msg, action }` — discard-and-reload prompt.
- `Modal::QuitWithUnsavedEdits` — v0.6.2 added explicit variant.
- `Modal::SampleSet` — present in enum.
- `Modal::Search` — present in enum.

Of these, NewPattern (3 phases × snapshot = 3 snaps), EditRegex (with and without error
banner = 2), and ColorPicker (default state = 1) are the MUST-have additions — 6 more
snapshots, all high-traffic visual surfaces. Without them, the snapshot infra ships with
known render-blindness on the highest-churn UI elements.

→ Recommended action: §6.3 expand to 13 snapshots OR explicitly document why these are
deferred. Don't ship a "render snapshot infra" without the modals that are MOST likely to
silently regress.

### I-5. §6.2 — snapshot helper hardcodes path and breaks parallel test execution.

Helper at line 535-540:

```rust
let abs_path = format!("{}/{snap_path}", env!("CARGO_MANIFEST_DIR"));
if std::env::var("UPDATE_SNAPSHOTS").is_ok() {
    std::fs::write(&abs_path, &rendered)...
```

Two issues:
1. `env!("CARGO_MANIFEST_DIR")` is the CRATE root at COMPILE time. For tests in
   sub-crates or workspace context this would resolve wrong — tayf is currently a
   single-crate workspace, fine, but pin the assumption.
2. `UPDATE_SNAPSHOTS=1 cargo test` runs all tests in parallel by default. If two tests
   regenerate two different snap files, they race on writes to the same DIRECTORY (and on
   process-level filesystem state). Each test writes its own file, so no actual race in
   practice — but writing inside `cargo test` workdir during a test run is generally
   discouraged because of permission/CI surprises. The bigger gotcha: the test that's
   updating snapshots returns early and PASSES, so a misconfigured CI run with
   `UPDATE_SNAPSHOTS=1` (e.g. accidentally exported) would silently mint snapshots and
   render all snapshot tests vacuous.

→ Recommended action: §6.2 add a CI guard: "snapshot tests refuse to run with
UPDATE_SNAPSHOTS=1 when `CI=true` env is set; this prevents silent gold regeneration in
CI". Also document that snapshots are LF-only (mentioned in §14 mitigations but not
asserted in helper code — `.gitattributes` is necessary but the helper should also strip
CR before write).

### I-6. §5.2 — corpus item enumeration drops audit §C-16 (URL trailing paren).

Audit doc §C-16 ("URL trailing paren not trimmed", Wikipedia-style) is in audit's "DEFER"
list (line 562-564 in audit). It IS pinned in tests (test `url_preserves_wikipedia_...`).
But it's NOT in §5.2's 7-item table. That's correct (it's pinned, not open) — but the spec
should explicitly say so. Currently §5.1 "v0.5.6 commit'leri MUST bundle ship etti" lists
C-16 as already-pinned, so it's defensible. **Marginal NIT; including for explicit
fold-or-defer clarity.**

A real miss: **audit §F-7** ("docker.container_id ↔ uuid interior overlap") was flagged
"Not tested" in v0.5.5 audit. v0.5.6 implemented the priority mechanism and added the pin
`docker_container_id_wins_over_interior_uuid_via_priority` (profiles.rs:1268). So this is
closed. The spec mentions C-12/13/14/B-1/B-2 closure but not F-7's status. Defensible
omission but worth a line in §5.1.

### I-7. §5.4 — karar enum does not enforce memory mandate on threshold.

Memory `feedback_builtin_pattern_bar`: "high-FP patterns go to user config". §5.4
documents this as "Memory mandate: Yüksek-FP (>%5 / total NEG) → TIGHTEN veya DEMOTE
zorunlu." But §5.4 lists three karar options with no hard-fail enforcement — implementer
could measure 20% FP and still choose KALSIN by writing it in the table. The pin tests
(EXPECTED_FP_*) lock the measurement but not the karar.

→ Recommended action: §5.4 add a test-side enforcement: `tests/audit_corpus.rs` asserts
`if (fp as f64 / nneg as f64) > 0.05 then karar != KALSIN`. This makes the mandate
machine-checked, not implementer-discretion.

### I-8. §7.1 — `git grep` enumeration is incomplete.

Spec §7.1 lists 3 sites for stale `v0.7+` cleanup. Actual grep run against current `main`
finds at least these additional hits:

- `src/config_tui/edit.rs:10-11` — module-level `#![allow(dead_code)]` reason refers to
  v0.7+ capture-group style slots.
- `src/config_tui/events.rs:944` — `Other tabs yield None — v0.7+ may extend (Themes /
  Profiles).` This is the `resolve_selected_rule_id` extension that §1.3 DEFER table moves
  to v0.8+ on community demand. The comment still says v0.7+ — needs REFRESH to v0.8+
  (with caveat per memory `feedback_stale_dead_code_reason_drift`).
- `src/config_tui/events.rs:975` — `not catalog-resolved (v0.7+)`. Same v0.8+ refresh.
- `src/config_tui/widgets/sample_set.rs:5` — `lands in v0.7+` — same v0.8+ refresh.

So the cleanup is **8 sites, not 3** — and 5 of them need REFRESH (not DELETE) to v0.8+
per memory mandate against stale forward-pointers. Memory
`feedback_stale_dead_code_reason_drift` explicitly says cleanup-pass must strip
phase-drifted annotations, and re-running clippy might surface new dead-code masks.

→ Recommended action: §7.1 expand table to 8 rows. Action per row: DELETE (3 spec-listed
sites — referenced behavior shipped) vs REFRESH to v0.8+ (5 deferred items).

### I-9. §8 — Item 5 → Item 2 dogfood dependency analysis is right; integration ordering missing test.

§8 says Item 5 (LCS) must land before Item 2 (snapshot) because the snapshot helper calls
`crate::config_tui::widgets::save_diff::build_diff` in the failure message (§6.2 line 552).
Implementer order is correct.

Concern: if Item 5 lands first, then Item 4 lands, then Item 2 lands — Item 4's tests
between are also unit tests that don't trigger snapshot machinery. So sequencing is fine.
But Item 2's snapshot golden-file generation (§6.5) happens manually — implementer runs
`UPDATE_SNAPSHOTS=1 cargo test ...`. The cross-cutting review at §8 step 6 then runs
against the committed goldens. If the LCS diff in step 6's failure messages produces
different output for a "diff'd snapshot" assertion vs. raw text comparison, the cross-
cutting review reads visually-misleading messages.

→ Minor — recommended action: §6.5 step 4 add explicit "verify the panic message LCS diff
output by intentionally desyncing one snapshot and inspecting the output is readable
before committing".

---

## 4. NIT findings

### N-1. §4.3 u16 LCS table comment is confusingly self-questioning.

Line 282-283: `// u16 yeterli: MAX_DP_CELLS = 100_000 < u16::MAX = 65535 line-count olmaz?`
The comment is half-Turkish-question and the inequality is backwards (100_000 > 65535,
not less than). The reasoning then says 316*316=99_856, max LCS = 316, well within u16.
**Defensible but confusingly written**; rewrite as plain assertion.

### N-2. §4.5 #5 test name `_v0_5_4_bug_fix` violates collision pin pattern.

Memory `feedback_collision_pin_pattern`: `_yields_to_*_<version>_limitation` suffix pins
the WRONG behavior, broken by fix. `_v0_5_4_bug_fix` is the opposite — pins the CORRECT
post-fix behavior — but bakes the version reference in. When the next bug surfaces, this
test still passes; no grep-able enumeration value. Rename to
`build_diff_duplicate_line_removal_visible` (drop the version suffix) since the test pins
forward-going invariant, not a known limitation.

### N-3. §6.6 LOC budget — golden files in src/ tree.

Snapshot golden files in `src/config_tui/snapshots/*.snap` — these are git-tracked TEST
fixtures living inside `src/`. Most projects put goldens in `tests/snapshots/` or
`tests/fixtures/snapshots/`. Putting them in `src/` makes the source tree noisier (`ls
src/config_tui/` becomes 4 KB of golden files). Not a correctness issue, defensible since
the test sites are mod tests inline in `src/config_tui/widgets/`. Worth a one-line
justification.

### N-4. §13 forward-pointer ban contradicts §5.4 TIGHTEN actions.

§13 says "Yeni 'v0.X+ may add Y' comment'i v0.7 PR'larında forbidden". But §5.4 says
TIGHTEN action involves pattern changes — and if a tighten is incomplete (e.g. C-9 JWT
needing known-TLD list, audit doc says "huge maintenance burden — 4000+ entries"), the
implementer might want to drop a "future allowlist TODO" comment. Memory mandate is that
TODOs must be tied to a tracked issue. Reconcile: §13 should add "exception: TIGHTEN
patterns that adopt a documented-incomplete fix MAY carry `// TODO(#NNN): see audit §C-N
followup`, but NOT a bare `v0.8+` annotation." Marginal; the discipline IS the right
default.

---

## 5. Compliments

- The dogfood symmetry (Item 5 LCS → Item 2 snapshot failure messages; Item 4 per-element
  merge → Item 2 ConflictList snapshot inputs) is genuinely elegant and a strong
  cohesion-creating choice. Worth defending.
- §3.4 enumerated test pins (11 cases) with rename+flip discipline matches memory
  `feedback_collision_pin_pattern` cleanly.
- §1.1/1.2 review-consumption tables are exactly the discipline memory
  `feedback_consume_prior_review` mandates.
- §10 edge-case tables for each item give the implementer a concrete pin list.
- §3.6/§4.6/§5.6/§6.6 LOC budgets are an unusual but useful self-discipline — they make
  scope creep visible.
- Zero new dependencies (memory `feedback_dependency_minimalism`) plus zero changes to
  DOKUNULMAZ modules (rules.rs / pipeline.rs / pty.rs / runtime.rs) — clean v0.4.0-class
  scope.

---

## 6. Summary table

| ID | Severity | Section | Finding | Recommended action | Disposition |
|----|----------|---------|---------|--------------------|----|
| C-1 | CRITICAL | §3.2 | Absent-side render contract not pinned | Add explicit `render_item(None)` bullet | |
| C-2 | CRITICAL | §3.2 | Convergent-insert different-order silently reorders | Add order-divergence guard | |
| C-3 | CRITICAL | §4.2 | LCS trace-back boundary conditions + tie rule under-specified | Explicit boundary handling; pin tie convention | |
| C-4 | CRITICAL | §3.3 | Rename-vs-modify generates AotElementMissing toast (misleading) | Apply-layer translates to remove | |
| C-5 | CRITICAL | §5.3 | Per-rule harness misses priority + overlap + profile context | Add `pipeline_spans` helper for karar measurement | |
| I-1 | IMPORTANT | §3.5 | Same-side duplicate fallback silent | Toast + debug_assert! invariant | |
| I-2 | IMPORTANT | §3.3 | SemVer claim needs source citation | Cite merge.rs:101 visibility | |
| I-3 | IMPORTANT | §3 | is_array_block semantic change audit incomplete | Add conflict_list.rs:54 to §7.1; audit events.rs test fixtures | |
| I-4 | IMPORTANT | §6.3 | 6 v0.6-shipped modals missing from snapshot coverage | Expand to 13 snapshots OR document defer | |
| I-5 | IMPORTANT | §6.2 | UPDATE_SNAPSHOTS=1 silent in CI risk | Add CI=true guard | |
| I-6 | IMPORTANT | §5.2 | F-7 closure not noted; C-16 status ambiguous | One-line §5.1 update | |
| I-7 | IMPORTANT | §5.4 | Karar enum threshold not machine-enforced | Test-side `>5%` assert | |
| I-8 | IMPORTANT | §7.1 | 5 additional v0.7+ sites missed (3→8) | Expand table to 8 rows with REFRESH vs DELETE | |
| I-9 | IMPORTANT | §8 | Snapshot golden manual-review process risk | Add desync check in §6.5 | |
| N-1 | NIT | §4.3 | u16 comment self-questioning | Rewrite as assertion | |
| N-2 | NIT | §4.5 #5 | Test name `_v0_5_4_bug_fix` mis-pins | Drop version suffix | |
| N-3 | NIT | §6.6 | Goldens under `src/` not `tests/` | One-line justification | |
| N-4 | NIT | §13 | Forward-pointer ban vs TIGHTEN incomplete fixes | Exception clause for tracked-issue TODOs | |

---

*— independent review, 30-min lens, opus 4.7 senior*
