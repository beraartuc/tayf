# v0.5.4 spec re-review — UI/UX senior

**Reviewer:** opus 4.7 senior, TUI specialist (re-review of initial NEEDS_REVISION verdict)
**Initial review:** `docs/superpowers/reviews/2026-05-26-ui-ux-senior-v0.5.4-spec-review.md` (ef6b661 base)
**Revised spec:** `docs/superpowers/specs/2026-05-26-tayf-v0.5.4-config-tui.md` (commit 2e4298a)
**Date:** 2026-05-26

---

## Verdict

**CLEAN_SHIP**

All three Critical findings are pinned with byte-level spec text (not implementer-discretion language). All ten Important findings have substantive folds (not name-checks). Nits 1-10 either folded or explicitly carryforwarded to v0.6+ with documented reason. New UX surface introduced by the revision (Shift+P, Space=activate, D first-run, `g`-then-digit color-picker mode, sticky-filter FSM, conflict-y two-press preview, context-aware help) is internally consistent and an implementer can build to it without inventing decisions. Minor housekeeping nits below — not blockers.

---

## §1. Folding verification

### Critical findings

- **🔴 #1 (F-key collision)** — **CLEAN FOLD.** Spec §7.4 narrow-tab letters changed from single-char `P/T/F/S` to two-char visual labels `Pat/Thm/Pro/Sta` with explicit "keyboard binding DEĞİL" pin (line 567, 570). `F` global retired; replaced by `Shift+P` with `V` Helix-idiom alias (§7.2 line 513, §9.5 line 909, §9.5 line 911, §12.1 line 1103). Rename propagated to §7.2 (key dispatch enum), §9.5 (mini-preview header line + full-overlay sentence), §12.1 (global key table), and §12.6 help footer pin. Single-char tab binding ambiguity eliminated. `Shift+P` ergonomics: easy on US-QWERTY (left pinky shift + right hand `P`), no collision with any other binding in the table. **No regression introduced.**

- **🔴 #2 (Enter semantic asymmetry)** — **CLEAN FOLD.** Uniform "Enter = focus detail, Space = activate" applied across all three list tabs (§12.2 Patterns line 1145-1146, §12.3 Themes/Profiles line 1159-1160), with explicit rationale block at §12.3 line 1163 explaining the muscle-memory hazard the change eliminates. Themes/Profiles **detail pane** existence pinned in §12.3 line 1159: "theme/profile detail view — name, source, rules contained, etc. **Read-only browse**, no mutation". `Space` is a new binding but does not collide with anything — neither global table nor other tab tables use Space. Patterns tab Enter→detail matches §6.1 App `TabFocus` shape (which already includes per-tab focus state) and §7.3 render dispatch (no schema change required).

- **🔴 #3 (quit-confirm modal)** — **CLEAN FOLD.** §12.1.1 added as a dedicated subsection (lines 1109-1136). Wording byte-pinned in a code-block:
  - Three-choice: `[n / Esc / Enter] Cancel  /  [s] Save and quit  /  [d] Discard and quit`
  - Default = Cancel; `Enter` selects default; `d` is deliberate keystroke with explicit "no Enter shortcut" pin (line 1135).
  - Save-and-quit path defined (line 1128: opens SaveDiff inline, auto-quit on success). Discard path is explicitly destructive but mis-key-resistant. lazygit/gitui pattern named as precedent. CHANGELOG-grade UX text.

### Important findings

- **🟡 #1 (mini-preview auto-hide signal)** — **FOLD.** §7.4 (line 566) status-bar marker `[preview hidden — press P to force show]` + `P` force-show override + `⚠ pattern won't compile` priority marker pinned. All three sub-asks (visual signal, P override semantics, compile-error visibility regardless of preview state) addressed.

- **🟡 #2 (search tab-aware)** — **FOLD.** §12.1 line 1105 `/` rewritten as "Search (current tab list filter) — Patterns: rule name; Themes: theme name; Profiles: profile name; Status: reload.log line filter". Sticky-filter visual marker `filter: "foo"` in status bar also pinned (same line).

- **🟡 #3 (color-picker go-to-index)** — **FOLD.** §12.4 lines 1173-1174 add `g` to enter goto-input mode + `0-9` for typing index + Enter accept. Status line `goto idx: _` pinned for feedback. `Esc` updated to "Cancel input mode if active, else cancel modal" (line 1177) — handles new state correctly.

- **🟡 #4 (conflict `m` double-confirm)** — **FOLD.** §8.1 modal key table line 639 + §12.5 line 1201-1203 pin secondary "[y/N]" confirm overlay with `default = N`. Destructive action gated.

