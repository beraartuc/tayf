# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.5.4] - TBD

### Added

- `tayf config` interactive TUI for browsing and visually staging
  edits to pattern / theme / profile config (ratatui 0.30 + crossterm
  0.29). Four tabs (Patterns / Themes / Profiles / Status), per-tab
  focus + dispatch, modal overlay layer (Confirm / Error / QuitWithUnsavedEdits
  + ColorPicker / SaveDiff / Search / SampleSet / FullPreview),
  200 ms debounced live preview (sample-input is no-debounce per
  spec §9.2), Y hybrid color picker (ANSI 16 / 256-palette / truecolor
  hex), SaveDiff modal with clean / conflict-pending / conflict-merged /
  conflict-discard-confirm FSM, atomic write with backup rotation
  (last 5), narrow-terminal degradation (< 60×16 hard block, 60-79
  short tab labels, 80×18-23 mini-preview auto-hides), RAII terminal
  restore with `Once`-gated panic-hook chain. ~3000 LOC across 19
  modules in `src/config_tui/`.
- `tayf config dump [--kind patterns|themes|profiles]` non-interactive
  catalog dump to stdout as TOML. Round-trips through `toml::de`;
  `--kind` selector restricts output to one section.
- `tayf config status` non-interactive resolved-config view: config
  path, active theme, active profile, bg-detect mode + reload event
  log tail (last 100 events from `<cfg_dir>/runtime/reload.log`).
  Byte-pinned line shapes per memory `feedback_test_assertion_specificity`.
- `<cfg_dir>/runtime/reload.log` line-append event log written by the
  existing `ReloadOrchestrator` on every reload outcome (additive
  plumbing; precedence chain invariant unchanged per memory
  `feedback_reload_precedence_snapshot`). POSIX `O_APPEND` atomicity
  pinned (`concurrent_appends_do_not_interleave_within_line` test);
  per-append size-threshold warning fires once per band, not per append.

### Changed (breaking)

- **CLI argument struct layout.** Existing fields on `Args` (e.g.
  `args.shell`, `args.theme`, `args.profile`, `args.config`,
  `args.login`, `args.no_color`, `args.bypass`, `args.no_hot_reload`)
  moved to `args.run.*` via a flattened `RunArgs` sub-struct, enabling
  the new `Option<Cmd>` subcommand at the root. CLI invocation forms
  remain byte-identical: `tayf --shell /bin/fish` works exactly as
  before. Migration for any `tayf` library users:

  ```rust
  // was: args.shell, args.theme, ...
  //  now: args.run.shell, args.run.theme, ...
  ```

### Fixed

- `src/profiles.rs:113-118` — drop stale doc-comment forward-pointer
  ("a unit test added in Task 6 will pin") that was orphaned when
  v0.5.3 shipped the actual test
  (`embedded_profile_count_matches_shipped_library`).
- `tests/integration_profiles_library.rs::docker_profile_renders_…`
  rename + tighten — prior loose `has_some_sgr_around` was satisfied
  by both correct image_tag styling AND the documented v0.5.3
  fqdn-vs-image_tag collision; renamed to `..._renders_container_id_and_partial_image_tag`
  with magenta-FG-35 SGR assertion specifically.

### Dependencies

- **New direct deps:**
  - `ratatui = { version = "0.30", default-features = false, features = ["crossterm", "underline-color", "layout-cache", "macros", "all-widgets"] }` — TUI framework. MIT, ratatui-org Council governance, weekly active commits.
  - `toml_edit = "0.25"` — round-trip TOML (preserves comments + ordering + formatting). MIT/Apache-2.0. Anchors winnow 1.x, aligning with existing transitive surface.
- **Transitive surface:** ~25 new crates (kasuari, mio, signal-hook-mio,
  parking_lot, bitflags, derive_more, etc.). `cargo audit` clean.
  `cargo deny check` clean. `cargo geiger` baseline recorded.
- **Stripped release binary delta:** ~+340 KB (measured 2026-05-26).

### Invariants

- Hot path (`apply_rules` + Pipeline + IO loop + PTY) byte-identical
  to v0.5.3; no bench-CI baseline regen needed. DOKUNULMAZ list per
  spec §5.4 confirmed zero-touch by final cross-cutting review.
- `tests/integration_smoke.rs` byte-identical pass.
- Public API: `Args` field-path migration is the only break (above);
  `Cmd`, `ConfigArgs`, `ConfigAction`, `DumpArgs`, `DumpKind`, `RunArgs`
  are additive.
- Lib test count: 525 → 554 (+29 across C-phase + D3 in-source +
  carryover absorption).

### Known limitations (deferred to v0.5.5+)

Per memory `feedback_consume_prior_review`, every Session 4 cross-cutting
review finding and the F1 reviewer-flagged scope gap is explicit. See
spec §2.2 for the full list:

- **`build_new_content` toml_edit reconciliation** — v0.5.4 ships the
  full save scaffolding (PendingEdits aggregator, DocumentMut parse,
  SaveDiff modal, atomic write, backup rotation, RuleId merge-collision
  semantic) but `src/config_tui/save.rs::build_new_content` is a
  pass-through returning `snapshot.raw_bytes` unchanged. Tabs stage
  edits faithfully into `app.edits`; the reconciliation that walks
  `PendingEdits` into `DocumentMut` lands in v0.5.5+. Save is currently
  byte-preserving (atomic write of input bytes + backup rotation).
- **Shift+D first-run init flow** — `ConfirmAction::InitFromDump`
  declared but unwired.
- **`V` alias for Shift+P** full-preview overlay.
- **Help modal** (`Modal::Help` + `?` keystroke) — Toast::warn
  placeholder in v0.5.4.
- **Search filter applying to tab lists + filtered top/bottom
  navigation** — capture/display landed; list-side filtering unwired.
- **Save-diff `↑` / `↓` scroll** — long-diff vertical scrolling
  unwired.
- **`apply_confirm` `DiscardEditsAndReload` + `InitFromDump`** —
  Toast::warn placeholders (DeleteUserRule + ResetUserOverride
  arms wired in `9b2f24d`).
- **Span-emitting preview pipeline** (§5.4 DOKUNULMAZ blocker on
  `apply_rules`) — mini-preview ships uncolorized.
- **`Modal::ColorPicker` side-channel unification** — borrow-checker-driven
  split between inline-state ColorPicker and side-channel SaveDiff /
  Search / SampleSet; v0.6+ refactor candidate.
- **List-side regex source field debouncer wiring** — inline editor is
  Toast::warn stub; debouncer hook lands with the editor.
- **`sigwinch_propagates_to_wrapped_tui` integration test** — shipped
  `#[ignore]`'d (manual smoke per §10.5); v0.6+ adds debug Toast
  scrape path.

## [0.5.3] - 2026-05-26

### Added
- Built-in profile library: `aws`, `k8s`, `docker`, `gcp`, `network` ship
  in `assets/profiles/`. Activate with `tayf --profile <name>` or
  `[general] profile = "<name>"` in `~/.config/tayf/config.toml`. Profile
  semantics (whitelist filter + append_rules + theme override) unchanged
  from the v0.5.2 mechanism.
- `aws` profile: `instance_id` (`\bi-[a-f0-9]{17}\b`), `arn` (right-
  anchored to prevent trailing punctuation), and `region` (34-region
  exhaustive enumeration covering commercial, GovCloud, and China
  partitions). Append-only — all 13 built-ins remain active.
- `k8s` profile: `pod_name` pattern using K8s
  `apimachinery/pkg/util/rand` base32 alphabet
  (`bcdfghjklmnpqrstvwxz2456789`). Audit-deviation documented in TOML
  comments — v0.5.1 spec §11.2 recommended `[a-f0-9]{10}` but real K8s
  ReplicaSet hashes use base32 subset; hex-only shape would miss ~99%
  of pods. Append-only.
