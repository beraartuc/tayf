# Final cross-cutting review: tayf v0.3.7

**Tag:** v0.3.7 (`439b7bb`)
**Reviewer:** Senior Rust (opus 4.7, post-ship retrospective)
**Date:** 2026-05-24

## Verdict

**CLEAN_SHIP.** v0.3.7 closes the v0.3.6 half-fix gap completely, adds a
load-bearing security improvement (control-byte sanitization) as a
side-effect of the delegation, and ships zero new latent issues. CI was
green on first push on both ubuntu-latest and macos-latest; the v0.3.6
local-only `input_thread_joins_promptly_after_child_exit` flake **passed on
CI** in this push (ubuntu, `2026-05-24T18:31:19.5034811Z ... ok`),
reinforcing the local-only diagnosis.

## Findings

### 🔴 Critical (address before v0.4.0 brainstorm)

none.

### 🟡 Important (capture as memory / follow-up)

**I-1. The Display-bypass drift pattern still exists at one other call site
in `src/rules.rs` and should be audited before v0.4.0.**

After v0.3.7 the two map-iteration arms (KeyMalformed at `src/rules.rs:957`,
IndexOutOfRange at `src/rules.rs:985`) correctly delegate via
`format!("rule '{}': {kind}", rule.name)`. **However**, the `key == "0"`
fast-path at `src/rules.rs:923-947` still carries its own inline format
string for the `CaptureGroupIndexZeroForbidden` user-config arm:

```rust
RuleSource::UserConfig => {
    return Err(Error::Config {
        path: user_cfg_path_or_sentinel.to_owned(),
        line: 0,
        message: format!(
            "rule '{}': styles.\"0\": group 0 is the entire match; \
             use the 'style' field instead",
            rule.name
        ),
    });
}
```

The `ThemeRuleErrorKind::CaptureGroupIndexZeroForbidden` Display impl
(`src/error.rs:94-96`) produces byte-identical text today
(`styles.\"0\": group 0 is the entire match; use the 'style' field instead`),
so there is **no current user-visible defect**. But this is the exact same
drift surface the v0.3.6 final review caught: if the Display impl wording
ever changes (e.g. someone adds a hint about `[rules.styles."N"]` dotted-form
in v0.4), the user-config arm will silently keep emitting the old wording
and theme-config arm will emit the new one. The fix is a one-line
mechanical translation matching the pattern v0.3.7 just established:

```rust
RuleSource::UserConfig => {
    let kind = crate::error::ThemeRuleErrorKind::CaptureGroupIndexZeroForbidden;
    return Err(Error::Config {
        path: user_cfg_path_or_sentinel.to_owned(),
        line: 0,
        message: format!("rule '{}': {kind}", rule.name),
    });
}
```

Recommendation: fold into v0.4.0 cleanup, **not** a v0.3.8. This is "no
user-visible defect today; latent drift surface", not a regression.

### 🔵 Nits / observations

**N-1. Defense-in-depth on sanitization is actually doubled, which is
correct but worth documenting.** The shipped code path for an adversarial
key like `"0\x07evil"` runs sanitization twice: first `ThemeRuleErrorKind::
CaptureGroupKeyMalformed::Display` (`src/error.rs:88-93`) escapes BEL →
literal `\x07` inside `kind`; then `Error::Config::Display` (`src/error.rs:
162`) calls `sanitize_for_display(message)` on the already-formatted string,
which is a no-op for the now-printable text. This is **correct** — both
sanitization layers are independently load-bearing (Error::Config wraps
arbitrary `message` strings including from `src/config.rs:143-531` which
have no upstream sanitization), but it's worth a doc-comment somewhere
noting the layering, otherwise someone "optimizing" the outer
sanitization in v0.4 could regress `src/config.rs` paths.

**N-2. Test wording asserts `"capture-group key must be a positive
decimal"` substring (rules.rs:2126) — this is the right level of
specificity per the user's memory entry on test-assertion specificity (not
too brittle, catches a meaningful semantic substring).** No action needed;
calibration check passes.

**N-3. The `> [Note]` admonition syntax in CHANGELOG.md:30-34 is not a
Keep-a-Changelog standard element**, but it renders fine as a blockquote
in GitHub-flavored Markdown and adds genuine context about why a hotfix
exists one day after v0.3.6. Acceptable.

**N-4. `RuleSource::Builtin => unreachable!()` (no message) appears at
`src/rules.rs:967` and `src/rules.rs:996`** while the sibling arm at line
941-945 has a multi-line explanation. The v0.3.7 commit faithfully
preserved this pre-existing asymmetry rather than touching unrelated code,
which is **correct discipline for a hotfix**. But the bare `unreachable!()`
form violates CLAUDE.md ("with `unreachable!(\"reason\")`"). Fold the same
message into all three on the v0.4.0 cleanup pass.

