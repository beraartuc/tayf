# v0.5.4 spec re-review — Rust senior

**Reviewer:** Opus 4.7 (rust-senior persona, same hat as initial review)
**Date:** 2026-05-26
**Spec (revised):** `docs/superpowers/specs/2026-05-26-tayf-v0.5.4-config-tui.md` (`2e4298a`)
**Initial review:** `docs/superpowers/reviews/2026-05-26-rust-senior-v0.5.4-spec-review.md` (`ef6b661`)

## Verdict

**NEEDS_REVISION**

All three Critical findings (C-1, C-2, C-3) are properly folded. Most Important findings are well-folded with named test pins. However, the revision **incompletely applied I-8** (the `dump.rs` / `status.rs` → `_cmd` suffix rename) — six stale references remain across §8.5, §10.2, §10.8, §11.5, §13.2. This will compile but the spec contradicts itself; the implementer will pick whichever name they read first. Additionally the revision introduced **two new spec-structural defects** (duplicate `§11.4` heading; `§9.6` inserted out of order before `§9.4`/`§9.5`) and **one minor binding collision** (capital `D` first-run shortcut vs. lowercase `d` patterns-delete — uppercase-vs-lowercase distinction is real but undocumented in the §12.1 / §12.2 cross-reference). These are mechanical cleanup, not architectural; a single revision pass closes them.

The architectural folds (atomic write hardening, conflict-y merge semantic pin, `Args` break declared, ratatui feature line) are excellent — exactly what the initial review asked for, and the new `TmpFileGuard` abstraction is correctly scoped.

---

## §1. Folding verification

### Critical findings

- **C-1 — ratatui feature gate** — ✅ FOLDED — §11.1 lines 1022-1028 + §5.4 Cargo.toml row line 320 + §11.1 rationale block lines 1031-1032. Feature set is now `["crossterm", "underline-color", "layout-cache", "macros", "all-widgets"]` — exactly the recommended set. §11.2 transitive table line 1040 also says "feature set §11.1 explicitly enables `all-widgets` to pull in the widgets sub-crate" — table↔Cargo line no longer contradict.

- **C-2 — `toml_edit 0.23` stale** — ✅ FOLDED — bumped to `0.25` everywhere (§1 line 24, §2.1 line 42, §11.1 line 1029, §5.4 line 320, §11.2 line 1041). New §11.1 fold paragraph lines 1034 explicitly cites `feedback_dependency_minimalism` and pins that 0.25 is on `winnow 1.x` (matches existing transitive, may shrink the `src/lib.rs:47` allow-comment). A1 verification step at §13.1 line 1231 cleanly captures the post-bump audit. Solid.

- **C-3 — `Args` rename is public-API break** — ✅ FOLDED — §4.3 (renamed `Backward compat invariants + acknowledged API break`) lines 215-222 declares the break, names CHANGELOG `### Changed (breaking)` requirement, names the rejected shim alternative with rationale. §5.3 line 309 cross-refs back. §13.9 release ceremony step 3 line 1281 includes the `### Changed (breaking)` mandate with example migration comment. Honest break, fully owned. Exactly option (a) from the recommendation.

### Important findings

- **I-1 — atomic write sync_all + EXDEV + macOS APFS** — ✅ FOLDED — §8.1 commit flow step 4 (line 697) uses `tmp.sync_all()`, step 4 (line 682) has `debug_assert_eq!(tmp_path.parent(), cfg_path.parent(), "tmpfile MUST be in target's parent dir (EXDEV safety)")`, step 6 (lines 706-713) wraps parent-dir `sync_all` in `if let Ok(dir)` + APFS warn-log. All three sub-fixes in one block. Test pin at §10.2 `save.rs` row line 927 `tmp_path.parent() == cfg_path.parent() invariant`.

- **I-2 — mode preservation + tmpfile cleanup on panic** — ✅ FOLDED — §8.1 step 4 lines 676-695 has `preserved_mode` snapshot with `0o600` default + `TmpFileGuard::create_in_parent_dir` RAII. §8.1 invariants block lines 729-730 pins both. Test pins at §10.2 line 927 `mode preservation` + `TmpFileGuard::drop unlinks tmpfile on panic`. Custom guard chosen over `tempfile` prod-dep per `feedback_dependency_minimalism` — defensible reasoning made explicit. **NOTE on `TmpFileGuard` API contract clarity** — spec describes Drop behavior + `persist()` semantics but does not pin the exact method shape; see §2 New Issue NI-3 below for a small clarification request.

- **I-3 — backup rotation order + read_dir failure** — ✅ FOLDED — §8.1 step 1 (line 653) "rotate_backups_to(target_max = MAX_BACKUPS - 1 = 4)" with inline comment "rotate BEFORE backup write". §8.3 (line 755) explicitly pins the new order + read_dir failure path. Test pin §10.2 `save.rs` row "backup rotation (7 backups → save → 5 remain, newest)". Excellent.