- `docker` profile: `container_id` (12-hex) and `image_tag` (registry-
  host required or bare `:latest` tag). Collision caveats with git short
  hashes and UUID segments accepted by design — profile activation
  signals domain context. Append-only.
- `gcp` profile: filter-only whitelist of 10 built-ins relevant to
  gcloud CLI output. Excludes `permission` (Unix-perms, not GCP IAM
  JSON), `mac` (rare in gcloud), and `filename` (Cloud Storage paths
  covered by `url` built-in).
- `network` profile: filter-only whitelist of 8 network-relevant
  built-ins (tcpdump / netstat / dig focus). Excludes `uuid`,
  `permission`, `email`, `duration`, `filename`.
- Embedded profile discovery completed — `src/profiles.rs::load_with`
  now consults an `include_str!`-backed `EMBEDDED_PROFILES` table
  between disk discovery and `NotFound`. User disk profiles still
  shadow embedded by writing
  `~/.config/tayf/profiles/<name>.toml`. v0.5.2 shipped the mechanism
  with the embedded path stubbed (returned NotFound); v0.5.3 lights
  it up.

### Changed
- `Error::Profile { kind: RegexCompile, .. }` exit code now splits on
  source path: embedded profile (path label
  `<embedded:profile/...>`) maps to 70 (`EX_SOFTWARE`, tayf library
  bug); user disk profile maps to 64 (`EX_USAGE`, user TOML error).
  v0.5.2 single-mapping was correct when zero embedded profiles
  shipped; v0.5.3 first library mandates the split.

### Known Limitations
Both items below share the same architectural root cause: tayf's
pipeline (`src/pipeline.rs::apply_rules`) resolves overlapping rule
matches by RegexSet pattern-order priority — built-in rules
(indices 0-12) outrank profile `append_rules` (indices 13+) on
any byte-overlap. A future sub-version may revisit this (either
elevate profile rules to higher priority OR tighten the built-in
patterns); both pinned tests below will fail visibly when that
landing happens, by design.

- **`aws.arn` envelope yields to built-in `ipv6` on canonical empty-
  region ARNs.** The built-in `ipv6` pattern's compressed-form
  alternation matches `3::`, `f::`, etc., consuming a substring
  inside ARNs like `arn:aws:s3:::my-bucket` before `aws.arn` can
  match the envelope. Behavior pinned by
  `aws_arn_yields_to_interior_region_pattern_v0_5_3_limitation` and
  documented in `assets/profiles/aws.toml`. Collision-free ARN
  shapes (e.g., IAM role ARNs without hex segments —
  `arn:aws:iam:::role/MyRole`) still match the envelope correctly.

- **`docker.image_tag` envelope yields to built-in `fqdn` on
  registry-host image references.** The built-in `fqdn` pattern
  matches the registry-host prefix (e.g., `gcr.io`, `docker.io`)
  before `docker.image_tag` can match the full envelope. Behavior
  pinned by
  `docker_image_tag_registry_host_yields_to_fqdn_v0_5_3_limitation`.
  Bare `:latest` shapes (without a registry-host prefix — e.g.,
  `nginx:latest`) still match the envelope correctly.

### Tests
- Per-profile PTY integration tests verify SGR injection on
  representative domain input (one test per profile, 6 total —
  including the limitation pins).
- AWS region exhaustive enum unit test — 34 per-region assertions
  plus a negative-regression unit test pinning enum-exhaustivity
  (invented future regions must not match).
- Per-pattern positive + negative regression tests for `instance_id`,
  `arn`, `pod_name`, `container_id`, `image_tag` (16 unit tests
  total).
- EX_USAGE/EX_SOFTWARE split unit tests (3 cases: embedded
  RegexCompile, disk RegexCompile, embedded non-RegexCompile).
- Schema invariant unit tests pin `Profile`/`ProfileRule`
  byte-identical to v0.5.2 plus the `#[serde(deny_unknown_fields)]`
  enforcement.
- Embedded profile discovery unit tests (8 across Tasks 1, 2, 4, 5,
  6): per-profile load success, disk override of embedded by same
  name, NotFound diagnostic includes the embedded namespace in
  `searched`, and the EMBEDDED_PROFILES table-shape invariant
  (count = 5, sorted names).
- Five new criterion benches characterize the profile-active path
  with canonical baselines recorded post-tag for Linux and macOS.

### Internal
- No documented public API change. `Profile` / `ProfileRule` /
  `RuleSource` / `Error` variants byte-identical to v0.5.2. The
  `src/lib.rs` change is a single new helper added inside the
  existing `#[doc(hidden)] pub mod __bench__` adapter module
  (`load_profile_rules`) — preserves the v0.1.1 convention that
  bench access is hidden, opaque, and not part of the documented
  public surface.
- Profile-inactive hot path byte-equal to v0.5.2 (existing
  `hot_path_unchanged_when_no_profile` test passes unchanged).
- `RuleSource::` match-site count remains 72 (no new variant, no
  new match arms).
- Duplicate-formatter audit clean (5 sites in `src/rules.rs`
  unchanged).
- `assets/profiles/.gitkeep` removed — directory now ships 5 files.

## [0.5.2] — 2026-05-26

### Added
- **Profile system mechanism.** New `--profile NAME` CLI flag +
  `[general] profile = "NAME"` user-config field. Profiles loaded from
  `~/.config/tayf/profiles/<name>.toml` (disk) or embedded sources
  (none shipped in this release — curated library lands in v0.5.3).
  Each profile may define:
  - `rules`: whitelist of built-in rule names. Omit to keep all
    built-ins; empty array filters them all out.
  - `append_rules`: array of new rules (`name`, `pattern`, optional
    `style` / `styles`). Mandatory pattern. Names cannot collide with
    built-ins — use `[[rules]]` at user-config level to override
    built-ins.
  - `theme`: optional theme override. Slot 3 in the 4-tier precedence
    chain: CLI `--theme` > config `[general] theme` > `profile.theme`
    > bg-detect default.
- New `Error::Profile` + `Error::ProfileValidation` variants on the
  `#[non_exhaustive]` `Error` enum. Backed by `ProfileErrorKind`
  (NotFound, ParseError, PathCanonicalization, RegexCompile — all
  String-message-carriers, no `toml`/`io`/`regex` crate types leaked
  into public API) + `ProfileRuleError { rule_name, kind }` with
  `ProfileRuleErrorKind` Phase 1 shape variants (RuleUnknown,
  RuleNameInvalid, AppendRuleConflictsWithBuiltin,
  AppendRuleConflictsWithOther, ThemeNameInvalid) + a
  `StylesKey(ThemeRuleErrorKind)` wrapper that delegates capture-group
  key diagnostics to existing Display impls (no duplicate-formatter
  regression).
- Hot-reload picks up `[general] profile` changes per the v0.5.1
  spec §11.1 C-4 mandate: `reload_once` re-resolves the full
  precedence chain on every reload. CLI `--profile` is snapshotted
  at startup and never mutates during the session. bg-detect result
  also snapshotted to avoid OSC 11 re-query on every reload.

### Fixed
- `apply_user_rules_with_source` silently skips theme/profile rule
  references when the target built-in was filtered out by Step 2
  `profile.rules` whitelist. Pre-fix: a misleading "rule '<name>':
  appears twice with conflicting `enabled` values" diagnostic fired
  when a profile combined `rules = [...]` whitelist with a theme that
  referenced filtered-out built-ins. Per spec §5.4 — whitelist is an
  exclusion mechanism, not a validation contract.
