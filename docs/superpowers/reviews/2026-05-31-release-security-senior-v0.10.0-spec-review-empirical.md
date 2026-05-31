# Release-Engineering & Supply-Chain Security Review (Empirical) — tayf v0.10.0 "Public Flip" Spec

- **Date:** 2026-05-31 (web verification carried into 2026-06-01)
- **Reviewer lens:** Senior release-engineering + supply-chain security (adversarial, empirical)
- **Subject:** `docs/superpowers/specs/2026-05-31-tayf-v0.10.0-public-flip.md`
- **Cross-checked artifacts (current HEAD):** `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `Cargo.toml`, `Cargo.lock`, `deny.toml`, `SECURITY.md`, `README.md`, `.cargo/config.toml`, `src/pty.rs`
- **Method:** Every non-trivial claim verified against live GitHub Actions / crates.io / Sigstore docs (sources at end). File-state claims verified by reading current HEAD.

> **Note on the sibling stub** `2026-05-31-release-security-senior-v0.10.0-spec-review.md`: that earlier draft reviewed a `release.yml.disabled` file and a `ci.yml` with a space-broken SHA pin at line 96 — **neither matches current HEAD.** Current HEAD has `release.yml` (`name: release-dryrun`, `workflow_dispatch` only, NO publish step) and `ci.yml` with clean contiguous SHA pins. That draft's blocking findings (broken SHA "F1", dry-run-publishes "F5") **do not reproduce** here, and its web facts were self-labeled `[web-unverified]`. This file supersedes it with current-HEAD reads + live web verification. (I could not overwrite the original path due to a concurrent file-state lock; this empirical version is authoritative.)

**Severity legend:** 🔴 critical (security/correctness blocker) · 🟡 important (latent failure / weakened guarantee) · 🔵 info (nit / confirmation / forward-pointer).

**Counts:** 🔴 2 · 🟡 8 · 🔵 7

**Overall verdict: REVISE.** Architecture sound; every OPEN-item conclusion holds. Two 🔴 underspecifications (idempotency guard using the cached read API as the write-safety authority; ambiguous "provenance attached to release" + missing cross-job artifact handoff) plus several 🟡 gaps must be folded into spec rev1. All fixes are mechanical with concrete corrected mechanisms below; none block the overall plan.

---

## A. Fork-PR RCE prevention (§5, OPEN-4, OPEN-6)

### A-1 🔵 Job-level `if` prevents fork dispatch to the self-hosted runner — CONFIRMED
A job whose top-level `if` evaluates false is `skipped` and **never scheduled onto any runner** — GitHub's control plane evaluates the condition before runner assignment, so no checkout/step runs on `[self-hosted, Linux, X64]`. (A *step*-level `if` would be too late.) The spec's expression
```yaml
if: >-
  github.event_name != 'pull_request' ||
  github.event.pull_request.head.repo.full_name == github.repository
