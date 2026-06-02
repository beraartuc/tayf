# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.12.2] - 2026-06-02

### Fixed
- `tayf init`'s shell hook now installs at the **top** of the rc file and locates
  the tayf binary by absolute path — trying `~/.local/bin`, `~/.cargo/bin`,
  `/opt/homebrew/bin`, and `/usr/local/bin` before a `PATH` fallback — instead of
  a bare `exec tayf`. Appended at the bottom with a PATH-relative `exec`, the
  hook silently failed to auto-start in two common setups: a prompt framework
  (e.g. Powerlevel10k instant prompt) that redirects stdout during init made the
  `-t 1` guard fail, and an install directory not yet on `$PATH` made `exec tayf`
  a no-op — so new terminals opened uncolored and `exec tayf` had to be run by
  hand. Re-running `tayf init` relocates and refreshes an older bottom-of-file
  hook to the top.

## [0.12.1] - 2026-06-02

### Fixed
- `permission` now colorizes the SELinux/extended-attribute trailing-indicator
  form `drwxr-xr-x.` (the `.` an SELinux context adds, shown by `ls -l` on
  Fedora and other SELinux systems) in addition to the ACL `+` and macOS
  xattr `@` forms. Previously every permission string on such systems carried a
  trailing `.` and was skipped entirely, so no file modes were colorized.
- Changing a built-in rule's color in the `tayf config` TUI (`c` then `Ctrl+S`)
  now persists to `config.toml` instead of reverting to the default on reload.
  The color change was being applied only to the live preview; it is now
  written as a name-keyed `[[rules]]` style override (merged with any
  enable/disable flip into a single entry).
- A line with no trailing newline that is immediately followed by an escape
  sequence — e.g. a wrapped shell's colored prompt, or zsh's reverse-video `%`
  "missing newline" indicator — is now colorized. Previously the following
  sequence merged into the unterminated line and, under
  `respect_existing_colors`, suppressed rule application, so the line was left
  uncolored (intermittently: a fast-arriving prompt sequence lost the race with
  the idle flush, producing "uncolored on the first run, colored on a retry").
  The plain content is now rule-applied before the interrupting sequence.

## [0.12.0] - 2026-06-02

### Added
- **`tayf init`** — one-step first-run setup. Writes the default config and,
  for bash/zsh, installs an always-on rc guard (timestamped backup, idempotent
  marker block, `--uninstall` to remove). fish and other shells get a printed
  snippet via `--print`. Flags: `--shell`, `--no-shell-hook`, `--print`,
  `--uninstall`, `--force`, `--config`.
- **Six domain rules promoted to built-ins (indices 12–17).** tayf now
  colorizes AWS, Docker, and Kubernetes output without requiring a profile:
  - `arn` — AWS ARN (`arn:aws[…]`), priority 200, default **on**.
  - `instance_id` — EC2 instance ID (`i-` + exactly 17 lowercase hex digits),
    priority 100, default **on**.
  - `region` — exhaustive 34-entry AWS region enumeration (dated snapshot
    2026-05-26; new regions require a patch + CHANGELOG), priority 100,
    default **off** (opt-in: `[[rules]] name = "region" enabled = true`).
  - `container_id` — Docker short hash (12-hex), priority 100, default **off**
    (collides with git short hashes; opt-in).
  - `image_tag` — Docker/OCI registry image reference with tag, priority 200,
    default **off** (opt-in).
  - `pod_name` — Kubernetes pod suffix (10 + 5 base32 hash), priority 100,
    default **off** (opt-in).
- **Per-rule `enabled` toggle.** Every built-in can be individually
  enabled or disabled in `config.toml` or a named profile:
  `[[rules]] name = "container_id" enabled = true` enables a default-off
  built-in; `enabled = false` disables a default-on one. The TUI's Space key
  stages the toggle interactively.
- The syslog (`Jun  1 12:35:01`), Apache common-log
  (`01/Jun/2026:12:45:30 +0300`) and RFC 2822
  (`Mon, 01 Jun 2026 12:40:00 +0300`) timestamp formats now receive the same
  date / time / zone sub-coloring as ISO 8601 timestamps; previously they were
  colorized as a single flat span.
- A chrome accent theme ("Brass & Slate") for the `tayf config` TUI: the tab
  strip, pane borders, section headers, selected-row highlight, and modal titles
  are now colored, adapting to a light or dark terminal background. Colorized
  content and the live-preview strip keep your real rule colors.
- The color picker shows a **Current** indicator — a live swatch plus the
  selected color's value and kind — so you can see what you are about to bind.
- The Patterns Detail pane shows the selected rule's current color (a swatch
  plus its code, e.g. `#7c5cff`), and pressing `c` opens the color picker
  pre-filled with that color instead of empty.
- The Patterns tab shows a contextual key-hint row
  (`n:new  e:edit  c:color  o:override  r:reset  d:delete  Space:on/off`) so the
  available actions are discoverable without opening the help modal. The Detail
  pane shows the selected rule's `Status: enabled`/`disabled`, and default-off
  rules render dimmed with a `[ ]` marker so they are visible while browsing.
- The navigable lists (Patterns, Themes, Profiles) mark the selected row with a
  `▶` caret, so the highlighted entry is unmistakable regardless of how the
  terminal renders the selection background.
- The live-preview sample now includes an example token for every built-in
  pattern (AWS/Docker/Kubernetes shapes included), so enabling a default-off
  rule shows its color in the preview immediately.

### Changed
- **Profiles are now personal, switchable presets.** A profile is a
  `[[rules]]` list (override / enable-disable / recolor built-ins, add custom
  patterns) plus an optional `theme`, stored at
  `~/.config/tayf/profiles/<name>.toml`. `config.toml` is the default profile.
  When a named profile is active its rules **replace** `config.toml`'s
  `[[rules]]`; the built-ins remain the substrate. Replace is total: per-rule
  overrides written in `config.toml` do not carry into a named profile.
- **Color picker reworked** in the `tayf config` TUI: hex is now the primary
  input (type `#rrggbb`, or `@0-255` for a 256-palette index); the large
  256-color swatch grid was removed. ANSI-16 swatches and the
  bold/italic/underline toggles remain.
- The `tayf config` TUI redraws only on a state change (key, resize, debounce,
  or toast) instead of ten times a second while idle, lowering idle CPU.
- Re-toned the default Neon palette to a brighter, softer cyan/amber family:
  `ipv4` is now `#33c7ff` and `duration` `#f7d17f`, with the other ten rule
  colors retuned to match. The `permission` / `timestamp` gray (`#83838d`) is
  unchanged. `assets/themes/dark.toml` mirrors the new defaults.
- `tayf config dump` now shows all 18 built-in rules in the patterns section,
  with `enabled = false` annotating each default-off rule. The retired embedded
  profile enumeration is replaced by a usage note.

### Removed
- **Embedded profile library retired.** The bundled `aws`, `k8s`, `docker`,
  `gcp`, and `network` profiles no longer exist. `--profile aws` (and the
  others) now report `profile not found` — there is no compatibility shim. The
  domain rules those profiles contained are now built-in (or default-off
  built-ins) and active for everyone without a profile selection. Personal disk
  profiles (`~/.config/tayf/profiles/<name>.toml`) are unaffected.

