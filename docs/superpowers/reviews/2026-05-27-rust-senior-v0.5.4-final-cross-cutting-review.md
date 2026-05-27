---
title: tayf v0.5.4 — final cross-cutting review (Opus 4.7 Rust senior)
date: 2026-05-27
range: v0.5.3..bc98c16 (50 commits)
reviewer: Opus 4.7 (1M context) — final pre-tag senior pass
verdict: SHIP_WITH_FIXES (3 documentation-only fixes applied as `a5462fc` before tag)
---

# Cross-Cutting Review — tayf v0.5.4 (`v0.5.3..HEAD`, 50 commits)

## 1. Spec coverage

**🟡 IMPORTANT — `build_new_content` is still pass-through.**

Spec §1 line 5 advertises **"full read-edit-save TUI"** and §2.1 #6 advertises **"Round-trip TOML edit via toml_edit 0.25 (comments + ordering + formatting preserved)"**. The implementation at `src/config_tui/save.rs:163-171` is a literal pass-through that returns `snapshot.raw_bytes`:

```rust
pub(crate) fn build_new_content(snapshot: &..., _edits: &...) -> String {
    String::from_utf8_lossy(&snapshot.raw_bytes).into_owned()
}
```

Consequences (verified by code reading):
- `tabs/themes.rs:74` Space toasts "staged theme = {name}; Ctrl+S to save" — but `PendingEdits::general.theme` is dropped at save time.
- `tabs/patterns.rs:84` `o` (override) inserts `RuleId::UserConfig(...)` into `app.edits.rules` — dropped at save time.
- `events.rs:84` color picker accepts colors with toast "(binding to selected rule lands in v0.6+)" — at least this one is honest about the gap.
- `widgets/save_diff.rs:16-17` comment explicitly states "v0.5.4 commits the TUI-side content unchanged because build_new_content is still pass-through (save.rs §C1c)".
- Save-roundtrip tests (`save.rs:315, 359, 382`) all use `PendingEdits::default()` and assert "Disk content preserved (pass-through C1c stub)" — zero coverage of the edits-to-disk path.

The plan itself (`docs/superpowers/plans/2026-05-26-tayf-v0.5.4-config-tui.md:2720-2721`) admits "full toml_edit reconciliation lands in C3 when edits from each tab flow in" — but C3 commits (`4f9514d`, `bee0f45`, `0ecf27f`) never touched `src/config_tui/save.rs`. The §2.2 amendment (`e643ec0`) lists 9 deferrals but does **NOT** explicitly defer `build_new_content`-real-impl + edits-to-disk persistence. This is the largest spec/implementation gap.

The TUI is genuinely "browse + visual staging + atomic save of unchanged bytes" — useful as scaffolding, but the user-visible "save" promise is unfulfilled.

**Other spec items spot-checked:**
- §2.1 `tayf config dump` — ✅ landed (`67ad00b`); 6 dump tests pass.
- §2.1 `tayf config status` — ✅ landed (`c562c0f`, `f60300e`); reload event log read + emit verified.
- §2.1 layout A2 (4-tab + mini-preview + status bar) — ✅ landed (`3421ffe`, `27e0fc0`, `4f9514d`, `bee0f45`, `0ecf27f`).
- §2.1 hybrid color picker Y — ✅ landed (`a2754f0`).
- §2.1 SaveDiff modal — ✅ landed (`cc4f1ed`); but commits unchanged bytes (see above).
- §2.1 debounced live preview — ✅ landed (`5af3a53`); sample-input correctly excluded per §9.2.
- §2.1 first-run UX (§9.6) — partial: status-bar marker + tab placeholder landed; Shift+D init unwired (correctly deferred in §2.2 amendment).

## 2. DOKUNULMAZ invariant

✅ **PASS.** Only `src/profiles.rs` (+11/-2 lines for doc-fix + `embedded_profile_names()` accessor) and `src/themes.rs` (+9/-5, un-`#[cfg(test)]`ing `names()`) touched. `src/runtime.rs`, `src/pipeline.rs`, `src/io_loop.rs`, `src/pty.rs`, `src/rules.rs`, `src/tty_guard.rs`, `src/signals.rs` zero-touch. `BUILTIN_NAMES` was pre-existing in `src/rules.rs:504` (confirmed via `git show v0.5.3:src/rules.rs`).

## 3. EN/TR language compliance

✅ **PASS.** The only Turkish-char hits in the code diff are in TEST DATA (multibyte fixture `+[2026-...] ñame façade 完了`), which is intentional Unicode coverage. No identifiers, comments, or doc-comments in code use Turkish characters. Spec/plan/review markdown files are Turkish as expected by CLAUDE.md §1.

## 4. Test assertion specificity

