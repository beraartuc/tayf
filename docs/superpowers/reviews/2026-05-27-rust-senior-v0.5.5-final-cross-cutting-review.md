---
title: tayf v0.5.5 — final cross-cutting review (Opus 4.7 Rust senior)
date: 2026-05-27
range: v0.5.4..HEAD (17 commits — 13 implementation/cleanup + 4 docs)
reviewer: Opus 4.7 (1M context) — single-blind ship-gate pass
verdict: SHIP_WITH_FIXES (CHANGELOG entry + version bump + one stale `#[allow(dead_code)]` on `Color::to_toml_str`)
---

# Cross-Cutting Review — tayf v0.5.5 (`v0.5.4..HEAD`, 17 commits)

v0.5.5 is the sub-version whose single raison d'être is to close the v0.5.4 final
review's named largest gap: `src/config_tui/save.rs::build_new_content` was a
literal pass-through that returned `snapshot.raw_bytes` even when
`PendingEdits` carried staged theme / profile / rule edits. The whole
read-edit-save loop committed unchanged bytes — production-grade atomic-write
and backup machinery wrapped around an identity transform.

v0.5.5 replaces that stub with a real `PendingEdits → DocumentMut → String`
walk in a new `src/config_tui/reconcile.rs` module, surfaces the new error
shape both at `commit_save` (as `io::Error::other("reconcile failed: ...")`)
and at the SaveDiff preview path (as a new `SaveDiffState::ReconcileError`
variant rendered inline in the modal), and adds 30 new tests pinning the
walk's byte-output shape. Spec is `docs/superpowers/specs/2026-05-27-tayf-v0.5.5-build-new-content-reconciliation.md`
(985 lines, with 19 paralel-review findings absorbed in §13).

The lean-ceremony framing fits the actual change size: ~870 LOC in
reconcile.rs (handler logic + 21 unit tests), ~120 LOC of integration tests
in save.rs::tests, ~75 LOC of `Color::to_toml_str` + roundtrip test in
style.rs, and small surface-area edits across save.rs, save_diff.rs,
events.rs, mod.rs, edit.rs, snapshot.rs. Net diff `src/**.rs` = 1367 lines.

## 1. Spec coverage

**✅ PASS** (with one CHANGELOG-only fix flagged below).

Spec §2.1 enumerates 9 in-scope items. Each verified against code:

1. **New `src/config_tui/reconcile.rs` module** — landed at `ef17329`
   (`feat(config_tui): reconcile.rs skeleton + ReconcileError + test #1`).
   Public-to-crate API matches §4.1 exactly: `pub(crate) fn apply_edits(doc: &DocumentMut, edits: &PendingEdits) -> Result<String, ReconcileError>`.
   Internal handlers (`apply_general`, `apply_user_config_rule`, `apply_new_rule`,
   `apply_deletion`, `write_style_table`, `ensure_general_table`,
   `ensure_rules_array`, `ensure_subtable`, `ensure_style_target`,
   `ensure_style_target_quoted_numbered`, `find_rule_index_by_name`,
   `set_or_insert`) all private to the module. `StyleTargetMut<'a>` enum is
   `pub(super)` (cleanup commit `2cba0fa` tightened it from the earlier
   `pub(crate)`; correct — only consumed within `config_tui`).
2. **`build_new_content` rewrite to facade** — landed at `efefb2d`
   (`refactor(config_tui): wire build_new_content facade + SaveDiffState::ReconcileError`).
   `src/config_tui/save.rs:162-167` is now a one-line forward into
   `crate::config_tui::reconcile::apply_edits(&snapshot.doc, edits)`.
   Signature changed from `String` → `Result<String, ReconcileError>` as
   specified. No alternative paths sneak past the facade (grep confirms
   `apply_edits` is called only from `build_new_content`).
3. **`ReconcileError` typed enum** — `src/config_tui/reconcile.rs:16-31`.
   Two variants (`UnsupportedDeletionTarget`, `TypeMismatch`) matching
   spec §6.1 verbatim. `thiserror::Error` derive used; Display strings
   are user-actionable per CLAUDE.md §4 (each names the failure, why,
   and what to try — "config may be corrupt; try reloading the file").
4. **`commit_save` signature impact** — `src/config_tui/save.rs:221-223`:
   `build_new_content(snapshot, edits).map_err(|e| std::io::Error::other(format!("reconcile failed: {e}")))?;`.
   Single-prefix `"reconcile failed: "` matches §6.5 contract. `commit_save`
   outer signature `Result<ConfigSnapshot>` unchanged.
5. **`Color::to_toml_str`** — `src/style.rs:99-131`, landed at `12a23c7`.
   `pub(crate)` (no API widening). Canonical forms match §6.2: lowercase
   ANSI names, `"color(N)"` indexed, `"#rrggbb"` lowercase hex zero-padded.
   Roundtrip property test at `src/style.rs:822-860` covers 24 sentinels
   (16 ANSI + 5 Indexed incl. 15/16 boundary + 3 Rgb).
