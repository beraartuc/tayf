# Final cross-cutting review: tayf v0.3.6

**Tag:** v0.3.6 (`ac1cf4c1a1ea2a6942a3da3cd936ade36f5bb323`)
**Reviewer:** Senior Rust (opus 4.7, post-ship retrospective)
**Date:** 2026-05-24

---

## Verdict

**LATENT_ISSUES** — the release is operationally clean (CI green, public API
byte-stable, hot path untouched, BASELINE unchanged), the process is textbook,
and Fix B (test rename) is complete and correct. **However, Fix A is
half-done:** the v0.3.6 patch specialized `ThemeRuleErrorKind::Display` for
`n == 0` but did not touch the parallel formatter that lives in
`src/rules.rs` (Phase 7 user-config error path, introduced in v0.3.5 commit
`77da579`). That formatter still emits `"(valid: 1..=0)"` for the exact
user-facing scenario the CHANGELOG and spec claim to have fixed. Severity is
"important, not critical" — the diagnostic is ugly but the program still
exits 64 with the rule name visible, so users are not blocked. v0.3.7 hotfix
or v0.4.0 should close it. Detailed below as 🟡 Important #1.

The local-only `input_thread_joins_promptly_after_child_exit` flake on M2
Pro is **conclusively** not a v0.3.6 regression: the diff between the v0.3.5
shipped tag (`777b8e8`) and the v0.3.6 release commit (`6f04037`) touches
zero bytes outside `src/error.rs`, `src/pipeline.rs` (two test bodies +
comment), `tests/integration_capture_groups.rs` (one test rename),
`CHANGELOG.md`, and the version bump. No change can reach the runtime,
signals, PTY teardown, line buffer, TTY guard, or bench shim. See
"input_thread_join finding" below.

---

## Findings

### 🔴 Critical (must address in v0.3.7 hotfix or v0.4.0)

**none.** Nothing in this release is broken to the point of blocking users,
corrupting terminal state, leaking memory, or breaking the public API
contract. The "important" finding below is a documentation/scope mismatch
combined with one duplicated formatter — annoying but not corrupting.

### 🟡 Important (capture as memory / follow-up issue)

#### I-1. Fix A is incomplete: the user-config error path duplicates the buggy formatter and was not specialized

**Severity:** Important. The CHANGELOG promises a user-visible fix that does
not actually reach the most common code path that triggers it.

**Location:** `src/rules.rs:986-1001`, in `resolve_group_styles_for_rule`,
`RuleSource::UserConfig` branch of the `if parsed >= captures_len` block.

```rust
RuleSource::UserConfig => {
    let n = captures_len.saturating_sub(1);
    return Err(Error::Config {
        path: user_cfg_path_or_sentinel.to_owned(),
        line: 0,
        message: format!(
            "rule '{}': styles.\"{}\": rule's regex has {} capture \
             group{} (valid: 1..={})",
            rule.name, parsed, n,
            if n == 1 { "" } else { "s" }, n
        ),
    });
}
```

This is a copy of the `ThemeRuleErrorKind::CaptureGroupIndexOutOfRange`
Display logic — same `n == captures_len.saturating_sub(1)` math, same
pluralization helper, same `(valid: 1..={n})` suffix. The author copied
the formatter at v0.3.5 (commit `77da579`, "feat(rules): RuleSource enum +
compile-time captures-len validation") instead of routing user-config
errors through the same Display impl. v0.3.6 then fixed the canonical
Display path in `src/error.rs` but missed the duplicate.

**User-visible effect:**

Drop this into `~/.config/tayf/config.toml` and pipe anything through tayf:

```toml
[[rules]]
name = "ipv4"
styles = { "1" = { fg = "red" } }
```

`ipv4` is a no-capture-groups built-in (`captures_len == 1`). tayf exits 64
with stderr containing:

```
rule 'ipv4': styles."1": rule's regex has 0 capture groups (valid: 1..=0)
```

…which is the exact bad message v0.3.6 CHANGELOG says was fixed. The fix
only landed for the theme path (which goes through `Display` via
`Error::ThemeValidation { errors: Vec<ThemeRuleError> }`). The user-config
path constructs the error string inline and was missed.

**Why the v0.3.6 test suite did not catch it:**

Two existing tests cover this path (`src/rules.rs:2063` unit and
`tests/integration_capture_groups.rs:251` integration), but both assert
only `message.contains("valid: 1..=")` — a substring that satisfies both
the old `"valid: 1..=0)"` and any hypothetical new wording. They were
written in v0.3.5 to pin "a diagnostic exists with a range", not to pin
exact wording. The v0.3.6 spec's mechanical guard (Step 7:
`grep -nP '"valid: 1\.\.=0"' src/ -r` returns nothing outside the
regression-guard `assert!`) **passes** because the literal `0` is produced
by `{}` interpolation, not by a string literal — the grep cannot see it.