```
is the canonical trusted-context idiom: push/tag/dispatch short-circuit true on the left; fork PRs fail the right operand and skip. **Mechanism correct.** The guard MUST be on **all five** self-hosted jobs (`test`, `audit`, `bench-regression`, `msrv`, `fuzz-smoke`) — a single ungated self-hosted job defeats the whole contract.

### A-2 🔵 `pull_request` (not `pull_request_target`) — CONFIRMED `ci.yml:6`
`ci.yml:6` uses `pull_request:`. The workflow that runs is the **base-branch** definition, NOT the fork's → a malicious fork **cannot edit the `if`-gate or `smoke` job** to redirect onto self-hosted. For a fork PR on a public repo, `GITHUB_TOKEN` is **read-only** and **repo/org secrets are NOT injected** (corroborates §13.1c). Forward-pointer: the future auto-formula-bump (§15) must NOT switch to `pull_request_target` + checkout `head.ref` (classic pwn-request RCE) — put a header comment now.

### A-3 🟡 Fork-detection idiom correct; enumerate edge cases (esp. Dependabot)
`head.repo.full_name == github.repository` is canonical (same-repo PR → all of head/base/`github.repository` equal; fork differs). Edge cases: same-repo branch PRs → self-hosted (intended); **Dependabot PRs live in the same repo → route to self-hosted by default** (accepted for solo repo: read-only token, manifests-only, standard supply-chain surface — but an *implicit* decision the spec omits); `pull_request` is null for non-PR events but `||` short-circuits (no null-deref). **FOLD** the Dependabot-routing note into §5/§13.1.

### A-4 🟡 Strict fork-approval setting gates `smoke` too — CONFIRMED; state the real reason
Three tiers exist; public-repo default is "first-time contributors new to GitHub"; strictest is "Require approval for all outside collaborators" (spec picks this — correct). The gate holds the **entire fork-PR run** (hosted `smoke` included) until a maintainer approves. **🟡 The spec frames this only as self-hosted defense-in-depth — understating it.** The self-hosted runner is already fully protected by A-1/A-2. This setting's real job is **(a) capping GitHub-hosted-minute abuse** (the `if`-gate does NOT stop a fork from burning `ubuntu-latest` minutes via `smoke`) and **(b) gating the first auto-run of `smoke`** (the default tier would auto-run an established account's first fork PR). **FOLD** the corrected rationale into §5.3.

### A-5 🟡 `smoke` builds without mold — CONFIRMED; `.cargo/config.toml` IS tracked (removal is load-bearing)
Verified: `.cargo/config.toml` **is git-tracked** and sets `linker = "clang"` + `-fuse-ld=mold` for `x86_64-unknown-linux-gnu`. On `ubuntu-latest` without mold/clang, a tracked config forcing `-fuse-ld=mold` **fails the link**. So A2's "remove `.cargo/config.toml` from HEAD" is a **hard prerequisite for `smoke` to pass**, not cosmetic (the sibling stub wrongly assumed this was a no-op). cargo then falls back to default `cc`/`ld`; the self-hosted runner keeps an uncommitted `$CARGO_HOME/config.toml` for mold. **FOLD: flag removal as a hard `smoke` prerequisite.**

**OPEN-4 VERDICT: SPEC CORRECT.** `if`-gate + `pull_request` (not `_target`) + strict fork-approval is sound defense-in-depth matching verified semantics. Fold A-3 (Dependabot), A-4 (GH-minute rationale), A-5 (config removal prerequisite).

**OPEN-6 VERDICT: SPEC CORRECT (discovery-by-CI).** Keep `TAYF_DISABLE_BG_DETECT=1` (env-general OSC-11 hang). `RUST_TEST_THREADS=1` is plausibly self-hosted-specific (the `ci.yml:39-43` comment blames a runner-state/fs-watcher interaction) — **start `smoke` WITHOUT it** (parallel = faster + the config a contributor has); add only if the orchestrator hang reproduces on `ubuntu-latest`. Budget the one CI round-trip; add `timeout-minutes` so a hang fails fast (B-7).

---

## B. release.yml dry-run → live (§6, OPEN-2)

### B-1 🔴 Idempotency guard: cached read API is the WRONG write-safety authority — use `cargo publish` exit code
**Claim (§6.4/§7 D1):** `GET /api/v1/crates/tayf/<version>`; 200→SKIP, 404→publish. **🔴 because:** (1) **not atomic** — the crates.io JSON API is Fastly-cached and the sparse index lags a publish by seconds-to-minutes, so a 404 read doesn't guarantee unpublished at publish-time, and a re-run after a successful publish can read 404 and **double-publish**; (2) **endpoint subtlety** — a *yanked* version still returns 200 (yanked ≠ deleted), so the read model ≠ publish constraint; (3) **the real authority exists** — crates.io permanently rejects duplicate versions server-side and `cargo publish` **exits non-zero** with "already uploaded/exists". Corrected:
```yaml
- name: Publish to crates.io (idempotent)
  if: github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')
  env:
    CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
  run: |
    set -euo pipefail
    if out=$(cargo publish --locked 2>&1); then echo "$out"; echo "::notice::published"
    else
      echo "$out"
      if echo "$out" | grep -qiE "already (uploaded|exists)"; then
        echo "::notice::version already on crates.io — idempotent skip"
      else echo "::error::cargo publish failed for a non-idempotency reason"; exit 1; fi
    fi
```
Read-API may remain an advisory fast-path but MUST NOT be the sole guard. **FOLD into §6.4/§7 D1.** (Makes §11.8's "already exists → SKIP" robust regardless of propagation timing.)

### B-2 🟡 Tag-trigger split sound, add `concurrency` + rename workflow
No double-fire (tag push vs `workflow_dispatch` are distinct events). **🟡 Missing `concurrency`** — a force-re-pushed tag or re-dispatch can race two release runs:
```yaml
concurrency: { group: release-${{ github.ref }}, cancel-in-progress: false }
```
**🔵 Rename** `release-dryrun`→`release`; the header comment (`release.yml:3`) says "v1.0 will add tag trigger" but v0.10.0 does — fix the stale comment.

### B-3 🟡 Gate the publish/release JOBS at job level, not just steps
`IS_RELEASE` at workflow level is fine for steps, but a dry-run should never **enter** `environment: release` (else it waits on the required-reviewer prompt). Job-level: `if: github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')` + `environment: release`.