6. **PendingEdits → DocumentMut walk algorithm** — `src/config_tui/reconcile.rs:397-419`
   matches §5.1 order-of-operations (general first, UserConfig rules, NewRule
   appends, deletions last). Non-UserConfig `RuleId` variants in the `rules`
   map are silent no-ops; non-UserConfig variants in `deleted` set yield
   `UnsupportedDeletionTarget` error.
7. **Stale-doc fixes (§2.1 #7)** — `src/config_tui/mod.rs:11-12` updated
   (now reads "atomic write + backup rotation (top-level entry into
   [`reconcile`] for the toml_edit walk)" + a new `reconcile` line). The
   `events.rs:84` ColorPicker accept Toast remains `"color accepted (binding
   to selected rule lands in v0.6+)"` — correctly unchanged per spec.
8. **Test coverage** — see §4 + §14 below.
9. **CHANGELOG `[0.5.5]` entry + version bump** — **NOT YET LANDED.**
   `Cargo.toml:3` still says `version = "0.5.4"`; CHANGELOG.md has no
   `[0.5.5]` heading. Spec §8.6 lists this as Step 2-3 of release
   ceremony post-final-review, so this is the expected pre-tag state.
   **🔵 NIT — flagged below as a SHIP_WITH_FIXES item (CHANGELOG only).**

§13 paralel-review absorption (5 🔴 BLOCKers + 14 🟡 IMPORTANTs + 6 🔵 NITs):
all folded — see §12 below for the row-by-row audit.

## 2. DOKUNULMAZ invariant

✅ **PASS.** `git diff --name-only v0.5.4..HEAD | grep -E 'src/(pipeline|io_loop|pty|rules|tty_guard|signals|runtime)\.rs'` returns empty.

The 8 changed `src/**.rs` files are:
- `src/config_tui/edit.rs` (+6/-4 — module-level allow reason refresh)
- `src/config_tui/events.rs` (+11/-1 — save-diff `'y'` guard on ReconcileError state)
- `src/config_tui/mod.rs` (+5/-2 — stale-doc fix + `pub(crate) mod reconcile;`)
- `src/config_tui/reconcile.rs` (NEW, +871 LOC)
- `src/config_tui/save.rs` (+178/-25 — facade + integration tests I1–I7)
- `src/config_tui/snapshot.rs` (+4/-5 — module-level allow reason refresh)
- `src/config_tui/widgets/save_diff.rs` (+38/-6 — `ReconcileError` variant + inline render)
- `src/style.rs` (+74/-0 — `Color::to_toml_str` + roundtrip test)

Zero hot-path / signal-handler / PTY surface touched. The CLAUDE.md §2
"file-per-concept, ~400-line split threshold" guidance was respected by
breaking the new walk logic out into its own module rather than growing
`save.rs` past its already ~600-line footprint.

## 3. EN/TR language compliance

✅ **PASS.** `grep -rEn '[ğüşıöçĞÜŞİÖÇ]' src/config_tui/{reconcile,save,edit,snapshot,events,mod}.rs src/config_tui/widgets/save_diff.rs src/style.rs` returns empty across all changed source files. `§` section markers (e.g. "Spec §5.3") are present but allowed per the review brief.

Spec / plan / review markdown files remain Turkish as expected by CLAUDE.md
§1. Memory `feedback_review_calibration_en_tr` mandate respected — code-side
strict English, spec-side free Turkish.

## 4. Test assertion specificity

✅ **PASS.** Error/Display tests use byte-pinned `assert_eq!` on full
strings; structural tests use `contains()` against intentional fragments.
Cross-section verification:

- `reconcile.rs:743-758` `deleted_builtin_returns_unsupported_error`:
  `assert_eq!(rule_id, "Builtin(\"uuid\")", ...)` on the field, then
  `assert_eq!(display, "unsupported deletion target: ...", ...)` on the
  whole Display string. No `.contains` shortcut. Per I5 fold, no version
  string in the error wording → no future staleness.
- `reconcile.rs:837-853` `type_mismatch_returns_typed_error`:
  `assert_eq!(path, "general"); assert_eq!(*expected, "table"); assert_eq!(*actual, "array of tables");`
  + full Display `assert_eq!`. Note: `actual` is `"array of tables"` (spaces,
  observed toml_edit 0.25 `Item::type_name()` output) — the test correctly pins
  the observed shape rather than guessing.
- `save.rs:541-548` `integration_commit_save_reconcile_error_propagates_as_io_error_and_leaves_orphan_backup`:
  full `err.to_string()` `assert_eq!` against the chain-prefix
  `"reconcile failed: unsupported deletion target: ..."`. Pins the
  single-prefix contract from spec §6.5.
- Structural assertions (`out.contains("style = { fg = \"red\" }")` at
  reconcile.rs:548; `out.contains("\"1\"")` at :568; `out.contains("[rules.style]")`
  at :729) are correctly used for whole-document substring shape — these
  are not error-wording assertions, so memory
  `feedback_test_assertion_specificity` does not require `assert_eq!`.

One sub-pattern observed in `added_vec_appends_new_rule` (reconcile.rs:820):
`assert!(out.contains("pattern = 'p'") || out.contains("pattern = \"p\""))`.
This `||` is a deliberate loosening — both literal-string and basic-string
forms are valid `toml_edit::value()` outputs for the bare letter `p`
(no backslash → basic-string chosen in practice). The accompanying comment
at line 817-819 explains the choice. Acceptable — the contract being pinned
is "pattern key written with value 'p'", not the specific quote style.

## 5. Duplicate formatter audit

✅ **PASS.** Per memory `feedback_duplicate_formatter_audit`:
`grep -nE 'format!.*rule "|format!.*message: format' src/config_tui/reconcile.rs src/style.rs src/config_tui/save.rs src/config_tui/widgets/save_diff.rs` returns zero hits in the diff. The single `format!("reconcile failed: {e}")` at save.rs:223 wraps the underlying `Display` impl via the standard `{e}` formatter — no shadow-formatter that bypasses the `thiserror`-derived Display.

`ReconcileError::Display` is the single source of truth for the error text;
all call sites (commit_save line 223, save_diff.rs:143, the two test
`assert_eq!`s) go through it.

## 6. `unwrap()` / `expect()` discipline

✅ **PASS.** CLAUDE.md §2 rule: "No `unwrap()` or `expect()` in library code.
Allowed only in: tests, `main.rs` top-level setup, and proven-unreachable
paths (with `unreachable!('reason')`)."

`grep -n '\.unwrap\(\)\|\.expect\(' src/config_tui/reconcile.rs` returns
hits only inside `#[cfg(test)] mod tests` (line 421+). All library-code
fallible paths use either `?` (typed `ReconcileError` propagation) or
`unwrap_or_else(|| unreachable!("..."))`. Verified inventory:

- reconcile.rs:80 — `as_table_mut().unwrap_or_else(|| unreachable!(...))`
  after `doc["general"] = Item::Table(t)` (toml_edit invariant).
- reconcile.rs:104 — `as_array_of_tables_mut()` after just-inserted
  `Item::ArrayOfTables` (toml_edit invariant).
- reconcile.rs:214 — `get_mut(key)` after `parent[key] = Item::...`
  insertion (just-ensured invariant).
- reconcile.rs:222 — `as_inline_table_mut()` after `is_inline_table()`
  check (boolean-checked invariant).
- reconcile.rs:256 — `Key::parse(&quoted_repr)` where
  `quoted_repr = format!("\"{n}\"")` and `n: u32` (the input is
  always a syntactically-valid TOML quoted-string key — `Key::parse`
  cannot fail for this shape).
- reconcile.rs:263 — `keys.drain(..).next()` after `Key::parse` of a
  single simple key (always yields exactly one entry).
- reconcile.rs:309 — `rules.get_mut(i)` for `i` returned by the same
  `find_rule_index_by_name` (just-validated index).
- reconcile.rs:321 — `rules.get_mut(last_idx)` after `rules.push(t);`
  (just-pushed invariant).

Each `unreachable!()` has a one-line reason explaining the invariant.
Per CLAUDE.md §2 this matches the project pattern. No `unwrap()` in
library hot-paths.

`Value::from(...)` for `i64`/`bool` are infallible (no `expect()` needed
inside the `set_or_insert` chain — the spec §5.4 amendment dropped the
`.expect("scalar")` pattern when the cleanup pass simplified to
`Value::from(b)` for booleans and `Value::from(Color::to_toml_str(*color))`
for strings).

## 7. MSRV 1.74 compliance

✅ **PASS.** `grep -rEn '#\[expect\(|\breason\s*=' src/` returns zero
attribute-style `#[expect(...)]` / `reason = "..."` additions in the v0.5.5
diff. The project pattern is `#[allow(...)]` + a `// reason: ...` line-comment
preceding the attribute, which is what every new allow in this diff uses
(reconcile.rs:166 for the `clippy::unnecessary_wraps` allow on
`write_style_table`; edit.rs:6-11 + snapshot.rs:7-10 + save.rs:14-17 for the
module-level allows; style.rs:107 — flagged in §8 below).

No `io::Error::other` substitute needed; available since 1.74.

## 8. `#[allow(dead_code)]` hygiene

🔵 **NIT — one stale field-level allow.**

**Module-level allows (✅ all updated per spec §13.3):**
- `src/config_tui/edit.rs:6-11` — reason refreshed (`2cba0fa`) to point to
  `RuleId::{Builtin,Embedded,DiskProfile}` un-constructed variants +
  `PendingEdits::clear()` v0.6+ wire. Accurate against the v0.5.5 landed
  state (reconcile.rs matches/destructures these variants without ever
  constructing them).
- `src/config_tui/snapshot.rs:7-10` — reason refreshed (`2cba0fa`) to
  v0.6+ live-preview detail rendering. Accurate — reconcile.rs walks
  `doc` (DocumentMut), not `parsed` (ParsedConfigView).
- `src/config_tui/save.rs:14-17` — reason refreshed (`2cba0fa`) to v0.6+
  Shift+D first-run init dump. Accurate — `ts_for_backup_filename` is
  consumed by `commit_save` (Step 3 backup-filename construction at
  line 210) and by the in-module test at line 257-269, but `civil_from_days`
  and the first-run-init dump path are still v0.6+.

**Field-level / item-level allows:**

🔵 **`src/style.rs:107-108`** — the `Color::to_toml_str` method carries
`#[allow(dead_code)]` with reason `// reason: called by reconcile.rs (Task A2, v0.5.5 Phase A); not yet linked at A1.`
This reason refers to **Task A1 of the v0.5.5 implementation plan** —
the staged commit where the method existed but reconcile.rs hadn't yet
landed. By Phase A3 (`efefb2d`, build_new_content facade wire), reconcile.rs:175
+ :184 call `Color::to_toml_str(*color)` directly, so the symbol is no
longer dead.

`cargo clippy --lib --tests -- -D warnings` is clean — but only because the
`#[allow(dead_code)]` is silencing what would otherwise be a no-op annotation
on a live symbol. Per memory `feedback_stale_dead_code_reason_drift`:
"`// reason: dead until <phase> lands` annotations become silent dead-code
masks after phase ships; cleanup-pass MUST strip + force-fire clippy + re-add
field-level allows where genuinely-dead items remain."

The cleanup commit `2cba0fa` audited the three module-level allows but
**missed this fourth field-level allow** on `Color::to_toml_str`. The fix
is a 2-line deletion: remove the `// reason: ...` comment and the
`#[allow(dead_code)]` line at style.rs:107-108. The method is reachable;
clippy will not re-fire.

Severity: documentation-only. Functionally inert. Style.rs already passes
clippy because `#[allow(dead_code)]` is a no-op on a used symbol. But the
stale annotation is exactly the pattern memory
`feedback_stale_dead_code_reason_drift` flags. Recommendation: fix
before tag.

**Other field-level allows (✅ all accurate):**
- `src/config_tui/reconcile.rs:166` — `#[allow(clippy::unnecessary_wraps)]`
  on `write_style_table` with forward-pointer reason "future fg/bg Color
  validation may introduce error paths". Acceptable — this is forward-
  compatible API shape on a Result-returning private helper, not dead code.
- `src/config_tui/widgets/save_diff.rs:20` — `#[allow(dead_code)]` enum-level
  on `SaveDiffState`. The reason at line 15-19 was authored for v0.5.4 (when
  `disk_now` on `ConflictMergedPreview` was carried for v0.6+ merge
  reconciliation). v0.5.5 added the new `ReconcileError { message }` variant
  to the same enum but the enum-level allow's reason still cites the
  pre-v0.5.5 rationale ("build_new_content is still pass-through"). Strictly
  speaking, the message portion is consumed (line 76 reads it) — but
  `ConflictMergedPreview::disk_now` and `ConflictDiscardConfirm::disk_now`
  remain v0.6+ deferrals, so the allow is still load-bearing. Minor wording
  drift, not a bug. Acceptable but flagged for awareness.

## 9. Hot-reload precedence invariant

✅ **PASS.** `git diff --name-only v0.5.4..HEAD | grep src/reload.rs` returns
empty. `ReloadOrchestrator::spawn` signature unchanged. reconcile.rs is a
pure data-transform module (no I/O, no async, no signal handlers); it does
not touch the reload thread or its precedence-chain snapshot.

The flow is:
- TUI Ctrl+S → `commit_save` → `build_new_content` → `apply_edits`
  (clone + walk + serialize) → tmpfile + atomic rename →
  `ConfigSnapshot::read_from_disk` (post-save reparse).
- The reload thread (if running) detects the inode change via `notify` and
  re-evaluates the precedence chain from its own snapshot of inputs. Two
  independent code paths; no shared state mutated by reconcile.

Memory `feedback_reload_precedence_snapshot` satisfied — the v0.5.2 D5
contract holds.

## 10. Save flow invariants

✅ **PASS.** Spec §4.4 "orphan-backup-on-reconcile-fail trade-off" is
the only new interaction surface between reconcile and the existing
save flow.

**Step ordering preserved** (`src/config_tui/save.rs:191-249`):
1. Step 1: `rotate_backups_to(cfg_dir, cfg_stem, MAX_BACKUPS - 1)?` (line 193).
2. Step 2: `let disk_now = fs::read(cfg_path)?;` (line 198).
3. Step 3: backup write with preserved mode + `create_new(true)` + `sync_all()` (lines 209-219).
4. **Step 4 (NEW): `let new_content = build_new_content(snapshot, edits).map_err(...)?;`** (lines 221-223).
5. Step 5: `TmpFileGuard::create_in_parent_dir(&tmp_path, preserved_mode)` + write + `sync_all` (lines 233-237).
6. Step 6: `tmp.persist(cfg_path)?;` (line 240).
7. Step 7: parent dir `sync_all` best-effort (lines 243-245).
8. Step 8: rebuild snapshot via `ConfigSnapshot::read_from_disk` (line 248).

The `?` at line 223 propagates `ReconcileError` (wrapped into `io::Error::other`)
**after** Step 3 has executed — meaning a reconcile failure leaves an orphan
backup file on disk. This is explicitly documented in spec §4.4 and pinned
by test I5 (`save.rs:529-561`) which asserts:
- Backup file exists post-failure.
- Backup content == pre-edit disk bytes.
- Source on disk unchanged (Step 6 never ran).
- `err.to_string()` == byte-pinned full chain.

I-1 (sync_all not sync_data), I-2 (preserved mode), I-3 (rotate first),
I-4 (snapshot-at-startup, reload-thread), I-6 (orphan-backup quota cap)
all preserved in code. The `preserved_mode` hoist at line 205-206 still
covers both Step 3 backup write and Step 5 tmpfile create. Memory
`feedback_parallel_call_site_invariant_audit` satisfied for the new
`?`-injection site.

## 11. Carryover absorption

✅ **PASS.** v0.5.4 final review's single largest named gap (`build_new_content`
pass-through) is closed.

