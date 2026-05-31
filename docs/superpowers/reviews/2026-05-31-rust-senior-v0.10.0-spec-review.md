# Rust Senior — v0.10.0 "Public Flip" Spec Review (Empirical / Adversarial)

- **Tarih:** 2026-05-31
- **Reviewer lens:** Senior Rust engineer + project-convention correctness (CLAUDE.md §1/§2/§4).
- **Spec under review:** `docs/superpowers/specs/2026-05-31-tayf-v0.10.0-public-flip.md`
- **Baseline:** `main`, `Cargo.toml` version `0.9.1`, `cargo check` clean. Pre-existing untracked work in tree: `ARCHITECTURE.md`, `CONTRIBUTING.md`, `docs/demo/{README.md,sample-session.sh}`, the release-eng senior's review — these are partial v0.10.0 drafts, not yet committed.
- **Method:** every claim verified by file:line or executed command. The lock prune was tested destructively on a copy; the tree was restored (`git checkout -- Cargo.toml Cargo.lock`; re-verified working tree back to its prior state).

Severity: 🔴 critical (blocks / breaks a deliverable or convention) · 🟡 important (must fold before impl) · 🔵 info (note / nicety).

---

## Headline verdicts

- **OPEN-1 (CLAUDE.md disposition):** **KEEP + fix** — correct call, but widen the stale-ref scope beyond the spec's "5 refs" (actual: 6 textual refs + the whole "Project Layout (v0.1)" tree + the v0.1 framing). See D-1.
- **OPEN-5 (fancy-regex prune mechanism):** spec's stated mechanism is **WRONG**. The orphan does **not** drop on version bump or `cargo update`. Only **lockfile regeneration** (`cargo generate-lockfile`, or `rm Cargo.lock && cargo generate-lockfile`) prunes it. 🔴 — see B-1.
- **Overall: REVISE (small).** Architecture, marker design, and the already-drafted public docs are sound and high quality. Two findings must fold: one 🔴 factual error (OPEN-5) and one 🟡→🔴 unscoped cleanup (10 `docs/superpowers/...` spec-path citations in 9 src files + 1 `tayf-tasarim.md` citation become dangling on removal). Everything else is minor.

Counts: **🔴 1 · 🟡 5 · 🔵 7**

---

## A. TAYF_SESSION marker (spec §8)

### A-1 🔵 `cmd.env("TAYF_SESSION","1")` is correct for portable-pty 0.9's `CommandBuilder`.
**Claim (spec §8 E1):** one line `cmd.env("TAYF_SESSION","1")` before `spawn_command`; child inherits.

**Evidence (from the resolved crate source, `~/.cargo/registry/.../portable-pty-0.9.0/src/cmdbuilder.rs`):**
- `CommandBuilder::new` initializes `envs: get_base_env()` (line 218), and `get_base_env()` (line 74) seeds the map from `std::env::vars_os()` — i.e. **the builder inherits the full parent environment by default.** ✓
- `pub fn env<K,V>(&mut self, key, value)` (line 299, doc: "Override the value of an environmental variable") inserts/overrides a **single** entry; it does not clear others. There is a separate `env_clear()` (line 324) which we are not calling. So adding one var is purely additive. ✓
- Call site (`src/pty.rs`): `let mut cmd = CommandBuilder::new(spec.path.as_os_str());` at line 80; `pair.slave.spawn_command(cmd)` at line 85; the `if spec.login { cmd.arg("-l"); }` block is 81-83. The correct insertion point is **line 84** (after the login block, before line 85). The spec's cited range "80-85" brackets it; the exact new line is 84. 🔵 cosmetic.

**Security:** benign — does not touch the direct-argv / no-`sh -c` invariant (still `CommandBuilder::new(spec.path)` + `spawn_command`). Constant value, no user input. The §13 re-review of the spawn path is appropriate but will find nothing new.