- `Error::Profile` + `Error::ProfileValidation` now map to exit code
  64 (EX_USAGE), parallel to `Error::Config` / `Error::Theme` /
  `Error::ThemeValidation`. Pre-fix: these user-input errors fell
  through to the default arm and exited 70 (EX_SOFTWARE).

### Internal
- New `src/profiles.rs` module mirrors `src/themes.rs` shape.
- `themes::name_is_valid` visibility bumped from module-private to
  `pub(crate)` for re-export from `profiles`.
- `RuleSource::EmbeddedProfile` variant added to the module-private
  enum; every match site enumerated (67 sites) + updated with full
  4-arm exhaustive coverage.
- `BuiltinRule` schema: two booleans (`is_user_supplied` +
  `styles_override_from_theme`) replaced with a single `source:
  RuleSource` field. `rule_source_of` helper deleted; call sites
  read `rule.source` directly. `apply_user_rules_with_source`
  signature: `from_theme: bool` → `source: RuleSource`.
- `Compiled::load_with_theme` signature gains `profile:
  Option<&Profile>` + `profile_path: Option<&str>` (six total params).
- `ReloadOrchestrator::spawn` + `reload_once` gain `profile`
  parameters; `spawn` also gains a `bg_default: Option<String>`
  snapshot to preserve bg-detect result across reloads.
- Hot path byte-equal to v0.5.1 when profile is inactive (I-6
  regression test pins 13 built-ins).
- No public API breakage. No new Cargo deps. No MSRV change.

### Tests
- 14 new lib tests: 7 `profiles::tests` (Phase 1 validation) + 1
  `rules::tests` hot-path-unchanged + 1 `cli::tests` clap shape + 5
  `rules::tests` byte-pinned `EmbeddedProfile` dispatch + 1
  `rules::tests` three-way Theme/UserConfig/EmbeddedProfile identity
  + 12 `error::tests` byte-pin Display + 2 `main::tests` exit-code
  mapping (total +14 lib including `cargo test --bin tayf` unit
  suite for main.rs tests).
- 15 new integration tests in `tests/integration_profiles.rs` (new
  file): disk happy path, NotFound diagnostic, StylesKey(NameUnknown)
  diagnostic, user-config>profile precedence, 8-cell theme
  precedence matrix (CLI × config × profile.theme), CLI>config
  profile precedence, profile-active hot-reload C-4 (with
  MARK_PRE/MARK_POST sync markers + 1500ms post-edit sleep),
  whitelist+theme combination regression.

## [0.5.1] — 2026-05-25

### Fixed
- `themes::validate_theme_rules` Phase-1 grammar gate now defers non-digit
  styles-map keys (e.g. `styles.date`, `styles.scheme`, `styles.perm_owner`)
  to the dispatch-time named-resolution path. Previously these keys were
  rejected as `CaptureGroupKeyMalformed` before reaching dispatch, making
  named capture-group styling effectively unavailable from theme TOML
  (`assets/themes/*.toml` or `~/.config/tayf/themes/*.toml`). Built-in
  themes (`dark`/`light`) use no `styles` maps, so end-user impact was
  limited to advanced users authoring custom theme TOML.

### Tests
- Three new integration tests in `tests/integration_capture_groups.rs`
  cover the previously dead `RuleSource::Theme` dispatch arms: named-key
  happy path (`styles.date` styles a timestamp), unknown-name diagnostic
  (`styles.bogus` surfaces `CaptureGroupNameUnknown` with `available:`
  list), and duplicate-target diagnostic (`styles."1" + styles.date`
  colliding on the same slot surfaces `CaptureGroupDuplicateTarget`).
- One new unit test in `src/themes.rs` pins Phase-1 acceptance of
  non-digit styles keys.

### Internal
- No public API change. No new variants on `ThemeRuleErrorKind`. No
  signature changes. No `Cargo.toml` dependency delta. No MSRV change.
- Hot path byte-equal to v0.5.0; name → index resolution stays
  compile-time. Bench baseline regen not required.

## [0.5.0] — 2026-05-25

### Added
- Built-in patterns `permission`, `timestamp` (ISO 8601 branch), and `url`
  (`https?|ssh|ftp` branch) now use named capture groups (`(?P<name>...)`).
  Theme TOML and user-config `[[rules]] styles = {...}` maps may
  reference these by name (`styles.scheme = { fg = "cyan" }`) in addition
  to the existing positional form (`styles."1" = { fg = "cyan" }`). Both
  forms resolve to the same capture-group index; setting both forms
  targeting the same group is an error (`CaptureGroupDuplicateTarget`).
  Group names per rule: `permission` → `perm_type`, `perm_owner`,
  `perm_group`, `perm_other`; `timestamp` ISO branch → `date`, `sep`,
  `time`, `ms`, `tz`; `url` first branch → `scheme`, `sep`, `body`.
  Forward-pulled from the v0.5 roadmap.
- Two new `ThemeRuleErrorKind` variants (additive on the
  `#[non_exhaustive]` enum). `CaptureGroupNameUnknown { name, available }`
  — TOML key references a regex group name not present in the rule's
  regex; `available` lists the regex's actual named groups in positional
  order for a pedagogical diagnostic. `CaptureGroupDuplicateTarget {
  positional, named }` — both a numeric and a named key resolve to the
  same capture-group index.

### Changed
- CI workflow `.github/workflows/ci.yml`: bumped
  `actions/upload-artifact@v4` → `@v7` (Node.js 20 deprecation,
  2026-06-02 cutoff). Drop-in input schema; criterion artifact upload
  (`path: target/criterion/`, `retention-days: 14`) behavior preserved.
  The v0.4.1 final review §N-1 forward-pointer prescribed `@v5`; spec
  phase verified v5 is still Node 20 (`@v6` is the Node 24 transition,
  `@v7.0.1` current 2026-04-10), hence `@v7` is the correct target.

## [0.4.1] — 2026-05-25

### Added
- CI now runs `cargo bench --bench throughput` on every PR (ubuntu-
  latest + macos-latest matrix), compares results against canonical
  baseline JSON at `benches/baselines/latest/<os>.json`, and emits a
  workflow annotation when any of the four hot-path benches regresses
  past its threshold (`apply_rules/*` > +20%; `passthrough/write_all`
  > +30%, accounting for sub-µs jitter). Opt in by labeling the PR
  with `bench-ci-strict` to upgrade annotations to errors that fail
  the workflow. Baseline JSON is committed to the repo per release
  as part of the standard ceremony.

### Changed
- The two bare `unreachable!()` arms in
  `rules.rs::resolve_group_styles_for_rule` (the `RuleSource::Builtin`
  branches of the KeyMalformed and OutOfRange checks) now carry
  explanatory reason strings, per CLAUDE.md §2's "unreachable!()
  reason explanation" mandate. Behavior unchanged — these arms are
  constructor-guaranteed unreachable; the strings only surface if a
  future refactor breaks the invariant.

## [0.4.0] — 2026-05-25

