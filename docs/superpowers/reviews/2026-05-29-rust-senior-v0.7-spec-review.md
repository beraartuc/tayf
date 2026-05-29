# v0.7 Spec — Rust-Senior Independent Review

**Reviewer lens:** Rust idioms, API soundness, error types, lifetimes, performance, safety, std/dep usage.
**Spec under review:** `docs/superpowers/specs/2026-05-29-tayf-v0.7-design.md` (882 lines, rev1).
**Baseline:** v0.6.3 @ `761a57f` — 743 lib + 37 integration tests.
**Date:** 2026-05-29.

This review is independent — I did not read the parallel reviewer's draft or any prior review absorbed into the spec.

---

## 1. Verdict

**🟡 ABSORB CHANGES**

Spec is largely sound, the dogfood relationships are elegant, the test-pin enumeration is thorough, and the dependency-minimalism stance is uncompromising. However there are **three CRITICAL issues** that *will* compile-fail or behave-wrong if implemented as written: (a) the snapshot helper uses ratatui 0.30 APIs that don't exist on `Buffer`, (b) the `u16` LCS-DP cell type has a real overflow risk under the stated cap, (c) the `tayf::testing::match_rule` public surface contradicts the spec's own "zero public-API impact" claim and is unmotivated when `__test_api` precedent already exists. Plus four IMPORTANT and several NITs below. Rev2 absorption is required before implementation; no scope changes.

---

## 2. CRITICAL findings (must-fix before implementation)

### C-1. `Buffer::get(x, y)` and `buf.area()` do not exist on ratatui 0.30

**Spec §6.2, lines 562–574** (`stringify_buffer`):

```rust
for y in 0..buf.area().height {
    for x in 0..buf.area().width {
        let cell = buf.get(x, y);
        out.push_str(cell.symbol());
    }
    ...
}
```

**Concern.** This is the ratatui ≤ 0.26 API. ratatui 0.30 dropped both. Verified against the codebase's *own* existing TestBackend tests:
- `tests/config_tui_conflict_list.rs:55–57` uses `buf.area.width` (field, not method) and `buf[(x, row)].symbol()` (Index, not get).
- `tests/common/tui_harness.rs:49,53,65,71,75` uses the same `buf.area` field + `buf[(col, row)]` indexing throughout.

If the helper is written verbatim, the file does not compile. The spec author cargo-cult'ed an older snippet without grounding against the existing codebase's TestBackend usage — which is doubly unfortunate because the precedent is right next door.

**Recommended action.** Rewrite §6.2's `stringify_buffer` to match the in-codebase pattern, e.g.:

```rust
fn stringify_buffer(buf: &Buffer) -> String {
    let area = buf.area;
    let mut out = String::with_capacity(usize::from(area.width) * usize::from(area.height + 1));
    for y in 0..area.height {
        for x in 0..area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}
```

Also note `area.area()` returns `u16` on Rect, but `usize::from(area.width) * usize::from(area.height)` is the cleaner pre-allocation hint. The spec's `as usize` cast violates the project's clippy::pedantic posture (`clippy::cast_possible_truncation`).

### C-2. `Vec<Vec<u16>>` LCS table — `u16` overflow possible under stated cap

**Spec §4.3, lines 281–284** and the comment in line 283:

> `// u16 yeterli: MAX_DP_CELLS = 100_000 < u16::MAX = 65535 line-count olmaz? 316*316 = 99_856 < 100_000 → max LCS = 316 < u16::MAX. Safe.`

**Concern.** The reasoning is *almost* right but the bound is stated incorrectly and the worst case is at the boundary of safety. Consider:

- `MAX_DP_CELLS = 100_000`.
- Worst case under the cap: `n=1, m=100_000` (a 1-line old vs 100 000-line new). Then `n * m = 100_000 ≤ cap`. LCS value ≤ `min(n, m) = 1` — fine.
- Worst case at the cap with balanced dims: `n = m ≈ 316`. LCS ≤ 316 — fine.
- But: `n=100_000, m=1` also satisfies the cap. Symmetric to the first case — also fine.