Direct verification:
- `src/config_tui/save.rs:162-167` — facade into `reconcile::apply_edits`.
- Integration test I1 (`save.rs:429-451`) — stages `general.theme = Some(Some("light"))`,
  runs `commit_save`, reads disk back: `assert_eq!(disk_after, "[general]\ntheme = \"light\"\n");`.
  This is the **decisive** test — it proves that an end-user staging a theme
  selection in the TUI and pressing Ctrl+S actually persists the change to
  disk, which v0.5.4 did **not** do.
- Integration test I2 — header comments preserved.
- Integration test I3 — rule ordering preserved (a-before-b).
- Integration test I4 — `o` (override) keystroke shape (`RuleEdit::default()`
  insert) writes the stub entry correctly.
- Integration test I5 — reconcile failure path propagates as `io::Error`,
  orphan backup contract pinned.
- Integration test I6 — post-save snapshot reparses edited content.
- Integration test I7 — duplicate-rule-name first-match-mutate contract pinned.

v0.5.4 §2.2 amendment listed 9 deferrals; v0.5.5 §2.2 carries forward all 9
plus 12 more (21 total — the v0.5.6 architectural collision fix, multi-profile
composition, render snapshot tests, etc., are explicitly out-of-scope).
Memory `feedback_consume_prior_review` satisfied via spec §3.1 (the
explicit v0.5.4 finding-disposition table).