### Changed
- `apply_rules` hot path now uses `RegexSet` as a pre-filter. The
  per-line work was a linear scan over every compiled rule's
  `find_iter`/`captures_iter`; v0.4.0 first asks the `RegexSet` which
  rule indices can possibly hit, then dispatches only that subset.
  Pattern-definition order is preserved (regex crate stable contract),
  so the first-match-wins overlap semantics are byte-identical. Per
  v0.4.0 measurement on the existing bench fixtures (see
  `benches/BASELINE.md`):
  - `apply_rules/ipv4-heavy`: 2.4199 → 2.3335 ms (−3.57%).
  - `apply_rules/mixed-syslog`: 2.2948 → 1.7974 ms (−21.68%, the
    real-world headline gain).
  - `apply_rules/captures-heavy`: 4.5173 → 4.8755 ms (**+7.93%**, a
    regression on a synthetic worst-case fixture where every line
    fires ~4/13 patterns; the pre-filter automaton scan cost slightly
    exceeds the cost of the skipped per-rule scans on this shape).
  - `passthrough/write_all`: 1.3380 → 1.1563 µs (−13.58%, sub-µs
    noise band; passthrough path itself unchanged).

  The captures-heavy regression is intrinsic to a uniform RegexSet
  pre-filter: when most rules hit, the pre-filter pays its cost
  without recovering it. Users running workloads dominated by
  capture-styled rules firing on every line may want to evaluate
  the tradeoff against their input. The geomean across the three
  `apply_rules/*` rows is ~5.5% faster, dominated by the
  mixed-syslog gain.

- The per-call scratch `Vec`s inside `apply_rules`
  (`accepted_spans`, `runs`, `event_scratch`, `active_scratch`, plus
  the new `set_match_scratch`) are now Pipeline-owned and reused
  across lines via `Vec::clear()`. Per-line allocation in
  `PipelineScratch`'s surface is zero (was four `Vec::new` calls per
  line). `regex::bytes::RegexSet::matches()` itself internally
  allocates a small bitset per call (regex_automata
  `PatternSet::new(pattern_len)`); that upstream cost is opaque and
  unchanged from prior baselines. Capture-group styling output is
  byte-identical (the v0.3.5 `tests/integration_capture_groups.rs`
  suite passes without modification).

- The `RuleSource::UserConfig` arm in
  `rules.rs::resolve_group_styles_for_rule` for the "group 0
  forbidden" diagnostic now delegates to
  `ThemeRuleErrorKind::CaptureGroupIndexZeroForbidden`'s `Display`
  impl, closing the third (and final) duplicate-formatter drift
  surface that v0.3.6 and v0.3.7 progressively cleaned. Output is
  byte-identical for benign keys.

## [0.3.7] — 2026-05-24

### Fixed
- v0.3.6's `CaptureGroupIndexOutOfRange` Display fix now also reaches
  the user-config error path. Previously, a `~/.config/tayf/config.toml`
  with `styles = { "N" = ... }` on a rule whose regex has no capture
  groups (e.g., `ipv4`) would still emit `(valid: 1..=0)` because
  `src/rules.rs` carried a duplicate formatter that bypassed the
  `Display` impl. v0.3.7 routes that path through the same
  `ThemeRuleErrorKind` variant, producing
  `"rule's regex has no capture groups; styles cannot be set"`
  uniformly. The theme-config error path (fixed in v0.3.6) is
  unchanged.
- The parallel user-config diagnostic for malformed `styles` keys
  (e.g., `styles = { "01" = ... }` with a leading zero) is now also
  routed through the shared `ThemeRuleErrorKind::CaptureGroupKeyMalformed`
  Display impl. Benign keys produce byte-identical output; **adversarial
  keys containing control bytes are now sanitized** instead of being
  printed raw to stderr (previously a minor escape-sequence-injection
  risk per CLAUDE.md §3 mandate).

> [Note] v0.3.6's CHANGELOG read as if both Display fix paths were
> covered; in practice the parallel user-config formatters in
> `src/rules.rs::resolve_group_styles_for_rule` were literal copies
> that bypassed `ThemeRuleErrorKind::Display`. v0.3.7 closes both
> gaps in one commit and adds unit + integration regression guards
> sharp enough to catch this drift class going forward.

## [0.3.6] — 2026-05-24

### Fixed
- `ThemeRuleErrorKind::CaptureGroupIndexOutOfRange` Display now produces
  a meaningful message when the regex has no capture groups at all
  (`captures_len == 1`). Previously rendered `(valid: 1..=0)` — an
  empty range with no actionable guidance. New wording:
  `"rule's regex has no capture groups; styles cannot be set"`.
  The existing pluralization for `captures_len >= 2` is unchanged.
- Renamed two misnamed tests (`syslog_timestamp_match_renders_one_sgr`
  in `src/pipeline.rs` and `tests/integration_capture_groups.rs`) to
  `syslog_timestamp_substring_survives_colorization`. The bodies only
  assert substring survival; the previous name overstated what was
  actually being verified.

## [0.3.5] — 2026-05-24

### Added
- Per-capture-group styling. Regex `[[rules]]` blocks (built-in, user
  config, or disk themes) may now wrap individual capture groups of a
  match with separate styles via a new `styles = { "1" = { fg = "..." } }`
  inline-table map (also supported as the dotted-table form
  `[rules.styles."1"]`). Keys are 1-based positive-decimal indices
  encoded as strings; the entire match (group 0) is reserved for the
  existing `style` field. An empty `styles = {}` is silently accepted
  as a no-op. Validation against the rule's regex `captures_len`
  happens at config load. Feature forward-pulled from the v0.5 roadmap
  to satisfy real-world need for segmented timestamp rendering.
- Public error variants `tayf::ThemeRuleErrorKind::CaptureGroupKeyMalformed`,
  `CaptureGroupIndexZeroForbidden`, `CaptureGroupIndexOutOfRange` —
  additive to the existing `#[non_exhaustive]` enum.
- v0.3.4 configs that already (erroneously) included a `styles` field
  on a `[[rules]]` block would have failed parse with
  `unknown field 'styles'` — they now succeed.

### Changed
- The `timestamp`, `url`, and `permission` built-in patterns now expose
  capture groups (date / T-separator / time / milliseconds / timezone
  for ISO timestamps; scheme / "://" / host+path for HTTP-style URLs;
  type / user / group / other for permission triplets). Each group
  carries an ANSI Basic16-safe default color so downgrade is a no-op
  on every supported terminal.
- The non-ISO timestamp branches (syslog, Apache, RFC 2822) and the
  `git@` SSH branch of the URL pattern remain capture-less; their
  match is wrapped with the rule's default `style` (unchanged from
  v0.3.4).
- **BREAKING:** `tayf::ThemeRuleErrorKind` no longer derives `Copy`
  (two new variants carry `String` / `usize` payload). It still
  derives `Clone`; consumers that only use `Clone` are unaffected.
  Consumers that rely on `Copy` (e.g., `match k { ... => copy_it(*k) }`,
  `[ThemeRuleErrorKind; N]` array literals) must switch to `Clone`.

### Internal
- `src/pipeline.rs::apply_rules` switched to a selective dispatch:
  rules whose `group_styles` vector contains at least one `Some` entry
  go through a new `captures_iter` runs-per-match path; all others
  retain the v0.3.4 `find_iter` hot path. Match-level overlap detection
  now uses a sorted-by-start `accepted_spans: Vec<(usize, usize)>` +
  `partition_point` binary search — `O(log N)` per match regardless of
  how many runs an accepted match emits. Common-case throughput
  significantly improved (see `benches/BASELINE.md`).
- `src/rules.rs` `BuiltinRule` and `Compiled` gained `group_styles`
  (and `Compiled.uses_capture_styling` cache) fields; `pub(crate)`
  visibility unchanged.
- New `emit_capture_runs` boundary-event sweep algorithm — no per-byte
  paint array, no new dependency (pure `Vec<u32>` active-group stack
  reused across matches per line).
- `Compiled::downgrade_for_depth` now also walks `group_styles[*][*]`
  Some entries through the same depth pipeline as the main styles vec.

## [0.3.4] — 2026-05-24