- **I-4 — `feedback_reload_precedence_snapshot` name-checking** — ✅ FOLDED — §3.3 table line 104 corrected ("Earlier draft name-checked this memory inaccurately — corrected per Rust senior I-4"). §8.5 line 836 adds the actual additive-invariant paragraph: "ReloadLogger adding does NOT change any v0.2.1/v0.5.1/v0.5.2 precedence-chain snapshot…". This is option (b) from recommendation — strictly the better fold. Test pin §10.2 `reload.rs` row line 935 `reload_logger_does_not_affect_precedence_snapshot`.

- **I-5 — Scenario C input/output direction + SIGWINCH** — ✅ FOLDED — §8.4 row C lines 766 rewritten to explicitly separate **Output direction** (v0.3.0 state machine) from **Input direction** (always passthrough, unrelated to v0.3.0) from **SIGWINCH propagation** (kernel TTY layer → wrapper ioctl → child). Integration test pin §10.3 line 951 `sigwinch_propagates_to_wrapped_tui`. Clean.

- **I-6 — Scenario D merge-collision semantic** — ✅ FOLDED — §8.4 row D line 767 documents last-writer-wins-by-key + names the test pin `merge_collision_user_config_name_clobbers_silently`. §8.1 invariants line 731 mirrors. §10.2 `save.rs` row test enumerated. Option (a)+(c) from recommendation — lean.

- **I-7 — `reload.log` size warn cadence + concurrent append** — ✅ FOLDED — §8.5 lines 832-834 add per-append threshold check (`last_warned_size: Option<u64>` field-state); POSIX `O_APPEND` ≤ PIPE_BUF atomicity claim is explicit. Test pins §10.2 `reload.rs` row: `concurrent_appends_do_not_interleave_within_line` + `size_threshold_warn_per_append`. Good.

- **I-8 — `dump.rs` / `status.rs` module-name overload** — ⚠️ **PARTIAL — IMPLEMENTATION INCOMPLETE** — file tree §5.1 lines 263-264 + LOC table §5.2 lines 292-293 ARE renamed to `dump_cmd.rs` / `status_cmd.rs` + §5.1 rationale paragraph line 267 explains the convention. But the rename was NOT propagated to:
  - §8.5 line 814 `**Status reader side** (\`src/config_tui/status.rs\`):` — should be `status_cmd.rs`.
  - §10.2 line 929 `| \`dump.rs\` | All-kind dump output…` — should be `dump_cmd.rs`.
  - §10.2 line 930 `| \`status.rs\` | Resolve theme/profile/bg…` — should be `status_cmd.rs`.
  - §10.8 line 1009 `app.rs, events.rs, edit.rs, save.rs, snapshot.rs, debounce.rs, \`dump.rs\`, \`status.rs\`: %70+` — both old names.
  - §11.5 line 1081 `\`src/config_tui/dump.rs\` — serialize output…` — should be `dump_cmd.rs`.
  - §13.2 line 1237 `**B1** — \`dump.rs\` impl + unit tests…` — should be `dump_cmd.rs`.
  - §13.2 line 1238 `**B2** — \`status.rs\` impl + \`reload.rs\` additive logger…` — should be `status_cmd.rs`.

  This is the cleanest demonstration of incomplete fold I've seen in this review cycle: the rename was announced, the rationale was written, the tree was updated — and then six call-sites were missed. Per CLAUDE.md §4 ("rename the whole module in one commit") this is a project-rule miss; per memory `feedback_phase1_grammar_gate_blind_spot` "grep ALL call sites before declaring scope complete." Single `grep -n "dump\.rs\\|status\.rs" spec.md` would have caught all six. **MUST fix before plan**.

- **I-9 — `#[non_exhaustive]` on `RunArgs` + clap-derive interaction** — ✅ FOLDED — §13.1 A1 verification step line 1230 "Verify `#[derive(ClapArgs)] + #[non_exhaustive]` compiles cleanly on clap 4.6 — there was an early-clap-4 bug here, possibly fixed; if it fails, drop `non_exhaustive` from `RunArgs` only (keep on `Args`)". §15 #5 also reflects this as a Phase A1 verification deferral. Clean.

- **I-10 — `config status` exit code policy** — ✅ FOLDED — §4.4 table line 231 now shows `**64 (EX_USAGE)**`; §4.4 lines 233 fold paragraph names option (c) from recommendation explicitly. §15 #3 marked **DECIDED**. Clean.

### Nits

- **N-1 — `version_str()` regression** — ➖ N/A — not explicitly addressed but §4.1 lines 122-123 shows `#[command(name = "tayf", author, version = version_str(), about, …)]` on the wrapper `Args` and lets `RunArgs` flatten — implicitly OK. Not load-bearing for the spec.