### Fixed
- The `tayf config` live preview now matches what `tayf` actually renders.
  Previously it ignored terminal background detection and always previewed the
  dark-tuned built-in colors, so on a light-background terminal (which the
  runtime colorizes with the `light` theme) the preview disagreed with the real
  output. The preview now resolves the effective theme with the same precedence
  as the runtime (config theme > profile theme > background detection) **and**
  compiles at the terminal's detected color depth, so on a 256/16-color
  terminal it downsamples exactly as `tayf` does rather than showing full RGB.
- The `tayf config` TUI no longer dead-ends on a fresh machine: with no config
  file present it binds the default path and `Ctrl+S` / `Shift+D` create the
  file (previously `"first-run save requires init"`).
- `permission` strings carrying a macOS extended-attribute (`@`) or ACL (`+`)
  suffix — e.g. `drwxr-xr-x@` — are now colorized. The pattern previously
  accepted only a trailing `+`, so the common macOS `@` form was skipped
  entirely.
- `ipv6` addresses using `::` compression followed by more than one group —
  e.g. `fe80::203d:cff:fe0d:898e` — are now matched in full instead of only the
  leading `fe80::203d`.

## [0.11.0] - 2026-06-01

### Added
- One-line installer `install.sh` (`curl … | sh`): detects OS/arch, downloads
  the matching signed release binary, verifies SHA256 (mandatory) and Sigstore
  provenance (best-effort via an authenticated `gh`), and installs to
  `~/.local/bin` (no sudo; `TAYF_INSTALL_DIR` / `TAYF_VERSION` overrides).
- Static (musl) Linux release binaries for `x86_64-unknown-linux-musl` and
  `aarch64-unknown-linux-musl` — glibc-independent, run on any distribution and
  on x86-64 or ARM64.

### Changed
- The existing `x86_64-unknown-linux-gnu` binary is still published alongside
  the new musl binaries; the installer selects the musl binary on Linux.

### Security
- The installer's trust model: SHA256 guarantees integrity; authenticity is
  anchored by the per-binary Sigstore attestation (verified best-effort in the
  script, with the exact manual command printed when an authenticated `gh` is
  absent). A genuine attestation mismatch aborts the install.

## [0.10.0] - 2026-06-01

The first public release. tayf is now published on crates.io and Homebrew, with
signed GitHub Release binaries. Beyond the new always-on marker, runtime behavior
is unchanged — this cycle is distribution, CI hardening, and public documentation.

### Added

- **`TAYF_SESSION=1`** is set in the spawned child shell's environment, so
  always-on wrappers (an rc `exec tayf` guard) and tools can detect they are
  running inside tayf.
- **Distribution channels:** crates.io (`cargo install tayf`), a Homebrew tap
  (`brew install beraartuc/tayf/tayf`), and signed GitHub Release binaries
  (combined `SHA256SUMS`, Sigstore provenance bundles, and a CycloneDX SBOM).
- **`ARCHITECTURE.md`** and **`CONTRIBUTING.md`** for contributors.

### Changed

- **CI runs entirely on ephemeral GitHub-hosted runners** (the self-hosted
  runner was retired for the public flip). Workflow token permissions are
  default-deny; the heavier bench and fuzz jobs run on push only.
- **`release.yml` is live.** A tag push (`v*`) builds, signs (keyless Sigstore,
  SLSA Build L2), publishes to crates.io (idempotent, in a protected
  environment), and creates a signed GitHub Release; `workflow_dispatch` stays a
  dry-run that publishes nothing.
- **fuzz-smoke is now a hard CI gate** (previously non-blocking).
- The README was rewritten for the public: install methods, always-on setup, and
  the v0.9.1 Neon palette.

### Removed

- The internal design/process docs (`docs/superpowers/`, `tayf-tasarim.md`) are
  no longer in the published tree (they remain in git history); in-code design
  citations now point to `ARCHITECTURE.md`.
- The committed `.cargo/config.toml` (a local mold-linker optimization) and an
  OSC-11 debug repro example were removed from the tree. mold is documented as an
  optional local setup in `CONTRIBUTING.md`.

### Security

- CI has no self-hosted runner: every job runs on ephemeral GitHub-hosted
  infrastructure, and fork PRs get a read-only token and no secrets (the
  workflow uses `pull_request`, not `pull_request_target`).
- The crates.io publish token is crate-scoped, expiring, and confined to a
  protected release environment with a required reviewer.
- `SECURITY.md` now documents the benign signal-teardown `killpg` window and the
  crates.io immutability / yank rollback story.

### Notes

- The `fancy-regex` entry in `Cargo.lock` is a feature-gated, **uncompiled**
  transitive of `termwiz` (via `portable-pty`), not a removable orphan
  (`cargo tree -i fancy-regex` is empty). The lockfile is intentionally held at
  the audited v0.9.1 state for release stability.

## [0.9.1] - 2026-05-31

A visual refresh of the default colors. The built-in palette moves from the
named-ANSI set to a curated 24-bit "Neon" scheme, with a hand-authored light
variant and the previous palette preserved as an opt-in `classic` theme. No
runtime logic changed — only style values, theme presets, and their tests.

### Added

- **`classic` theme.** The pre-Neon, named-ANSI palette is preserved as
  `--theme classic` for users who prefer output that adapts to their terminal's
  own color scheme rather than the fixed 24-bit default.

### Changed

- **Default palette is now 24-bit "Neon".** The 12 built-in rules use curated
  `Color::Rgb` values (log_level hot-coral, ipv4 azure, ipv6 indigo, url blue,
  duration amber, …) instead of named-ANSI colors. On terminals below truecolor,
  values downgrade to the nearest 256/16 entry via the existing `downgrade()`
  path. `ipv4` is no longer bold — bold is now reserved for `log_level`, the one
  alert affordance — and `url` renders as a single underlined blue span.
- **Light theme recast for Neon.** `--theme light` is a hand-authored,
  dark-and-saturated, hue-spread adaptation tuned for contrast on a light
  background, and now also overrides capture-group sub-colors.
- **Profile rule colors harmonized with Neon.** The docker/aws/k8s profile
  rules move to dedicated hues that do not collide with the built-ins:
  `container_id`/`instance_id`/`pod_name` → coral, `image_tag`/`arn` → rose,
  `region` → emerald. These are dark-tuned; on a light background, override them
  in user config.

### Fixed

- The light theme now adapts capture-group sub-colors (the permission rwx triad
  and timestamp fields), fixing a latent issue where those kept their dark-tuned
  values on a light background.

### Notes

- The 24-bit palette emits longer SGR sequences than named-ANSI, so per-line
  colorization overhead is correspondingly higher on heavily-matched streams.
  This is a byte-count effect only (no functional change); the default palette
  can be reverted to the lighter-weight named-ANSI set with `--theme classic`.

## [0.9.0] - 2026-05-31

Security cycle. A comprehensive, empirical verification of the CLAUDE.md §3
threat model — adversarial PTY input, terminal-state corruption, process and
signal handling, config/filesystem, and supply chain — together with new
hardening and the (dry-run) public-release infrastructure. The systematic audit
(four threat-category reviewers plus a red-team adversarial-PTY pass and the
security-review skill) found zero vulnerabilities; the runtime posture is
verified, not merely asserted. No shipped runtime logic changed in this cycle
(the only `src/` additions are `#[cfg(fuzzing)]`-gated and absent from normal
builds). The actual public release (crates.io, Homebrew, signed binaries) is a
separate v1.0 cycle; v0.9 builds and dry-runs that infrastructure.