- **🟡 #5 (conflict `y` silent merge — preview missing)** — **FOLD.** §8.1 (line 637-638) + §12.5 (lines 1199-1200) pin two-press: first `y` shows merged-result diff panel, second `y` commits. Modal state-machine explicit; implementer cannot conflate the two states (each press has its own action row in the dispatch table). One open Q (§15 item 6) about whether second press should be uppercase `Y` — documented as defer-to-implementation-feel, not blocking.

- **🟡 #6 (modal stacking)** — **FOLD.** §7.2 lines 511-512: "Modal absorbs ALL keys except: `Esc` (close modal), `Ctrl+C` (force quit)". No modal stacking — `App.modal` is `Option<Modal>`; debug-assert against opening when occupied. Explicit implementer guidance.

- **🟡 #7 (color-depth fallback)** — **FOLD.** §12.4 lines 1180-1192 add a fallback policy table per detected depth (truecolor/256/16/8/no-color) with byte-pinned warning strings. Detection path documented as paralleling existing `bg_detect.rs`. "Saved verbatim, render-time downsamples" forward-compat rule pinned.

- **🟡 #8 (help context-aware)** — **FOLD.** §12.6 rewritten as two-section grid (top = active-modal-or-current-tab, bottom = global) with grouped sub-categories (`Always`/`Navigation`/`Action`/`View`). 80×24 footprint claim plausible: top 1/2 (~10 row × 4 col) + bottom 1/2 (~10 row × 4 col) + footer = 22 rows, fits with room. With mini-preview visible (5 rows reserved), help overlay still rendered as a modal (full-screen Clear; ratatui modal idiom), so the "fits in 80×24" math is independent of mini-preview state — verified consistent.

- **🟡 #9 (first-run UX)** — **FOLD.** §9.6 added (lines 881-895) with `D` shortcut + Confirm modal + render adjustments + Status bar marker `[no-config: D=init]`. Implementer signal of state is `app.snapshot.source_path == None` — single field, unambiguous. **One nit below about help-overlay disabled-state.**

- **🟡 #10 (search-input `n` collision)** — **FOLD.** §12.2 line 1147: "In active search-input mode (filter editing): typed as text. Post-Enter (filter sticky), `n` opens new-pattern modal as usual." Two-state FSM defined: editing-input vs. sticky-filter-active. Visual indicator (`filter: "foo"` in status bar) pinned in §12.1 line 1105. Implementer has both transition and rendering pin.

### Nits

- **🔵 #1 (Ctrl+S XON/XOFF)** — FOLD. §12.1 line 1098 adds `Ctrl+W` alt-binding with `🔵 #1 fold` annotation.
- **🔵 #2 (h/l Vim partial)** — FOLD. §12.2 line 1144 adds `h`/`l` focus jump.
- **🔵 #3 (Toast vs Modal::Error)** — FOLD. §8.2 lines 735-737 pin policy.
- **🔵 #4 (`h` for help)** — FOLD. §12.1 line 1101 reserves `h` for help when not in Vim-nav context; §15 item 8 flags discoverability check during manual smoke.
- **🔵 #5 (Esc overload)** — FOLD. §12.1 line 1107 pins precedence "(1) edit field, (2) modal, (3) search filter, (4) no-op" with "deepest context first" rationale.
- **🔵 #6 (backup naming long)** — NOT folded; carryforwarded as cosmetic v0.6 nit. Defensible — naming load-bearing for forensic ordering, change would itself be a v0.6 polish task. Acceptable.
- **🔵 #7 (delete/reset confirm)** — FOLD. §12.2 lines 1148, 1152 add Confirm modal trigger to `d` and `r`.
- **🔵 #8 (g/G with filter)** — FOLD. §12.2 line 1142 pins filtered-list semantic.
- **🔵 #9 (Unicode sample)** — FOLD. §9.3 line 871 adds 4th line with `ñame`/`façade`/`完了`; §9.3 line 877 narrates the test purpose.
- **🔵 #10 (status bar truncation)** — FOLD. §7.4 line 567 pins "Status bar shrinkable text widget … uzun field'lar truncate `theme: dr...`".

---

## §2. New UX issues

The revision is mostly clean; the issues below are nits worth pinning before plan-write but not verdict-changing.

### 🟡 N#1 — Module-naming I-8 fold partially propagated (cosmetic but spec-internal)

The Rust I-8 fold renamed `dump.rs`/`status.rs` → `dump_cmd.rs`/`status_cmd.rs` in §5.1 file tree (line 263-264) and §5.2 LOC table (line 292-293). But the rename is NOT propagated to §8.5 ("Status reader side", line 814: `src/config_tui/status.rs`), §10.2 test table (lines 929-930: `dump.rs`/`status.rs` rows), §10.8 coverage list (line 1009), §11.5 security review (line 1081: `src/config_tui/dump.rs`), and §13.2 Phase B (lines 1237-1238: `dump.rs impl`, `status.rs impl`). Not a UX issue per se — but in a spec the reader needs `grep`-clean references. **Fix:** sed `s/\bdump\.rs\b/dump_cmd.rs/g` + `s/\bstatus\.rs\b/status_cmd.rs/g` (avoid `tabs/status.rs` collision) across §8.5, §10, §11.5, §13.2.

