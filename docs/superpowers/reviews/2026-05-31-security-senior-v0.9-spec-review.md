# Spec-phase Security Review — tayf v0.9 (Security Audit, Hardening & Release Infra)

- **Date:** 2026-05-31
- **Reviewer lens:** terminal/TTY security + supply-chain/release-engineering (opus senior, empirical)
- **Spec:** `docs/superpowers/specs/2026-05-31-tayf-v0.9-security-audit-hardening-and-release-infra.md`
- **Tree:** `@ 8e018e3`. Tooling claims verified vs current authoritative docs; code claims vs source.
- **Verdict:** **NEEDS-REVISION** (fold 🔴-1, 🔴-2, 🟡-1..6 + §9 resolutions → SPEC-READY).

## 🔴 Blocking

### 🔴-1 — cargo-deny `unmaintained = "deny"` / `unsound = "deny"` is INVALID schema (§3, §4-A4, §9-Q5)

Per [cargo-deny advisories cfg](https://embarkstudios.github.io/cargo-deny/checks/advisories/cfg.html):
- `vulnerability`, `notice`, `severity-threshold` keys were **removed**; cargo-deny now **always errors** on vulnerability advisories (not configurable).
- `unmaintained` is a **scope selector**, not a lint level. Valid: `"all"` (default) / `"workspace"` / `"transitive"` / `"none"` — NOT `deny`/`warn`.
- `unsound` **re-added** (0.18.2) as a scope selector, same value set, default `"workspace"`.
- `yanked` remains a lint-level string (`deny`/`warn`/`allow`); current `yanked = "deny"` (`deny.toml:50`) is correct, keep.

§9-Q5 ("deny vs warn, measure what fails") is a **false premise** — no deny/warn dial. Real decision is *scope*: `"all"` (strictest) vs `"workspace"`. Given dep-minimalism + transitive risk, recommend `"all"`.

**Spec edit (replace A4 advisories bullet):**
```toml
[advisories]
yanked = "deny"        # lint-level; correct
unmaintained = "all"   # scope selector (all|workspace|transitive|none)
unsound = "all"        # re-added 0.18.2 as scope selector; default workspace
ignore = []
# vulnerability advisories ALWAYS error in cargo-deny >=0.16 (no longer configurable)
```
Pin cargo-deny `>= 0.18.2` in CI install (else config errors on older binaries). Rewrite §9-Q5: scope `all` vs `workspace` → recommend `all`; document future transitive advisories hard-fail CI by design.

### 🔴-2 — `cargo install --locked cargo-audit cargo-deny` is unpinned → silent config drift (§4 A4 / B4)

`ci.yml:59` installs both with no version pin; the new `unmaintained`/`unsound` keys require `cargo-deny >= 0.18.2`, and the advisories schema has changed twice recently. B4 pins Actions to SHAs but leaves the security tools floating — incomplete. **Spec edit:** pin `cargo install --locked cargo-audit@<x> cargo-deny@<y>`, or adopt `EmbarkStudios/cargo-deny-action@<sha>` (version-pins internally). Note deny.toml schema is coupled to the pinned version.

## 🟡 Should fix before impl

### 🟡-1 — SLSA-level claim imprecise (§3, §4 B2)
Per [GitHub artifact attestations](https://docs.github.com/en/actions/concepts/security/artifact-attestations): default = **SLSA v1.0 Build L2**; **L3 requires a reusable workflow**. Canonical action: **`actions/attest-build-provenance@v3`** (keyless Sigstore OIDC, no key storage — spec's "no key storage" is **correct**). Permissions: `id-token: write`, `attestations: write`, `contents: read`. **Spec edit (B2):** state "SLSA v1.0 **Build L2** via `actions/attest-build-provenance@v3`; **L3 is a v1.0-public follow-up** (reusable workflow) — explicit v0.9 non-goal." Add permissions block.

### 🟡-2 — Resolve §9-Q2 (SBOM): **cargo-cyclonedx** (§4 B2, §9-Q2)
`cargo-cyclonedx` sources both `cargo metadata` + `Cargo.lock` → per-binary SBOM, exact feature set, omits dev-deps, per-component licenses; `syft` (Cargo.lock-only) can't. OWASP-stewarded, actively maintained (revitalized Sept 2023 after a lull). `syft` (large Go binary) violates dep-minimalism. **Resolve Q2 → cargo-cyclonedx.** Pin version; verify valid SBOM for tayf's feature set in dry-run.

### 🟡-3 — `skip-tree` right tool, example crates imprecise (§3, §4 A4)
`cargo tree --duplicates` @ `8e018e3` actual irreducible dupes:
- `hashbrown` 0.16.1 (ratatui/kasuari/lru) vs 0.17.1 (indexmap→toml_edit)
- `signal-hook` 0.3.18 (crossterm→ratatui) vs **0.4.4 (tayf direct)**
- `thiserror` 1.0.69 (portable-pty→filedescriptor) vs 2.0.18 (direct + ratatui)
- `itertools` 0.13 (criterion, dev-only) vs 0.14 (ratatui)
- `winnow` 0.7 vs 1.0, `toml_datetime` 0.7 vs 1.1 (toml/toml_edit split)
- `bitflags` 1/2 **NOT observed** — spec's recon is stale; verify before writing a skip rule.

Per [cargo-deny bans cfg](https://embarkstudios.github.io/cargo-deny/checks/bans/cfg.html): `skip` pins versions, `skip-tree` skips a nexus + subtree to `depth`. Concrete config:
```toml
[bans]
multiple-versions = "deny"
skip-tree = [
  { crate = "ratatui@0.30", depth = 4, reason = "drags hashbrown 0.16, signal-hook 0.3, itertools 0.14 via crossterm/kasuari/widgets; upstream-owned" },
  { crate = "criterion@0.8", depth = 2, reason = "dev-only; pulls itertools 0.13" },
]
skip = [
  { crate = "thiserror@1.0.69", reason = "transitive via portable-pty->filedescriptor; we use 2.0 directly" },
  { crate = "winnow@0.7", reason = "toml 0.9 / toml_edit 0.25 winnow split; upstream migration in progress" },
  { crate = "toml_datetime@0.7", reason = "same toml/toml_edit split" },
]
```
**Spec edit:** replace `hashbrown 0.16/0.17 + bitflags 1/2` in §3/§4-A4 with the measured set; re-run `cargo tree --duplicates` at impl + rationale per entry; note `signal-hook` 0.3/0.4 needs a skip (don't silently fall back to `warn`).

### 🟡-4 — A3 ReDoS bench framing assumes backtracking the engine cannot have (§4 A3, §9-Q4)
Per [feedback-regex-engine-lookaround-check] + code (`rules.rs:372` "no backtracking risk"; `:522`/`:1570` "no lookahead"; `:1031-1033` size+dfa limit 1 MiB = `REGEX_SIZE_LIMIT_BYTES` `:15`): the engine is **linear by construction** — no catastrophic backtracking to bench *for*. A "pathological ReDoS blowup" can't exist; the A3 framing risks proving a tautology read as "we tested ReDoS." Meaningful claims: (1) linear scaling (input 2× → time ~2×) framed as *demonstrating* the guarantee; (2) the real DoS vector is *compile*/cache growth — validate `size_limit`/`dfa_size_limit` rejection fires (complement the existing `load_rejects_pattern_exceeding_size_limit` `rules.rs:2515`) + bounded lazy-DFA cache growth (worst-case constant-factor, not super-linear). **Spec edit:** retitle A3 "Linear-scaling proof + DFA-cache/size-limit validation bench"; state the no-backtracking fact; cross-ref the memory; ensure A1.4 `regex_compile` invariant stays "compile → Ok/clean-Err, no panic/OOM/timeout."

### 🟡-5 — Red-team mandate under-specifies the terminal-injection axis (§3, §4 Faz C) — HIGHEST-VALUE ADD
The ansi SM treats OSC/DCS/PM/APC payloads as **opaque** (locates terminators, never interprets OSC-52/2/8/133); passthrough test exists (`ansi.rs:907 osc_with_embedded_newline_does_not_emit_data`, payload `]52;c;base64`). Red-team should *verify this empirically*, not just "try OSC injection." Expand the Faz C red-team bullet into a NAMED checklist, each citing source, reframed from "try to break" to "verify documented property / accepted-risk boundary":
- **Terminal-query response injection** (DA1/DA2/DECRQSS/CPR `\x1b[c`/`\x1b[6n`): tayf passes queries to the real terminal which replies on the **input** path to the child — confirm tayf doesn't answer/duplicate; confirm a hostile `OSC 11` reply can't wedge bg-detect (already CI-disabled via `TAYF_DISABLE_BG_DETECT`, `ci.yml:37`).
- **Bracketed-paste / alt-screen mode confusion** (`\x1b[?2004h`, `\x1b[?1049h`; SM mode bitmask `ansi.rs:114`): feed unbalanced enter/leave across chunk boundaries → no state corruption, tty_guard still restores.
- **OSC opaqueness assertion**: OSC-52/2/0/8/9/777 pass **byte-identically**, tayf never originates one.
- **TOCTOU on config symlink**: `config.rs:350-355` has a deliberately-accepted canonicalize→open window (single-user justification). Job = **validate the accepted-risk boundary** (no cross-user escape: shared `XDG_CONFIG_HOME`, world-writable parent, sudo context), not "find the bug."
- **Signal-race during `child.wait()`/teardown**: SIGINT/SIGTERM during SignalGuard drop (`signals.rs:38-45`) or the child-exit→reap window; concurrent SIGWINCH + SIGHUP-reload (`signals.rs:113`). Confirm no use-after-close on the handle and no `killpg` to a reaped/recycled pgid (classic bug).

### 🟡-6 — Dry-run testability of B2: two steps degrade (§4 B2, §8)
- **Keyless attestation on a PRIVATE repo** uses GitHub's **private Sigstore instance** (not Public Good). It IS dry-runnable (`gh attestation verify` works) but the bundle differs; re-verify with Public Good at the v1.0 public flip.
- **SLSA L3** dry-run via `workflow_dispatch` produces a different trigger/subject than a real `release` event (moot if 🟡-1's L2 scope is accepted).
- **`cargo publish --dry-run`** is a genuine no-op — ADD it (+ `cargo package --list`) to v0.9 to validate packaging/`include` (`Cargo.toml:14-23`) at zero risk; real publish stays v1.0.
**Spec edit:** B2 note private-vs-public Sigstore instance + mandate re-verify at flip; add `cargo publish --dry-run` + `cargo package --list` v0.9 steps; add §8 risk row "private-repo bundle ≠ public-good → re-verify at v1.0."

## 🔵 Nice-to-have

### 🔵-1 — Resolve §9-Q1 (macOS) — GitHub-hosted runners for test AND build
Convention for a public Rust CLI: **`macos-14` (arm64) + `macos-13` (x86_64)** for test+build. Cross-compiling macOS from Linux needs the macOS SDK and **cannot run the test suite** — for termios/PTY/signal code (`tty_guard.rs`/`signals.rs`/`pty.rs`) where macOS↔Linux diverge (FSEvents vs inotify `Cargo.toml:43`, BSD vs Linux termios, killpg), you **must test natively on macOS**. Hosted macOS runners are free for public repos. Target matrix: `x86_64`/`aarch64` × `linux-gnu`/`apple-darwin`. **Resolve Q1: hosted macOS for test+build; cross-compile only as a flagged temporary fallback that ships *untested* macOS binaries (a real risk, not a "limitation").**

### 🔵-2 — `panic=abort` × fuzz × hook: sharper claim
Hook DOES run before abort (fires, then aborts) → `tty_guard.rs:123` restore executes; state as a known fact to *assert in a test*, not "verify" loosely. cargo-fuzz/libFuzzer wants abort-on-crash anyway → fuzz and release profiles agree; §8's "fuzz profili kendi ayarını kullanır" slightly overcomplicates. Credit `tty_guard.rs:121-134` once-guard + `PANIC_RESTORE_STATE` clear-on-drop (`:102-106`) which prevents stale double-restore.

### 🔵-3 — Fuzz access: prefer `--cfg fuzzing` + `#[cfg(fuzzing)] pub` over published feature
A published `fuzzing` *feature* appears in `cargo metadata`/SBOM → quasi-contract. `#[doc(hidden)] pub` still widens the real public surface. Cleanest: fuzz workspace built with `--cfg fuzzing` (auto-set by cargo-fuzz, NOT set for normal builds) exposing internals via `#[cfg(fuzzing)] pub` re-exports → vanish from shipped crate + SBOM. (Controller reconciled this with Rust-senior R2 in spec rev1: `#[cfg(fuzzing)] pub mod __fuzz__` for new wrappers + reuse always-on `__bench__` for pipeline/regex.)

### 🔵-4 — deny `[graph] all-features = true` vs SBOM feature graph
When `cargo publish --dry-run` lands (🟡-6), confirm deny's audited feature graph (`deny.toml:2`) matches the cyclonedx SBOM's feature graph — a mismatch is an inconsistency a sharp auditor flags.

## Open sub-decisions (§9) — resolved
| # | Question | Resolution |
|---|----------|-----------|
| Q1 | macOS CI | Hosted `macos-14`+`macos-13` test AND build (🔵-1); cross-compile can't test PTY natively. |
| Q2 | SBOM | cargo-cyclonedx (🟡-2); drop syft. |
| Q3 | Fuzz access | `--cfg fuzzing` + `#[cfg(fuzzing)] pub` (🔵-3); confirm with Rust-senior. |
| Q4 | A3 placement | Plan-phase; reframe A3 first (🟡-4). |
| Q5 | deny unmaintained | False premise (🔴-1); scope `all`, not deny/warn. |

## Credit (don't lose in rewrite)
Keyless-Sigstore "no key storage": correct. `attest-build-provenance` as native mechanism: correct/current (v3). Dry-run-without-flip (Karar 2): sound. Treating §3 "PRESENT & CORRECT" as baseline-to-verify: the right posture (code IS strong: tty_guard double-restore guard, killpg-to-group, O_EXCL atomic writes, opaque-OSC passthrough). Consuming prior reviews + folding EN nit: correct hygiene.

**Sources:** cargo-deny advisories/bans cfg + CHANGELOG · GitHub artifact-attestations docs · actions/attest-build-provenance · cyclonedx-rust-cargo · code @ `8e018e3`.