### Added

- **Fuzzing harness** (`fuzz/`, a separate cargo-fuzz workspace): four libFuzzer
  targets — `ansi_sm`, `line_buffer`, `pipeline_feed` (with an empty-rules
  byte-identity differential oracle), and `regex_compile` — exposed via a
  `#[cfg(fuzzing)]` access module that adds zero surface to normal/published
  builds. A nightly `fuzz-smoke` CI job runs each target briefly as a regression
  signal.
- **Adversarial regression tests** (`tests/adversarial.rs`, stable/non-nightly):
  CSI-split chunk-boundary invariance, OSC flood, and the over-cap synthetic-ST
  behavior — the permanent guard into which any fuzz crash is distilled.
- **`tests/integration_signal_int.rs`**: pins SIGINT forwarding to the child
  process group, complementing the existing SIGHUP and SIGWINCH tests.
- **`benches/redos.rs`**: a linear-scaling proof for the linear-time regex
  engine (input 2x => time ~2x), demonstrating the guarantee rather than hunting
  for impossible super-linear blowup.
- **`SECURITY.md`**: vulnerability-disclosure policy, supported-versions table,
  and explicit non-goals (sandboxing, Windows, multi-user isolation).
- **`.github/workflows/release.yml`** (dry-run only, `workflow_dispatch`): a
  cross-platform build matrix (Linux + macOS) producing SHA256SUMS, keyless
  cosign/Sigstore signing, a CycloneDX SBOM, and SLSA v1.0 Build L2 provenance
  via `actions/attest-build-provenance`. No real publish — that is a v1.0 step.
- **MSRV CI job**: verifies the project builds on its declared minimum Rust.

### Changed

- **cargo-deny hardened**: `multiple-versions` raised from `warn` to `deny`
  (with documented, version-pinned `skip`/`skip-tree` for irreducible transitive
  duplicates), and `[advisories]` updated to the current schema with
  `unmaintained = "all"` and `unsound = "all"` scope selectors.
- **CI supply-chain hardening**: all GitHub Actions pinned to immutable commit
  SHAs (closing the mutable-tag attack surface), and the `cargo install` tool
  versions (cargo-audit, cargo-deny, cargo-fuzz, cargo-cyclonedx) pinned.
- **MSRV raised from 1.74 (undeclared/untested) to 1.88 (verified in CI)**; the
  floor is set by transitive dependencies declaring `rust-version = 1.88`
  (`darling`, `time`, and `instability` via `ratatui`), with edition-2024
  (`clap_lex`) additionally requiring ≥ 1.85.

### Fixed

- **Published-crate packaging**: `assets/profiles/*.toml` were missing from the
  `include` list although `src/profiles.rs` embeds them via `include_str!` at
  compile time. The published crate would have failed to compile for downstream
  users; the dry-run release pipeline caught it before any public release.

### Security

- Comprehensive CLAUDE.md §3 threat-model audit — CLEAN, zero findings. The
  posture (panic-abort termios restore, killpg-to-process-group signal
  forwarding, structurally-SGR-only injection with a precise reset, opaque OSC
  passthrough, 1 MiB regex size/DFA limits on every user pattern, canonicalize +
  symlink-out config gate, atomic O_EXCL config writes, stderr-only gated
  logging) was empirically verified, including 2.3M+ fuzz iterations with zero
  crashes. See `docs/superpowers/reviews/2026-05-31-v0.9-systematic-security-audit.md`.

## [0.8.3] - 2026-05-31

Performance-series finale. A measurement-first (spike-first) investigation of the
two remaining optimization levers found neither worth shipping: the pipeline's
dominant cost is now the irreducible regex scan, and v0.8.0–v0.8.2 already captured
the structural wins. v0.8.3 ships an English-cleanup pass and records the finding;
no hot-path code changed.

### Changed

- Anglicized leftover Turkish `Karar`/`karar` → `Decision`/`decision` in
  doc-comments across `src/` and `tests/`, and corrected Turkish-jargon leaks
  (`DOKUNULMAZ`, `paralel`) in older CHANGELOG entries. Comment/text only; no
  behavior change.

### Performance

- **H5 (no-match fast-lane): investigated, not adopted.** A whole-run
  `RegexSet::is_match` pre-scan to skip per-line rule application on no-match runs
  yielded no throughput win (a byte-identical spike regressed `pipeline_feed/prose`
  rather than improving it). `RegexSet::is_match` over a run visits every byte just
  as the sum of per-line `RegexSet::matches` does, so the regex work is unchanged;
  the per-line bookkeeping a fast-lane would skip was already made cheap in
  v0.8.1/v0.8.2.
- **H6 (apply_rules internals): investigated, not adopted.** Skipping the final
  run-sort and the priority-sort on the common case measured 0% ± noise on the
  high-match shape — the sorts are below the measurement floor; the regex scan
  dominates `apply_rules`.
- **Perf series concluded.** The remaining overhead is the regex scan itself,
  irreducible without a different matching strategy (combined regex / Aho-Corasick
  literal prefilter / SIMD), which is out of scope here. The cumulative v0.8.x
  improvement vs v0.8.0 is roughly prose 3.0×, log 1.7×, ansi 2.1×. See
  `docs/superpowers/reviews/2026-05-31-v0.8.3-phase0-checkpoint.md`.

## [0.8.2] - 2026-05-30

Performance cycle (H4, security-gated): the output pipeline now processes input
chunk-by-chunk instead of byte-by-byte. The off-limits hot-path module `src/pipeline.rs` was
modified under a mandatory security review; `src/runtime.rs` and `src/pty.rs`
(termios / raw-mode / signal forwarding) were not touched.

### Performance
- `Pipeline::feed` rewritten from a per-byte loop to a chunk-level one: while in
  the ANSI ground state, the run up to the next escape is batched through a new
  byte-identical `LineBuffer::feed_data_run` (or written verbatim in TUI
  passthrough); only escape-sequence bytes are still processed per byte. This
  retires the former per-byte `feed_byte_with_overflow`. Measured end-to-end,
  `tayf` streaming time dropped ~54% (prose), ~27% (log), ~31% (ansi) versus
  0.8.1; the in-process pipeline micro-bench rose +221% / +35% / +92%. Output is
  byte-for-byte identical, pinned by a `feed_data_run` oracle against the former
  per-byte path plus chunk-boundary-invariance tests. `cat`'s `<20%` overhead
  target (spec §7) is still not met for a byte-transforming wrapper — see
  `benches/BASELINE.md` for the full disposition.
- One rule-set snapshot is now loaded per line instead of two, removing a
  redundant atomic load and a latent inconsistency where a reload landing
  mid-line could apply different snapshots to the skip gate and the styling.

### Changed
- `memchr` is now a direct dependency (it was already in the tree transitively
  via `regex`, so no new code is added) and backs the escape-delimiter and
  newline byte-scans (+11.5% prose / +9.7% ansi on the pipeline micro-bench).

## [0.8.1] - 2026-05-30

Profile-first performance cycle: measure where the v0.8.0-measured overhead
actually lives, then take the highest-ROI low-risk optimization. The hot-path
modules `src/pty.rs` / `src/runtime.rs` / `src/pipeline.rs` were NOT modified;
the only production change is in `src/line_buffer.rs`.