### B-4 🟡 `release` environment + env-scoped token isolates from build-matrix — CONFIRMED, caveats
Verified: environment secrets are **only available to jobs referencing the environment**; required reviewers **pause the job before steps/secrets materialize**. Build-matrix jobs (no `environment: release`) cannot read the token. ✓ Caveats: set `CARGO_REGISTRY_TOKEN` only as the **publish step's** `env:` (never top-level); **🔵 verify it's an ENVIRONMENT secret on `release`, NOT a repo/org secret of the same name** (a repo secret is readable by all jobs, silently defeating scoping — add to §11.4 checklist); don't pass the token to `--dry-run`.

### B-5 🔴 "Provenance attached to release" ambiguous/likely-wrong + missing artifact handoff + non-idempotent `gh release create`
(1) **🔴 The default attestation is NOT a release asset** — `attest-build-provenance` uploads to GitHub's attestations store / Sigstore (Rekor), not a file. §6.5's "provenance ekli" is wrong as written. Choose: (a) attach nothing, users `gh attestation verify --repo`; **or (b) export the `bundle-path` `.sigstore.json` and attach it** so offline `gh attestation verify --bundle` works. **Recommend (b).** (2) **🟡 Missing build-matrix→release-create handoff** — binaries build on 3 runners; the release job must `actions/download-artifact` (SHA-pinned) to collect them before `gh release create`; spec never mentions this. (3) **🟡 Idempotent `gh release create`** — it fails if the release exists; use `gh release view <tag> && gh release upload --clobber || gh release create`. **FOLD all three.**

### B-6 🔵 Minimal per-job permissions — CONFIRMED direction (already §6.6)
Move `id-token`/`attestations` to build/attest job; `contents: write` to release-create job only; workflow-level → `contents: read`. Real change from current `release.yml:7-10` (workflow-level grants). Already specced.

### B-7 🟡 Add `timeout-minutes` to every job
A hung GH-hosted job runs ~6h; a hung self-hosted job ties up the maintainer's machine. `ci.yml`'s `test` has a step timeout but no job cap. Given the orchestrator-hang lore, set job-level `timeout-minutes` everywhere (smoke ~15, release build ~30).

---

## C. Sigstore Public-Good verify (§6.3, OPEN-2)