### A-2 🔵 TAYF_SESSION is a brand-new env name — no collision.
`grep -rl TAYF_SESSION src tests` → **0 files**. No existing read or write site anywhere (the env reads in `src/lib.rs` are `TAYF_DISABLE` / `TAYF_DISABLE_BG_DETECT`, unrelated). Clean introduction. ✓

### A-3 🔵 The TDD approach is sound and the harness already supports it cleanly — no allowlist obstacle.
The test harness `tests/common/mod.rs` does **not** filter the child env to an allowlist (it relies on portable-pty's `get_base_env` full inheritance; the only `#![allow(...)]` lines are clippy attributes, not env filters). So a test can let tayf inject `TAYF_SESSION` and assert the spawned shell echoes `1`. The spec's marker-scan caveat ([[feedback-pty-substring-sgr-fragmentation]]) is the right discipline (scan for the literal `1` on the marker line; do not use an SGR-presence helper). ✓

### A-4 🔵 Nested-tayf is benign at the marker level.
The marker is idempotent (`"1"`). Recursion is handled entirely by the rc-exec guard (`§8 E3 B`, `-z $TAYF_SESSION`). No code-level nested handling needed; the spec's "optional documented test" is the right scope. ✓

---

## B. fancy-regex lock prune (spec §10, OPEN-5)

### B-1 🔴 The prune mechanism in the spec is FALSE. Neither version bump nor `cargo update` drops the orphan; only lockfile regeneration does.
**Spec claim (§10 / OPEN-5):** "version-bump lock-rewrite'ında orphan'ı düşür … `cargo tree -i fancy-regex` boş (zaten öyle) … Doğal düşmezse implementer araştırır."

**What's actually in the lock and why:**
- `Cargo.lock:558` = `[[package]] name = "fancy-regex" version = "0.11.0"`. `Cargo.lock:1840` = the `"fancy-regex",` edge inside the **`syntect`** package (syntect's `[[package]]` header is at `Cargo.lock:1786`). So fancy-regex is a transitive *of syntect*, not a near dep.
- **`syntect` is itself a dead orphan.** `cargo tree -i syntect` → "package ID specification `syntect` did not match any packages" (and same with `--all-features --target all`). It is not in the resolution graph under any feature/target — leftover lock cruft.
- Orphan family size: `grep -cE "fancy-regex|syntect|onig|bit-set" Cargo.lock` → **(several lines; the two cited 558/1840 plus the syntect stanza and its other transitives).**

**Empirical prune test (on a copy; tree restored after):**
| Action | fancy-regex present in lock? |
|---|---|
| baseline | yes (lines 558, 1840) |
| bump `Cargo.toml` 0.9.1→0.10.0 + `cargo check` (tayf lock entry verified rewritten to `0.10.0` at lines 301 & 1778) | **still present (unchanged)** |
| full `cargo update` | **still present** — `cargo update --dry-run` lists 21 *graph-member* bumps (autocfg, memchr, serde_json, …) and **does not mention fancy-regex/syntect**, because they aren't graph members to update |
| `rm Cargo.lock && cargo generate-lockfile` | **GONE** ✓ (`cargo check` still builds; tayf entry at `0.10.0`) |

**Conclusion:** the version-bump lock rewrite only touches the `tayf` node; `cargo update` only refreshes resolved graph members. Unreferenced stanzas are "sticky" until the lock is regenerated.

**Recommendation (rewrite §10 + OPEN-5):**
- Mechanism = **regenerate the lockfile**: `cargo generate-lockfile` (and if your cargo version doesn't prune in place, `rm Cargo.lock && cargo generate-lockfile`). Commit the regenerated lock.
- **Call out the side effect:** a fresh resolve will also bump *other* transitives to latest-compatible (the same ~21 patch bumps `cargo update` would do). That's a **wider diff than removing two lines** and must pass `cargo deny check` + `cargo audit` + the full self-hosted test matrix, ideally a bench-regression check (memchr/regex/ratatui patch bumps can move numbers). Make it its own atomic commit with the evidence transcribed ([[feedback-transcribe-measured-numbers]]).
- **Do NOT hand-delete lines 558 + 1840.** Removing fancy-regex without removing the whole `syntect` stanza (and its other deps) leaves an inconsistent lock that cargo will silently "repair" non-deterministically on the next build, re-introducing churn — and could even fail a `--locked` build mid-ceremony.

### B-2 🔵 deny/audit stay green after a clean regenerate.
fancy-regex 0.11.0 is dual MIT/Apache and carries no active advisory in the pinned set, so it isn't *red* even today — it's just dead weight. After regeneration it's simply absent. Verify deny/audit in CI anyway because the regenerate pulls the other patch bumps. 🔵

---

## C. File-surface removals — build/packaging/CI safety (spec §4 A2)

### C-1 🔵 `docs/superpowers/` removal is build- and packaging-safe.
- `Cargo.toml:14-24` `include = [...]` = `src/**/*`, `assets/themes/*.toml`, `assets/profiles/*.toml`, `build.rs`, `Cargo.toml`, licences, `README.md`, `CHANGELOG.md`. **`docs/` is not included** → not in the published tarball; removal doesn't change the crate artifact.
- No `include_str!`/`include!`/`include_bytes!` pulls from `docs/`: the only `include_str!` sites are `src/themes.rs` (`include_str!("../assets/...")`) and `src/profiles.rs`, both pointing at `assets/`. ✓
- (But spec-path *citations* inside src are a separate, real issue — see C-2.)

### C-2 🔴(merge with D-1) Removing `docs/superpowers/` + `tayf-tasarim.md` leaves **dangling in-code citations** that are NOT in the cleanup scope.
**Compilation does not break** (these are comments, and neither path is packaged). But on a public repo these are exactly the dangling refs CLAUDE.md §4 forbids. Empirically:
- `grep -rho "docs/superpowers" src | wc -l` → **10 occurrences across 9 src files**, citing concrete spec files (e.g. `docs/superpowers/specs/2026-05-21-tayf-v0.1-design.md`, `.../2026-05-22-tayf-v0.2.1-hot-reload.md`, `.../2026-05-26-tayf-v0.5.4-config-tui.md`, etc.). After A2 deletes the tree, all 10 point at nothing.
- `grep -rl tayf-tasarim src` → **1 file** (`src/config.rs`, 1 occurrence). Also dangles after `tayf-tasarim.md` removal.
- (Note: a naive `grep -rl tayf-tasarim` over the whole repo hits *plan/test/doc* files too, but those plan/spec files are themselves being deleted in A2, so only the 1 src occurrence is a real "file that stays" dangler.)

**Recommendation (fold into §4 A4 / §10):** add a src-citation sweep — repoint the 11 total in-code references to `ARCHITECTURE.md` (which now exists and covers the module map / I/O loop / pipeline), or demote to generic prose where no public successor section exists. ~10-file touch; small, but unscoped today. Severity 🔴 because it is guaranteed public dangling debt under §4 and currently nobody owns it.

### C-3 🔵 `examples/repro_osc11_hang.rs` removal is fully safe.
- Present on disk but **not** declared `[[example]]` in `Cargo.toml` (auto-discovered) and **not** in `include` → not packaged; cannot break `cargo publish` packaging.
- Zero references: `grep -rl repro_osc11 src tests build.rs Cargo.toml` → 0; `ci.yml` → 0 (`grep -ic repro_osc11 ci.yml` = 0). Safe to `git rm`. ✓
- 🔵 minor: an auto-discovered example *is* compiled by `cargo build --all-targets`; no CI step invokes `--examples` explicitly, so removal only drops that local compile.

### C-4 🟡 `.cargo/config.toml` removal: confirmed it is ONLY a Linux mold/clang linker opt — and CONTRIBUTING.md already documents the opt-in. Verify the self-hosted runner before deleting.
- Full contents (15 lines, mostly comments): `[target.x86_64-unknown-linux-gnu]` / `linker = "clang"` / `rustflags = ["-C","link-arg=-fuse-ld=mold"]`. Purely a link-speed opt for one triple. No build logic, env, fuzz, or sanitizer flags. The spec's "breaks mold-less contributor + GH-hosted smoke" claim is correct. ✓
- `ci.yml` has no mold/clang/config.toml reference (`grep -ic mold ci.yml` = 0) → no dangling CI ref. ✓
- **Already mitigated in docs:** `CONTRIBUTING.md:23-37` documents the *untracked* `.cargo/config.toml` opt-in with the exact stanza. Good. 🔵
- 🟡 **Operational caveat to fold into ceremony:** the *self-hosted* runner currently benefits from the committed config. After removal it falls back to the default linker (slower, still correct). Make "drop an uncommitted `.cargo/config.toml` on the self-hosted runner" an explicit pre-step so the first post-removal self-hosted run doesn't surprise on link time. Functionally safe regardless.

### C-5 🔵 `docs/design/color-sets-preview.html` (v0.9.1, §0) disposition.
Not packaged (`docs/` excluded), not referenced by build/src, not linked from README. Either keep or remove is build-safe; recommend **remove** for a clean public surface. 🔵

---

## D. Stale-ref consistency after removals (spec §4 A4, OPEN-1)

### D-1 🟡 OPEN-1 verdict: **KEEP + fix CLAUDE.md** — but the spec undercounts the fix.
**Enumerated dangling refs in files that STAY:**

**CLAUDE.md** (spec says "5 stale refs"; actual = **6 textual refs + the layout tree + the v0.1 framing**):
- `tayf-tasarim`: 3 (lines 5, 16, 99).
- `docs/superpowers`: 2 (lines 5, 16).
- `superpowers:`: 1 (line 146, the skills mandate).
- Plus the **"Project Layout (v0.1)"** ASCII tree (lines ~84-137) that depicts `docs/superpowers/specs|plans|reviews/...` and `tayf-tasarim.md` as live files — *and* is content-stale (shows the v0.1 module set; missing `themes.rs`, `profiles.rs`, `reload.rs`, `watch.rs`, `config.rs`, `bg_detect.rs`, `ansi.rs`, the whole `tui/` tree, etc. that exist today).
- The header literally reads "Project Guide for Claude … v0.1".

**README.md** (spec cites "455-456" ✓): `README:455` → `[`tayf-tasarim.md`](./tayf-tasarim.md)`; `README:456` → `[`docs/superpowers/specs/`](./docs/superpowers/specs/)`. Both dangle after A2. Also the v0.1 banner at `README:11` ("Status: v0.1 is the working skeleton") and "Known v0.1 limits" at `README:387` need updating — the §F1 rewrite covers these. ✓

**src/*.rs:** the 11 citations from C-2 (10× `docs/superpowers/...` + 1× `tayf-tasarim.md`).

**OPEN-1 = KEEP + fix, with required folds:**
1. KEEP is right — the engineering standards (§1-§4) are genuinely useful to contributors; an AI-assistant guide in a public repo is now normal.
2. The fix must: (a) drop/relink all 6 textual refs; (b) regenerate the "Project Layout" tree to the current module set and remove the `docs/superpowers/` + `tayf-tasarim.md` lines; (c) drop the "v0.1" framing; (d) the design-doc-reading mandate at line 145 must repoint to `ARCHITECTURE.md`. Consider migrating the *contributor-facing* gate (fmt/clippy/test) to CONTRIBUTING.md (already done — see G-1) and leaving CLAUDE.md as the internal-assistant guide.
3. **The 11 src citations (C-2) MUST be folded regardless** of the CLAUDE.md decision — they dangle independently.

### D-2 🔵 No other root-doc dangles.
`grep` of `ARCHITECTURE.md`, `CONTRIBUTING.md`, `SECURITY.md` → **0** `tayf-tasarim`/`docs/superpowers`/`superpowers:` refs. The new docs are already clean. Only CLAUDE.md + README + src are affected. 🔵 One soft spot: `SECURITY.md:24` says "See the project design docs for details" — vague but not a path; after the design docs leave HEAD, repoint this to `ARCHITECTURE.md` for precision (🔵 nicety).

---

## E. Test-invariant claims (spec §12)

### E-1 🔵 Adding TAYF_SESSION to the child env breaks no existing test.
No test snapshots/asserts the full child environment; the new var is additive and no assertion enumerates env. `grep -rl TAYF_SESSION tests` → 0. The "byte-preserved except marker + signals-env" claim holds for env-assert risk. ✓

### E-2 🟡 The signals-env fix is feasible with ZERO signature churn — the env-variant helper **already exists**; the spec's "Helper imzası gerekiyorsa genişletilir" is moot.
**Spec (§10, audit 🔵#4):** add `TAYF_DISABLE_BG_DETECT=1` to the `common::spawn_for_interaction` call in the SIGWINCH test.

**Evidence (`tests/common/mod.rs`):**
- `spawn_for_interaction(cmd, args, size)` (line 62) is a thin wrapper that delegates to **`spawn_for_interaction_with_env(cmd, args, &[], size)`** (line 67).
- `spawn_for_interaction_with_env(cmd, args, env: &[(&str,&str)], size)` (line 73) already loops `for (k,v) in env { builder.env(k,v); }` (lines 86-88) and its doc explicitly notes "host process env is left untouched (no `std::env::set_var`)" — so it is parallelism-safe. **No signature change is needed.**
- The SIGWINCH test (`tests/integration_signals.rs:25`) currently calls the 3-arg `common::spawn_for_interaction(tayf, &["--shell","/bin/sh"], …)`.

**Recommendation (fold into §10):** the SIGWINCH test simply switches to `common::spawn_for_interaction_with_env(tayf, &["--shell","/bin/sh"], &[("TAYF_DISABLE_BG_DETECT","1")], size)`. One-line, no new helper, no other call-site impact (other callers of the 3-arg wrapper — `integration_signals.rs` 2nd use, `e2e_overhead_smoke.rs`, `spawn_with_input_and_args` — are untouched). Reword §10 to drop the "imza genişletilir" contingency since the variant is already present. 🟡 (downgrade-able once worded correctly).

### E-3 🔵 Enumerate-the-tests discipline is satisfiable.
The two real touch points are now known by name: (1) the new TAYF_SESSION integration test, (2) `integration_signals.rs::sigwinch_to_tayf_resizes_child_pty` switching helper. Everything else in `tests/` is untouched. The spec's deferral of the by-name enumeration ([[feedback-enumerate-tests-for-invariant-claims]]) to impl phase is acceptable with these two pinned.

---

## F. Version-bump correctness (spec §10)

### F-1 🟡 The §10 version-ref list is correct on SECURITY.md/README but the lock detail and a real SECURITY.md table need spelling out.
- **`Cargo.toml:3`** `version = "0.9.1"` → 0.10.0 ✓ (single occurrence).
- **`Cargo.lock`**: the `tayf` entry rewrites automatically on the next `cargo check`/build (verified: bumped to `0.10.0` at lock lines 301 & 1778) — **no manual lock edit**, but the regenerated lock (B-1) supersedes this anyway. Commit it. ✓
- **`SECURITY.md`**: contrary to a first guess, it **does** have a "Supported Versions" table (`SECURITY.md:13-16`: `0.9.x ✅ / <0.9 ❌`). This is a **real ref to bump** to `0.10.x ✅ / <0.10 ❌` — matches §F3. ✓ (So §10's "SECURITY.md … version ref'leri" is accurate; just make sure the table rows, not prose, are edited.)
- **`README.md` badges**: README has no crates.io/MSRV badges yet; §F1 adds them — additive. MSRV badge must read **1.88** (matches `Cargo.toml:5 rust-version = "1.88"` + [[feedback-msrv-floor-from-ci-not-local]]). ✓
- **`src/version.rs` / `build.rs`**: 🔵 surface build-time SHA/rustc info via `CARGO_PKG_VERSION` (compile-time), so they pick up the bump automatically — correctly not listed in §10.
- No other hard-coded version string found in tracked non-doc files. CHANGELOG `[0.10.0]` handled in §F2 with date-bump-after-tag ([[tayf-release-workflow]]). ✓

---

## G. EN/TR calibration (project rule §1)

### G-1 🔵 The already-drafted public docs are English and convention-correct.
`ARCHITECTURE.md` and `CONTRIBUTING.md` exist (untracked) and are **English** (`grep -c` for Turkish chars/words → 0 in both). CONTRIBUTING.md correctly states MSRV 1.88, the fmt/clippy-`-D warnings`/pedantic gate, the no-`unwrap`/`expect` rule, English-everywhere-in-code, file-per-concept ~400-line rule, the mold opt-in stanza, and private-disclosure → SECURITY.md. ARCHITECTURE.md is a clean module/thread-model tour linking README + SECURITY.md. These are strong drafts; just keep them English on final edit. ✓

### G-2 🟡 Code-level EN/TR exposure from the plan: the 11 src citations point at **Turkish** docs.
Per [[feedback-review-calibration-en-tr]], code-level (not spec) language coupling is 🟡 minimum. The 10 `docs/superpowers/...` + 1 `tayf-tasarim.md` citations link English comments to Turkish, now-to-be-deleted files. Folding C-2 (repoint to English `ARCHITECTURE.md`) resolves the dangle AND the EN/TR smell in one pass. 🟡

### G-3 🔵 The rc-exec snippet (§8 E3 B) and demo script comments are English. `docs/demo/sample-session.sh` + `docs/demo/README.md` exist and read English; the README note ("record the cast against the released build") is sensible. ✓

---

## H. Anything else a senior would catch

### H-1 🔵 Premise correction: ARCHITECTURE.md/CONTRIBUTING.md/docs/demo already exist as untracked drafts.
The review brief and §A3 imply these are to-be-authored; they are in fact already drafted in the working tree (untracked). Good — it means §A3/§F4 are partly done. They just need committing + the small fixes above (keep English, no dangling refs — both already satisfied).

### H-2 🟡 `ci.yml` line-number drift in §10.
Spec says `continue-on-error: true` at `ci.yml:191` and the TODO at `187-190`. Actual: the single `continue-on-error: true` is at **`ci.yml:191`** ✓ (matches), but it sits under a `TODO(v0.9 A7)` comment block at **187-190** ✓ — these line numbers are actually correct. The 5 self-hosted jobs are `test`(10), `audit`(53), `bench-regression`(68), `msrv`(173), `fuzz-smoke`(184); triggers `push`(4)+`pull_request`(6). Note the inline `# TODO(v0.9 A7): …` is a tracked-style TODO (has the v0.9 A7 tag) so it's CLAUDE.md §4-compliant; removing it with the hard-gate is still correct. 🔵 (downgraded — numbers check out.)

### H-3 🔵 release.yml dry-run→live is feasible as described.
`.github/workflows/release.yml` is `workflow_dispatch:`-only (line 5; an explicit comment at line 3 already pre-announces "v1.0 will add `on: push: tags: ['v*']`" — note the spec wants this at **0.10.0**, so update that comment too). It has `cargo publish --dry-run --locked` (line 60), **zero `secrets.`**, no `tags:`, no `environment:`. All 7 `uses:` are **SHA-pinned** (40-hex; `grep -cE "@[0-9a-f]{40}"` → 7; `@v[0-9]` → 0), so the "GHA SHA-pin korunur" invariant is the existing baseline and the new live steps must preserve it. The live additions (tag trigger, `environment: release`, idempotent publish, `gh release create`, attestation re-verify) are net-new and structurally sound. 🔵 The detailed release-eng verdicts (OPEN-2/3/4) are owned by the release-security senior review (already drafted in the tree).

### H-4 🔵 `if`-gate fork-detection (OPEN-4) — Rust-side note only.
`github.event.pull_request.head.repo.full_name == github.repository` is the standard fork-detection idiom; `pull_request` (not `_target`) is the safe trigger. GHA semantics, owned by the release-eng reviewer per §16. Nothing in `src/`/tests depends on it. No Rust objection.

### H-5 🔵 Homebrew formula + asciinema demo feasibility.
- Binary-download formula shape (`on_macos`/`on_arm`/`on_intel` url+sha256 from `SHA256SUMS`) is the conventional Rust-CLI brew pattern; feasible once GH release assets exist (ceremony §11.10 ordering — formula after artifacts — is correct). 🔵
- asciinema: `docs/demo/` already has the capture script + a README explaining to record against the released build. Synthetic/svg-term is fine. Non-blocking. 🔵

### H-6 🟡 Ceremony ordering: lock regenerate must land BEFORE first-publish/tag.
The B-1 lock regenerate must be in `main` and CI-green (ceremony steps 1-2) **before** `cargo publish --locked` (step 5) and the tag (step 6), because `--locked` will refuse to build if the committed lock doesn't match a fresh resolve. If the prune is deferred or hand-edited, `--locked` could fail mid-ceremony. Make "regenerate lock + deny/audit/test/bench green" an explicit pre-tag gate. 🟡

---

## Fold-or-defer summary

| # | Sev | Finding | Disposition |
|---|---|---|---|
| B-1 | 🔴 | OPEN-5 prune mechanism wrong; needs `cargo generate-lockfile` (not version bump / `cargo update`); side-effects other transitive bumps | **FOLD** — rewrite §10 + OPEN-5; atomic commit w/ deny/audit/test/bench evidence |
| C-2/D-1 | 🔴 | 10 `docs/superpowers/...` + 1 `tayf-tasarim.md` in-code citations dangle on removal — unscoped | **FOLD** — add src-citation sweep (repoint to ARCHITECTURE.md) to §4/§10 |
| C-4 | 🟡 | `.cargo/config.toml` removal: self-hosted runner relies on it | **FOLD** — ceremony pre-step: uncommitted runner config (CONTRIBUTING already documents opt-in) |
| D-1 | 🟡 | CLAUDE.md stale-ref scope undercounted (6 refs + layout tree + v0.1 framing) | **FOLD** — widen A4; OPEN-1 = KEEP+fix |
| E-2 | 🟡 | signals-env: use existing `spawn_for_interaction_with_env`, no signature change | **FOLD** — reword §10 (drop "imza genişletilir") |
| F-1 | 🟡 | §10 version refs: SECURITY.md table IS real (bump rows); spell out lock regenerate supersession | **FOLD** — minor §10 wording |
| H-2/H-3/H-6 | 🟡/🔵 | release.yml comment says "v1.0" not "0.10.0"; ceremony ordering for lock | **FOLD** — minor corrections |
| A-1..A-4, C-1, C-3, C-5, D-2, E-1, E-3, F(version.rs), G-1, G-3, H-1, H-4, H-5 | 🔵 | verified-correct / informational | no action |

**Overall: REVISE** — one 🔴 factual (OPEN-5) + one 🔴 unscoped-cleanup (src citations), both small; five 🟡 wording/ordering folds. Architecture, marker design, and the already-drafted English public docs are sound. No public-API breakage, no security regression from the marker.