### Added
- Attribution benchmarks. `benches/e2e_overhead.rs` gains a `tayf --bypass`
  column that splits pure I/O-loop overhead (bypass vs `cat`) from total
  pipeline cost (full vs bypass). New `benches/pipeline_feed.rs` (criterion)
  measures `Pipeline::feed` in-process to a `Vec` sink, splitting the
  pipeline-internal cost across the prose / log / ansi shapes. Numbers and
  analysis in `benches/BASELINE.md`. No new dependencies.

### Performance
- `LineBuffer::feed_byte_with_overflow` (the per-byte hot path) no longer
  delegates to the general `feed_with_overflow`, which re-scanned the whole
  accumulated buffer for a newline on every byte (O(L²) across a line). It now
  pushes the byte and compares it to `\n` — O(1) — using the invariant that the
  buffer never holds an interior newline (guarded by a `debug_assert`). Measured
  end-to-end: tayf streaming time dropped ~28% (prose), ~18% (log), ~30% (ansi)
  versus the v0.8.1 Phase-1 baseline; the in-process pipeline micro-bench
  roughly halved on every shape. Behavior is byte-identical (two regression
  tests added). The general `feed_with_overflow` is unchanged.

### Notes
- Spec §7's "<20% overhead vs native `cat`" is still not met on bulk streams.
  The attribution shows the dominant remaining cost is the per-byte
  `AnsiSm::step` loop + single-byte line-buffer feed, which lives in the
  off-limits hot-path module `src/pipeline.rs`; reworking it to chunk-level is
  deferred to a future security-gated cycle.

## [0.8.0] - 2026-05-30

Measurement-first performance cycle. No `src/` changes — the hot-path
modules (`src/runtime.rs` / `src/pty.rs` / `src/pipeline.rs`) are measured
from the outside, not modified. Optimization is deferred to a data-driven
v0.8.1+ (behind a security-review gate if it touches the I/O loop).

### Added
- End-to-end PTY-vs-`cat` overhead benchmark (`benches/e2e_overhead.rs`,
  `cargo bench --bench e2e_overhead`): drives the release binary inside a
  real PTY and reports streaming-phase overhead % against the spec §7
  `<20%` target across three corpus shapes (prose / log / ansi). Reuses the
  existing `portable-pty` dependency — no new crates. A CI-covered smoke
  test guards the mechanism; there is no wall-clock perf gate (e2e timing is
  noisy).

### Performance
- First end-to-end measurement of the spec §7 "<20% overhead vs native
  `cat`" target (deferred since v0.1). **Result: the target is not met on
  sustained bulk streams** — with a memory-speed consumer, tayf streams at
  ~10–20 MiB/s versus cat's ~130–150 MiB/s. Even low-match input is ~8×, so
  the I/O loop, not only the regex scanner, is implicated. This is the
  pessimistic bound: a real terminal renders far slower than 150 MiB/s and
  gates both sides, so interactive and small-output latency are unaffected;
  the ceiling bites only on bulk output. Full numbers and analysis in
  `benches/BASELINE.md`. Optimization is deferred to v0.8.1+.

### Changed
- Anglicized the audit-corpus decision vocabulary: `check_karar_mandate` →
  `check_decision_mandate`, the `KALSIN` decision token → `KEEP`, and the
  related corpus headers / README (rename-only; behavior unchanged).

### Documentation
- Corrected Turkish leaks in the released `[0.7.0]` entry (`paralel` →
  `parallel`, `DOKUNULMAZ` → `Off-limits hot-path modules`).

## [0.7.1] - 2026-05-29

Pattern audit follow-up hotfix — closes the four built-in items v0.7.0
flagged but did not fix. The `regex` crate has no look-around (the
linear-time ReDoS guarantee in CLAUDE.md §3), so three of the four fixes
v0.7.0 proposed are not implementable; only C-8 has a clean data fix. The
other three retain their built-ins (valuable common case; the audit corpus
is an adversarial stress-test, not frequency-weighted) and become documented
limitations. The hot-path modules `src/pty.rs` / `src/runtime.rs` / `src/pipeline.rs`
were untouched; `src/rules.rs` lost one extension entry.

### Fixed
- `pkg.go.dev/foo` is no longer mis-styled as the filename `pkg.go`: `go`
  was dropped from the `filename` extension catalog (audit C-8, FP
  10% → 0%). Go source files (`main.go`) now style as `fqdn` (blue)
  rather than `filename` (bright cyan).

### Changed
- New `ACCEPT-DOCUMENTED` decision value in the audit-corpus harness for
  high-FP built-ins with no clean fix under the linear-time regex engine.
  `check_decision_mandate` enforces it is used only at >5% FP. Items C-4
  (filename single-letter prose, 33%), C-9 (fqdn JWT, 60%), and E-1
  (ipv4 5-segment version, 12.5%) are reclassified from `TIGHTEN` to
  `ACCEPT-DOCUMENTED`.
- README gains a `## Known limitations` section documenting the three
  collisions above plus a recipe to disable a noisy built-in
  (`enabled = false`).

### Documentation
- Corrected the `[0.7.0]` "Known issues" fix paths (see below): the
  look-around fixes they proposed do not compile under the `regex` crate.

## [0.7.0] - 2026-05-29

Minor bump bundling five engineering-quality items that had been queued as
`v0.7+` forward-pointers from prior cycles. v0.4.0-class scope (~1100 LOC src
+ ~3300 LOC tests/corpora/snapshots) with full ceremony — parallel opus 4.7
spec review (Rust idiom + tayf-architecture lenses, 8 CRITICAL + 13
IMPORTANT + 11 NIT absorbed into spec rev2) and final cross-cutting opus
4.7 review (1 CRITICAL + 5 IMPORTANT + 3 NIT absorbed). Zero new
dependencies. Off-limits hot-path modules `src/pty.rs` and `src/runtime.rs` untouched;
`src/rules.rs` and `src/pipeline.rs` gained only `pub(crate)` test shims +
`Compiled::names` plumbing for the audit-corpus harness.

### Added
- Per-element merge of `[[rules]]` array-of-tables: edits to individual
  named rules now produce field-level conflicts in the Config TUI's save
  modal instead of v0.6.2's whole-array conflicts. Identity is the `name`
  string field — required by `UserRule` (config.rs:93-94). Rules without
  a `name` field fall back to the prior whole-array conflict (preserves
  v0.6.2 behavior for malformed configs). Convergent inserts that diverge
  in order trip an explicit order-divergence guard (RegexSet first-match-
  wins order is semantically meaningful). An absent AoT key on one side
  is treated as an empty array, so deleting the entire `[[rules]]`
  section against the other sides' modifications surfaces as element-
  level conflicts rather than a single whole-key conflict.
- `WriteToPathError::AotElementMissing { path, element_name }` typed
  error variant (pin format-string assertion in the merge tests). The
  apply-layer in `events::build_final_doc` translates this error (and
  `MissingIntermediate`) to a remove when the conflict is a delete-modify
  case, so the user's Ours/Theirs pick does what they intuit.
- LCS-DP line diff in the save-diff modal: replaces the v0.5.4 `HashSet`
  implementation that hid duplicate-line edits ("a\na\nb\n" → "a\nb\n"
  showed "(no changes)"). Hunt-McIlroy algorithm with strictly-greater
  dp neighbour preference + Add-on-tie convention in the backward walk;
  after reverse this yields the canonical `diff -u` Remove-before-Add
  forward order. Flat `Vec<u16>` cell layout (cache-friendly, single
  alloc; `u16` bound proven `<= floor(sqrt(MAX_DP_CELLS)) = 316`).
  `MAX_DP_CELLS = 100 000` defensive cap with literal removal+addition
  fallback for pathological sizes.