## What v0.3.7 did right

- **Diff is surgical and proportional to the problem.** 10 files,
  +2149/-26 — but the actual production-code delta is `src/rules.rs:75
  changed (mostly tests)` + one Cargo version bump. The rest is spec, plan,
  CHANGELOG, prior-review record. No scope creep.
- **Single source of truth for the wording is preserved.** `grep -n "valid:
  1..="` finds it only in `src/error.rs:107` (the Display impl), `src/error.
  rs:570,580,590` (Display unit tests), and one regression-guard `!contains`
  in `src/rules.rs:2087`. The previous v0.3.6-era duplicate in `src/rules.
  rs:993` is gone.
- **Same for the malformed-key wording:** `grep -rn 'capture-group key
  must be'` finds only `src/error.rs:90` (Display) + two regression guards.
- **Public API surface byte-stable.** `git log v0.3.6..v0.3.7 -- src/lib.rs
  src/error.rs` returns empty. The spec promise held.
- **`src/error.rs` unchanged.** The two-branch Display impl, the
  pluralization helper, and `sanitize_for_display` are all load-bearing for
  v0.3.7's correctness, and v0.3.7 did not touch any of them.
- **Two regression guards landed alongside the fix** (1 unit test in
  `src/rules.rs:2090-2135` for sanitization with BEL; 1 integration test
  diff in `tests/integration_capture_groups.rs:243-289` for the
  out-of-range path). Both assert positive substrings AND negative
  regression guards (`!contains("valid: 1..=0")`, `!message.as_bytes().
  contains(&0x07)`).
- **Pre-tag senior spec review caught I-1** (the parallel KeyMalformed
  duplicate). Rev2 (`4cbc014`) widened scope to fix both formatters in
  one commit instead of deferring KeyMalformed. This is the right call:
  shipping two hotfixes a day apart for the same architectural defect
  would have been worse than one slightly-larger hotfix.

## Process observations

**Commit sequence (oldest first):**

1. `f3a429d` spec
2. `4cbc014` spec rev2 (senior-review applied)
3. `5b5ccda` plan
4. `1401464` vision umbrella row (**pre-tag**)
5. `7770072` fix (production code + tests)
6. `05c3469` CHANGELOG entry (date `TBD`)
7. `d44c9fe` version bump → tag `v0.3.7`
8. `743ea3b` CHANGELOG release date (**post-tag**)
9. `eec0792` vision shipped marker (**post-tag**)

**Ceremony split N-5 verdict: improvement.** The v0.3.6 cycle inserted the
umbrella vision row and the shipped marker both post-tag, which made the
tag itself read "v0.3.7 not yet planned" to anyone browsing
`docs/superpowers/specs/.../vision.md`. v0.3.7 split this:
`1401464` inserts the row as "hotfix planned" *before* the tag, and
`eec0792` flips it to "✅ shipped" *after*. This means at every point in
git history the vision document is internally consistent — no "in-flight"
state where the tag exists but the vision row doesn't acknowledge it.
**Recommend codifying this as the standard release ceremony.**

**CHANGELOG hygiene.** The shipped CHANGELOG entry (lines 7-34) matches
the spec's quoted entry (`specs/...v0.3.7....md:263-289`) byte-for-byte
except for the planned `TBD → 2026-05-24` substitution in `743ea3b`. The
`> [Note]` admonition at lines 30-34 lands correctly and provides genuine
explanatory value (why this hotfix exists one day after v0.3.6). Good.

## v0.3.6 half-fix retrospective

**Gap closed: yes.** v0.3.6 final review found that v0.3.6's CHANGELOG
read as if the Display fix reached both error paths, when in fact
`src/rules.rs:986-1001` carried a literal-copy formatter that bypassed
`ThemeRuleErrorKind::Display`. v0.3.7's `src/rules.rs:985-994` now reads
`message: format!("rule '{}': {kind}", rule.name)` where `kind` is the
shared enum variant — so the user-config arm and the theme-config arm
produce byte-identical text from a single source of truth, and any future
Display impl change automatically reaches both paths.

