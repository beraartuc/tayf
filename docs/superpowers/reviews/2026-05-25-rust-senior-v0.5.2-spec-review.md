# v0.5.2 Spec Review — Rust Senior

**Date:** 2026-05-25
**Reviewer:** opus 4.7 Rust senior (spec-phase, pre-implementation)
**Spec:** `docs/superpowers/specs/2026-05-25-tayf-v0.5.2-profile-system-mechanism.md`
**Verdict:** SHIP_WITH_FIXES

---

## 🔴 Critical (blocks ship)

- **`rule_source_of` cannot disambiguate `EmbeddedProfile` from `Theme`/`UserConfig` without a new `BuiltinRule` discriminator.** `src/rules.rs:794-810` derives the source from two bools (`is_user_supplied` + `styles_override_from_theme`). The spec §4.3 adds the `EmbeddedProfile` enum variant but never says how a profile-sourced `BuiltinRule` is *tagged* upstream — `apply_user_rules_with_source` (`src/config.rs:430`) takes a single `from_theme: bool` and `BuiltinRule` has no profile bool. Without a third state (e.g. a `source: RuleOrigin` field replacing the two bools, OR a new `from_embedded_profile: bool`), every `append_rules` entry will be misclassified as `UserConfig` or `Theme` and the dispatch arms added in §5.4 Step 6 will be dead. The spec must specify the `BuiltinRule` schema change AND the corresponding signature change on `apply_user_rules_with_source` (or its replacement). This is the same blind-spot class as memory `feedback_phase1_grammar_gate_blind_spot`.

- **`ProfileErrorKind` `#[derive(Clone)]` is unsound as written.** §6.2 declares `#[derive(Debug, Clone)] pub enum ProfileErrorKind { ParseError(#[from] Box<toml::de::Error>), PathCanonicalization(#[from] std::io::Error), ... }`. `std::io::Error` is not `Clone`; `regex::Error` is not `Clone` either (relevant for `RegexCompile { source: Box<regex::Error> }`). The derive will not compile. The spec acknowledges "manual Clone impl in plan Task 1 Step 2" but no such plan section is referenced in the spec body, and `Error` itself (`src/error.rs:185`) intentionally does NOT derive `Clone` precisely because of these types. Two valid resolutions: (a) drop `Clone` from `ProfileErrorKind` (consistent with `Error`), (b) hand-write `Clone` by rebuilding `io::Error` via `io::Error::new(orig.kind(), orig.to_string())` and `regex::Error` via `Box::new(orig.clone())` — but `regex::Error` IS `Clone` in modern `regex` crate, while `io::Error` and `toml::de::Error` need either string-rebuild or `Arc`. Pin the choice in the spec, do not punt to "plan Task 1 Step 2."

## 🟡 Important (fold before implementation)

- **`themes::name_is_valid` is module-private (`src/themes.rs:78`, not `pub(crate)`).** Spec §4.1 line 164 declares `pub(crate) fn name_is_valid(name: &str) -> bool { crate::themes::name_is_valid(name) }`. The re-export will fail to compile because the source symbol is `fn name_is_valid`, not `pub(crate) fn`. Spec must either (a) call out a visibility bump in `themes.rs` (one-line change, but unmentioned), or (b) move the predicate to a shared utility module. Currently silent.

