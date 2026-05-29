# Final Cross-Cutting Review — tayf v0.7.1 (pattern-audit follow-up hotfix)

- **Reviewer:** opus 4.8 (1M) — mandatory final cross-cutting reviewer
- **Date:** 2026-05-29
- **Base:** `b5efc68` (v0.7.0 ship) → **Head:** `272b66b`
- **Implementation range:** `237aede..272b66b` (commits `eb2adcc` C-8, `25d1ef8` karar variant, `7c35435` README, `272b66b` CHANGELOG)
- **Lens:** holistic correctness + cross-cutting consistency. Per-task spec/quality reviews already passed.

## Verdict

**SHIP-READY for tag `v0.7.1` — yes (no fixes required).**

This is a textbook LEAN hotfix: one source line removed (`"go"` from `FILENAME_EXTENSIONS`), the rest is corpus-decision metadata + a new machine-enforced karar variant + documentation. The DOKUNULMAZ hot-path modules are byte-identical to v0.7.0. The truth-chain (corpus headers ↔ consts ↔ spec §11 ↔ CHANGELOG ↔ README) is fully consistent. All five permitted gates pass. No CRITICAL or IMPORTANT findings. Two cosmetic NITs, both defer-OK.

---

## CRITICAL

None.

## IMPORTANT

None.

## NIT (cosmetic — defer to v0.8 or fix-in-passing, not tag-blocking)

### NIT-1 — README cross-link anchor (Task-3 review item 8a): NOT a defect; downgrade.
`README.md:376` links `[Disable a built-in by name](#configuration-v02)`. The Task-3 review flagged this as "points at a code-comment not a heading." **This is incorrect.** The GitHub-flavored-markdown auto-slug `#configuration-v02` resolves to the real heading `## Configuration (v0.2)` (`README.md:76`). The *link text* happens to echo a code-comment inside that section (`# Disable a built-in by name.`, `README.md:93`), but the anchor target itself is a valid heading. The link works. No action needed; recommend recording this as a non-issue so it is not re-flagged.

### NIT-2 — recipe blank-line/style parity (Task-3 review item 8b): cosmetic, defer-OK.
The Known-limitations recipe (`README.md:380-386`) adds a `# ~/.config/tayf/config.toml` path-comment line that the Configuration-section recipe (`README.md:88-107`) does not. This is a *helpful* divergence (it tells the reader where the file lives), not a regression. Both blocks share the `[[rules]] / name / enabled = false` shape, so they are stylistically congruent. Leave as-is; if anything the path comment is an improvement.

---

## Checklist results (all verified independently)

| # | Item | Result |
|---|------|--------|
| 1 | **DOKUNULMAZ** `git diff b5efc68..272b66b -- src/pipeline.rs src/pty.rs src/runtime.rs` | **EMPTY** ✓. `src/rules.rs` delta = exactly the one removed `"go",` line, nothing else ✓ |
| 2 | **C-8** `go` gone from `FILENAME_EXTENSIONS`; no other ext lost; `EXPECTED_FP_C8=0`, `DECISION_C8="KALSIN"`; corpus passes | ✓ — `ts/tsx/jsx` and `java/kt/kts` neighbours intact; `pkg.go.dev/foo` NEG no longer FPs; `main.go` now styles as `fqdn` (≥2 labels, 2-char TLD `go`) — accepted trade-off documented in const doc + CHANGELOG |
| 3 | **ACCEPT-DOCUMENTED mandate soundness** | ✓ — BOTH guards present in `check_karar_mandate` (`audit_corpus.rs:130-148`): original `>5% ⟹ decision != KALSIN` AND new `ACCEPT-DOCUMENTED ⟹ fp_rate > 0.05`. `fp_rate` declared ONCE (line 129); the second guard re-uses it (no shadow/redeclare). Self-test `check_karar_mandate_rejects_accept_documented_below_threshold` genuinely catches the panic via `catch_unwind` (0/20 = 0% → asserts `is_err`) and **passed** in the gate run. The variant cannot mask a clean rule: a 0%-FP rule tagged ACCEPT-DOCUMENTED fails the floor and panics the suite. C-4/C-9/E-1 = "ACCEPT-DOCUMENTED" with EXPECTED_FP 5/6/1 **unchanged** from v0.7.0 ✓ |
| 4 | **Truth-chain consistency** | ✓ — for each item, corpus `.txt` header ↔ `DECISION_*`/`EXPECTED_FP_*` const ↔ spec §11 table ↔ CHANGELOG `[0.7.1]` ↔ README all agree: C-4 33%/ACCEPT-DOCUMENTED, C-8 **0%**/KALSIN, C-9 60%/ACCEPT-DOCUMENTED, E-1 12.5%/ACCEPT-DOCUMENTED, D-7/F-3/F-4 0%/KALSIN. No divergence found |
| 5 | **README presence test** | ✓ — `readme_limitations.rs` slices the `## Known limitations` section (header → next `## ` or EOF), then asserts `filename`/`fqdn`/`ipv4` + `enabled = false` *within that slice* (not whole-file `contains`, which would false-pass off the Configuration section). Recipe `name = "fqdn"` / `enabled = false` matches `UserRule` schema (`config.rs:93-100`: `name: String`, `enabled: bool`); `fqdn`/`filename`/`ipv4` are all in `BUILTIN_NAMES` (`rules.rs:542-553`), and the disable-by-name path is exercised by `config.rs` tests (`enabled: false, ..user_rule("fqdn")`). Prose is technically accurate (linear-time/no-look-around rationale; 4000+-TLD-allowlist for C-9; leading `1.2.3.4` of `1.2.3.4.5` for E-1) |
| 6 | **EN/TR (CLAUDE.md §1)** | ✓ for v0.7.1 additions — the `[0.7.1]` CHANGELOG block, README `## Known limitations` section, and new `readme_limitations.rs` test are fully English. The only Turkish-scan hits on added lines are `KALSIN`/`karar`/`TIGHTEN`/`DEMOTE` — these are the established **decision-vocabulary tokens** carried unchanged from v0.7.0, not prose. See "Pre-existing Turkish" below for the released-`[0.7.0]` observation |
| 7 | **Consume-prior-review** | ✓ — I-4 folded: v0.7 spec §9 now reads `782 lib + 46 integration = 828` AND the Net-delta cell `+48 (39 lib + 9 integration)` — and the gate run measured **exactly 782 lib**, confirming the arithmetic. I-5 was a no-op: corpus `e1_semver_vs_ipv4.txt:24` is exactly `NEG: 1.2.3.4.5 long`. I-1 traceability satisfied via corpus header + const doc + spec §11 + README Limitations + new presence-test cross-ref chain (replaces "tracked issue" for a permanent-limitation disposition) |
| 8 | **Cosmetic NITs** | See NIT-1 (anchor is actually valid — Task-3 review item was wrong) and NIT-2 (path-comment divergence is benign). Both defer-OK |
| 9 | **Gate (5 allowed commands)** | **ALL PASS** (see below) |