- Render snapshot tests for the Config TUI: 13 plain-text goldens under
  `src/config_tui/snapshots/`. Helper at `src/config_tui/test_support.rs`
  uses ratatui 0.30 TestBackend + plain-text buffer stringify; mismatch
  panics with an LCS diff (dogfoods the new line-diff). `UPDATE_SNAPSHOTS
  =1` regenerates locally only — refused under `CI=true`.
  `.gitattributes` enforces LF eol on `.snap` files. Coverage: 3 tab
  inits (Themes/Rules/Profiles) + 4 modals (Edit / ConflictList /
  SaveDiff Clean / Help) + 3 NewPattern wizard phases + 2 EditRegex
  states (valid + error) + 1 ColorPicker.
- Adversarial-corpus regression harness in `tests/audit_corpus/` for
  built-in pattern false-positive / false-negative tracking across seven
  audit-flagged items (C-4 filename single-letter ext, C-8 filename ↔
  fqdn Go pkg paths, C-9 fqdn JWT, D-7 log_level delimiters, E-1/E-2
  semver vs ipv4, F-3 duration μs, F-4 URL trim across schemes). Each
  corpus declares Measurement mode (PIPELINE for decision measurement, RULE
  for debugging); FP/FN measured via `tayf::__test_api::pipeline_spans`
  (full production pipeline — priority sort + overlap suppression +
  profile gating, per audit doc §0.2). `check_decision_mandate` machine-
  enforces memory `feedback_builtin_pattern_bar`: > 5% FP rate forbids
  KEEP — TIGHTEN or DEMOTE is required.
- `tayf::__test_api::{match_named_rule, pipeline_spans}` — two
  `#[doc(hidden)] pub fn` extensions on the existing test-only module.
  `match_named_rule` returns the leftmost match span for a single rule
  in isolation (no priority, no overlap, no profile). `pipeline_spans`
  returns the post-priority post-overlap `(rule_name, matched_span)`
  list — the production view. No stability guarantees.

### Changed
- `KeyConflict::is_array_block` semantics narrow to flag only fallback
  paths (no name identity, same-side duplicate name, or order
  divergence). Per-element AoT conflicts carry deeper paths and the
  prior flag value would be misleading. Internal SemVer impact only
  (the field is `pub(crate)` per v0.6.3 demote).
- `widgets/conflict_list.rs:55` suffix string changed from
  `"⚠ array merge v0.7+"` to `"⚠ array-shape conflict (no name
  identity)"` to reflect the v0.7 fallback semantic. The corresponding
  integration pin in `tests/config_tui_conflict_list.rs` was renamed
  and assertion-flipped per memory `feedback_collision_pin_pattern`.

### Fixed
- Stale `v0.7+` forward-pointer comments removed (one DELETE at
  `save.rs:651` — per-key conflict UI shipped in v0.6.2) or refreshed
  to `v0.8+ on community demand` (four sites covering capture-group
  TUI wires, Themes/Profiles `resolve_selected_rule_id` extension,
  Embedded/disk profile catalog resolution, sample set incremental).
  These map to spec §1.3 DEFER items and are not in v0.7 scope.

### Known issues

The audit-corpus harness measured higher than the 5% FP threshold on
four items. The current behavior is documented and pin-regressed by the
corpus assertions (so silent drift cannot occur), but the actual
pattern fixes are deferred to a v0.7.1 hotfix cycle:

- **C-4 (filename single-letter extension prose collision):** 33.3% FP
  (5/15 NEG inputs misfire). Fix path: drop `a`, `o`, `r`, `v`, `m`
  from `FILENAME_EXTENSIONS` and re-anchor those single-letter exts to
  require a path separator. The five POS cases (`libfoo.a`, `out.o`,
  `run.r`, `top.v`, `class.m`) become FNs without that anchor; corpus
  redesign required.
- **C-8 (filename ↔ fqdn Go pkg path):** 10% FP (1/10 NEG inputs).
  `pkg.go.dev/foo` matches `pkg.go` because `go` is in
  `FILENAME_EXTENSIONS`. Fix: drop `go` from the extension list, OR
  add a path-separator left-anchor to the filename regex.
- **C-9 (fqdn JWT 3-segment):** 60% FP (6/10 NEG inputs). The fqdn
  regex `\b(?:label\.)+[A-Za-z]{2,24}\b` fires on base64url JWT
  segments. No clean pattern fix without a known-TLD allowlist
  (4000+ entries — maintenance burden). v0.7.1 will surface this as
  a documented limitation + ship a user-config recipe README entry.
- **E-1/E-2 (semver vs ipv4 fifth-octet prose):** 12.5% FP (1/8 NEG).
  `1.2.3.4.5 long` matches as the `1.2.3.4` prefix because `\b` is
  satisfied between digit `4` and dot `.`. Fix: negative lookahead
  `(?!\.\d)` appended to the ipv4 pattern.

The corpus `EXPECTED_FP_*` constants lock the current numbers so any
unintended drift surfaces immediately. The v0.7.1 pattern fixes will
re-measure and update the constants in lockstep.

> **Correction (v0.7.1):** the fix paths above are partly wrong. The `regex`
> crate has no look-around, so the `(?!\.\d)` (E-1) and path-anchor lookbehind
> (C-4) fixes do not compile, and C-4's "drop `a o r v m`" would not reach
> <5% (the offenders are `.c` ×4 + `.v` ×1, and `c` is retained). v0.7.1 fixes
> C-8 (drops `go`) and reclassifies C-4 / C-9 / E-1 as `ACCEPT-DOCUMENTED`
> documented limitations — see the `[0.7.1]` entry and README "Known limitations".

- Pre-existing local PTY flake (OSC 11 bg-detect query leak in
  `integration_ansi` / `integration_signals` / `integration_themes`)
  carried forward from v0.6.x — local-only, CI green at ship time.

## [0.6.3] - 2026-05-29

Pure cleanup cycle — closes all three IMPORTANT findings (I1 / I2 / I3)
and three NITs (a / b / c) from the v0.6.2 cross-cutting review. No
behavior changes outside the I3 fix; the rest is dead-code removal,
public-API tightening, and test-coverage backfill.

### Changed
- `config_tui::merge` demoted to `pub(crate)` (I2). The prior `pub mod
  merge` re-exported `toml_edit::DocumentMut` through the public
  surface — locking tayf's SemVer to `toml_edit`'s major-version
  cadence (memory `feedback_toml_edit_025_quirks` documents that we
  ride 0.25-specific behaviors). The 12-test integration suite
  (`tests/config_tui_merge_3way.rs`) was moved inline as
  `#[cfg(test)] mod tests` in `src/config_tui/merge.rs`. Two pure-data
  types (`ConflictValueShape`, `KeyConflict` — no `toml_edit` types in
  their fields) are re-exported through the `#[doc(hidden)]`
  `__test_api` namespace so the conflict-list render test suite can
  keep fabricating fixtures.