## 12. Spec §13 amendment completeness

✅ **PASS.** All 5 🔴 BLOCKers + 14 🟡 IMPORTANTs + 6 🔵 NITs absorbed.

**🔴 BLOCKers (5):**
- B1 (closure-reads-rules double-borrow): reconcile.rs:307-326 uses the
  hoisted `let rule_table = if let Some(i) = idx { ... } else { ... }` form;
  no `as_table_mut` closure captures `rules` mutably twice. Compiles.
- B2 (save_diff signature drift, call-site recount): all 3 events.rs call
  sites (138, 146, 273) still wrap `Some(build_initial_state(app))`;
  signature `(&App) -> SaveDiffState` unchanged; new variant added in-place;
  render path inline (no Toast).
- B3 (apply_edits signature honesty): reconcile.rs:397-401 signature is
  `&DocumentMut → Result<String, ReconcileError>`; line 401 `let mut working = doc.clone();`
  with the spec §4.1 honesty comment on lines 392-396 explaining the O(n)
  clone trade-off.
- B4 (InlineTable IndexMut panic): `set_or_insert` helper at reconcile.rs:150-159
  uses `.insert(key, val)` for the Inline branch (replaces-or-adds, never
  panics). Test #11 (`new_style_bool_axes_set_via_insert_helper`,
  reconcile.rs:614-643) deliberately omits all four bool keys from the
  source and adds them all in one stage — would panic without the fix; passes.