**Why the senior reviewers missed it:**

Both prior reviews (vision review C-1, spec review) focused on what
v0.3.6's spec claimed to do — specialize `ThemeRuleErrorKind::Display`.
Neither reviewer cross-checked whether other code paths emitted the same
broken message via a duplicated formatter. The spec author's "scope:
Display impl only" framing was accepted at face value.

**Recommendation for v0.3.7 (proposed):**

1. Delete the inline `format!` at `src/rules.rs:986-1001` and replace with
   a single call that goes through `ThemeRuleErrorKind::Display`. Two
   shapes available:
   - **Option A (minimal diff):** construct the
     `ThemeRuleErrorKind::CaptureGroupIndexOutOfRange { group, captures_len }`
     value, call its `Display` impl via `.to_string()`, and prepend the
     `"rule '{name}': "` prefix in the `Error::Config { message }`. This
     keeps the user-config error type (`Error::Config`) unchanged.
   - **Option B (cleaner but larger):** add a new
     `Error::UserConfigRuleStyle { path, line, rule_name, kind:
     ThemeRuleErrorKind }` variant so the rule-level error kinds are
     shared across both provenances. This requires a `#[non_exhaustive]`
     enum addition and a CHANGELOG `Added` entry. Better long-term, but
     a v0.3.7 patch should prefer Option A.
2. Strengthen both existing tests to pin the exact wording for the n==0
   case: `assert!(message.contains("no capture groups; styles cannot be
   set"))` and `assert!(!message.contains("valid: 1..=0"))`. The
   regression guard belongs in both unit and integration suites.
3. Add a mechanical guard to the CI/spec checklist: `grep -rn 'capture
   group.*valid: 1\.\.=' src/ --include='*.rs'` should return exactly one
   match (the `Display` impl in `src/error.rs`). Any second hit is a
   duplicated formatter and a review-blocking smell.
4. CHANGELOG-document the v0.3.7 hotfix honestly: "v0.3.6 fixed the
   theme-side diagnostic; v0.3.7 closes the symmetric user-config-side
   diagnostic that was missed."

#### I-2. The spec's "mechanical check" is too narrow to catch interpolated literals

**Severity:** Important (process). The spec at line 191 prescribes
`grep -nP '"valid: 1\.\.=0"' src/ -r --include='*.rs'` as the mechanical
guard. This only catches string-literal occurrences. The duplicated
formatter at `src/rules.rs:991` produces the same bytes via `format!("...
(valid: 1..={})", n)` with `n == 0` — invisible to the grep. Future spec
authors should prefer **output-shape** guards (assertions on actual
rendered strings from representative inputs) over **source-shape** guards
(greps for literals). The TDD test in `src/error.rs:583` does exactly this
for the Display impl; the spec just didn't ask for an equivalent test for
the user-config formatter because it didn't know that formatter existed.

### 🔵 Nits / observations

#### N-1. Integration test comment at `tests/integration_capture_groups.rs:248` is now stale

```rust
//    no capture groups (`captures_len = 1`), so the diagnostic prints
//    `valid: 1..=0`.
```

This comment documents the **current** (buggy) behavior of the user-config
path. After I-1 is fixed in v0.3.7, the comment will be wrong. Update it
when I-1 lands; or, since the test only asserts `valid: 1..=` (a substring
that survives the fix), it doesn't actively mislead today. Low priority.