✅ **PASS** with one prior-fix-confirmed acknowledgement. `041e352` (D1/D3 review fix) tightened the two genuinely loose Display/Toast assertions in `tests/integration_tui_smoke.rs:59-60` and `src/config_tui/save.rs:398`. Remaining `.contains("theme:")` / `.contains("[[patterns]]")` style assertions are structural-label markers (TOML section header, line prefix), not Display/error wording — substring is correct here per memory `feedback_test_assertion_specificity`. `src/config_tui/save.rs:354,398` use `assert_eq!(count, MAX_BACKUPS, ...)` for the count contract (correct strict pin).

## 5. Duplicate formatter audit

✅ **PASS.** Zero new hits for `format!.*rule "` or `format!.*message: format`. No Display-impl bypass.

## 6. `unwrap()` / `expect()` discipline

✅ **PASS.** Every `.unwrap()`/`.expect()` in library code paths is inside `#[cfg(test)]` blocks: `src/config_tui/events.rs:505,518` are inside `mod tests` (line 416+). `src/config_tui/snapshot.rs:216,229,245` are inside `mod tests` (line 205+). `save.rs:277-422` are all in `mod tests`. `cli.rs:202+` are all in `mod tests`. No library-path violations.

## 7. MSRV 1.74 compliance

✅ **PASS.** Zero `#[expect(...)]` or `reason = ` (attribute-style) additions. Project pattern `#[allow(...)]` + `// reason: ...` line-comment used consistently.

## 8. `#[allow(dead_code)]` hygiene

✅ **PASS.** `src/config_tui/edit.rs:10`, `snapshot.rs:12`, `save.rs:18` module-level allows have explicit reasons referencing v0.5.5+/v0.6+ deferrals; all 6 field-level `#[allow(dead_code)]` in `app.rs` (lines 63, 71, 79, 107, 113, 162) point to specifically deferred items in §2.2 amendment. `widgets/save_diff.rs:19` enum-level allow references the same C1c stub. Per memory `feedback_stale_dead_code_reason_drift`, all reasons accurately reflect the (still-active) deferred work — no stale annotations.

## 9. Hot-reload precedence invariant

✅ **PASS.** `src/reload.rs:206` `spawn` signature unchanged (8 args + return Self). Logger constructed **inside** the thread closure at line 230 (`config_path.as_deref().and_then(|p| p.parent()).map(ReloadLogger::new)`), not from the caller. The precedence-chain inputs (`config_path`, `theme`, `profile`, `bg_default`) are still snapshotted by the closure's `move` capture at startup. I-4 fold preserved.

## 10. Save flow invariants (C1c review fix)

✅ **PASS.** `src/config_tui/save.rs:209` hoists `preserved_mode`; line 220 applies it to the backup-file `OpenOptions::mode(preserved_mode)`, line 238 applies it to `TmpFileGuard::create_in_parent_dir(..., preserved_mode)`. `sync_all()` applied at line 222 (backup) and line 240 (tmp). Both writes use `create_new(true)` (O_CREAT | O_EXCL). Memory `feedback_parallel_call_site_invariant_audit` satisfied.

## 11. Carryover absorption

✅ **PASS.** `src/profiles.rs:113-118` doc-comment updated in `cd21494` (no longer references "Task 6 will pin"). `tests/integration_profiles_library.rs:135` renamed to `docker_profile_renders_container_id_and_partial_image_tag` with FG 35 (magenta) SGR assertion at line 158-165. `embedded_profile_count_matches_shipped_library` test at `src/profiles.rs:693` pins count=5 + sorted names list.

## 12. Spec §2.2 amendment completeness

🟡 **IMPORTANT — one major gap.** Commit `e643ec0` lists 9 v0.5.5+ deferrals + 5 inline amendments. The 9 listed (Shift+D init, V alias, Help modal, search-filter list-side, save-diff scroll, apply_confirm two arms, ColorPicker side-channel, span-emitting preview, list-side regex debouncer) — all verified accurate against code. The 5 inline amendments (§5.2 dump_cmd accessor symbols, §6.1 hand-rolled SHA-256, §9.5 uncolorized preview, §10.3 sigwinch `#[ignore]`, §10.3 save_roundtrip relocation) — all verified.