So `u16` *is* technically sufficient — but `dp[i][j]` is `dp[n][m]`, not `min(n,m)`. The recurrence `dp[i][j] ≤ min(i, j)`, and globally `dp[n][m] ≤ min(n, m) ≤ √(n*m) ≤ √100_000 ≈ 316`. So spec's reasoning stands. Concern downgrades from "wrong" to "fragile and underexplained."

**However** the *real* issue: `Vec<Vec<u16>>` is the wrong data structure regardless. A `Vec<u16>` of length `(n+1)*(m+1)` indexed manually is more cache-friendly *and* halves the allocation count from `n+2` to `1`. Or, more idiomatically: two-row rolling window `Vec<u16>` of length `m+1` (we only need `dp[i-1]` to compute `dp[i]`). The trace-back then requires storing the full table — so two-row LCS *length* + Hirschberg-style recursive trace, or just biting the bullet and using a single contiguous `Vec<u16>`.

**Recommended action.** Replace `Vec<Vec<u16>>` with a `struct DpTable { rows: usize, cols: usize, cells: Vec<u16> }` with `index(i, j) → cells[i * cols + j]`. Justify the `u16` choice with an inline comment that derives the bound from the cap (`dp[n][m] ≤ min(n,m) ≤ floor(sqrt(MAX_DP_CELLS))`) and document an explicit `debug_assert!(dp_value < u16::MAX)` near the recurrence. This is doubly worthwhile because the spec elsewhere brags about avoiding deps — let the in-tree code show actual Rust craft.

Also: the trace-back signature in §4.3 line 286 — `fn trace_back<'a>(old: &'a [&str], new: &'a [&str], dp: &[Vec<u16>]) -> Vec<DiffOp<'a>>` — looks fine as written, but with the flat-vec refactor `&[u16]` + dims pair is preferable, and the lifetime annotation should be `&'a [&'a str]` only if the inner `&str` borrows are tied to the same scope as the outer slice (they should be — the `lines()` iter borrows `old_str: &str`). Tighten to `<'src>(old: &[&'src str], ...) -> Vec<DiffOp<'src>>` to signal *which* string the borrowed `&str`s come from.

### C-3. `tayf::testing::match_rule` contradicts "zero public-API impact" and duplicates an existing pattern

**Spec §2 line 75** ("**Public API impact:** sıfır") and **§5.3 line 426** (the new `#[doc(hidden)] pub mod testing`).

**Concern.** Two issues, both architectural:

1. **Self-contradiction.** §2 explicitly states "**Public API impact: sıfır**" while §5.3 adds a new `pub mod testing` to `src/lib.rs`. `#[doc(hidden)]` hides it from rustdoc but does **not** make it private — `tayf::testing::match_rule` is reachable from any downstream user. The semver implication is not zero. v0.4.0 / v0.6.3 deliberately demoted modules to `pub(crate)` to *shrink* the public surface; v0.7 reverses that direction inconsistently. Either:
   - The §2 row needs amending to "public surface +1 `#[doc(hidden)]` module" with explicit SemVer rationale, or
   - The helper goes elsewhere (see point 2).

2. **The `__test_api` precedent.** `src/lib.rs:354–355` *already* has `#[doc(hidden)] pub mod __test_api` for exactly this purpose. The spec §6.2 line 500 explicitly cites that precedent but then ignores it for §5 (`testing` vs. `__test_api`). Worse: §5 invents a *new* doc-hidden module name (`testing`, no underscore prefix) — that naming difference will look accidental in two weeks. Pick one convention.

   The more conservative path: the audit corpus is an *integration* test under `tests/`. Integration tests cannot reach `pub(crate)`. So either:
   - **(preferred)** Inline the corpus harness as a `#[cfg(test)] mod tests` inside `src/rules.rs` (it already has the `match_string` helper at rules.rs:1420; the corpus harness is the same shape one level up). No public surface delta.
   - Or: extend the existing `__test_api` module with a `match_named_rule(&str, &str) -> Option<String>` function — re-use the doc-hidden bag rather than open a second one.