### 🟡 N#2 — §9 subsection order broken (§9.6 inserted between §9.3 and §9.4)

§9 reads §9.1 → §9.2 → §9.3 → **§9.6** → §9.4 → §9.5. The first-run UX fold inserted §9.6 in the wrong slot. Renumber to §9.4 (first-run) and bump subsequent (compile guard → §9.5, mini-preview → §9.6). Cross-refs at line 1106 (`§9.6`) and §15 item 7 (`§9.6`) follow whichever number it lands at.

### 🟡 N#3 — Duplicate §11.4 heading (CI audit gate vs. manual-path LOC breakdown)

Lines 1056 and 1071 both labeled `### §11.4`. Renumber CI audit gate → §11.5, security review → §11.6. Cross-ref in §15 item 1 (`§8.5`) unaffected; check §11.5 self-references.

### 🟡 N#4 — `D` first-run shortcut: not specified whether visible-but-disabled or hidden when `source_path.is_some()`

§12.1 line 1106 says "Only when source_path is None ... Disabled otherwise." Implementer ambiguity: does the help overlay (`?`) list `D` always (with "(disabled)" qualifier) or hide it when `source_path.is_some()`? §12.6 help overlay groups `D` under `View` (line 1213) — implies always listed. Recommend: pin in §12.6 that context-aware help **hides** disabled bindings (so first-run users see `D`, post-init users do not). Single sentence add. Without this, post-init users see a `D` they can never use → confusion. (Not verdict-blocking; just an obvious smoothing.)

### 🟡 N#5 — Color picker §12.4 `g`-mode: precedence with §12.1 `Esc` rule

§12.1 line 1107 pins Esc precedence: "(1) close active edit field if focused; (2) close modal if open ...". The new `g`-then-digit input mode in §12.4 (line 1177) defines "Cancel input mode if active, else cancel modal". This is consistent with the global precedence rule (input mode IS "active edit field"), but the spec doesn't explicitly bind the two together. Implementer might treat the color-picker input mode as modal-internal state and skip the global Esc precedence. Recommend one line: §12.4 explicitly notes "go-to-index input mode counts as 'active edit field' for §12.1 Esc precedence purposes." Two-sentence pin.

### 🟢 N#6 — Conflict-y two-press FSM is clearly pinned

I called this out as a worry-axis; verifying it: §8.1 modal key table (lines 637-638) and §12.5 (lines 1199-1200) BOTH list `y (1st press)` and `y (2nd press, post-preview)` as distinct table rows with distinct actions. The implementer reads two rows = two states. No conflation risk. **Pass.**

### 🟢 N#7 — Sticky filter FSM is clearly pinned

UX #10 fold + §12.1 line 1105 visual indicator → implementer has both state transition (Enter commits filter to sticky, Esc clears) and rendering pin (`filter: "foo"` in status bar). The §12.2 line 1147 row for `n` further disambiguates per-mode key behavior. **Pass.**

### 🟢 N#8 — Space=activate does not collide with any existing binding

Audited global table (§12.1), Patterns tab (§12.2), Themes/Profiles tab (§12.3), ColorPicker (§12.4), SaveDiff (§12.5) — `Space` appears only in §12.2 and §12.3 (both = activate). No other tab/modal uses Space. **Pass.**

### 🟢 N#9 — Shift+P ergonomics OK on common layouts

US-QWERTY, ANSI-UK, ANSI-DE: all `Shift+P` is left-shift + right-hand `P` (or right-shift + left-of-center for some `P` placements; still ergonomic). No conflict with terminal-emulator default chords (`Ctrl+Shift+P` is sometimes a command palette in alacritty/wezterm, but plain `Shift+P` is unbound). **Pass.**

---

## §3. Recommendation

**CLEAN_SHIP.** All three Critical UX gaps pinned at byte level; all ten Important findings substantively folded (not name-checked); nits acceptable. The five small new-issue items (§2 N#1-N#5) are spec hygiene — three numbering/naming nits (N#1, N#2, N#3) and two single-line clarifications (N#4 hidden-when-disabled, N#5 Esc precedence chain into color-picker input mode). Author can fold these inline during plan-write (no re-review round needed) or land them as a `docs(spec): v0.5.4 housekeeping` commit before plan dispatch. The spec is now implementation-grade for UX decisions; an implementer reading §6+§7+§8+§9+§12 cover-to-cover does not face a single ambiguous TUI behavior decision they need to invent.