**MISSING from amendment:** the `build_new_content` pass-through gap (Concern #1). The spec §2.1 advertises "Round-trip TOML edit via toml_edit 0.25" as in-scope; the implementation defers it. This needs to be added as a 10th §2.2 deferral entry. Memory `feedback_consume_prior_review` explicitly warns: "silent omission is the v0.4.0 failure mode."

Also: `src/config_tui/mod.rs:11` says "`save` — toml_edit roundtrip + atomic write + backup rotation" — the "toml_edit roundtrip" half is stale documentation; should read "atomic write + backup rotation (toml_edit roundtrip integration deferred to v0.5.5+)".

## 13. CI workaround discipline

✅ **PASS.** `.github/workflows/ci.yml:42` `RUST_TEST_THREADS: "1"` intact (commit `528548b`); test step has `timeout-minutes: 10`. Self-hosted Linux runner switch documented in `f07f3f8`. macOS matrix-drop explicitly justified (private-repo quota). Baseline file rename `ubuntu-latest.json → linux.json` consistent.

## 14. Triad final state

- ✅ `cargo fmt --check` — clean.
- ✅ `cargo clippy --all-targets -- -D warnings` — zero warnings, finished in 2.28s.
- ✅ `cargo test --lib` — **554 passed; 0 failed; 0 ignored**. Matches spec amendment claim "552→554".
- ✅ Sampled integration suites pass: `integration_tui_smoke` (4/4), `integration_tui_in_wrapper` (1 pass, 1 `#[ignore]`'d per spec amendment), `integration_profiles_library` (6/6), `integration_config` (4/4).

## 15. Public API surface

✅ **PASS.** `src/lib.rs:74` `pub mod config_tui;` plus `pub use cli::{Args, Cmd, ConfigAction, ConfigArgs, DumpArgs, DumpKind, RunArgs}`. Inside `config_tui`, only `pub fn run`, `pub fn dump`, `pub fn status` are public (called from `main.rs`). Everything else is `pub(crate)`. No leakage.

## 16. Half-feature toast stubs

Each stub classified:
- ✅ `events.rs:84` "binding to selected rule lands in v0.6+" — honest stub, consistent with the broader `build_new_content` pass-through. Should be folded into the new §2.2 entry recommended in concern #1.
- ✅ `events.rs:122,127` "help overlay lands in v0.5.5+" — covered by §2.2 #3.
- ✅ `events.rs:343` "init-from-dump and discard-reload deferred" — covered by §2.2 #1 + #6.
- ✅ `tabs/profiles.rs:88` + `tabs/themes.rs:82` "override copy lands in v0.6+" — explicitly out-of-scope per spec §2.2 line 55.
- ✅ `tabs/patterns.rs:113` "new-pattern editor lands in v0.6+" — explicit v0.6+ scope per §9.6.
- ✅ `tabs/patterns.rs:122` "inline regex source editor lands in v0.6+" — covered by §2.2 #9.
- ✅ `widgets/preview.rs:10` "true colorized preview lands in v0.6+" — covered by §2.2 #8 (span-emitting preview).
- ✅ `widgets/sample_set.rs:5` "paste support lands in v0.6+" — single-line contract documented at module head.

## 17. `portable_pty` test invariant (D2)

✅ **PASS.** `tests/integration_tui_in_wrapper.rs:39-43` uses `CommandBuilder::new(tayf_bin())` then `cmd.arg("--shell")`, `cmd.arg("/bin/sh")`, `cmd.arg("--no-color")`, `cmd.arg("--no-hot-reload")` — each arg passed separately, no shell concatenation. Lines 73-74 explicitly document non-use of `has_some_sgr_around` per memory `feedback_pty_substring_sgr_fragmentation`.

---

# Verdict

## SHIP_WITH_FIXES

The full v0.5.4 cycle landed cleanly across 50 commits with strong discipline:

- DOKUNULMAZ invariant preserved (only sanctioned profiles.rs + themes.rs edits).
- Triad green (fmt + clippy + 554 lib tests + integration suite spot-checks).
- EN/TR language compliance perfect.
- I-2 + I-4 + sample-input no-debounce all verified at code-level.
- 9-item §2.2 amendment + 5 inline amendments correctly encode prior-review carryover.

But one substantive **scope-vs-amendment gap** remained: the `build_new_content` pass-through means TUI edits to themes/profiles/rules do not actually persist to disk, despite spec §2.1 #6 + §1 line 5 promising "full read-edit-save" + "Round-trip TOML edit". The save-diff modal, atomic-write machinery, and conflict-detection are all production-grade — they just commit unchanged bytes. The user-visible behavior contradicts the spec headline.

**Fixes applied as commit `a5462fc` before tag:**

1. Spec §2.2 — added 10th carryover entry naming `build_new_content` toml_edit reconciliation deferral; cited `src/config_tui/save.rs::build_new_content` + related stale-doc fixes (`mod.rs:11` + `events.rs:84`).
2. Spec §1 line 18 + §2.1 #6 inline amendments — "browse + visual edit staging" + "scaffolding" honesty.
3. `src/config_tui/mod.rs:11` module-level doc — "atomic write + backup rotation" + deferral pointer.

After fixes landed: triad re-verified green; `bc98c16` (version bump) committed; CI run 26511332971 green on all jobs; tag `v0.5.4` pushed.