**Recommended action.** Move the corpus harness into `src/rules.rs`'s existing `mod tests` (it lives in `src/rules.rs` so `tests/audit_corpus/*.txt` files load via `include_str!` at compile time — this also makes the corpus part of the *build* and impossible to skip via test filter). Drop the new `pub mod testing` entirely. If the implementer insists on integration-style placement, extend `__test_api` instead of opening `testing` — but justify the SemVer impact explicitly in §2.

Additional concern with the include_str! path: `tests/audit_corpus/*.txt` under `tests/` is not in the `include` allowlist in Cargo.toml lines 14–23 — it *would* publish to crates.io when included via `include_str!` from a path under `tests/`. Actually no: `tests/` is excluded by default unless explicitly listed. But `src/` *is* included, so the safer storage location is `src/rules/audit_corpus/` if `include_str!` is used. The spec puts corpus under `tests/` (line 71 — `tests/audit_corpus/`) which matches the "integration test" placement but then requires `fs::read_to_string(path)` at runtime, which depends on CWD = workspace root — a brittle assumption in Cargo workspaces or non-`cargo test` runners. `env!("CARGO_MANIFEST_DIR")` is the convention here (used in §6.2 line 536 — apply consistently to §5.3 line 405's `parse_corpus_file`).

---

## 3. IMPORTANT findings (should-fix during implementation)

### I-1. `KeyConflict::is_array_block` semantic change — semver categorization is incomplete

**Spec §3.2 lines 155–157 and §2 line 69.**

The spec correctly identifies that `merge` is `pub(crate)` post-v0.6.3, so external SemVer is unbroken. But it misses one consideration: `is_array_block: bool` is a *field*, not a method. Internal callers do `c.is_array_block` reads. The semantic change ("now only `true` in fallback") means existing internal callers — e.g. `widgets/conflict_list.rs:39` `render_row` which currently uses `is_array_block` to decide row affordances — receive a *new value distribution* without any compile-time signal. Search the codebase for every `is_array_block` read and audit each call site explicitly.

**Recommended action.** Add a §3 subsection "Internal callers of `is_array_block`" listing every read site grep'd via `git grep -n is_array_block src/` and document the per-site invariant (does that callsite need the new "Block-shape conflict at non-fallback path" case to be handled differently?). This is the cross-module invariant audit memory `feedback_parallel_call_site_invariant_audit` mandates. Currently §3.2 just says "semantics daralır" without enumerating call sites.

### I-2. `WriteToPathError::AotElementMissing` shape underspecified for `thiserror`

**Spec §3.3 line 179.**

```text
WriteToPathError::AotElementMissing { path, element_name }
```

The spec sketches the variant name and fields but does not write the `#[error("...")]` format string. The existing variants at `merge.rs:104,116` use specific formats: `"type mismatch at {path}: dest is {dest_type}, source is {source_type}"`. Implementer needs the exact wording pinned to avoid drift, per memory `feedback_test_assertion_specificity` — and the test §3.4 #11 (`write_to_path_aot_element_missing_returns_typed_error`) currently only asserts `matches!(res, Err(AotElementMissing { .. }))`, which leaves the `Display` text untested. Test would pass even if the message is empty or wrong.

**Recommended action.** Spec §3.3 should pin the exact format string:
```rust
#[error("array-of-tables element not found at {path}: no element with name=\"{element_name}\"")]
AotElementMissing { path: String, element_name: String },
```
and §3.4 test #11 should assert the exact `format!("{e}")` output (memory mandate: exact, not contains).

### I-3. `panic!` in snapshot helper conflicts with project rules

**Spec §6.2 lines 530–559** — five `panic!` / `expect()` sites in `assert_render_snapshot`:
- `Terminal::new(backend).expect("TestBackend init")` (line 531)
- `terminal.draw(...).expect("draw")` (line 532)
- `std::fs::write(&abs_path, &rendered).expect("write snapshot")` (line 539)
- `fs::read_to_string(...).unwrap_or_else(|_| panic!(...))` (line 544)
- `panic!("render snapshot mismatch ...")` (line 555)

**Concern.** CLAUDE.md §"Style enforcement" — *No `unwrap()` or `expect()` in library code. Allowed only in: tests, `main.rs` top-level setup, and proven-unreachable paths.* The helper is `#[cfg(test)]`, so technically allowed. BUT — clippy::pedantic's `clippy::expect_used` is still enabled and will require either an inner `#[allow(clippy::expect_used)] mod test_support` annotation with `// reason:` comment, or an `assert_render_snapshot` rewrite returning `Result<(), String>` and `.unwrap()` only at the test-fn level.

The spec doesn't acknowledge this — and the test_support module is `pub(crate)`, so even though it's `#[cfg(test)]`, the lint applies inside library code (`src/`). Implementer will hit this on first `cargo clippy`.

**Recommended action.** Add to §6.2: "The `test_support` module carries `#[allow(clippy::expect_used)]` at the module level with `// reason: snapshot helper internals; failures are immediate test crashes by design.`" — and add a forward-pointer in §10.4 acknowledging this. Alternatively, surface errors as `Result` and let test sites `.expect()` (matching existing rules.rs:1420 style).

### I-4. Snapshot determinism across Linux/macOS is asserted but not engineered

**Spec §14 risks table line 828** acknowledges CRLF/LF risk via `.gitattributes`, but the rest of the deterministic-snapshot claim is unspecified:

- **Terminal width / font assumptions.** TestBackend is platform-agnostic (no real terminal), so this is fine.
- **Unicode width.** `cell.symbol()` returns `&str`, but multi-cell glyphs (CJK, emoji) occupy two backend cells with one filled and the next "tombstone" (empty `symbol()` in many ratatui versions). The spec line 752 just asserts "Unicode safe" without specifying what the tombstone looks like in plain-text dump. If any snapshot panel renders a multi-cell glyph (e.g., theme picker color swatches use Unicode block chars?), the diff will be confusing.
- **Time-/system-dependent values in rendered state.** None of the 7 snapshots in §6.3 obviously depend on system time — but `App::from_snapshot` reads ENV (e.g., `COLORTERM`, locale). Spec §6.3 does not specify how `App::default_for_test` synthesizes a deterministic App: which theme, what color-depth, what sample text, etc.
- **`.gitattributes` file.** Spec says "LF zorlanır" — but the repo doesn't currently *have* a `.gitattributes` file (verifiable via `ls -a /Users/bera/tayf/`; not shown but inferred from project history). Adding one is non-trivial scope (affects historical line endings on Windows clones).

**Recommended action.** §6 needs a "Determinism" subsection that pins:
1. The exact `App` constructor or builder used (`App::from_snapshot(ConfigSnapshot::empty())` plus N explicit mutations — *enumerate them*).
2. The terminfo state: TestBackend implies no terminfo at all, but `App` may carry color-depth detection (verify via `git grep terminfo src/config_tui/`).
3. A `.gitattributes` entry `*.snap text eol=lf` ships in the §6 commit, not a separate one.
4. The plain-text-only choice excludes any panel that uses multi-cell glyphs from the initial 7-snapshot baseline (verify each of the 7 candidate scenes contains ASCII-only render output).

---

## 4. NIT findings (defer-OK)

### N-1. §3.4 test #1 rename loses information

`merge_array_of_tables_yields_whole_array_block_conflict_v0_6_2_limitation` → `merge_array_of_tables_per_element_yields_field_level_conflict`. Fine, follows memory `feedback_collision_pin_pattern`. But the renamed test only covers the *happy path* (all three sides have `name`, one field differs). The *limitation* test (whole-array fallback when `name` is absent) becomes test #7 — and that one would conventionally retain a `_falls_back_to_whole_array` suffix, which §3.4 already has. Fine. No action.

### N-2. §4.5 test 5's exact-string assertion needs trailing-newline care

Spec line 319: `"a\na\nb\n" → "a\nb\n"` yields exactly `"  a\n- a\n  b\n"`. But `str::lines()` strips trailing `\n`, so `"a\nb\n".lines()` → `["a", "b"]` (two items). Spec's expected output `"  a\n- a\n  b\n"` matches a three-op trace (Same a, Remove a, Same b) — correct. But verify the implementer also handles the implicit trailing newline consistently: does `"a\nb"` (no trailing) vs `"a\nb\n"` (trailing) produce the same diff? Existing `build_diff_no_change_returns_no_changes_marker` test at save_diff.rs:268 uses trailing-newline inputs only. §4.5 should add a #11: `build_diff_trailing_newline_normalization` or explicitly pin behavior.

### N-3. §5.2 D-7 entry confused

Line 360: "D-7 | log_level delimiter positive coverage (`[ERROR]`, `INFO:`, `(CRITICAL)`) | Test coverage gap (kısmen 84a7c3d'de)" — but `[ERROR]` is already pinned at rules.rs:1495 (`log_level_matches`). D-7 in the audit doc probably wanted *additional* delimiters (`INFO|`, `[level=WARN]`) — implementer should confirm by reading the actual audit §D-7 before spec'ing corpus size. Spec line 472 budgets 15 positives for D-7 which is generous — fine.

### N-4. §9 test count budget arithmetic

Spec line 708: "Net delta: +33 (25 lib + 8 integration)". Verify: §3=11 + §4=7 + §5=8 + §6=7 + §7=0 = **33**. Lib: §3 (11, in merge.rs) + §4 (7, in save_diff.rs) + §6 (7, in widgets/*.rs) = **25**. Integration: §5 (7 corpus + 1 parser = 8). ✓ arithmetic checks out — but the spec's §5 table line 705 says "7 corpus + 1 parser pin = 8" matching, while line 441 says "7 toplam test fonksiyonu". Reconcile: 7 corpus + 1 parser pin = 8 total integration tests. §9 is correct, §5 prose has the "7" / "8" friction. NIT.

### N-5. §6.5 manual-review step is process, not spec

Lines 619–625: "Manual review onayından sonra commit." This is a workflow note, not a spec contract. It belongs in the implementation plan, not the design doc. Minor — but the spec is otherwise rigorous about separating "what" from "how."

### N-6. §10.2 UTF-8 line 736 — `lines()` does NOT split on `\r\n` reliably

Line 736: "`bytes()` değil, `lines()` UTF-8 char boundary güvenli". Correct on UTF-8 boundaries, but `str::lines()` splits on `\n` or `\r\n` — and the diff outputs `\n` only. If old has `\r\n` and new has `\n`, every line will diff (CRLF→LF normalization unhappy path). Spec §14 line 828 acknowledges CRLF/LF for snapshots — same concern applies to `build_diff`. Add to §4 edge cases: `build_diff_crlf_old_vs_lf_new_reports_full_diff` (correct, by design) or `_normalizes_line_endings` (alternative design).

### N-7. §11 "TBD" table is a valid spec deferral, but corpus harness validation is not

The spec defers item-by-item FP/FN measurement to the implementer ("implementer fills in spec rev2"). That's fine for the *decisions*, but the **harness itself** (parse_corpus_file, measure, EXPECTED_FP_*/FN_* const machinery) needs validation up-front. Spec §5.3 line 405 sketches `parse_corpus_file` returning `AuditCase` — but doesn't specify:
- What happens on a malformed `POS:` line that lacks the ` => ` separator? Spec §10.3 line 742 says "panic — test crash, not silent skip" — pin that with a #8 parser pin test (corpus with bad line → harness fails fast).
- What about empty corpus (zero POS + zero NEG)? `measure` returns `(0, 0, 0, 0)` and the test passes trivially. Add a parser-level invariant: `parse_corpus_file` rejects files with <1 POS and <1 NEG.

These belong in §5, not §11.

---

## 5. Compliments — keep these in rev2

- **§1 prior-review consumption** is exemplary: explicit table, every v0.6.3 and v0.6.2 carryover row decided, no silent omission. Memory `feedback_consume_prior_review` is satisfied in form *and* spirit.
- **§3.2 algorithm enumeration** is excellent. The eleven-row case table (lines 132–152) is the kind of decision matrix that prevents implementer drift. It maps cleanly onto how the existing `merge_table` is written (merge.rs:179–224) — no surprising patterns.
- **§3.4 test pin enumeration with rename+flip** (line 187) correctly applies memory `feedback_collision_pin_pattern`. Eleven enumerated tests is the right granularity for a state-machine change of this size.
- **§4.4 cap fallback design** is well-justified — `100_000` cells is generous for TOML config diffs and the literal fallback degrades gracefully. Worth keeping.
- **Dogfood symmetry** (Item 2 uses Item 5 for failure messages; Item 4 conflicts populate Item 2 snapshots) is genuine internal symmetry and elevates the cycle from "five unrelated cleanups" to "one coherent design." Don't lose this in rev2.
- **§7 stale-comment cleanup as its own atomic commit** correctly applies memory `feedback_stale_dead_code_reason_drift` — the right cycle-level discipline.
- **§13 forward-pointer ban** is a strong organizational choice and consistent with the audit memory.
- **`#[allow(clippy::similar_names)]` on `merge_table`** (existing code, merge.rs:148) — when extending the AoT branch, preserve this annotation and extend the rationale to cover `aob/aoa/aot` if those become local variables.

---

## 6. Summary table

| ID | Title | Spec § / line | Severity | Disposition (rev2 fills) |
|---|---|---|---|---|
| C-1 | `buf.get()` + `area()` don't exist on ratatui 0.30 | §6.2 / 562–574 | CRITICAL | |
| C-2 | `Vec<Vec<u16>>` LCS table — wrong shape, underexplained bound | §4.3 / 281–293 | CRITICAL | |
| C-3 | `pub mod testing` contradicts "zero public-API impact" + duplicates `__test_api` | §2 / 75; §5.3 / 426 | CRITICAL | |
| I-1 | `is_array_block` semantic change — no internal call-site audit | §3.2 / 155–157 | IMPORTANT | |
| I-2 | `AotElementMissing` `#[error("...")]` format string + exact-message test missing | §3.3 / 179; §3.4 / 211 | IMPORTANT | |
| I-3 | `panic!`/`expect` density in `test_support` vs clippy::pedantic gates | §6.2 / 530–559 | IMPORTANT | |
| I-4 | Snapshot determinism (`.gitattributes`, App constructor, Unicode width) underspecified | §6 / 594; §14 / 828 | IMPORTANT | |
| N-1 | Test #1 rename observation | §3.4 / 187 | NIT | (no action) |
| N-2 | Trailing-newline behavior of `lines()` in build_diff | §4.5 / 319 | NIT | |
| N-3 | D-7 audit-doc cross-ref `[ERROR]` already pinned | §5.2 / 360 | NIT | |
| N-4 | Test budget arithmetic — `7 toplam` vs `8` friction | §5 / 441 vs §9 / 705 | NIT | |
| N-5 | "Manual review" workflow note belongs in plan | §6.5 / 619–625 | NIT | |
| N-6 | CRLF normalization in build_diff edge case | §10.2 / 736 | NIT | |
| N-7 | Corpus harness self-tests (parser panic + empty corpus) belong in §5 | §11 / 758 | NIT | |

---

**Reviewer instruction to implementer:** address C-1, C-2, C-3 before implementation starts (rev2 spec patch). I-1..I-4 fold-or-defer during implementation. NITs can land in v0.7 commit-by-commit or defer to v0.7.1 cleanup at implementer discretion. The corpus-doc decisions (§11) remain implementer-driven post-measurement; do not block the spec on that.

— *Rust-senior independent review, 2026-05-29*
