# Spec-phase Rust-engineering review — tayf v0.9 security-audit cycle

- **Date:** 2026-05-31
- **Reviewer lens:** Rust-engineering correctness (opus senior, empirical)
- **Spec:** `docs/superpowers/specs/2026-05-31-tayf-v0.9-security-audit-hardening-and-release-infra.md`
- **Tree:** `@ 8e018e3`. All claims verified against source + authoritative docs.
- **Verdict:** **NEEDS-REVISION** (fold R1–R4 before impl; R5–R8 at plan/impl time).

## 🔴 Blocking

### R1 — The `pipeline_feed` differential oracle is WRONG. No-rules passthrough is NOT byte-identical.

Spec §4 A1.3 + §7: *"no rules → çıktı girdiyle byte-identical."* False for two verified input classes:

1. **`ForceStringTerminate` injects bytes the input never had.** When an OSC/DCS/PM/APC string exceeds `SEQUENCE_BYTES_CAP` (4096, `src/ansi.rs:112`), the SM emits `ForceStringTerminate` and `Pipeline::feed` writes a synthetic 2-byte ST `\x1b\\` to stdout — normal path (`src/pipeline.rs:513`) and TUI-passthrough path (`src/pipeline.rs:506`). A fuzzer finds a 4097-byte unterminated OSC in seconds → false crash.
2. The `LineBuffer` overflow path (`src/line_buffer.rs:58-70`, `:121-131`) emits the blob verbatim (that one IS byte-identical), but the ST injection above is the hard counterexample.

Existing tests encode the *correct, narrower* invariant: `apply_rules_no_set_hits_emits_line_byte_identical` (`src/pipeline.rs:1122`) and `bench_pipeline_passes_plain_text_unchanged` (`src/lib.rs:1062`) assert byte-identity only at the **`apply_rules` level for non-matching lines**, never at the **`Pipeline::feed` level for arbitrary bytes**.

**Spec edit.** Replace the oracle with two precise properties:
- **(a) Passthrough oracle at `apply_rules`/`select_runs` granularity:** with `Compiled::empty()`, `apply_rules` over any line == input byte-for-byte.
- **(b) Injection-shape oracle at `Pipeline::feed`:** the only bytes `feed` may emit that aren't in the input are (i) SGR introducers `\x1b\[[0-9;]*m` + the exact reset `\x1b[0m` around matches (`src/style.rs:449-476,480`), and (ii) the synthetic ST `\x1b\\` only on `ForceStringTerminate`. With empty rules, (i) never fires → invariant collapses to *output == input modulo zero-or-more `\x1b\\` ST insertions at cap-overflow points*; assert "no wide-effect codes introduced" (no `\x1b[2J`, no DECSET the input lacked) rather than raw equality.

## 🟡 Should-fix before implementation

### R2 — Fuzz-target access: commit to candidate (a); the tree already proves the pattern.

Reachability today: **NONE** of the four targets reachable externally. `ansi::AnsiSm`/`SmState`/`StepEvent` are `pub(crate)` (`src/ansi.rs:35,58,136`); `line_buffer::LineBuffer`/`feed_data_run`/`MAX_BUFFER_BYTES` `pub(crate)` (`src/line_buffer.rs:12,21,94`); `pipeline::Pipeline::feed`/`apply_rules`/`select_runs`/`PipelineScratch` `pub(crate)` (`src/pipeline.rs:430,469,69,416`); regex builder path private inside `src/rules.rs:1031`. Lib root is `pub(crate) mod` (`src/lib.rs:49-70`). A sibling `fuzz/` crate cannot see `pub(crate)` items.

Candidate (b) `pub(crate)` + `--cfg fuzzing` on the *fuzz crate* does nothing for the *lib*; needs `RUSTFLAGS=--cfg fuzzing` rebuild + `#[cfg_attr(fuzzing,...)]` gating in the lib — more machinery than (a).

Candidate (a) is the house pattern: `#[doc(hidden)] pub mod __bench__` (`src/lib.rs:1077`) already exposes `BenchPipeline`/`Pipeline::feed` (`:1097-1121`), `apply_rules` (`:1170`), `Compiled::load_builtins/load_with` — exactly the pipeline_feed + regex_compile surfaces. Also `#[doc(hidden)] pub mod __test_api` (`src/lib.rs:355`). Both documented "not part of public API." CLAUDE.md §4 satisfied by the `#[doc(hidden)]` + disclaimer convention.

**Spec edit (resolves §9.3).** A `#[doc(hidden)] pub mod __fuzz__` mirroring `__bench__`, gated behind a non-default `fuzzing` feature, for the new `ansi_sm`/`line_buffer` wrappers; reuse `__bench__::BenchPipeline` for `pipeline_feed` and `load_builtin_rules`/`load_with` for `regex_compile`. **Cargo-fuzz structure:** `cargo fuzz init` creates `fuzz/Cargo.toml` with `path = ".."` dep + `[package.metadata] cargo-fuzz = true` + adds an empty `[workspace]` to `fuzz/Cargo.toml` (its own root). Parent currently has **no `[workspace]` table** — do NOT add one (would pull `fuzz/` into the product tree). Fuzz crate enables `tayf = { path = "..", features = ["fuzzing"] }`.