- **N-2 — `StyleKey::Numbered(u32)`** — ➖ N/A — informational nit; not folded, not required.

- **N-3 — `rfc3339_ms_filename_safe` helper** — ✅ FOLDED — renamed to `ts_for_backup_filename(now: SystemTime) -> String` in §8.1 step 2 line 664 + §8.3 line 752 + test pin `ts_for_backup_filename_byte_pinned` in §10.2 row line 927.

- **N-4 — `/tmp/ratatui-size-check` phantom citation** — ✅ FOLDED — §11.3 lines 1047 add "Methodology pin (N-4 fold)" paragraph + new §11.4 "Manual-path LOC estimate breakdown" table for permanence. (See however §2 NI-1 — this fold created a duplicate `§11.4` heading.)

- **N-5 — `OpenOptionsExt` clarifier** — ➖ N/A — implementation detail; §8.1 step 4 reads "mode preservation via OpenOptionsExt" but doesn't enumerate the import. Acceptable for spec level.

- **N-6 — crossterm 0.29 vs 0.28** — ✅ FOLDED — §13.1 A1 step 1228 + §11.2 line 1042 explicit `cargo tree -e features | grep crossterm` check.

- **N-7 — `DEFAULT_PREVIEW_SAMPLE` `host:port` collision** — ✅ FOLDED — §9.3 line 870-872 sample replaced `10.0.0.5:5432` with `gateway.internal`. Fold paragraph line 875 explains the rationale and preserves the collision case via session-only `s` sample-set modal.