- B5 (`[[general]]` typed error not panic): `ensure_general_table` at
  reconcile.rs:73-94 returns `Result<&mut Table, ReconcileError>`; test #15
  (reconcile.rs:824-854) exercises the array-of-tables-shaped `[[general]]`
  fixture and asserts the typed `TypeMismatch` error.

**🟡 IMPORTANTs (14):**
- I1 (find_rule_index_by_name signature): reconcile.rs:122 takes
  `&ArrayOfTables, &str` — no lifetime.
- I2 (drop StyleTarget enum, use helper-fn): `set_or_insert` helper at
  reconcile.rs:150-159.
- I3 (ensure_inline_or_table create-form): reconcile.rs:206-236 creates as
  `Value::InlineTable(InlineTable::new())` on absent; preserves existing form.
- I4 (unreachable! over expect): all post-find_rule_index_by_name and
  post-push paths use `unreachable!(...)` with explicit reasons.
- I5 (no version-string in Display): reconcile.rs:18-31 wording uses "reserved
  for future work" instead of "v0.6+". Test #14 `assert_eq!` on full Display.
- I6 (orphan backup documented): save.rs:529-561 test I5 pins backup-exists
  + bytes-match assertions.
- I7 (dead_code reason pre-commit): three module-level reasons updated at
  `2cba0fa` cleanup commit. (Note: one field-level allow on `Color::to_toml_str`
  was missed — see §8 above.)