### Added
- Disk-based custom themes. Files placed at
  `~/.config/tayf/themes/<name>.toml` (honoring `$XDG_CONFIG_HOME`) are
  loaded through the same 1 MiB read cap and symlink-out whitelist as the
  user config. Disk themes have the same TOML shape as shipped presets:
  `[[rules]]` blocks with `name` and `style`, no `pattern` field, no
  `enabled = false`, no `[general]` section.
- Public error types `tayf::ThemeRuleError` and `tayf::ThemeRuleErrorKind`,
  bundled into the new `tayf::Error::ThemeValidation` variant. Theme
  validation errors are now collected in a single pass so users see every
  issue at once rather than the previous one-at-a-time loop.

### Changed
- A disk theme that shadows a built-in preset name (`dark`, `light`) is
  rejected with an actionable error rather than silently overriding the
  preset. The check is case-insensitive (protects macOS APFS users from
  accidentally bypassing the gate via case typos). Rename the file
  (e.g. `my-dark.toml`) and reference it as `--theme my-dark`.
- A disk theme that sets `[general]` is rejected — themes only override
  style, and `[general]` fields belong in the user config.
- `tayf::themes::load` returns a richer `LoadedTheme` struct (internal
  API; not part of the public surface). Users of the public `tayf::Tayf`
  facade see no change.
- `--theme` `--help` text updated to mention disk-loaded themes.

### Internal
- `src/config.rs` `config_base` helper extracted from `resolve_path`;
  shared with `src/themes.rs::load_with`.
- `src/main.rs::map_error_to_exit_code` maps `Error::ThemeValidation` to
  EX_USAGE (64).
- `tests/integration_bypass.rs` doc-comment lowercase "error" → "ERROR"
  (test was correct; comment was misleading).
- `benches/BASELINE.md` "TUI passthrough path" claim updated to reflect
  the v0.3.0 ANSI state-machine routing.
- New `tests/integration_disk_themes.rs` for PTY-based SGR assertions;
  `tests/integration_themes.rs` gains no-PTY assertions for the new
  collision, validation, [general] reject, listing, and help-text paths.
- `tests/integration_hot_reload.rs` regression guard for mid-session
  theme-file collisions surfacing as warn-only (no runtime termination).

## [0.3.3] — 2026-05-23

### Added

- `--bypass` CLI flag and `TAYF_DISABLE` environment variable. When either is set (CLI takes precedence; env truthy values: `1`, `true`, `yes`, case-insensitive), tayf still wraps the PTY, forwards signals, and protects the terminal via its raw-mode RAII guard, but skips all rule matching, SGR injection, automatic background detection, and hot config reloading. Equivalent to running the shell directly except for PTY ownership and signal plumbing. Intended for the `[[ -n "$TAYF_DISABLE" ]] || exec tayf`-style conditional wrap in shell rc files, and for one-shot overrides like `TAYF_DISABLE=1 my-tool`.
- `--no-hot-reload` CLI flag. When set, the file watcher and reload orchestrator threads are not spawned. Config still loads at startup as usual; only the *re*-load pipeline is off. With no config file present, `--no-hot-reload` is a no-op (no watcher would have spawned anyway).
- `[general] show_reload_banner` config field (default `false` — opt-in). When set to `true`, a one-line dim banner (`tayf: config reloaded`) is written directly to `/dev/tty` after each successful hot reload (file change or `SIGHUP`). The banner is wrapped in DECSC/DECRC (cursor save/restore) so multi-line shell prompts (zsh ZLE, `RPROMPT`, `PROMPT_SP`) keep their visual cursor position; SGR is balanced via `\x1b[2m` / `\x1b[22m` (dim/bold cancel only — does NOT clobber prompt-side SGR state). Reload *failures* do not write the banner — they continue to surface via the existing `warn_msg!` stderr log path. The banner is naive about TUI / alt-screen state: when an opt-in user is inside vim / less, the banner will be drawn into the alt-screen buffer and vanish when the program exits. Alt-screen-aware queuing is deferred to v0.4.

### Changed

- **BEHAVIOR CHANGE — SIGHUP forwarding.** `SIGHUP` is now forwarded to the child process group in all configurations, mirroring `SIGINT` and `SIGTERM`. Previously (v0.2.1 through v0.3.2), `SIGHUP` was forwarded *only* when the hot-reload pipeline was wired — i.e. when a config file existed AND the reload orchestrator had been spawned. In every other case (no config, or — with v0.3.3 — `--no-hot-reload`), `SIGHUP` was silently dropped by the signal thread, leaving the child shell unaware of tmux detach, terminal-emulator close, or `kill -HUP` and orphaning its foreground processes. v0.3.3 fixes this: the child process group always receives the signal, AND (when hot-reload is wired) the orchestrator additionally receives the reload trigger. Users who relied on the v0.2.1 silent-drop behavior (rare — it was undocumented and arose from the v0.2.1 hot-reload design accidentally intercepting `SIGHUP` without a forwarding fallback) should expect the child shell to terminate or behave per its own `SIGHUP` trap on detach / `kill -HUP`. Most interactive shells (`bash`, `zsh`, `fish`) install a `SIGHUP` trap and tolerate the signal cleanly.

### Notes

- No new dependencies. v0.3.2 (and earlier) config files remain shimless backward-compatible — the new `show_reload_banner` field defaults to `false` when absent.
- The `env_truthy` helper (formerly `bg_detect::env_truthy`, v0.3.2) was moved from `src/bg_detect.rs` to `src/lib.rs` module root (`crate::env_truthy`) so that both the `TAYF_DISABLE_BG_DETECT` and `TAYF_DISABLE` parse paths share a single-source utility. Parsing semantics are identical (`1` / `true` / `yes`, case-insensitive). Module-private behavior change only; no caller outside the crate exists.
- `Args` (the parsed CLI surface, `pub use cli::Args;`) gained `#[non_exhaustive]`. Downstream users who construct `Args { ... }` via struct-literal syntax will need to use a parse-from helper (e.g. `Args::try_parse_from(["tayf"])`) instead. One-time minor break that allows future field additions without semver bumps.
- No new public API surface beyond the additive CLI flags and the additive config field. `Tayf::run` signature unchanged. The internal `ReloadOrchestrator::spawn` (`pub(crate)`) gained a sixth `Option<Box<dyn BannerSink>>` parameter; not part of the public API.

[0.3.3]: https://github.com/beraartuc/tayf/releases/tag/v0.3.3

## [0.3.2] — 2026-05-23

### Added

- `url` built-in now matches `git@host:path` SSH-form Git URLs alongside the existing `https://`, `ssh://`, and `ftp://` schemes. Host class is label-aware (start and end alphanumeric; `.` and `-` allowed in the middle). Path segment shares the URL trim semantics below. Resolves the v0.2.2 deferral.
- `duration` built-in now matches bare-suffix durations (`5s`, `30m`, `2h`, `7d`) AND compound forms (`2d3h`, `1h30m20s`) as single spans. Compound coverage picks up `kubectl get pods` AGE columns and `docker ps` STATUS columns directly. Bare-unit forms require no whitespace between digit and unit (`5m` matches, `5 m` does not — false-positive guard for prose).
- `TAYF_DISABLE_BG_DETECT` environment variable. When set to `1`, `true`, or `yes` (case-insensitive), `bg_detect::resolve()` short-circuits to `BgTheme::Dark` before any `/dev/tty` I/O. Documented as **test-only** — production users should rely on automatic detection or pin a theme via `--theme` / `[general] theme`. Exists for CI environments where `/dev/tty` is a `portable-pty` slave that cannot respond to OSC 11.

### Changed