### C-1 🔵 Public repo → Sigstore public-good — CONFIRMED; flip-before-tag CAUSES it
Verified: **public repo → public-good instance** (Fulcio 10-min cert keyed to workflow OIDC; attestation in **public Rekor**). **Private/internal → GitHub's internal instance, NO public transparency log.** v0.10.0 flips public (§11.3) before the tag (§11.6) → public-good Rekor. **🔵 This is the *real* (load-bearing) reason for the ordering — stronger than the spec's "links resolve."** Corollary: the v0.9 dry-run was on a private repo → exercised the GitHub-internal instance, so the v0.10.0 tagged release is the **first-ever public-good attestation — verify it hard at §11.9** (don't assume the dry-run proved this path). Optionally do one post-flip/pre-tag dry-run to smoke public-good first.

### C-2 🟡 `gh attestation verify` right; pin commands, prefer `--bundle` offline, note gh ≥ 2.49
Verified: in-workflow online `gh attestation verify <bin> --repo beraartuc/tayf` verifies Fulcio chain + signature + cert OIDC identity + **Rekor inclusion proof**. The current `release.yml` has **no** verify step — the v0.9 carryover wants it added (needs `GH_TOKEN: ${{ github.token }}`). User/ceremony (prefer offline): `gh attestation verify ./tayf --repo beraartuc/tayf --bundle ./tayf-<target>.sigstore.json`. **Requires gh ≥ 2.49**; verify against a freshly-downloaded asset. **FOLD** into §6.3 + §9 F1 + §11.9.

### C-3 🔵 cosign NOT required
`gh attestation verify` is first-party/sufficient for `attest-build-provenance` output; cosign can verify the same bundle but needs manual identity regex + roots. Standardize on `gh`; mention cosign only as a no-`gh` alternative. Not in the workflow.

**OPEN-2 VERDICT: CORRECT-BUT-UNDERSPECIFIED.** Public→public-good (confirmed); `gh attestation verify <bin> --repo beraartuc/tayf` right (confirmed). Folds: flip-before-tag causes public-good + v0.9 dry-run was private so verify hard (C-1); add in-workflow verify + pin user/ceremony cmds incl. `--bundle` + gh ≥ 2.49 (C-2); attach the bundle (B-5). No cosign.

---

## D. crates.io token scoping + first publish (§7 D1, OPEN-3)

### D-1 🔵 `publish-new` endpoint scope gates first-publish — CONFIRMED; premise imprecise, conclusion right
Verified (RFC 2947; "Improved API tokens" Rust blog; token UI). Two scope axes: **endpoint** (`publish-new`/`publish-update`/`yank`/`change-owners`) and **crate** (name patterns; glob; evaluated every call → matches present+future owned crates). **You CAN type a crate-scope pattern for a not-yet-existing name** (backend accepts, warns if it matches no owned crate) — so the spec's literal premise ("crate-scoped token cannot be created before the crate exists") is **slightly wrong**. **BUT** publishing a brand-new crate name needs the **`publish-new` endpoint scope** (which CI must not carry), so the **operational conclusion holds**: first publish needs `publish-new`; CI thereafter only `publish-update`. Least-privilege first token = `publish-new` (+`publish-update`), crate-scoped `tayf`, short expiry, revoked. **FOLD precision fix** to OPEN-3/§7 D1.

### D-2 🟡 "Manual first publish, then CI" right — CONFIRMED, preconditions
CI token never holds `publish-new` (can't squat names if leaked) — only `publish-update` scoped `tayf`. ✓ **FOLD preconditions:** verified email REQUIRED before any publish (confirm `bera@korp.com.tr`); enable account 2FA; run `cargo publish --dry-run --locked` + `cargo package --list` for a final manifest check (v0.9 caught the `include_str!` packaging bug this way); **JIT re-verify `tayf` 404 immediately before §11.5** (my live fetch did not return a definitive body — re-check at ceremony time; names can be squatted). Ownership: post-publish `beraartuc` is sole owner; CI token inherits — no `cargo owner --add` for solo.

### D-3 🔵 Idempotent CI publish no-ops on the manual v0.10.0 — CONFIRMED with B-1
With the exit-code guard (B-1) the tag's `cargo publish` hits "already uploaded" → benign skip, independent of API propagation. §11.8 holds once B-1 applied.

**OPEN-3 VERDICT: CONCLUSION CORRECT, PREMISE IMPRECISE.** Manual `publish-new` first publish (short expiry, revoke) + CI `publish-update` scoped `tayf` = right least-privilege. Precision: `publish-new` *endpoint scope* gates first-publish (crate-scope-by-pattern is independently available even for a non-existing name). Add email/2FA/manifest preconditions + JIT name re-check.

---

## E. Supply chain (§10, §13)

### E-1 🟡 fancy-regex orphan does NOT drop from a bare version bump — explicit prune; regenerate SBOM from pruned lock
Verified: `fancy-regex` v0.11.0 at `Cargo.lock:558` (package) + `:1840` (edge), deps `bit-set`+`regex`; not declared in `Cargo.toml`; `regex` does not pull it → true orphan. **A `[package].version` bump edits only the `tayf` entry and does NOT GC orphans** → OPEN-5's "drops naturally?" = **NO.** Recipe (FOLD §10): (1) `cargo tree -i fancy-regex` → not found; (2) `cargo update` (regenerates lock, GCs orphans) OR delete the two stanzas + `cargo build`; (3) in order: `cargo build --locked` → `cargo deny check` → `cargo audit` green; (4) **add `--locked` to the cyclonedx call** — `release.yml:64` is `cargo cyclonedx --format json --all` with no `--locked`, which could re-resolve a phantom component; SBOM is consistent only from the pruned, committed lock.

### E-2 🟡 SHA-pin new release.yml steps — and the missing `download-artifact`
Verified: all current `uses:` are SHA-pinned (checkout `93cb6ef`, rust-toolchain `3c5f7ea`, rust-cache `e18b497`, attest-build-provenance `977bb37`, upload-artifact `043fb46d`). ✓ New live steps are `run:` scripts (no new `uses:`), EXCEPT the cross-job artifact handoff (B-5.2) which needs **`actions/download-artifact` — SHA-pin it.**

### E-3 🔵 deny.toml unaffected — CONFIRMED
fancy-regex is MIT/Apache (allowed anyway); removal only shrinks the graph; `skip`/`skip-tree`/`multiple-versions` unaffected.

### E-4 🔵 cargo-cyclonedx pinned; only add `--locked` to the call (E-1)
`release.yml:63` installs `@0.5.9 --locked`. ✓ Only `:64` invocation needs `--locked`.

**OPEN-5 VERDICT: SPEC OPTIMISTIC.** Orphan does NOT drop from the version bump — fold the prune recipe + `cyclonedx --locked`. deny/audit stay green; SBOM consistent only from the pruned committed lock.

---

## F. Ceremony ordering (§11)

### F-1 🔵 One-way sequence sound; public-flip-before-tag CORRECT and load-bearing
**No window exposing self-hosted to forks:** `if`-gate merged at §11.2 BEFORE public-flip §11.3; pre-flip the repo is private (no fork PRs); post-flip the gate is live → zero window. Public-flip before tag is REQUIRED for public-good Sigstore (C-1) — the strongest argument, under-sold by the spec.

### F-2 🟡 Apply fork-approval (§11.4) BEFORE/WITH public-flip (§11.3)
Between §11.3 and §11.4 the repo is public but only the **default** approval tier is active → an established account could auto-run `smoke` (GH minutes) before the strict setting lands. Self-hosted (RCE surface) is protected regardless (A-1/A-2) → 🟡 not 🔴. Set the policy while still private (it persists) or as the same approved action as the flip. **FOLD: reorder.**

### F-3 🔵 Manual first publish (§11.5) before tag (§11.6) — CORRECT
Tag CI publish is `publish-update`-only; first crate creation needs `publish-new` (D-1). Publish after flip also makes `repository`/`homepage` links resolve. ✓

### F-4 🔵 Brew last (§11.10) — CORRECT
Formula needs `sha256` from `SHA256SUMS`, available only after `gh release create` (§11.8). ✓

---

## G. Other findings

### G-1 🟡 Homebrew tap: per-asset sha256, immutable versioned URLs, tap-repo 2FA
Verified (Formula Cookbook): url+sha256 both required; one byte off fails install. **FOLD:** each `url` gets a matching `sha256` read **verbatim** from published `SHA256SUMS` (tee→Read→verify, [[feedback-transcribe-measured-numbers]]); use **versioned immutable URLs** (`.../download/v0.10.0/...`), never `latest`; `on_arm`/`on_intel` per-arch blocks ✓; **enable 2FA/branch-protection on the `beraartuc/homebrew-tayf` repo** (anyone who can push serves a formula). README `brew install beraartuc/tayf/tayf` syntax correct ✓.

### G-2 🟡 Missing yank/rollback story
crates.io publishes are **immutable**; remediation for a bad v0.10.0 = `cargo yank --version 0.10.0` → fix → publish 0.10.1 (GitHub Release is mutable: `gh release delete`/recreate). Leaked CI token → yank won't help; **revoke + rotate immediately.** **FOLD a Rollback subsection** into §11/SECURITY.md — currently absent.

### G-3 🟡 Branch protection on `main` before flip
Spec hardens fork-PR CI but not `main` protection (require PR + status checks green + no force-push). The ceremony tags from `main`; unprotected `main` on a public repo is a tag/release-integrity risk. **FOLD** a §11.3-adjacent item. 🟡 (solo reduces, not eliminates).

### G-4 🔵 README/MSRV — stale state confirmed, in-scope for §9 F1
`README.md:11-14` still shows the "v0.1 working skeleton" banner; `:53` lists IPv4 "bold yellow" (pre-Neon); `:455-456` link the to-be-removed `tayf-tasarim.md` + `docs/superpowers/specs/` (dangling after A2). §9 F1 rewrite + §4 A4 repoint cover these. MSRV must read **1.88** everywhere (matches `Cargo.toml:5` + `ci.yml:174`); spec consistent ✓. crates.io badge 404s until first publish (fine).

### G-5 🔵 TAYF_SESSION marker benign
`src/pty.rs:80-85` is the spawn path (`CommandBuilder::new` + optional `-l`, then `spawn_command`). Adding `cmd.env("TAYF_SESSION","1")` (constant, no user input) before spawn doesn't change exec semantics (direct-argv, no `sh -c`) — matches §8 E1/§13.3. rc-guard breaks recursion. Marker leaks "tayf in use" to child env — benign under single-user threat model. No change.

### G-6 🔵 fuzz-smoke hard-gate routing
Dropping `continue-on-error` (`ci.yml:191`) makes it a hard gate; with the §5 `if`-gate on `fuzz-smoke`, fork PRs skip it (good — nightly+ASan on trusted runner), push/same-repo run it; `tests/adversarial.rs` in `test` stays the blocking guard. Confirm the `if`-gate is literally on `fuzz-smoke`.

---

## Summary of folds (spec rev1)

**🔴:** (1) B-1/D-3 make `cargo publish` "already uploaded" exit code the authoritative idempotency gate (read-API → advisory only). (2) B-5 correct §6.5 — default attestation is NOT a release asset; export+attach `.sigstore.json` bundle (offline verify); add SHA-pinned `download-artifact` build→release handoff; make `gh release create` idempotent.

**🟡:** A-3 Dependabot routing, A-4 strict-setting=GH-minute gating, A-5 `.cargo/config.toml` removal is a hard `smoke` prereq, B-2 concurrency+rename, B-3 job-level release gate, B-4 publish-step-only token env + env-not-repo-secret check, B-7 timeout-minutes, C-2 in-workflow verify + pinned cmds + `--bundle` + gh≥2.49, D-2 email/2FA/manifest + JIT name re-check, E-1 fancy-regex prune + `cyclonedx --locked`, E-2 SHA-pin download-artifact, F-2 fork-approval before/with flip, G-1 tap sha256-verbatim + immutable URLs + tap 2FA, G-2 yank/rollback, G-3 main branch protection.

**🔵:** A-1/A-2, B-6, C-1 (+causality), C-3, D-1, E-3/E-4, F-1/F-3/F-4, G-4/G-5/G-6.

## OPEN-item verdicts
- **OPEN-2:** CORRECT-BUT-UNDERSPECIFIED. Public→public-good Fulcio/Rekor (confirmed); `gh attestation verify <bin> --repo beraartuc/tayf` right. Fold C-1 (flip-before-tag causes public-good; v0.9 dry-run was private → verify first public release hard), C-2 (add in-workflow verify + pin cmds + `--bundle` + gh≥2.49), B-5 (attach bundle). No cosign.
- **OPEN-3:** CONCLUSION CORRECT, PREMISE IMPRECISE. Manual `publish-new` (short expiry, revoke) + CI `publish-update` scoped `tayf`. Precision: `publish-new` *endpoint scope* gates first-publish, not crate-pattern existence. Add email/2FA/manifest + JIT name re-check.
- **OPEN-4:** CORRECT. False job-level `if` skips before runner assignment; `pull_request` (not `_target`) makes the base gate authoritative + fork-uneditable; secrets/token withheld; `head.repo.full_name==github.repository` canonical. Add Dependabot-routing note + `.cargo/config.toml` removal note.
- **OPEN-6:** CORRECT (discovery-by-CI). Keep `TAYF_DISABLE_BG_DETECT=1`; start `smoke` WITHOUT `RUST_TEST_THREADS=1` (add if hang reproduces on ubuntu-latest); add `timeout-minutes`.

---

*Empirical sources:* GitHub Actions docs (using-conditions-to-control-job-execution; automatic-token-authentication; environments/secrets scoping — "only available to jobs that use the environment", "can only access after configured rules pass"; approving-workflow-runs-from-public-forks — three tiers); GitHub Security Lab "Preventing pwn requests"; `actions/attest-build-provenance` README + GitHub artifact-attestations docs (public→public-good Fulcio/Rekor; private→GitHub instance, no public transparency log); `gh attestation verify` CLI manual + offline-verification docs (`--repo`, `--bundle`, gh≥2.49, Rekor inclusion); crates.io token scopes — RFC 2947 + "Improved API tokens for crates.io" Rust blog (publish-new/publish-update/yank/change-owners; glob crate-scope matches present+future owned; not-yet-existing pattern accepted with warning); Cargo Book publishing + crates.io duplicate-version rejection ("a publish is permanent; the version can never be overwritten"); Homebrew Formula Cookbook (url+sha256 required; per-arch); community #25217 (same-repo vs fork `head.repo.full_name`). Live `tayf` 404 re-check flagged as impl-time re-verification (no definitive body this session).