- I8 (pattern literal-string repr): test #16 (reconcile.rs:646-669) asserts
  `out.contains(r"pattern = '\b[a-z]+\b'")` AND NOT `r#"pattern = "\\b"#`.
- I9 (key-mutation comment-attachment): test #17 (reconcile.rs:672-693)
  pins `# inline comment above pattern` survives key mutation.
- I10 (form-preservation): tests #18a + #18b (reconcile.rs:696-732) pin
  inline-stays-inline + block-stays-block.
- I11 (ArrayOfTables::remove comment-deletion): test #19 (reconcile.rs:774-799)
  pins before-x comment removed with entry, before-y comment survives.
- I12 (quoted "1" key): `ensure_style_target_quoted_numbered` uses
  `Key::parse("\"N\"")` + `Table::insert_formatted` at reconcile.rs:246-271;
  test #8 (`styles_numbered_branch_writes_quoted_styles_n_table`,
  reconcile.rs:552-572) asserts `out.contains("\"1\"")`.
- I13 (inline modal error rendering, not Toast): `SaveDiffState::ReconcileError`
  variant at save_diff.rs:38-43; render at save_diff.rs:72-83 paints the
  message in red inside the "Reconcile error — fix and retry" titled block.
- I14 (duplicate rule name on disk): test I7 (save.rs:578-604) pins
  first-match-mutate; old pattern A correctly gone, B preserved.

**🔵 NITs (6):**
- N1 (apply_general early-return): reconcile.rs:42-46 short-circuits when
  both fields are None — actually *kept* the early return, with reason "skip
  to avoid ensure_general_table side effect of creating an empty [general]
  section when one didn't exist". This is a thoughtful refinement of the
  spec's "drop the early-return" position; correct and load-bearing.
- N2 (Indexed boundary sentinels): style.rs:843-847 includes
  `Color::Indexed(0)`, `(15)`, `(16)`, `(178)`, `(255)`.