#### N-2. CHANGELOG wording slightly overclaims

The v0.3.6 CHANGELOG says the diagnostic was fixed "when the regex has no
capture groups at all". A user reading this will assume both the theme
path and the user-config path were fixed. The wording is technically about
`ThemeRuleErrorKind::Display` (a public type), but the user-facing
implication is broader. If I-1 is not fixed before any external eyes land
on the project, consider amending the CHANGELOG with: "Only the
`ThemeRuleErrorKind::Display` impl is specialized; the parallel
user-config error formatter at `src/rules.rs::resolve_group_styles_for_rule`
will be aligned in v0.3.7."

#### N-3. Two-branch `if/else` over `match captures_len` is fine — but a `match` on the `n` value would be more idiomatic

```rust
match n {
    0 => write!(f, "...no capture groups; styles cannot be set"),
    _ => write!(f, "...{n} capture group{s} (valid: 1..={n})"),
}
```

A pure style nit. The current `if/else` is correct and equally readable.
No action needed.

#### N-4. `tests/integration_capture_groups.rs::syslog_timestamp_substring_survives_colorization` only asserts substring survival, but the second test below (`iso_timestamp_match_renders_five_distinct_sgrs`) asserts an exact SGR count

The pair-up is fine — the syslog branch is capture-less and the ISO
branch has 5 groups; the asymmetry is intentional and correct. But the
rename has now made the asymmetry explicit, which is good. No action.

#### N-5. `src/pipeline.rs::syslog_timestamp_substring_survives_colorization` comment now correctly disclaims the multi-SGR reality

```rust
// Other rules (e.g., log_level on "msg") may add additional SGRs; this test
// only asserts the timestamp substring survives colorization intact.
```

Better than before. Worth keeping a one-line cross-reference to the
integration test of the same name so a future contributor sees both
pinning the same invariant.

---

## What v0.3.6 did right

- **Tiny, surgical patch.** Five files touched, ~30 lines net added. Pure
  patch — no behavior changes on the hot path, no BASELINE shift, no
  public API surface drift (`git diff v0.3.5..v0.3.6 -- src/lib.rs` is
  empty).
- **Cargo.lock diff is exactly one line** (the `tayf` version bump). No
  transitive dependency churn snuck in alongside the patch — a
  meaningful achievement given how easy it is for a routine
  `cargo update` to creep in.
- **Public surface byte-stable as promised.** Spec said "BREAKING: none,
  pure patch"; verified.
- **Process matched v0.3.5 precedent exactly.** Same commit shape (vision
  → vision-rev2 → spec → spec-rev2 → plan → fix → CHANGELOG TBD →
  version bump → tag → CHANGELOG date → umbrella-shipped marker). Same
  use of post-tag housekeeping commits for the date and shipped marker.
- **Senior reviews applied before tagging.** Both the vision (3 critical,
  5 important, 5 nits) and the spec (4 nits) were reviewed and revised
  in rev2 commits before any code was written. This caught real issues
  pre-flight rather than at PR time.
- **Test rename is complete.** Grep for the old name (`grep -rn
  syslog_timestamp_match_renders_one_sgr` over `src/` + `tests/`)
  returns zero hits in code; only plan/spec docs reference the old name
  as historical context.
- **TDD red→green discipline for Fix A.** The new unit test
  `theme_rule_error_kind_out_of_range_no_capture_groups_specialized`
  pins the exact wording AND carries a regression guard
  (`!s.contains("valid: 1..=0")`). Future refactors of the Display impl
  will trip this guard. (The guard only protects the Display path —
  see I-1.)
- **Pluralization helper untouched.** The `n >= 1` branch is byte-identical
  to the pre-v0.3.6 format string. Spec promised it and delivered it.

---

## Process observations

### Commit shape