- `url` built-in no longer includes trailing sentence punctuation (`.`, `,`, `;`, `:`, `!`, `?`) at the end of a match. Closing brackets (`)`, `]`) **stay** in matches to preserve Wikipedia/MDN URLs ending in `)` (e.g. `Foo_(disambig)`) and IPv6 literal hosts (e.g. `https://[::1]`). Trade-off: a URL wrapped in parens (`(https://example.com)`) keeps the trailing `)` in the match — most click-to-open terminals tolerate this; users can override via `[[rules]]`.
- `respect_existing_colors=true` (the v0.3.0 default) remains the recommended configuration. Users who set `respect_existing_colors=false` opt into v0.1-class SGR collision with bare-unit duration matches (e.g. `49m` inside `\x1b[49m`). The default is structurally safe; the opt-out trade-off is explicit. Pinned by the new `bare_units_collide_with_sgr_when_respect_existing_colors_is_false` unit test.

### Fixed

- `watch::drop_stops_debounce_thread` no longer flakes on macOS CI under load. The test now drains in-flight events from the initial write before asserting, then polls for `mpsc::TryRecvError::Disconnected` with a 5-second overall budget. Tolerates the 100–500 ms FSEvents runloop shutdown that previously blew through the prior 500 ms `recv_timeout` budget.
- OSC 11 background-detection hang on macOS `portable-pty` subprocesses not isolated to a single root cause within the v0.3.2 investigation budget. The diagnostic example (`examples/repro_osc11_hang`, mirrors `detect_from_osc11`'s 10 phases) when spawned under `portable-pty` on macOS with `COLORFGBG` scrubbed never flushes any stderr to the master within 15 seconds — total silence prevents phase-level localisation, which ruled out responsible Senaryo 1 (per-phase fix) and Senaryo 2 (kqueue) paths. Added `TAYF_DISABLE_BG_DETECT` env-var bypass (see Added) and replaced the v0.3.1 CI `COLORFGBG=15;15` workaround with `TAYF_DISABLE_BG_DETECT=1`. Production behavior unchanged. Further investigation deferred.

### Notes

- No new dependencies. No public API changes; no config schema changes; no CLI flag changes. v0.3.1 (and v0.2.x) config files remain shimless backward-compatible.
- New repo artifact: `examples/repro_osc11_hang.rs` — standalone diagnostic tool for the `bg_detect` OSC 11 path, mirroring the production function's 10 phases with per-phase wall-clock timing on stderr. Reusable for future regression triage.
- `tests/integration_bg_detect.rs` pins the `TAYF_DISABLE_BG_DETECT` bypass against future regression (scrubs `COLORFGBG` to prove the bypass — not the env-var fast path — is what completes startup within budget).

[0.3.2]: https://github.com/beraartuc/tayf/releases/tag/v0.3.2

## [0.3.1] — 2026-05-23

### Added

- Automatic terminal background detection at startup (best-effort). When you haven't pinned a theme via `--theme` or `[general] theme`, tayf tries `COLORFGBG` first, then an OSC 11 query against `/dev/tty` with a 100 ms timeout, then falls back to `dark`. The matching preset (`light` or `dark`) is applied automatically. tmux is supported; tmux ≥3.3 requires `set -g allow-passthrough on`. GNU screen is not supported. Detection is also skipped when stdout is not a TTY, when `--no-color` is set, or when `TERM=dumb`. **Detection is a starting point, not authoritative** — modern terminals have dozens of light/dark variants and binary detection can't match all of them. README "Themes" section documents limitations and the custom-palette path.

### Fixed

- **I4 (deferred from v0.3.0):** when the ANSI state machine's 4 KiB sequence cap fires while in a string state (OSC / DCS / PM / APC), `Pipeline` now emits a synthetic 7-bit ST (`\e\\`) to stdout to close the unterminated string sequence on the terminal side. Prevents the terminal from absorbing subsequent shell output as part of the never-terminated sequence when fed an adversarial unterminated string payload (>4 KiB OSC, etc.).

### Notes

- No public API changes; no new dependencies. v0.3.0 config files are shimless backward-compatible.
- The new `src/bg_detect.rs` module manages its own short-lived termios snapshot and process-wide panic hook (parallel to `tty_guard`), necessary because release builds use `panic = "abort"` and `Drop` does not run on panic.
- I4 cap-fire-in-string-state coverage is pinned by the `pipeline_writes_st_on_cap_fire_in_string_state` unit test in `src/pipeline.rs`, which drives the same 5 KiB unterminated-OSC input through `Pipeline.feed` deterministically. The binary-level integration test originally planned for `tests/integration_ansi.rs` was deferred until a non-blocking PTY-read primitive lands — `portable-pty`'s blocking reader doesn't honor a deadline between shell exit and EOF propagation on macOS CI runners, so the test could hang past the runner budget.

[0.3.1]: https://github.com/beraartuc/tayf/releases/tag/v0.3.1

## [0.3.0] — 2026-05-23

### Changed

- **`respect_existing_colors` is honored by default.** The config field defaults to `true` and was parsed but ignored in v0.2.0–v0.2.4. Starting with v0.3.0 it is wired into the hot path: any line that already contains an ANSI SGR sequence (`\e[…m`) bypasses tayf's rules and is written to stdout byte-for-byte. Users whose input was already colored (e.g. piped `git log --color=always`, `journalctl` with `SYSTEMD_COLORS=true`) will see tayf stop overlaying its own rules on those lines. Migration: set `[general] respect_existing_colors = false` to restore the v0.2 effective behavior of running rules on every line.

### Added

- OSC, DCS, PM, and APC sequence handling. `\e]…`, `\eP…`, `\e^…`, `\e_…` are now classified as terminal-control payloads and pass through to stdout verbatim. Lines containing such sequences are written byte-for-byte and skip rule application, so OSC 8 hyperlinks (`\e]8;;URL\aLABEL\e]8;;\a`) render correctly without the URL being matched by tayf's `url` rule.
- Non-CSI ESC sequences (`\e=`, `\eM`, `\e7`, `\e8`, `\ec` RIS) and multi-byte ESC sequences (`\e(B` G0 designate, `\e#8` DEC alignment test) are now parsed as control sequences rather than leaking their payload bytes into the rule engine.
- Trigger sequence bytes (`\e[?1049h` alt-screen entry, `\e[31m` SGR, etc.) used to land in the line buffer alongside surrounding text. They are now collected in a per-pipeline scratch buffer and routed by sequence type: stdout for TUI toggles, line_buffer for SGR/other CSI/ESC completions.

### Internal

- New module `src/ansi.rs` (47 unit tests) implementing a 16-state subset of the Paul Williams VT500 ANSI parser (https://vt100.net/emu/dec_ansi_parser). Replaces the manual `TuiModeSm` that lived in `src/pipeline.rs` since v0.1.
- `Pipeline::feed` rewritten with a three-path architecture (TUI passthrough / sequence accumulation in scratch / OSC-payload direct-to-stdout). Existing callers unchanged.
- New 4 KiB internal cap on unterminated CSI/ESC byte accumulation. Defense against malicious input keeping the parser in a non-Ground state forever.
- `Compiled` struct gains a `respect_existing_colors: bool` field. Hot-reload-aware via the existing `ArcSwap<Compiled>` (snapshotted at every line boundary).

### Notes

- No new dependency. No public CLI / config schema change.
- Cross-line SGR state is not tracked: a multi-line color block (e.g. `git log --color=always` with prompts that span lines) is honored on each SGR-bearing line, but rules may still run on intermediate lines that have no SGR themselves. Segment-level semantics are planned for v0.4.
- Known limitation: when the 4 KiB unterminated-sequence cap fires while the state machine is mid-OSC / DCS / PM / APC, the partial payload bytes already on stdout are unterminated. Modern terminals will keep absorbing subsequent bytes as payload until they see their own terminator (or hit their own cap). Mitigation lands in v0.3.1 — the state machine will emit a synthetic ST (`\e\\`) on cap-fire-in-string-state so the terminal closes the sequence promptly.

[0.3.0]: https://github.com/beraartuc/tayf/releases/tag/v0.3.0

## [0.2.4] — 2026-05-23

### Changed

- README "Themes" section moved under `## Configuration (v0.2)` as a subsection. Themes are a configuration source, not a sibling concept; readers find them while looking for how to configure colors.
- `Compiled::load` proxy removed. The v0.2.3 series kept a thin `load(config, path, depth)` wrapper around `load_with_theme(config, path, None, depth)` to spare existing callsites. With the layering settled, the proxy is dead surface — production callers use `load_with_theme` directly and the remaining test callers have been inlined. `Compiled::load_builtins` is the only convenience shim left.
- CI workflow upgraded `actions/checkout` from v4 to v5 ahead of the Node.js 20 deprecation enforced June 2026.

### Documentation

- `src/rules.rs::compile_error_for` doc-comment now spells out that theme TOML rules ride the same error-routing path as user `[[rules]]` after `themes::validate_theme_rules` runs.

### Notes

- No new dependencies. No public API change. Patch release containing only the polish items flagged in the senior Rust review of v0.2.3.

[0.2.4]: https://github.com/beraartuc/tayf/releases/tag/v0.2.4

## [0.2.3] — 2026-05-23

### Added

- Preset color themes embedded in the binary. `dark` ships as the explicit defaults — identical to running tayf without a theme, useful as a copy-and-edit template. `light` ships as a re-tuned palette for light-background terminals.
- `--theme <NAME>` CLI flag. CLI value overrides the config-file value.
- `[general] theme = "..."` in `~/.config/tayf/config.toml`. Defaults to `None` (no theme).
- New `Error::Theme { name, available }` variant; unknown theme names exit with code 64 (`EX_USAGE`) and list the available themes on stderr.

### Fixed

- Light-background terminal portability. Built-in styles `permission` (`White + dim`), `timestamp` (`BrightBlack`), `ipv6` (`BrightYellow`), and `ipv4` (`Yellow + bold`) were invisible or low-contrast on light backgrounds. The new `light` theme provides a readable palette without requiring a user config.

### Notes

- No new dependencies. No public API change beyond the additive `--theme` CLI flag and the additive `[general] theme` config field.
- Theme styles apply on top of built-in defaults; your `[[rules]]` in `~/.config/tayf/config.toml` still win over the theme. Same precedence as before, with the theme as a new middle layer.
- Theme files are baked into the binary at build time. Users wanting a custom theme should still use the user-config layer.
- Automatic background detection (`COLORFGBG` / OSC 11) is deferred to v0.3.

[0.2.3]: https://github.com/beraartuc/tayf/releases/tag/v0.2.3

## [0.2.2] — 2026-05-22

### Added

- Five new built-in patterns, growing the default rule set from 8 to 13:
  - `permission` — POSIX `ls -l` file mode strings (`-rwxr-xr-x`, `drwxr-xr-x`, ACL `+` suffix). Styled `White` + `dim`.
  - `timestamp` — multi-format (ISO-8601, syslog, Apache/nginx, RFC 2822 incl. obsolete US zones EST/EDT/CST/CDT/MST/MDT/PST/PDT). Styled `BrightBlack`.
  - `uuid` — canonical 8-4-4-4-12 hex form, case-insensitive. Styled `BrightMagenta`.
  - `url` — `https?://`, `ssh://`, and `ftp://` URLs. Styled `BrightBlue` + `underline`. `git@host:path` SSH URL alt-form deferred to v0.3.
  - `email` — RFC 5322 simplified shape. Styled `BrightGreen`.
- All five new patterns are subject to the existing user-config override / disable / restyling mechanism shipped in v0.2.0 — no config schema changes required.

[0.2.2]: https://github.com/beraartuc/tayf/releases/tag/v0.2.2

## [0.2.1] — 2026-05-22

### Added

- **Config hot reload.** Editing `~/.config/tayf/config.toml` (or the path passed to `--config`) takes effect in the running tayf within ~250 ms — no restart, no shell respawn. The file watcher uses `notify` 8 (inotify on Linux, FSEvents on macOS) with a 200 ms manual debounce window.
- **`SIGHUP` triggers a reload.** `pkill -HUP tayf` or `kill -HUP <pid>` forces an immediate reload that bypasses the watcher debounce — useful from scripts. SIGHUP also re-resolves the config path, so if you didn't have a config at startup and create one later, SIGHUP picks it up.
- **Fail-safe semantics.** If your edit produces invalid TOML or a bad regex, tayf keeps the **previous** rule set in effect and logs a warning to stderr (`TAYF_LOG=warn`, the default). The terminal session is never disrupted by a typo in the config.
- Integration test for `SIGWINCH` delivery (filling a v0.2.0 coverage gap surfaced during the signal-hook 0.4 review). Three integration tests for hot reload covering file edit, parse failure, and SIGHUP.

### Changed

- `signal-hook` upgraded from 0.3 to 0.4. No behavioral change in tayf's signal path; 0.4.2 includes a bug-fix in the `Handle::close` codepath that `SignalGuard::drop` exercises.
- `Pipeline.rules` now lives behind `Arc<ArcSwap<Compiled>>`. `apply_rules` snapshots the handle once per line via `ArcSwap::load_full`, so reloads landing mid-line take effect on the next line — never split the current one.
- New direct dependencies: `arc-swap 1.9` (wait-free atomic Arc swap) and `notify 8.2` (cross-platform filesystem watcher, `default-features = false`).
- `signals::spawn_handler` now takes a third argument `Option<Sender<ReloadRequest>>`; non-`None` enables SIGHUP forwarding to the reload orchestrator.
- `deny.toml` allow list widened to permit `CC0-1.0` (notify itself) and `ISC` (the inotify family on Linux). Both are OSI-recognized permissive licenses; neither imposes copyleft contagion.

### Internal

- New modules `src/reload.rs` (orchestrator + `reload_once` function) and `src/watch.rs` (notify wrapper + manual debounce loop).
- New threads at runtime: `tayf-debounce` (notify event coalescing) and `tayf-reload` (parse + compile + atomic swap). Total in v0.2.1: 6 threads (main, `tayf-output`, `tayf-input`, `tayf-signals`, `tayf-debounce`, `tayf-reload`).
- `Tayf::run` shutdown sequence carefully ordered: `_orchestrator` is declared last among the threading scaffolding so any `?` failure on earlier setup never drops it with live `reload_tx` clones (which would deadlock the join). After `runtime::run` returns, watcher and signal guard are explicitly dropped *before* the orchestrator's implicit Drop, so the reload thread sees the channel close cleanly.
- `info_msg!` macro added to `src/log.rs` mirroring the existing `warn_msg!` shape. Successful reloads emit at `TAYF_LOG=info`; default behavior is silent.

[0.2.1]: https://github.com/beraartuc/tayf/releases/tag/v0.2.1

## [0.2.0] — 2026-05-22

### Added

- TOML config (`~/.config/tayf/config.toml`, fallback to `$XDG_CONFIG_HOME/tayf/config.toml`, override via `--config <PATH>`). Override built-in rule styles by name, disable built-ins via `enabled = false`, or append custom regex rules. Without a config file, behavior is byte-identical to v0.1.
- Color string parser for TOML values: ANSI names (`"red"`, `"bright_cyan"`), 256-indexed (`"color(178)"`), 24-bit hex (`"#ff8800"`), and functional rgb (`"rgb(255, 136, 0)"`).
- Color depth downgrade: `Style::downgrade(depth)` collapses Rgb / Indexed values into whatever the terminal supports (detected from `$COLORTERM` and `$TERM`). Pre-baked into the compiled rule set at startup so the hot path is unchanged.
- New `Error::Config { path, line, message }` variant routed to exit code 64 (`EX_USAGE`) with friendly stderr diagnostics.
- `serde = "1"` and `toml = "0.9"` direct deps; `tempfile = "3.27"` dev-dep (used by integration tests).

### Changed

- `rules::Compiled::load` now takes `(config: Option<&Config>, config_path: Option<&str>, depth: ColorDepth)`. The `config_path` is threaded into user-rule validation/regex errors so diagnostics carry the real file path. The previous `Compiled::load_builtins()` is preserved as a thin wrapper so the `__bench__` shim and existing call sites compile unchanged.
- `config::load` now returns `Option<(Config, PathBuf)>` so callers know which file was loaded without re-resolving the XDG/home cascade.
- `BuiltinRule::name` migrated from `&'static str` to `String` to hold user-supplied rule names. Cost: eight heap allocations at startup.
- Every regex (built-in and user) now compiles with both `RegexBuilder::size_limit(1 MiB)` (NFA program cap) and `dfa_size_limit(1 MiB)` (DFA lazy-cache cap) to bound the memory a single user regex can consume.
- **Breaking:** `Error` enum gained the `Config { path, line, message }` variant and is now marked `#[non_exhaustive]`. External callers using non-exhaustive matches need a `_ => ...` arm. The `#[non_exhaustive]` attribute prevents future variant additions from being silent breaks.

### Security

- Default-path resolution canonicalizes both the candidate config file and the configured base directory; the file is rejected if it resolves outside the base. Protects against `~/.config/tayf/config.toml` being a symlink to `/etc/shadow` or a hostile shared mount. `--config <PATH>` is the documented opt-out for project-local configs.
- 1 MiB cap on the size of the loaded config file.
- `--config <PATH>` verifies the target is a regular file before reading.

[0.2.0]: https://github.com/beraartuc/tayf/releases/tag/v0.2.0

## [0.1.3] — 2026-05-22

### Changed

- Bumped `thiserror` 1.0 → 2.0. Mechanical migration; the `tayf::Error` enum's derive surface is source-compatible. Done ahead of v0.2.0 so upcoming TOML config error variants can be written in modern syntax from the start.
- Bumped `criterion` 0.5 → 0.8 (dev-dep). `criterion::black_box` is now deprecated in favour of `std::hint::black_box`; updated the import in `benches/throughput.rs`. Bench semantics unchanged.

[0.1.3]: https://github.com/beraartuc/tayf/releases/tag/v0.1.3

## [0.1.2] — 2026-05-22

### Changed

- Bumped `portable-pty` 0.8 → 0.9; transitively drops unmaintained `serial 0.4` (RUSTSEC-2017-0008) and `bitflags 1.x` (on Unix).
- Bumped `nix` 0.27 → 0.28 to match portable-pty 0.9's internal nix; collapses the duplicate nix version in the tree.
- Refreshed requirement lines: `clap` 4.5 → 4.6, `regex` 1.10 → 1.12, `tempfile` 3.10 → 3.27 (resolved versions unchanged; hygiene only).
- `nix::unistd::pipe()` returns `(OwnedFd, OwnedFd)` directly in 0.28, so one `unsafe { OwnedFd::from_raw_fd(...) }` block in `runtime.rs` is gone (3 unsafe sites in `src/` now, down from 4).

### Removed

- `built` (build-dep) — replaced with a 60-line `build.rs` calling `git rev-parse` and `rustc --version` via stdlib `Command`. Drops ~25 transitive crates: git2, libgit2-sys, libz-sys, url, idna, and the entire ICU stack (icu_collections, icu_normalizer, icu_properties, icu_provider, yoke, zerovec, tinystr, displaydoc, etc.).
- `tracing` + `tracing-subscriber` — replaced with a 30-line `src/log.rs` (env-gated `eprintln!` wrapper via `AtomicU8` level + `Once`-guarded init from `TAYF_LOG`). Drops 11 transitive crates: tracing-core, tracing-attributes, tracing-log, matchers, sharded-slab, thread_local, nu-ansi-term, smallvec, valuable, plus a second `regex-automata` copy via `env-filter`. The single `warn_msg!` call site preserved at `pipeline.rs` line-buffer overflow.

### Fixed

- `RUSTSEC-2017-0008` (unmaintained `serial 0.4` via portable-pty) is gone; `deny.toml` ignore entry removed.

[0.1.2]: https://github.com/beraartuc/tayf/releases/tag/v0.1.2

## [0.1.1] — 2026-05-22

### Added

- GitHub Actions CI matrix (Linux + macOS) with fmt, clippy, build, test.
- `cargo audit` and `cargo deny` in CI for dependency hygiene.
- `criterion` throughput benchmarks (`benches/throughput.rs`).
- Pipeline tick-flush: interactive prompts now colorize within 50 ms of idle.
- Input thread self-pipe wakeup: clean shutdown without OS-reap.

### Changed

- CLI parse errors exit with 64 (`EX_USAGE`) per BSD sysexits.
- `TIOCGWINSZ` ioctl consolidated into a single `terminfo::winsize` helper.
- Public API trimmed to spec §3.2: `Args`, `Error`, `Result`, `Tayf::run`.
- `cli` and `error` modules hidden behind crate-root re-exports.

[0.1.1]: https://github.com/beraartuc/tayf/releases/tag/v0.1.1

## [0.1.0] — 2026-05-21

### Added

- PTY-based shell wrapper that spawns the user's shell inside a pseudo-terminal
  and pipes output through a regex-driven rule engine.
- Eight built-in patterns: IPv4, IPv6, MAC, log level, HTTP status, FQDN,
  duration metric, and filename extension (curated catalog covering archives,
  source code, configuration, documents, media, and binaries).
- Shell discovery cascade: `$SHELL` → `/etc/passwd` (`getpwuid`) → `/bin/sh`,
  with `--shell <path>` and `--login` overrides.
- CLI flags: `--shell`, `--login`, `--no-color`, `--help`, `--version`.
- Build-time SHA and rustc version surfaced via `--version`.
- `TAYF_LOG` environment variable activates `tracing-subscriber` diagnostics on
  stderr (off by default).
- 5-state DEC private mode parser detects TUI modes (alt-screen
  `\x1b[?1049h` and legacy variants `47`/`1047`, bracketed paste `2004`,
  mouse tracking `1000`/`1002`/`1003`/`1006`) and switches the pipeline to
  passthrough so vim, less, htop, neovim, Claude Code, lazygit, k9s, gum,
  bubbletea-based tools, and similar TUIs render unaltered.
- 64 KB hard-capped, UTF-8-safe line buffer.
- Raw-mode termios RAII guard with panic-hook fallback so the terminal is
  restored on any exit path.
- Signal handling thread (signal_hook): `SIGWINCH` resizes the PTY, `SIGINT`
  and `SIGTERM` are forwarded to the child process group with `killpg`.
- Exit-code propagation from the child shell; OS-error and software-error
  codes per BSD sysexits when tayf itself fails.
- Automatic colorization bypass when stdout is not a TTY.

### Security

- All built-in patterns are linear-time (verified by inspection); no
  catastrophic backtracking.
- Line buffer is hard-capped to bound memory under adversarial output.
- ANSI emission is restricted to SGR sequences by a single audited function
  (`Style::to_sgr`) with a unit test asserting the output grammar.
- Shell is spawned via `argv`-style `CommandBuilder`, never via `sh -c`.

[0.1.0]: https://github.com/beraartuc/tayf/releases/tag/v0.1.0