---

## Gate run (the 5 permitted commands only)

| Command | Result |
|---------|--------|
| `cargo fmt --check` | **PASS** (exit 0, no diff) |
| `cargo clippy --all-targets -- -D warnings` | **PASS** (zero warnings/errors) |
| `cargo test --lib` | **PASS** — 782 passed; 0 failed (matches I-4 forecast exactly) |
| `cargo test --test audit_corpus` | **PASS** — 10 passed; 0 failed (incl. new mandate self-test) |
| `cargo test --test readme_limitations` | **PASS** — 1 passed; 0 failed |

PTY integration suites (`integration_ansi/signals/themes/smoke`) deliberately NOT run — documented macOS OSC-11 bg-detect hang; CI is authoritative for those.

---

## Pre-existing Turkish (observation, NOT a v0.7.1 blocker)

Per the immutability of released CHANGELOG entries, these are reported as observations:

- The released `[0.7.0]` CHANGELOG block contains `DOKUNULMAZ` and `paralel` (Turkish). The new `[0.7.1]` block is clean.
- `check_karar_mandate` — the identifier (`karar` = Turkish "decision") and the corpus decision-token vocabulary (`KALSIN` = "let it stay") are pre-existing across the audit-corpus harness since v0.7.0. They are domain tokens, not freshly introduced.

**Recommendation:** these are cosmetic CLAUDE.md §1 deviations in *already-released* code. Per "catch and fix on sight" they should eventually be Anglicized (`karar` → `decision_mandate`, `KALSIN` → `KEEP`, plus the `[0.7.0]` prose), but doing so touches a released changelog entry and the corpus token-grammar — out of scope for a LEAN pattern-only hotfix. Suggest a dedicated `v0.8` EN-cleanup pass rather than expanding v0.7.1. **Not blocking the tag.**

---

## Summary

A clean, tightly-scoped hotfix. One source-line change, fully test-pinned (`EXPECTED_FP_C8=0` guards drift). The `ACCEPT-DOCUMENTED` variant is a sound formalization: it both satisfies the existing `>5% ⟹ !KALSIN` mandate AND is itself floor-guarded against masking clean rules, with a passing self-test proving the panic path. Documentation, spec, corpus, and CHANGELOG are mutually consistent. No correctness, security (hot path untouched), or consistency defects.

**Disposition: ship-ready for tag `v0.7.1` — yes.** Bump `Cargo.toml` 0.7.0 → 0.7.1, push main, await CI green, then tag (per project-release-workflow). The two NITs and the pre-existing-Turkish observation are non-blocking; route the EN-cleanup to v0.8.