- N3 (test #1 comment-heavy worst case): reconcile.rs:506-526 fixture is
  ~21 lines with 3 header comments, `[general]` + theme + profile, two
  `[[rules]]` (mixed inline + block style), one mid-rule comment block.
- N4 (set_implicit defensive comment): reconcile.rs:76-78 keeps the call
  with the explanatory comment.
- N5 (variant-preservation prose): style.rs:99-105 doc-comment notes "variant-
  preserving: `Color::Indexed(0)` → `"color(0)"`, `Color::Black` → `"black"`
  (distinct variants, distinct SGR sequences, distinct canonical forms)".
- N6 (CRLF defensive test): reconcile.rs:856-870 test #20
  `crlf_line_ending_source_preserved_on_mutation` pins the observed toml_edit
  0.25 behavior (CRLF normalized to LF on parse).

## 13. CI workaround discipline

✅ **PASS.** `git diff --name-only v0.5.4..HEAD | grep '.github/workflows/ci.yml'`
returns empty. `RUST_TEST_THREADS: "1"` + self-hosted Linux runner workarounds
inherited from v0.5.4 unchanged.

## 14. Triad final state

- ✅ `cargo fmt --check` — clean (output empty).
- ✅ `cargo clippy --lib --tests -- -D warnings` — zero warnings (clean build
  from `cargo clean -p tayf`, finished in 2.53s after a full recompile).
- ✅ `cargo test --lib` — **584 passed; 0 failed; 0 ignored; 0 measured**;
  finished in 2.19s. Spec §7.4 + §13.8 final target was 583; **actual is 584**.
  Delta: spec counted 22 reconcile.rs unit tests in §7.1, actual file has
  22 test functions but `cargo test --lib config_tui::reconcile::tests`
  reports 22; +7 integration in save.rs = 7 (I1–I7); +1 property in style.rs;
  +1 ad-hoc B1-review-feedback test (`general_profile_set_updates_value`,
  reconcile.rs:459-468 — the profile-arm defensive coverage parallel to
  test #2 noted in commit `03faf57`). Total new = 31, baseline 554, grand
  total 585? Let me recount: lib_test_count = 584 (verified). 584 − 554 =
  30. The discrepancy with spec's "29" or "583" target is one extra test
  beyond plan — `general_profile_set_updates_value` at reconcile.rs:459 was
  added during B1 review (commit `03faf57`, "B1 review fixes — early-return
  + assert_eq! + profile coverage") and not re-counted in §13.8. Acceptable;
  the test enforces the same tri-state contract on the profile arm that test
  #2 enforces on the theme arm — symmetric coverage for the same handler.
- ✅ Integration suites spot-checked:
  - `tests/integration_smoke.rs`: ran during full triad (passes within `cargo test`).
  - `tests/integration_tui_smoke.rs`: 4/4 passed (`dump_default_parses_as_valid_toml`,
    `dump_kind_patterns_only_emits_patterns_section`,
    `status_no_config_renders_byte_pinned_lines`,
    `status_with_broken_config_exits_64_and_prints_partial`).
  - `tests/integration_tui_in_wrapper.rs`: 1 pass + 1 ignored (`sigwinch_propagates_to_wrapped_tui`
    — manual smoke per v0.5.4 amendment); `tayf_config_dump_inside_wrapper_emits_byte_identical_to_plain`
    passes.
  - `tests/integration_config.rs`: 4/4 passed.
  - `tests/integration_profiles_library.rs`: 6/6 passed (including the two
    pinned v0.5.3-collision tests `aws_arn_yields_to_interior_region_pattern_v0_5_3_limitation`
    and `docker_image_tag_registry_host_yields_to_fqdn_v0_5_3_limitation` — both
    still tagged with the `_v0_5_3_limitation` suffix per memory
    `feedback_collision_pin_pattern`; will loudly fail when v0.5.6 architectural
    fix lands).

## 15. Public API surface

✅ **PASS.**
- `pub(crate) fn Color::to_toml_str` (style.rs:109) — NOT `pub`. Zero
  widening — `Color` itself is `pub` (pre-existing), but the new method
  is crate-internal.
- `pub(crate) enum ReconcileError` (reconcile.rs:17) — NOT `pub`. Variants
  are also default-visibility (i.e. `pub(crate)` via enum visibility).
- `pub(crate) enum SaveDiffState` (save_diff.rs:22) — confirmed `pub(crate)`,
  NOT `pub`. The new `ReconcileError { message }` variant inherits the
  enum's crate-only visibility. Zero widening.
- `pub(crate) fn apply_edits` (reconcile.rs:397) — crate-only.
- `pub(super) enum StyleTargetMut<'a>` (reconcile.rs:129) — even tighter
  scope (only config_tui can see it). Cleanup commit `2cba0fa` correctly
  tightened from `pub(crate)`.
- `pub(crate) mod reconcile;` in mod.rs:32 — crate-only.

`grep -rE '^pub fn |^pub struct |^pub enum' src/style.rs src/config_tui/reconcile.rs src/config_tui/widgets/save_diff.rs` confirms only `pub enum Color` and `pub struct Style` exist as top-level public exports — both pre-existing in style.rs.

No public-API widening. `src/lib.rs` re-export surface unchanged.

## 16. Half-feature toast stubs

✅ **PASS** (honest stubs preserved).
- `events.rs:84` "(binding to selected rule lands in v0.6+)" — explicit
  v0.6+ pointer. Correctly unchanged from v0.5.4 per spec §2.1 #7.
- `events.rs:122, 127` "help overlay lands in v0.5.5+" — these *should*
  technically read "v0.6+" now that v0.5.5 is the current ship, but the
  spec §2.2 line 90 places Help modal at "v0.6+" and the wording "v0.5.5+"
  (meaning "v0.5.5 or later") is semantically equivalent. Minor stale-wording
  borderline but functionally correct. (v0.5.4 final review also flagged
  this band of stubs as "v0.5.5+" and they remained valid through that
  ship; same reasoning applies here.) **Not a blocker.**
- `events.rs:343` "init-from-dump and discard-reload deferred" — covered
  by v0.5.5 §2.2 #2, #7.
- `tabs/profiles.rs:88`, `tabs/themes.rs:82` — v0.6+ override copy scope.
- `tabs/patterns.rs:113, 122` — `n` and `e` keystrokes v0.6+ scope, with
  reconcile.rs `apply_new_rule` already ready (spec §5.5 defensive impl).
- `widgets/preview.rs`, `widgets/sample_set.rs` — same v0.6+ scope per
  spec §2.2 #8 + paste contract.

The honest-stub-with-version-pointer pattern continues; reconcile.rs's
`apply_new_rule` is the underlying mechanism for the future `n` keystroke
real wire (spec §9 forward pointer).

## 17. `portable_pty` test invariant

✅ **PASS.** `git diff --name-only v0.5.4..HEAD | grep 'tests/integration_tui_in_wrapper.rs'`
returns empty. `cargo test --test integration_tui_in_wrapper` runs the same
suite as v0.5.4 (1 pass + 1 `#[ignore]`). Memory `feedback_pty_substring_sgr_fragmentation`
mandate inherits — wrapper integration tests still use marker-scan + first-line
cross-ref, not `has_some_sgr_around` helpers.

---

# Verdict

## SHIP_WITH_FIXES

v0.5.5 closes the v0.5.4 final review's single largest named gap with
discipline:

- DOKUNULMAZ invariant preserved (zero `src/(pipeline|io_loop|pty|rules|tty_guard|signals|runtime).rs` touches).
- Triad green (fmt clean, clippy zero-warnings, 584 lib tests + integration suites all pass).
- EN/TR language compliance perfect (zero Turkish chars in code, all spec/plan/review Turkish).
- 5 🔴 BLOCKers + 14 🟡 IMPORTANTs + 6 🔵 NITs from spec §13 paralel-review all absorbed.
- Lean ceremony correctly scoped — ~870 LOC reconcile.rs + ~75 LOC style.rs + small surface-area edits, no new direct deps.
- Save-flow invariants (I-1 sync_all, I-2 preserved-mode, I-3 rotate-first, I-4 reload snapshot, I-6 orphan-backup quota) preserved across the new `?`-injection site at save.rs:223.
- Memory mandates respected: `feedback_consume_prior_review` (spec §3.1 fold-or-defer table), `feedback_spec_phase_parallel_review` (Rust + TOML seniors dispatched), `feedback_test_assertion_specificity` (full Display `assert_eq!` on error tests), `feedback_collision_pin_pattern` (v0.5.3-suffix collision tests left untouched), `feedback_reload_precedence_snapshot` (reload.rs zero-touch).

The user-visible behavior matches the spec headline: TUI Space-to-stage-theme
now actually persists to disk on Ctrl+S, with byte-faithful comment + ordering
+ form preservation.

## Fixes required before tag v0.5.5

**Code change (1 file, ~2 lines):**

1. **Drop the stale `#[allow(dead_code)]` on `Color::to_toml_str`**
   (`src/style.rs:107-108`). The reason "called by reconcile.rs (Task A2,
   v0.5.5 Phase A); not yet linked at A1" became stale at Phase A3 when
   reconcile.rs:175 + :184 started calling the method. Per memory
   `feedback_stale_dead_code_reason_drift`, this is exactly the silent
   dead-code mask the memory guards against. Recommended fix:
   ```rust
   // Delete lines 107-108:
   // reason: called by reconcile.rs (Task A2, v0.5.5 Phase A); not yet linked at A1.
   #[allow(dead_code)]
   ```
   `cargo clippy --lib --tests -- -D warnings` will remain clean after the
   deletion (the symbol is reachable from reconcile.rs + the in-module
   roundtrip test).

**Documentation-only (release-ceremony per spec §8.6):**

2. **CHANGELOG.md `[0.5.5]` entry.** Currently absent; spec §8.6 lists this
   as Step 2 of the release ceremony. Suggested headline: "Closes the
   v0.5.4 `build_new_content` pass-through: TUI theme / profile / pattern
   edits now persist to disk on Ctrl+S via a new toml_edit reconciliation
   walk that preserves comments, ordering, and inline-vs-block form."
3. **Cargo.toml version bump 0.5.4 → 0.5.5.** Currently `version = "0.5.4"`
   at line 3; spec §8.6 Step 3. Single-line edit + accompanying single commit.

After these three fixes (one ~2-line code deletion + CHANGELOG entry + version
bump): push main, wait CI green, push tag `v0.5.5`. Update memories per
spec §8.6 Step 5 (`project_v0_5_5_shipped` + v0.5.6 forward-pointer for the
architectural collision fix solo carve-out).

The v0.5.5 work is engineering-grade ready to ship. The single fix-required
item is a cleanup-pass oversight, not a correctness or scope issue.