- **Spec/plan signature inconsistency on `Compiled::load_with_theme`.** §4.4 specifies ONE new parameter (`profile: Option<&Profile>`). The orchestrator brief flags a plan Task 11 that adds a SECOND parameter (`profile_path: Option<&str>`). The spec body (§7.2 step 5, `Compiled::load_with_theme(config.as_ref(), path.and_then(|p| p.to_str()), effective_theme, loaded_profile.as_ref().map(|lp| &lp.profile), depth)`) only passes one new arg. If `format_profile_validation` needs a path label (it does — §6.3 wrapper format includes "loaded from <path>"), then `path_label` must flow somewhere. Resolve: either (a) `Profile` carries its own `path_label` field (then §4.1's `LoadedProfile { profile, path_label }` is what gets passed, not `&Profile`), or (b) add the second arg explicitly. Spec must pick one before implementation.

- **`#[from]` on enum variants with named/struct fields is invalid.** §6.2 `ProfileErrorKind` declares `ParseError(#[from] Box<toml::de::Error>)` — `#[from]` is a thiserror attribute, but `ProfileErrorKind` is not declared `#[derive(thiserror::Error)]` in the snippet (only `#[derive(Debug, Clone)]`). Either add `thiserror::Error` to the derive or remove `#[from]`. Same issue on `PathCanonicalization(#[from] std::io::Error)`. Compare `src/error.rs:185-204` where `Error` uses `#[derive(Debug, thiserror::Error)]`.

- **`ProfileErrorKind::ParseError(Box<toml::de::Error>)` and friends place `toml::de::Error` in a *public* `#[non_exhaustive] pub enum`.** This exposes `toml` as a public-API surface dependency — a future `toml` major bump becomes a tayf public-API break. v0.4.x convention has been to wrap such carriers in opaque `String` fields (`Error::Config { message: String, ... }`) precisely to avoid this. Either (a) accept the dep coupling and add it to CHANGELOG / forward-pointer list, or (b) wrap as `ParseError { detail: String }` mirroring `Error::Config`. Cite N-2 mandate in §3.2 — "no removed/modified existing items" applies, but ADDING a new dependency-leaking public type is still a soft break.

- **Rule merge order: spec §5.4 has user-config overriding profile, but `Compiled::build_from_loaded` currently applies user config AFTER theme (`src/rules.rs:694-701`).** Spec says Step 4 = profile.append_rules (new), Step 5 = user-config. The existing implementation applies theme (Layer 1) → user (Layer 2). The spec inserts Steps 2 + 4 in the middle; verify the new code shape preserves "user-config writes overwrite any prior theme-tagged `styles_override` (REPLACE semantics, Rev2 Karar 27)" extends to profile-tagged overrides too. Spec §5.4 declares the semantic but does not show the implementation site — `apply_user_rules_with_source` will need a third call (or a separate `apply_profile_append_rules`). Pin the call site explicitly.

- **`profiles::synthetic_path` shape underspecified.** §4.1 line 159 says `"<embedded:profile/{name}>" for shipped; canonical disk path for disk-loaded`. But `themes::load` returns a `LoadedTheme { source, path_label }` where `path_label` is the synthetic-or-canonical string; `synthetic_path(name)` is a *separate* helper that ALWAYS returns the synthetic form (`src/themes.rs:567` per test). Clarify which semantic `profiles::synthetic_path` carries — the spec conflates them.

- **`format_profile_load`/`format_profile_validation` are referenced but not declared.** §6.1 uses `#[error("{}", format_profile_load(name, source_path, kind))]` and `#[error("{}", format_profile_validation(...))]`. These free functions must be added to `src/error.rs` mirror of `format_theme_validation` (`src/error.rs:330`). Spec elides the signature; pin it for symmetry with the existing helper.

## 🔵 Nits / observations

- **`StylesKey(ThemeRuleErrorKind)` Display delegation is sound** for byte-equality with theme path (§6.3 last row). The cross-path identity test `duplicate_formatter_theme_and_user_paths_byte_identical_diagnostic` (`src/rules.rs:2601`) will need a third arm covering the EmbeddedProfile path. Spec §8 lists test #12 (StylesKey byte-pinned) but does NOT add a three-way variant of the duplicate-formatter test. Recommend extending that test (3-tuple assertion: theme == userconfig == profile).

- **Hot-reload precedence snapshot (§7.2) correct.** CLI `--profile` snapshot via `ReloadOrchestrator::spawn` parameter mirrors the existing `theme: Option<String>` snapshot (`src/reload.rs:143-152`). The "CLI never mutates during session" invariant is structural — args are consumed once in `main`.

- **`RuleSource::EmbeddedProfile` variant additivity is correct** on the module-private enum (`src/rules.rs:83-98`). The 13 existing match arms in `resolve_group_styles_for_rule` (lines 967, 998, 1025, 1067, 1116 + their UserConfig/Builtin siblings) MUST each get an `EmbeddedProfile` arm. Spec §4.3 calls out the grep audit but does not enumerate the count — recommend pre-pinning "expect ≥ 5 dispatch arms + 1 in `rule_source_of` + N in tests" so D7 verification has a concrete target (memory `feedback_phase1_grammar_gate_blind_spot`).

- **`#[serde(deny_unknown_fields)]` discipline is correct on both `Profile` and `ProfileRule`.** Catches `aapend_rules` typos at load time. Consistent with `GeneralSection` and `UserRule` (`src/config.rs`).

- **Mandatory `pattern: String` on `ProfileRule` vs `Option<String>` on `UserRule` is the right asymmetry.** A profile append-rule without a pattern has nothing to add; failing at deserialize-time (missing required field) gives a clearer error than a post-load shape check.

- **`assets/profiles/.gitkeep` mechanism (§4.2)** — fine. Empty directory is the right v0.5.2 scope.

## What the spec got right

- **v0.5.1 §11.1 fold-or-defer table (§3.1) is exhaustive and disciplined.** 8/8 fold, 0 defer, zero silent omission. The `feedback_consume_prior_review` mandate is structurally honored.
- **Theme model mirror (Option C) is the right architectural choice.** Uniform shape across embedded + disk, no header drift, single deserialization path. Future composition stays additive (`--profile-overlay`, schema unchanged).
- **Risk table §9.1 names the right risks** — `RuleSource::` match-site audit, double-load symmetry, hot-path regression — and pins the I-6 unit test as the structural guard. The "shipped embedded profile accidentally" risk + the `.gitkeep`-only acceptance criterion (§9.2) is good belt-and-suspenders.

---

**Synopsis:** Architecture is right; the contracts have two compile-blocking gaps. The `BuiltinRule` discriminator extension (how a profile-sourced rule is *tagged* upstream so `rule_source_of` returns `EmbeddedProfile`) is missing entirely, and `ProfileErrorKind`'s `Clone` derive will not compile against `io::Error`/`toml::de::Error`. Plus several thiserror attribute mismatches and a quiet visibility bump on `themes::name_is_valid`. Fix these in the spec before D2 dispatches and the implementation should land clean.