**Bonus closure (I-1 from the spec senior review):** The parallel
KeyMalformed duplicate at `src/rules.rs:957-967` was also folded into
v0.3.7. This was originally a v0.3.8 candidate but was correctly pulled
forward because (a) it was the same architectural defect, (b) the fix
shape was mechanical and bounded, (c) KeyMalformed has an
**additional security benefit**: the previous formatter inlined raw `key`
bytes into the message; the new path routes through `Display` which calls
`sanitize_for_display(key)`, which is a defense-in-depth win against
CLAUDE.md §3's escape-sequence-injection mandate even though the outer
`Error::Config::Display` also sanitizes.

**Remaining drift surface:** one (see I-1 above) — the
`CaptureGroupIndexZeroForbidden` user-config arm at `src/rules.rs:923-947`
still inlines its wording rather than delegating. **No user-visible
defect**, but the same architectural pattern that bit v0.3.5→v0.3.6 and
again v0.3.6→v0.3.7 will bite v0.3.7→v0.4 if the wording ever changes.
Fold into v0.4.0 cleanup.

**Test coverage completeness (review prompt §7).** The new unit test uses
BEL (`0x07`) as the adversarial byte. `sanitize_for_display` (`src/error.
rs:244-259`) uses `ch.is_control() && ch != '\n' && ch != '\t'`, which is a
*class predicate* covering ASCII C0 (0x00..=0x1F + 0x7F) **and** Unicode
C1 (U+0080..U+009F including the 8-bit CSI introducer U+009B). The
function is correct by construction, so testing one representative byte
(BEL) is sufficient for the unit test — the alternative (one test per
control byte) would be over-engineering. The function is also independently
tested in `src/error.rs:545-552` for `ThemeRuleErrorKind::
CaptureGroupKeyMalformed` with `"01"` (non-control case) and at
`src/error.rs:594-602` with `"\x07abc"` (control case). The BEL test is
**representative, not insufficient.** No action.

## Memory recommendations

The controller should write **one new memory entry**. The other two
candidates I considered (sanitization layering doc; CHANGELOG note
admonition) are too narrow.

**ENTRY 1 (new): "Audit for duplicate formatters when Display impls
specialize."** Pattern: every time we add a non-trivial branch to a
`std::fmt::Display` impl on an error type, grep `src/` for inline `format!`
calls that build the same conceptual message. Concrete prompt for future
sessions:

> When a Display impl on `ThemeRuleErrorKind` or any error-kind enum gets
> a new specialization branch (e.g., the `captures_len == 1` no-capture-
> groups branch added in v0.3.6), immediately run
> `grep -rn 'format!("rule\b\|message: format!' src/` and verify every
> match either (a) uses the Display impl via `format!("...: {kind}", ...)`
> or (b) is doing something semantically different. Duplicate formatters
> shipped in v0.3.5 caused both v0.3.6 (one hotfix) and v0.3.7 (a second
> hotfix). Two releases of drift cost from one missed grep.

This complements the existing memory entry on test assertion specificity
(strong assertions catch this class of drift if they exist, but they didn't
in v0.3.5).

## Recommendations for v0.4.0 cycle

1. **Fold I-1** (the `CaptureGroupIndexZeroForbidden` user-config arm
   delegation) into v0.4.0 cleanup, NOT a v0.3.8. Three releases for the
   same pattern would suggest we don't trust our own audit; one v0.4.0
   commit closes it.
2. **Fold N-4** (bare `unreachable!()` → `unreachable!("reason")` in
   three places in `src/rules.rs`) into the same v0.4.0 cleanup commit.
3. **Codify the v0.3.7 release ceremony split** (pre-tag vision insert →
   tag → post-tag shipped marker) as the standard pattern. The user's
   `project_release_workflow.md` memory entry could absorb this step.
4. When the v0.4.0 RegexSet pre-filter work begins, the new code paths
   should be designed to **never carry an inline message: format!("...")
   that duplicates Display output**. If a call site needs to construct an
   Error::Config from an enum variant, it should always be
   `format!("...: {kind}", ...)` or simpler.
5. **Do not introduce a third hotfix.** I-1 and N-4 are real but
   non-shipping defects. Pulling them forward to v0.3.8 would burn another
   release cycle for zero user benefit and would itself violate the "no
   half-features behind feature flags / no busywork releases" spirit.

---

**End of review.** Tag `v0.3.7` is a clean ship; CI is green on both
platforms; the v0.3.6 gap is closed with the bonus of a defense-in-depth
sanitization improvement; the single remaining drift surface
(`CaptureGroupIndexZeroForbidden`) is a v0.4.0 cleanup item, not a v0.3.8
trigger. Process-wise, the release ceremony split (pre-tag plan insert /
post-tag shipped marker) is a genuine improvement over v0.3.6's
post-tag-only marker and should become standard.