- **N-8 — Phase C2 over-stuffed** — ✅ FOLDED — §13.3 lines 1243-1245 split into C2a (app+events+quit FSM, ~450 LOC), C2b (render+narrow-term gate, ~180 LOC), C2c (tabs/* stubs, ~50-100 LOC). Each sub-phase is single-task-shaped. **Phase decomposition check**: C2a→C2b→C2c sequential dependency is real — C2a needs the App state, C2b needs `App` to render against, C2c needs `App.tab` + render dispatch hook. The chain is genuinely sequential and the decomposition is correct.

- **N-9 — `cargo public-api` not in CI** — ✅ FOLDED — §5.3 line 309 "cargo public-api not in CI; manual baseline diff during code review only". §13.1 A1 step line 1229 says "one-shot baseline; not added to CI". Decision is consistent.

- **N-10 — `theme_ref` vs `theme`** — ✅ FOLDED — §6.1 line 354-358 — `ParsedConfigView` now uses `theme: Option<String>` + `profile: Option<String>` with doc-comments explaining "Mirrors `config::GeneralSection.theme`". Clean.

---

## §2. New issues found

### Important

- **🟡 NI-1 — Duplicate `§11.4` heading** — §11.4 appears TWICE: line 1056 (`Manual-path LOC estimate breakdown`, new from N-4 fold) and line 1071 (`CI audit gate`, original). Mechanical: the N-4 fold inserted a new §11.4 without renumbering the existing one. Renumber: the new breakdown should be `§11.4`, the CI audit gate should become `§11.5`, and the existing `§11.5 — Security review gate` becomes `§11.6`. (Or insert the breakdown as `§11.3.1` to avoid the cascade.) Cross-refs at line 1052 (`breakdown §11.4`) and line 1047 (`breakdown table in §11.4`) point to the new one — both readers will land on the correct content by adjacency, but the duplicate is confusing on first read.

- **🟡 NI-2 — `§9.6` inserted out of section order** — §9.6 (First-run UX) appears at line 881, before §9.4 (Compile cost guard, line 897) and §9.5 (Mini-preview render, line 902). The UX #9 fold inserted §9.6 between §9.3 and §9.4. Renumber to §9.4, push the existing §9.4 → §9.5, §9.5 → §9.6. Trivial. (Implementer will be confused if they navigate by section number — `§9.6` and `§9.4` both exist with the wrong order.)

### Nits

- **🔵 NI-3 — `TmpFileGuard` API contract underspecified** — §8.1 lines 729-730 describe Drop behavior + the `persist()` disarm semantic, and the commit-flow code sketch at line 684 calls `TmpFileGuard::create_in_parent_dir(&tmp_path, preserved_mode)?` and at line 702 calls `tmp.persist(&cfg_path)?`. But the spec doesn't pin:
  - Does `TmpFileGuard` deref to `&mut File` for `write_all` / `sync_all`, or is it a wrapper exposing `write_all` directly? (Line 696 `tmp.write_all(...)` suggests deref, line 697 `tmp.sync_all()` likewise.)
  - Does `persist` consume `self` (the disarm pattern) or just flip an internal `armed: bool`? Consuming is the leak-proof shape.
  - What happens if both `persist` succeeds AND drop runs (double-unlink would error)? Standard answer: `persist` consumes, drop only runs on the un-persisted path.

  Recommend adding a 3-line API contract block in §8.1 invariants:
  ```
  TmpFileGuard API: { create_in_parent_dir(path, mode) -> Result<Self>;
                      Deref<Target=File>; fn persist(self, dst) -> Result<()> }
  ```
  Implementer guesses are likely correct, but the spec should pin it (avoids review-time back-and-forth in Phase C1).

- **🔵 NI-4 — `D` (capital) shortcut vs `d` (lowercase) in Patterns tab — case discrimination undocumented at cross-ref** — §12.1 line 1106 `\`D\`` (capital) = first-run init; §12.2 line 1148 `\`d\`` (lowercase) = delete user-config rule. These are different keys (crossterm distinguishes `KeyCode::Char('d')` vs `KeyCode::Char('D')` + `Shift` modifier), so functionally no collision. But:
  - §12.1.1 line 1129 `\`d\`` (lowercase) = Discard and quit (inside Quit-confirm modal).
  - §12.2 line 1148 `\`d\`` (lowercase) = Delete user-config rule.
  - §12.1 line 1106 `\`D\`` (capital) = first-run init.

  Three different actions on the d/D key, all in close proximity. The §12.1 entry should explicitly note `(Shift+d)` or `(uppercase D — distinct from lowercase d)` to avoid implementer ambiguity when wiring the event handler. §9.6 line 890 just says "**`D` shortcut**" — implementer might wire `KeyCode::Char('d')` (lowercase, the wrong one) if they skim. Test pin `first_run_D_shortcut_requires_uppercase_not_lowercase` recommended.

- **🔵 NI-5 — §12.1.1 quit-confirm modal vs §7.2 modal-absorbs rule** — §7.2 rule 1 line 511 says modal absorbs ALL keys except `Esc` and `Ctrl+C`. §12.1.1 quit-confirm modal is itself a modal, opened by `q`/`Ctrl+C` when dirty. Concretely:
  - User has SaveDiff modal open (`Ctrl+S` pressed). Presses `Ctrl+C`. Per §7.2 this routes to "force quit". Per §12.1.1 this opens quit-confirm modal. But SaveDiff modal is still open. §7.2 rule 2 "no modal stacking" → debug-assert fires.
  - Per §7.2 rule 1 narration ("`Ctrl+C` (force quit — routes to quit-confirm modal **stacking-replacement**, see §12.1.1)") the spec author DID anticipate this: `Ctrl+C` *replaces* the current modal with quit-confirm. But §12.1.1 doesn't say this — it just describes the modal as if it opens from the no-modal state.

  Recommend: §12.1.1 should explicitly say "If a modal is currently open, `Ctrl+C` discards it (no save) and replaces it with the quit-confirm modal. `q` is NOT bound while a modal is open (per §7.2 rule 1)." This closes the §7.2↔§12.1.1 ambiguity.

- **🔵 NI-6 — §9.6 first-run `D` confirm modal wording is byte-pinned but doesn't say "byte-pinned"** — §9.6 line 891 shows the confirm modal `msg:` value. Per memory `feedback_test_assertion_specificity` Display tests must be byte-pinned. Spec doesn't enumerate the test — recommend `first_run_init_confirm_modal_msg_byte_pinned` at §10.2 `app.rs` row (since it's `Modal::Confirm`-shaped). One-line add.

- **🔵 NI-7 — `Cmd` enum re-export at `src/lib.rs`** — §5.3 line 304 `pub use crate::cli::{Args, RunArgs, Cmd, ConfigArgs, ConfigAction, DumpArgs, DumpKind};` re-exports `Cmd` + sub-args + `DumpKind` from the library. These are CLI parser types. If a library consumer ever wanted to embed tayf's CLI, this is useful — but it also expands the public surface significantly. The spec rationale says "Implementation modules pub(crate)" but the re-exports themselves are public. Justifiable, just worth flagging: every one of these enums/structs adds to the contract surface, and `cargo public-api` (when run as the A1 baseline) will see ALL of them as additions. **Not a blocker** — additive surface is forward-additive, and `#[non_exhaustive]` is on the right structs. Just be aware.

---

## §3. Recommendation

Spec is **one mechanical revision pass away from CLEAN_SHIP**: fix the six stale `dump.rs`/`status.rs` references (I-8 incomplete fold), renumber duplicate `§11.4` and out-of-order `§9.6`, add the `TmpFileGuard` API contract block + the `D`-vs-`d` case-discrimination clarifier. None of this requires re-thinking; ~20 minutes of `Edit` tool work. After fixes, no need for another full re-review — a `grep -n "dump\.rs\|status\.rs"` + section-heading order check from the user is sufficient sign-off, then proceed to plan.