### Fixed
- `ConflictChoice::Skip` on a `Block`-shape conflict with `base = absent`
  no longer surfaces the misleading `"merge apply failed: write_to_path
  at <key>: missing intermediate at <key>"` toast (I3). The default
  Block-shape selection is `Skip`, so this was the most-likely user
  path for `[[rules]]` array-of-tables conflicts. New `path_exists`
  predicate in `merge.rs` short-circuits the write when base also lacks
  the path; `auto_merged` already carries no value at conflicting keys
  by construction, so the no-op is correct.
- The per-row pick → `final_doc` walk extracted from
  `apply_conflict_selections` into a pure `build_final_doc` helper —
  no IO, no app-state mutation, directly unit-testable.

### Removed
- `SaveDiffState::ConflictDiscardConfirm` variant + cascade-dead
  `SaveDiffOutcome::DiscardAndReload(_)` outcome +
  `MergePending.disk_now` field + `apply_save_diff_outcome`'s
  `DiscardAndReload` arm (NIT a). Zero producers in the dispatcher
  since v0.6.2; deletion makes the conflict-list state machine match
  what actually runs.
- Stale `#![allow(dead_code)]` at `src/config_tui/save.rs:14-18` whose
  reason claimed `ts_for_backup_filename` was unreachable — G8 (v0.6.2)
  wired it into `commit_bytes`, the canonical save path (I1). Strip
  surfaced no genuinely-dead items; clippy stays clean.

### Tests
- 7 new tests in `src/config_tui/merge.rs` and `src/config_tui/events.rs`:
  `merge_three_way_convergent_deletion_removes_key` (NIT b regression
  guard), `path_exists_traverses_existing_segments_and_returns_false_on_
  first_miss`, `build_final_doc_skip_on_absent_base_leaves_auto_merged_
  untouched_at_that_key` (I3 fix guard),
  `build_final_doc_skip_on_present_base_copies_base_value_to_final_doc`,
  `enter_on_conflict_list_modal_invokes_apply_conflict_selections_and_
  succeeds`, `o_and_t_keystrokes_on_block_shape_row_emit_warn_toast_and_
  preserve_skip_selection`, and `j_and_k_navigation_wraps_focused_row_
  modulo_conflict_count` (NIT c dispatcher coverage).

## [0.6.2] - 2026-05-29

### Added
- Config TUI ColorPicker `bold` / `italic` / `underline` bool-axis row with
  tri-state edit (Unset → On → Off → Unset) and `c` clear to wipe the
  whole color block. Plumbed through `NewStyle.{bold,italic,underline}:
  Option<Option<bool>>` end-to-end (G3 — Item 1, spec §3.1).
- Patterns tab user-rule render union: built-in and user-config rules now
  appear under two DIM section headers (`── Builtin ──` / `── User ──`).
  Search filter applies symmetrically to both sections. `resolve_selected_
  rule_id` returns `RuleId::Builtin` or `RuleId::UserConfig` based on
  which section the focus falls in (G5 — Item 2, spec §3.2).
- 4-variant rule deletion in `compile_pending`: Builtin / UserConfig /
  Embedded / DiskProfile all routable through the canonical
  `apply_user_rules_with_source` path. ConfirmAction payloads widened to
  carry `RuleId` instead of `String` (G4 — Item 3, spec §3.3).
- `'o'` profile / theme override copy: copies the selected embedded
  source to `~/.config/tayf/{profiles,themes}/<name>.toml` so the user
  can edit it on disk. Already-on-disk → toast skip (path-explicit
  wording); symlink dest or parent canonicalizing outside the tayf root
  → toast reject (CLAUDE.md §3 mandate). Post-copy snapshot reload
  refreshes the catalog (G6 — Item 4, spec §3.4 + §3.10).
- `save::commit_bytes(snapshot, body, now)` helper: the 8-step atomic-
  write ceremony (rotate backups → preserved_mode → backup → tmpfile →
  sync_all → atomic rename → parent sync → reparse) is now a single
  shared helper, called from both the Clean Confirm path and the
  merge-resolve path (G8 — Item 6 §3.0 invariant share, memory
  `feedback_parallel_call_site_invariant_audit`).
- AST-level 3-way merge module `config_tui::merge`: pure
  `merge_three_way(base, ours, theirs) → MergeResult { auto_merged,
  conflicts }` recursive walk over `toml_edit::DocumentMut` documents.
  Auto-merges disjoint / convergent / one-sided changes; per-key
  `KeyConflict { path, base_value, ours_value, theirs_value, shape,
  is_array_block }` for the rest (G7A — Item 5, spec §3.5).
- Per-key conflict resolution UI: `Modal::ConflictList` renders a
  single-screen list (`▶` focus marker + `[O]`/`[T]`/`[S]` choice
  marker + truncated ours/theirs preview), driven by `merge_three_way`.
  `j`/`k` nav; `o`/`t`/`s` toggle the focused row (Block-shape rows
  reject `o`/`t` with a pinned toast); Enter bulk-applies via
  `commit_bytes` and routes through the same `request_snapshot_reload`
  + `pending_save_and_quit` invariants as Clean Confirm (G7B+G8 —
  Item 6, spec §3.6).
- Save-quit single-step `s` in the `QuitWithUnsavedEdits` modal:
  `pending_save_and_quit` flag tracked across the SaveDiff /
  ConflictList round-trip; every non-commit exit clears it (T-I6
  invariant) (G2 — Item 8, spec §3.8).

### Changed
- `ConfirmAction::DeleteUserRule(String)` → `DeleteRule(RuleId)`;
  `ResetUserOverride(String)` → `ResetOverride(RuleId)`. 15 callsite
  rename across `lib.rs`, `events.rs`, `app.rs`, `tabs/patterns.rs`,
  `tests/integration_tui_delete_alias.rs`.
- `NewStyle.{bold,italic,underline}` widened from `Option<bool>` to
  `Option<Option<bool>>` so the ColorPicker can stage the tri-state
  intent (None = unset, Some(None) = explicitly cleared, Some(Some(b))
  = on/off).
- `SaveDiffState::ConflictPending` and `ConflictMergedPreview`
  RETIRED — replaced by `SaveDiffState::MergePending` which carries
  the merge inputs + per-row selection + focused row, and is driven
  by `Modal::ConflictList` rather than the SaveDiff modal.
- `commit_save` reduced to: reconcile → `commit_bytes`. Reconcile is
  now a pre-flight; failing reconcile no longer leaves an orphan
  backup (test renamed + rewritten).

### Security
- `'o'` override-copy refuses when the destination is a symbolic link
  or its canonical parent resolves outside `~/.config/tayf/` per
  CLAUDE.md §3 mandate. Outside-target files remain byte-identical
  through the rejection path.

### Limitations (v0.7+)
- `[[rules]]` array-of-tables: fine-grained per-index merge not yet
  implemented. v0.6.2 surfaces whole-array changes as a single
  Block-shaped conflict (`_v0_6_2_limitation` test suffix pins).
- ColorPicker `dim` axis: CLI flag only (TUI surface lands later).
- ConflictList Block-shape expand-modal: forced Skip default + toast
  on `o`/`t`; full inline edit lands in v0.7+.
- Wide-char regex column-correct truncation: current `chars().take(N)`
  truncation may misalign columns for CJK content.

## [0.6.1] - 2026-05-28