All seven commits in the v0.3.6 window have valid Conventional Commits
prefixes (`docs(vision)`, `docs(spec)`, `docs(plan)`, `fix`,
`docs(changelog)`, `chore(release)`, `docs(vision)` post-tag,
`docs(changelog)` post-tag). All under the 70-char title limit. The
`fix: specialize Display for no-capture-groups + rename syslog tests`
title bundles two logically distinct changes into one commit — defensible
because they're both "deferred bugfix" scope, but a stricter atomic-commit
discipline would have split them into `fix(error): ...` and
`test: rename ...`. Low priority; the existing CLAUDE.md "one logical
change per commit" rule could be interpreted either way.

### Tag style

`v0.3.6` matches the precedent (`v0.3.5`, `v0.3.4`, …). Tag points at the
version-bump commit (`6f04037`), not the fix commit (`8c32d3c`) — correct;
the version bump is the release boundary.

### CHANGELOG hygiene

- v0.3.5 → v0.3.6 boundary is clean: `## [0.3.6] — 2026-05-24` on line 7,
  `## [0.3.5] — 2026-05-24` on line 22. No merged sections, no orphan
  entries.
- The `### Fixed` section accurately describes the test rename and the
  Display change. **The "fixed Display" entry overclaims relative to
  actual scope — see N-2 / I-1.**
- The post-tag `docs(changelog): v0.3.6 release date` follows the v0.3.5
  precedent (commit `e748633`). Good consistency.

### Umbrella update

`docs(vision): mark v0.3.6 shipped` (commit `3516244`) follows the same
post-tag pattern as `docs(vision): mark v0.3.5 shipped` (commit
`777b8e8`). v0.4.0 and v0.4.1 rows remain unmarked as expected.

### Acceptance gate

Local M2 Pro acceptance gate ran `cargo fmt && cargo clippy -- -D
warnings && cargo test`. One deterministic failure
(`integration_smoke::input_thread_joins_promptly_after_child_exit`,
`elapsed=2.008s` vs 2s budget). CI on both `ubuntu-latest` (1m17s) and
`macos-latest` (52s) Test jobs passed. Decision to ship was correct (see
"input_thread_join finding" below) — but the user's recorded MEMORY
entry on macOS flake skepticism was not consulted **before** the
subagent verification; the verification happened to be airtight, but if
it hadn't been, the local fail could have caused a wrong-call ship.
Worth pre-mortem checking the MEMORY entries against any local-only
failure before deciding to ship.

---

## input_thread_join finding

The subagent's "not a regression" conclusion is **airtight** and stronger
than the original investigation suggested.

**Original investigation scope:** `git diff 777b8e8..6f04037 -- src/runtime.rs
tests/integration_smoke.rs` returned empty. Subagent concluded "not a
regression".

**My broader scope:** `git diff 777b8e8..6f04037 --stat -- src/ tests/
benches/ build.rs` returns:

```
 src/error.rs                        | 33 +++++++++++++++++++++++++--------
 src/pipeline.rs                     |  8 ++++----
 tests/integration_capture_groups.rs |  2 +-
```