> **Controller note (conflict resolution vs security-senior 🔵-3):** the security reviewer prefers `#[cfg(fuzzing)] pub` re-exports (no published feature → cleaner `cargo metadata`/SBOM) over a `fuzzing` *feature*. Synthesis adopted in spec rev1: **`#[cfg(fuzzing)] pub mod __fuzz__`** (cfg auto-set by cargo-fuzz across the path-dep graph; absent in normal/`publish` builds → zero published surface, no feature in metadata) for the NEW ansi_sm/line_buffer wrappers, while reusing the existing always-on `#[doc(hidden)] pub mod __bench__` for pipeline_feed/regex_compile. Verify at impl that the cargo-fuzz version in use sets `--cfg fuzzing` for path deps (it does by default).

### R3 — `panic = "abort"` claim CORRECT, but conflates hook path with Drop path.

`std::panic::set_hook` docs: the hook runs "with both the aborting and unwinding runtimes" → the hook (`src/tty_guard.rs:113-135`) **does** restore termios before abort under `panic="abort"` (`Cargo.toml:84`). Claim right. But the **`Drop`** impl (`src/tty_guard.rs:93-108`) does **NOT** run under abort (no unwind → destructors skipped). Under release, restoration relies *entirely* on the hook. Module doc (`:11-21`) lists "Normal drop" + "Panic hook" separately but doesn't note abort demotes panic to hook-only.

**Spec edit.** §8 + Faz C agent-1: split into two checkable claims — (a) under abort, hook restores (true vs std docs); (b) under abort, Drop is bypassed → hook is sole panic-time restorer. The A2 "panic-in-output-thread" integration test MUST run under a **release/`panic=abort` build**, not dev/unwind (dev passes via Drop, silently skips the abort-only path). Pin the profile.

### R4 — cargo-fuzz nightly CI smoke is a separate job; fuzz profile uses unwind.

cargo-fuzz requires NIGHTLY (`-Z sanitizer=address`); project CI is `@stable` everywhere (`ci.yml:21,57,77`). A nightly fuzz job is a **distinct job** with its own `@nightly` toolchain → no conflict, feasible. Don't imply mutating stable jobs. cargo-fuzz builds under its own profile (sanitizer forces `panic=unwind`); parent `[profile.release] panic=abort` does NOT propagate into the excluded `fuzz` workspace → fuzzing unaffected, BUT the fuzz harness therefore **cannot** validate the abort-time restore (R3) — that's the `tests/` regression's job under a release/abort build. Self-hosted Linux runner + ASan needs `llvm-tools` + permissive kernel mmap → verify before declaring the smoke job (add to §9 / Faz B CI-senior brief).

## 🔵 Nice-to-have

### R5 — `cargo +1.74` MSRV job will likely FAIL; treat "discover real MSRV" as the deliverable.
`rust-version = "1.74"` (`Cargo.toml:5`) declared, untested. Local toolchain 1.95; aggressive deps (`ratatui 0.30`, `toml 0.9`, `toml_edit 0.25`, `thiserror 2.0`, `nix 0.28`) — several raised MSRV above 1.74 in the resolved versions. **Spec edit (§9/B3):** use `cargo +1.74 check --locked`; budget for (a) pin offending transitive deps in `Cargo.lock` or (b) bump `rust-version` to the true floor + CHANGELOG. Don't assume 1.74 builds; the job's deliverable is "discover and record the real MSRV." Avoid `cargo update` in the job (defeats the lockfile pin).

### R6 — EN/TR calibration clean; one watch item.
Proposed code/file identifiers all English (`ansi_sm`, `line_buffer`, `pipeline_feed`, `regex_compile`, `SECURITY.md`, `release.yml`, `__fuzz__`). Enforce at impl: fuzz/test comments + `SECURITY.md` body + new `tests/` names must be English. Fold v0.8.3-carryover EN nit (`decision 5`/`decision 3` → `Decision N` at `src/rules.rs:737` + `src/shell.rs:1`).

### R7 — A3 ReDoS bench: new `[[bench]]`, not grouped into `throughput`.
bench-regression CI runs only `--bench throughput` with four hard-coded IDs (`ci.yml:83,89,99,121`). Adversarial-input timing is noisier than throughput → would cause flaky breaches in the gate. A separate `[[bench]] name = "redos"` keeps it out of the regression gate (it's a linearity *demonstration*, not a perf guard) and matches the existing per-concern split. If it needs internals, route via `__bench__` (`load_builtin_rules` + `apply_rules`, `src/lib.rs:1129,1170`).

### R8 — §3 cites `src/style.rs:496-541` as the audit-gate "unit test" — confirmed accurate.
`:496` is `fn to_sgr_emits_only_sgr_sequences()` in `#[cfg(test)] mod tests` (`:485`); `reset_sgr()=="\x1b[0m"` asserted `:545`. The Faz C red-team should know the gate is test-time; the stronger property is that `Style::to_sgr` (`:449`) is *structurally* incapable of emitting non-SGR (only joins numeric params into `\x1b[…m`). Spec should note this so Faz C doesn't waste a cycle "breaking" a test.