### Added
- **Builtin / Embedded / DiskProfile rule overlay routing** in
  `compile_pending`. ColorPicker now updates the live preview for ALL
  rule sources, not just user-config (v0.6 limitation closed). Strategy
  is dedupe-then-mutate-or-push: if `user_rules` already contains an
  entry with the same name (e.g., snapshot's `[[rules]]` override of a
  builtin), mutate in place; otherwise push a synth `UserRule` that the
  canonical `apply_user_rules_with_source` (`src/rules.rs` zero touch)
  will name-match against builtins + theme + profile.append_rules and
  apply in place there. (Group A — 8e91bc0.)
- **`Ctrl+R` keystroke:** reload config from disk. With pending edits,
  opens a Discard-and-Reload confirm modal; with a clean state,
  reloads directly. (Group B — 828b030.)
- **`Shift+D` keystroke:** write the built-in default config to a
  missing bound path. Opens an InitFromDump confirm modal first; warns
  if the config file already exists. (Group B.)
- **`V` keystroke:** alias for `Shift+P` (full preview overlay).
  (Group B.)
- **`Delete` keystroke:** alias for `d` in the Patterns tab — both
  open the same DeleteUserRule confirm modal. (Group C — cc9826f.)
- **Search filter (`/`)** now actually hides non-matching entries on
  Patterns / Themes / Profiles tabs (was stored but not rendered in
  v0.6). Filter scope is builtin name catalogs (user-rule render in
  Patterns tab is deferred to v0.7). (Group D — db904bf.)
- **Save-diff modal scrolling:** Up/Down advance one line;
  PageUp/PageDown advance by `PAGE_STEP = 10`; Home/End jump to top /
  effective end (ratatui's `Paragraph::scroll` clamps the over-scroll
  at content-end). (Group D.)
- **New module:** `src/config_tui/search.rs` exposes a generic
  `filter_names_lowercase` helper used by all three list tabs.
  (Group D.)

### Fixed
- **`apply_confirm` for `DiscardEditsAndReload` and `InitFromDump`**
  no longer prints the "lands in v0.5.5+" placeholder toast — both
  flows are fully implemented. (Group B.)
- **`tests/common/tui_harness.rs::find_text`** now documents its
  ASCII-only byte-offset assumption (multi-byte UTF-8 cells split
  across columns; non-ASCII assertion paths must compare
  `frame.buffer.content[i].symbol` directly). (Group E — 1b55d62.)
- **v0.6 spec §13 disposition count drift** (`9+17+14=40 fold+4
  drop=44` → actual `9+18+14=37 fold+4 drop=41`). (Group E.)
- **Stale `(v0.6+)` forward-pointer comments** refreshed across
  `src/config_tui/`: sites implemented in v0.6.1 lose the gate;
  sites still deferred forward-point to `(v0.7+)`. (Group E.)

### Internal
- `src/rules.rs` **zero touch** — the overlay route reuses the
  canonical `apply_user_rules_with_source` mechanism (single source of
  truth, memory `feedback_parallel_call_site_invariant_audit`).
- `src/{pty,io_loop,tty_guard,signals,runtime,pipeline}.rs` zero
  touch.
- 21 new tests across the cycle: 7 Group A lib (overlay matrix), 1
  Group B lib (reload helper), 4 Group B integration
  (`tests/integration_tui_apply_confirm.rs`), 1 Group C integration
  (`tests/integration_tui_delete_alias.rs`), 3 Group D lib
  (`search.rs::tests`), 5 Group D integration
  (`tests/integration_tui_polish.rs`). Final lib test count: 700
  (was 679 at v0.6.0).
- Ceremony: LEAN per memory `feedback_lean_process_small_subversions`
  — single release with parallel spec review (Rust + TUI senior) +
  final cross-cutting opus 4.7 review on the full v0.6.0..v0.6.1
  diff (mandate `feedback_cross_cutting_review_value` —
  not skipped in lean cycle).

### Deferred (v0.7+)
- **bool-axes-clear** `c` keystroke (item 9 from v0.6 §14.2): the
  ColorPicker axis-row UI does not ship in v0.6, and adding it was
  out of LEAN budget. Plan-phase review (TUI rev1 B1) explicitly
  deferred.
- **Patterns tab user-rule render** — union with builtins so a user's
  `[[rules]]` entries become visible in the list.
- `RuleId::Embedded` / `DiskProfile` deletion (only UserConfig
  deletion is wired in v0.6.1).