Three files. **Zero changes** to:
- `src/runtime.rs` (the two-thread I/O loop, the join logic)
- `src/signals.rs` (signal forwarding to child PG)
- `src/pty.rs` (PTY creation, decomposition, FD teardown)
- `src/tty_guard.rs` (RAII raw mode + panic hook)
- `src/line_buffer.rs` (UTF-8 accumulator)
- `src/shell.rs` / `src/cli.rs` / `src/lib.rs` (spawn, dispatch)
- `benches/` (no bench shim changes — eliminates the "bench harness
  affecting timing" possibility)
- `build.rs` (no build-time codegen changes — eliminates rebuild-order
  flake)

`src/pipeline.rs` changes are confined to the `#[cfg(test)] mod
rule_tests` block (renaming a single test function and rewording its
comment); no production code path runs differently. `src/error.rs`
changes the Display impl for an error variant that is never produced in
the input-thread-join scenario (which spawns a no-config shell with no
TOML rules). `tests/integration_capture_groups.rs` is a completely
different integration test file from `tests/integration_smoke.rs`.

**Conclusion:** No code change between `v0.3.5` (commit `777b8e8`,
"docs(vision): mark v0.3.5 shipped") and `v0.3.6` (commit `6f04037`,
"chore(release): bump version to 0.3.6") could plausibly affect the
input-thread-join code path. The local 2.008s elapsed-vs-2.000s budget is
a system-load artifact, exactly the class of flake the user's MEMORY
entry on "Flaky watch::drop test on macOS CI" describes — a sub-50ms
race against a hardcoded timeout under contention. CI on both Linux and
macOS runners confirmed by passing.

**Recommended follow-up (not blocking):** the 2s timeout in
`input_thread_joins_promptly_after_child_exit` is the same shape of
flake-magnet as the watch::drop test. A `INPUT_THREAD_JOIN_DEADLINE` env
override (defaulting to 2s but bumpable to e.g. 5s in CI containers and
local-under-load runs) would let CI-style runs stay strict while local
"I'm building four things at once on this M2" runs don't false-positive
the acceptance gate. Capture as a v0.4.x quality-of-life issue.

---

## Recommendations for future cycles

1. **Add a "duplicate formatter audit" step to the release checklist.** Before
   any "specialize Display for X" patch, grep for `format!.*{same shape}`
   in the rest of the crate. The v0.3.6 spec author would have caught I-1
   in five seconds with `rg -nF '(valid: 1..=' src/`.

2. **Prefer routing error rendering through a single Display impl, not
   inline `format!` in callsites.** The user-config path at
   `src/rules.rs:986-1001` should construct a `ThemeRuleErrorKind` value
   and either embed it in a new error variant or stringify it via
   `Display`. Avoiding duplicate formatters is the real fix; specializing
   only one site is symptomatic.

3. **Strengthen tests that assert on error message shape to pin exact
   wording, not loose substrings.** The pre-v0.3.6 assertions
   (`message.contains("valid: 1..=")`) accepted both the broken and the
   fixed wording — they never could have caught the bug. After I-1 is
   addressed, switch both unit and integration tests to assert on the
   full sentence.

4. **Source-shape guards (greps) should be paired with output-shape
   guards (rendered-string assertions).** The v0.3.6 spec's Step 7 mechanical
   check only saw literal `"valid: 1..=0"`. It missed the interpolated
   form. Always add a rendered-string TDD test alongside any grep guard.

5. **Run MEMORY-entry consultation BEFORE deciding to ship on a local
   acceptance failure.** The "not assuming defects on macOS CI flakes"
   entry should be triggered by any local-only fail, not by the after-the-
   fact "wait, this looks familiar" reflex. A pre-ship checklist item:
   "If acceptance gate fails locally but passes CI, list relevant MEMORY
   entries and confirm none of them apply before treating it as a real
   defect."

6. **Consider a bumpable timeout for `input_thread_join` and the
   watch::drop test.** Both are deadline-vs-elapsed races. Env override
   (read once at test start, default to the current strict value)
   eliminates the flake-class entirely without weakening CI assertions.
   Capture as v0.4.x quality-of-life.

7. **Bundled vs atomic commits.** Commit `8c32d3c` (`fix: specialize
   Display for no-capture-groups + rename syslog tests`) bundles two
   unrelated changes. CLAUDE.md §4 says "one logical change per commit".
   These were two. A stricter reading would have split into
   `fix(error): specialize Display for no-capture-groups` and
   `test: rename misnamed syslog tests`. Not a blocker, but the
   precedent leaks: future cycles will normalize larger bundles. Worth
   re-stating the rule in the v0.4 vision.

---

## TL;DR

v0.3.6 is a clean, surgical, process-textbook ship — except that Fix A
only landed at one of two sites that produce the diagnostic. The
user-config error path (`src/rules.rs:986-1001`) still prints `(valid:
1..=0)` for the exact `n == 0` case the CHANGELOG says was fixed. This
is a duplicated formatter that was added in v0.3.5 (commit `77da579`)
and missed by every reviewer (myself included, on the prior two passes
of this v0.3.6 cycle) because the spec scoped itself to `Display` alone.
Schedule for v0.3.7 hotfix or fold into v0.4.0. The local input-thread
acceptance flake is **definitively** not a v0.3.6 regression and should
be treated as the same flake class as the user's recorded
`watch::drop` MEMORY entry.