- `mark_edit_clear` debouncer helper (v0.6 §14.2 nice-to-have).
- `Modal::EditRegex` Esc-cancel debouncer pending state cleanup
  (v0.6 §14.3 #1).
- Save-quit single-step (events.rs:412 separate quit flag).

## [0.6.0] - 2026-05-28

### Added
- **Span-emitting preview pipeline.** New `apply_rules_spans` API in
  `src/pipeline.rs` yields a list of `StyleSpan` runs for TUI consumers
  alongside the existing byte-emit `apply_rules`. Both share a new
  `select_runs` helper so matching, overlap rejection, and priority
  resolution have a single source of truth — `apply_rules` byte output
  is byte-identical to v0.5.7 (golden parity tests).
- **Config TUI live preview.** Mini-preview strip and Shift+P full-screen
  overlay now render the user's sample input with actual rule styling
  (including capture-group substyling) via the new `PreviewState`.
  Initial compute runs on TUI boot; debounced recompile (200 ms quiescent
  window) on edits.
- **Config TUI editing core (spec §12.4).**
  - `c` on a selected rule opens the ColorPicker; Accept binds the chosen
    color to the rule via `PendingEdits` and triggers a preview recompile.
  - `n` opens a 3-phase new-pattern modal (Name → Regex → Style). Name
    requires `[A-Za-z0-9_-]+`; Regex is validated inline with
    `RegexBuilder::size_limit` (ReDoS guard, mirrors `rules.rs`'s
    1 MB limit). Style phase delegates to an inline ColorPicker.
  - `e` opens an inline regex-source editor for the selected rule.
    Buffer initialized from the rule's current pattern; debouncer-driven
    preview recompile while typing; Enter commits to
    `edits.rules[rule_id].pattern`; Esc cancels.
  - `?` / `F1` opens a read-only Help modal listing all keybindings; any
    key dismisses (vim/less convention — dismissing key discarded).
- `src/config_tui/compile_pending.rs`: rebuilds a `Compiled` from a
  `ConfigSnapshot` plus a `PendingEdits` delta, backing the live-preview
  recompile path.
- `src/rules.rs::compile_from_config` `pub(crate)` entry-point —
  additive (does not change `load_with_theme` semantics).
- `src/config_tui/style_ratatui.rs`: `Style::to_ratatui()` helper kept
  out of core `src/style.rs` to avoid a ratatui dependency in core.
- `Modal::NewPattern`, `Modal::EditRegex`, `Modal::Help` variants plus
  `PatternDraft` / `NewPatternPhase` modal-local state.
- TestBackend integration-test harness (`tests/common/tui_harness.rs`)
  with `boot_tui_with_sample` + key-driven assertions; new
  `tests/integration_tui_preview.rs`, `tests/integration_tui_editor.rs`,
  `tests/integration_tui_help_modal.rs` integration suites.

### Changed
- Stale forward-pointers stripped: 7 `// reason: ...` annotations removed
  or rewritten to reflect what landed in v0.6, plus 4 `Toast::warn` v0.6+
  stubs replaced with real implementations (D1 ColorPicker bind, D2
  NewPattern open, D3 EditRegex open, D4 Help open). KEEP set for
  v0.6.1 / v0.7 defers documented in spec §11.1/§11.2.

### Fixed
- Config TUI live preview no longer shows raw sample text; renders
  colorized output matching the runtime byte-emit path.

Runtime behavior change to existing rule application: **none**. The
`apply_rules` byte-emit path is byte-identical to v0.5.7 via golden parity
tests in `src/pipeline.rs` (see the rev2 spec §3 / §4 parity contract).

## [0.5.7] - 2026-05-28

### Changed
- Documentation-only cleanup. `src/profiles.rs` ARN test section comment now
  reflects v0.5.6's ipv6 tighten implicitly resolving the previously-
  documented `arn:aws:s3:::my-bucket` interior-collision case (branch 2's
  new `[hex]{3,4}` prefix rejects the single-char `3:` shape). `src/pipeline.rs`
  pre-filter comment now describes the priority-sort tie-break contract
  instead of the obsolete "first-match-wins depends on pattern order"
  phrasing.

No code change; 636 lib tests unchanged.

## [0.5.6] - 2026-05-27

### Added
- **Per-rule overlap-resolution priority (`priority: i32`).** Tier convention:
  - `0` — built-in rules; user-config rules without explicit `priority`.
  - `100` — profile interior rules (instance_id, region, container_id, pod_name).
  - `200` — profile envelope rules (arn, image_tag).
  - Any i32 — user-config opt-in (`priority = N`, negative legal).

  Higher-priority rules accept their match span before lower-priority rules,
  with overlap detection bidirectional (interior or envelope overlap both
  block). Resolves AWS profile interior-collision issues where envelope
  rules (`arn`, `image_tag`) used to lose to interior built-ins (`uuid`,
  `ipv4`, `region`, `fqdn`).
- `UserRule::priority: Option<i32>` schema field (additive, backward-
  compatible). Themes cannot override priority (rejected with typed
  `ThemeRuleError::StraySchemaField`).
- `ProfileRule::priority: Option<i32>` schema field; envelope rules in
  `assets/profiles/aws.toml` (arn) and `assets/profiles/docker.toml`
  (image_tag) ship `priority = 200`.
- ipv6 pattern: dedicated `::1` loopback branch promoted to first; bare
  `::[hex]{1,4}` tightened to require additional hex groups. Negative
  regression for Rust module paths (`foo::bar::baz`, `std::io::Read`,
  `serde::de::Deserialize`).
- Test coverage for IPv4 invalid octets (256.0.0.0, leading-zero), MAC
  7-pair shape, log_level delimiter contexts ([ERROR], INFO:, WARN -,
  (CRITICAL)), μs Greek-letter duration, URL ssh:// / ftp:// / SCP scheme
  coverage.
- `FILENAME_EXTENSIONS` doc-comment with canonical 1-to-1 single-letter
  ext attribution (`a` archive, `c` C source, `h` C header, `m`
  Objective-C, `o` object, `r` R, `v` Verilog).

### Changed
- **AWS ARN now wins envelope styling over interior IPv4, UUID, region
  matches.** Profile pin tests `aws_arn_yields_to_interior_region_pattern_v0_5_3_limitation`
  and `docker_image_tag_registry_host_yields_to_fqdn_v0_5_3_limitation`
  renamed to `aws_arn_wins_over_interior_region_pattern` and
  `docker_image_tag_wins_over_registry_host_fqdn` with flipped assertions.
- **Docker `container_id` now wins over the built-in `uuid` rule** when
  a UUID contains a 12-hex container_id-shaped substring. Inside the
  docker profile, container_id (priority 100) iterates before uuid
  (priority 0); accepts its 12-hex span; uuid envelope sees overlap →
  suppressed. User-visible: a logged UUID with a 12-hex segment renders
  as container_id color (cyan) inside `--profile docker`, not uuid color
  (bright magenta). Outside the docker profile, uuid behavior is unchanged.
- `src/pipeline.rs` doc-comment block above `apply_rules` updated for the
  priority contract; "first-match-wins by pattern order" replaced with
  "highest-priority match wins; ties broken by pattern-definition order;
  overlap detection bidirectional".

### Removed
- `http_status` built-in rule. The pattern `(?:^|[\s/:])([1-5]\d{2})\b`
  matched any 3-digit number 100-599 prefixed by whitespace/`/`/`:`,
  producing false positives on VLAN IDs, port numbers, line numbers,
  AWS account-ID prefixes (`:111:...`), and unrelated 3-digit literals.

  **Migration recipe (preserves v0.5.5 behavior exactly):**
  ```toml
  [[rules]]
  name = "http_status"
  pattern = '(?:^|[\s/:])([1-5]\d{2})\b'
  style = { fg = "magenta", bold = true }
  ```

  **Improved migration (leading punct neutral, group-1 only):**
  ```toml
  [[rules]]
  name = "http_status"
  pattern = '(?:^|[\s/:])([1-5]\d{2})\b'
  style = {}
  styles = { "1" = { fg = "magenta", bold = true } }
  ```

  v0.6+ may ship an opt-in `http` profile bundling HTTP-domain rules.

### Fixed
- ipv6 third branch matched bare `::xxxx` Rust path syntax
  (`foo::bar::baz` → `::ba`); now requires additional hex groups.
- ipv4 negative regression coverage gap (256.0.0.0, 1.01.30.4, etc.).
- mac negative regression coverage gap (7-pair shape, IPv6-tie pin).

## [0.5.5] - 2026-05-27

### Added
- `tayf config` TUI now persists staged edits to disk (`build_new_content`
  toml_edit reconciliation). Theme, profile, and built-in override (`o`
  keystroke) selections are now serialized through `toml_edit::DocumentMut`,
  preserving comments, ordering, and formatting.
- New `Color::to_toml_str` canonical encoder (inverse of `parse_str`),
  variant-preserving and `parse_str`-roundtrip-stable.
- `SaveDiffState::ReconcileError` variant for inline error rendering in
  the SaveDiff modal (no more silent failures or transient toasts).

### Changed
- `build_new_content` is now a thin facade over `reconcile::apply_edits`;
  reconcile errors propagate as `io::Error::other("reconcile failed: ...")`
  through `commit_save`, surfaced inline in the SaveDiff modal preview.
- Module-level `#[allow(dead_code)]` reason annotations in
  `src/config_tui/{edit,snapshot,save}.rs` refreshed to point at
  v0.6+ forward work after reconciliation consumed previously-deferred
  variants.

### Fixed
- v0.5.4 cross-cutting review §1/§12 named single largest gap: TUI staged
  edits previously dropped at save time (pass-through `build_new_content`).
  v0.5.5 closes this gap; the user-visible "save" promise is now fulfilled.

### Notes
- Architectural collision fix (`aws.arn` ↔ `ipv6`, `docker.image_tag`
  ↔ `fqdn`) remains carved out to v0.5.6 (Approach A scope brainstorm
  decision 2026-05-27).
- ColorPicker → selected-rule binding remains v0.6+ (`c` keystroke still
  toasts "binding to selected rule lands in v0.6+"); reconcile.rs walk
  is ready for the wire when it lands.

## [0.5.4] - 2026-05-27

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
  to v0.5.3; no bench-CI baseline regen needed. Off-limits hot-path module list per
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
- **Span-emitting preview pipeline** (§5.4 off-limits hot-path blocker on
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
