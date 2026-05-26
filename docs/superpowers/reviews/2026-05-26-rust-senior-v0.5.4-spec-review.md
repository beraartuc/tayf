# v0.5.4 spec — Rust senior review

**Reviewer:** Opus 4.7 (rust-senior persona)
**Date:** 2026-05-26
**Spec:** `docs/superpowers/specs/2026-05-26-tayf-v0.5.4-config-tui.md`
**Base:** v0.5.3 (`9ab9659`)

## Verdict
**NEEDS_REVISION**

Three load-bearing issues must be resolved before the plan is written:
(1) the `ratatui` feature selection in §11.1 will not compile the code §5.2/§7.3 describe;
(2) `toml_edit = "0.23"` is three minors behind current (`0.25.x`) — this violates the memory `Dependency minimalism + freshness` mandate ("always use latest published versions") and was not justified;
(3) `Args::try_parse_from_env` is a `pub` API and §4.1's `Args` → `RunArgs` rename is a **public-API break** that the spec claims is non-breaking. None individually fatal, but each forces concrete spec text revision rather than plan-time discovery.

The architectural shape (TUI on its own code path, hot path untouched, atomic-write + backup, ratatui dep choice) is sound. Several smaller items below.

---

## §1. Critical findings (🔴)

### 🔴 C-1 — `ratatui` 0.30 feature gate as specified will not build the spec's render code (§11.1 vs §5.2/§7.3)

`§11.1` declares:

```toml
ratatui = { version = "0.30", default-features = false, features = ["crossterm"] }
```

Verified against `ratatui` 0.30.0 `Cargo.toml` (upstream):

```
default = ["crossterm", "underline-color", "all-widgets", "macros", "layout-cache"]
crossterm = ["std", "dep:ratatui-crossterm"]
```

Setting `default-features = false, features = ["crossterm"]` drops **all of**: `underline-color`, `all-widgets`, `macros`, `layout-cache`. Critically, `all-widgets` enables the `ratatui-widgets` crate. Without it, **`Paragraph`, `List`, `Block`, `Tabs`, `Clear`, `Layout` constraint primitives, and every widget the spec uses are not in the crate at all** (they live in `ratatui-widgets`, which `crossterm` does not pull in).

The spec's `render::frame` (§7.3) uses `Layout::vertical([Constraint::Length(1), Constraint::Min(10), ...])`, the modal stack uses `Clear`, `widgets/preview.rs` uses `Paragraph + Style spans`, `widgets/save_diff.rs` renders a unified diff (`Paragraph`/`List`), and `widgets/color_picker.rs` paints a swatch grid. None of these compile under the proposed feature set.

**Additional cost driver:** turning off `layout-cache` is a real perf hit on a re-rendered TUI — `Layout::vertical(...).split(area)` is solved by `kasuari` (cassowary) every frame; the cache is what amortizes it.

**Recommended fix.** Use a positive minimum-features set, NOT `default-features = false`:

```toml
# Recommended:
ratatui = { version = "0.30", default-features = false, features = [
    "crossterm",       # backend
    "underline-color", # cheap, future-proof
    "layout-cache",    # measurable perf — every frame re-solves layout otherwise
    "macros",          # ergonomic line!/text!/span! — used by render code
    "all-widgets",     # Paragraph/List/Block/Tabs/Clear/etc.
] }
```

Or simply enable default features and explicitly opt out of unneeded backends (`termion`, `termwiz` are not in default — already off). The "minimize features" optic was correct intent; the chosen subset is wrong.

Also: `§11.2` table says `"core + widgets + crossterm sub-crates (v0.30 split mimari)"` — this is the **transitive surface description**, but the spec's actual feature line excludes widgets. The table and the Cargo line contradict each other.

---

### 🔴 C-2 — `toml_edit = "0.23"` is stale; memory mandates "always use latest" (§11.1)

Current latest on crates.io: **`0.25.11`** (verified `cargo search toml_edit`).

Memory `Dependency minimalism + freshness` (`feedback_dependency_minimalism.md`): *"always use latest published versions"*. The spec pins `"0.23"` with no audit-deviation justification. Memory mandate was even listed in §3.3 as "consumed" — silent omission is the v0.4.0 failure mode the memory was written to prevent.

`toml_edit` 0.23 → 0.25 has had real bug fixes including round-trip preservation edge cases (which is the **entire reason** this spec adopts it). Two of v0.5.4's load-bearing claims — "comments + ordering preserved" and "conflict-aware diff" — depend on those round-trip correctness fixes.

There is a second `toml_edit`-related concern: §6.1 declares `pub(crate) doc: toml_edit::DocumentMut`, but **the live tree on this repo already pulls a `winnow 0.7` + `winnow 1.0` duplication** through `toml 0.9` → `toml_parser` (see `src/lib.rs:47` `clippy::multiple_crate_versions` allow with that exact justification). `toml_edit 0.25` is on `winnow 1.x`; `toml_edit 0.23` is on `winnow 0.7`. Pinning the older one likely re-anchors winnow 0.7 transitively — wrong direction for the existing tech-debt note in lib.rs:47.

**Recommended fix.**
1. Bump spec to `toml_edit = "0.25"` (latest minor); record `cargo geiger`/`cargo deny check` against that version specifically.
2. Verify whether the lib.rs:47 `clippy::multiple_crate_versions` `// reason: …` justification can be **shortened** (one fewer winnow dup) or whether it stays unchanged — record the result in v0.5.4 spec §11.
3. If 0.25 brings any new direct transitive dep beyond what 0.23 would, surface it in §11.2 audit table.

---

### 🔴 C-3 — `Args` rename is a public-API break, contradicts §4.3's "byte-identical" claim (§4.1, §5.3)

`src/lib.rs:72` currently has `pub use cli::Args;`. `Args::try_parse_from_env()` is `pub` on that struct, and the field path is `args.shell`, `args.theme`, etc. — externally observable via:

- `cargo public-api` diff (mentioned in §5.3 as expected to be diff-clean — it will NOT be).
- Any downstream consumer (no known consumers today, but tayf is "open-source from day one" per CLAUDE.md §1; the contract is the surface, not the user count).
- The `Tayf::run(args: Args)` signature stays `Args` but `Args::shell` becomes `Args::run.shell` — silent field-path break for any pattern-match or struct-update syntax.

§4.3 declares "Mevcut tüm `src/cli.rs::tests` test'leri compile + pass (sadece `Args` field path'i `args.shell` → `args.run.shell` shape'ine güncellenir)" — this is field-path migration *in our own tests*. That's the leak. If our own tests need migrating, downstream consumers' code does too. Per CLAUDE.md §4: "Public API stability: Any public item (anything `pub` outside `pub(crate)`) is contract. Breaking changes require a major version bump and CHANGELOG entry."

This isn't a reason to block — it's a reason to **document and own the break** correctly:

**Recommended fix.** Either:

- (a) **Honest break**: Update §4.3 + §5.3 to declare a CHANGELOG `### Changed (breaking)` entry, and acknowledge `cargo public-api` will report the field-path move. Add a migration note. Defensible because we're pre-1.0 and tayf-as-library use is `Tayf::run` only.
- (b) **Backward-compat shim**: Keep `Args` fields at the root via `#[derive(Deref)]`-style or accessor methods (`impl Args { pub fn shell(&self) -> &Option<PathBuf> { &self.run.shell } }`). Costs ~30 LOC of trivial accessors; preserves source-level compat. Not worth it pre-1.0 IMO, but should be the **explicit alternative** considered in spec.

Pick (a) and update spec language to match.

---

## §2. Important findings (🟡)

### 🟡 I-1 — Atomic write sequence omits a load-bearing fsync (§8.1 step 4-6)

§8.1's commit flow:

```
4. tmp.sync_data()?                  // fsync tmpfile content
5. fs::rename(&tmp_path, &cfg_path)? // atomic on POSIX
6. dir.sync_all()                    // fsync parent dir
```

Two problems:

**(a) `sync_data` vs `sync_all` on the tmpfile.** `sync_data` (fdatasync) does not flush file metadata, which on some filesystems (notably ext4 with `data=writeback`) is required before the rename to guarantee size is correct after crash-replay. The canonical sequence is `sync_all` on the tmpfile (fsync, not fdatasync), THEN rename, THEN sync_all on the parent directory. Cite: SQLite atomic commit docs + ext4 atomic-rename folklore. Use `sync_all`.

**(b) EXDEV not addressed.** `fs::rename` returns `EXDEV` if `tmp_path` and `cfg_path` are on different filesystems. The tmpfile naming chosen (`format!("{}.tayf-tmp-{}-{}", cfg_path, pid, ms)`) is **same-directory**, which makes EXDEV near-impossible — but bind mounts, overlayfs, and `~/.config` on an NFS automount can still surprise. The spec should pin this invariant explicitly: tmpfile MUST be in the same directory as the target (which it already is given the naming) — add a code comment + a test asserting `tmp_path.parent() == cfg_path.parent()`.

**(c) `sync_all()` on the directory — macOS APFS behavior is unspecified.** Linux ext4/xfs definitely sync the directory entry; APFS's behavior here is famously underdocumented. Best-effort treatment is correct (§8.2 says "logged via TAYF_LOG=warn; save sayılır"). Recommended: keep the call but document the macOS nuance in the function doc-comment.

**Recommended fix.**
- Change `tmp.sync_data()?` to `tmp.sync_all()?` in §8.1.
- Add explicit "tmpfile MUST be in target's parent dir" invariant + test.
- Add macOS APFS doc-comment caveat on the dir sync_all step.

---

### 🟡 I-2 — `OpenOptions::create_new(true)` race window + tmpfile cleanup on panic (§8.1 step 4, §8.2)

`OpenOptions::new().write(true).create_new(true).open(&tmp_path)` maps to `open(O_WRONLY | O_CREAT | O_EXCL)` — atomic on POSIX. Good. But two gaps:

**(a) No `mode` set.** Default Rust `OpenOptions` creates the file with `0o666 & !umask`, which on a typical user umask 022 yields `0o644`. The destination file may already have stricter perms (e.g., user-set `0o600`); after rename, the file inherits the **new** inode's mode, silently relaxing perms. The fix: `OpenOptions::new().mode(perm_of(cfg_path).unwrap_or(0o600)).custom_flags(libc::O_EXCL).create_new(true).open(tmp_path)` — i.e., snapshot the target's mode before write, apply to tmpfile. Important for users who deliberately set `chmod 600 config.toml`.

**(b) Panic between create_new and rename leaks the tmpfile.** No `Drop` guard on the tmpfile path. If anything between step 4 and step 5 panics, `<cfg>.tayf-tmp-<pid>-<ms>` sits in the config dir forever. The spec already uses `tempfile = "3.27"` as a dev-dep; promoting it to a prod dep and using `tempfile::NamedTempFile::persist()` solves both this leak AND simplifies the cleanup path on every error branch in §8.2.

**Recommended fix.** Either:
- (i) Add `tempfile` as a direct prod dep and rewrite §8.1 step 4-5 using `NamedTempFile::new_in(cfg_dir)?` + `persist(&cfg_path)?`. This is the cleanest answer; `tempfile` is already trusted (dev-dep) and battle-tested for this exact pattern.
- (ii) Roll our own `struct TmpFileGuard { path: PathBuf }` with `Drop` that unlinks on drop unless `forget()` called before rename. ~30 LOC. Avoids the new prod dep but reinvents the wheel.

Plus: explicit mode preservation per (a).

---

### 🟡 I-3 — Backup rotation `read_dir` failure path unspecified (§8.3)

§8.3 says "Best-effort — `unlink` fail → `TAYF_LOG=warn` log, sav proceed eder." But what about `read_dir` itself failing? Permission flap, ENFILE, or transient filesystem issue. If we cannot enumerate, we cannot rotate, and unbounded backup growth is a real failure mode for users who edit-save tens of times per session.

Also: ordering of backup-write vs rotation is wrong-direction. §8.1 step 1 writes the new backup, then step 2 rotates to keep "latest 5". Means we briefly have 6 backups on disk; if the user is at quota, the backup write succeeds, rotation fails, we keep 6 forever (next save makes 7, etc.). Order should be: read_dir + plan rotation → write new backup → unlink eldest. Or: rotate-to-(N-1) first, then write the Nth.

**Recommended fix.**
- Specify the `read_dir` failure path: `Toast::warn("Backup rotation skipped: <reason>")`, save still proceeds (write is independent), backup count may exceed 5.
- Reorder: rotate first (to 4), then write new (5th) backup.
- Add a unit test: 5 existing backups → save → still 5 (newest is just-written, eldest is unlinked).

---

### 🟡 I-4 — `feedback_reload_precedence_snapshot` memory not actually followed in §8.5

§3.3 claims `feedback_reload_precedence_snapshot` is consumed at §8.5: "logger create startup'ta yapılır, daha sonra new'lenmez (snapshot disiplinine paralel)." That's a syntactic parallel, not the actual invariant the memory describes.

Re-reading `feedback_reload_precedence_snapshot.md`: the memory's prescription is that **ALL precedence-chain inputs** must be snapshotted at startup, including bg-detect, so that a later reload re-runs the full chain against the same inputs. It's about **precedence chain inputs**, not generic "create-once-at-startup".

`ReloadLogger` is not a precedence-chain input — it's a side-effect sink. The "create once at startup" pattern is fine here, but it's not the same invariant. Calling it `feedback_reload_precedence_snapshot` consumption is **name-checking**, exactly the v0.4.0 failure mode the consume-prior-review memory was written to prevent (per `feedback_consume_prior_review`).

**Recommended fix.** Either:
- (a) Strip the false memory citation from §3.3 / §8.5 and explain ReloadLogger lifecycle on its own terms.
- (b) Actually fold the memory: §8.5 should pin that adding the logger does NOT change any precedence-chain snapshot — i.e., the reload thread still snapshots `theme`/`profile`/`bg_default` at startup exactly as today, and the logger is purely additive plumbing. This is more useful and more honest.

Prefer (b) — it's a one-paragraph addition that turns name-checking into an actual invariant.

---

### 🟡 I-5 — Scenario C TUI-in-wrapper passthrough claim is asserted, not analyzed (§8.4)

§8.4 row C: "ratatui startup `\x1b[?1049h` (alt-screen enter) yazar → wrapper'ın v0.3.0 alt-screen passthrough state machine bytes-through mode'a geçer ... Zero new infra; v0.3.0 zaten çözdü."

Two specifics the spec does not address:

**(a) Direction of bytes.** The alt-screen passthrough state machine in v0.3.0 lives in the **output** path (PTY → user terminal) — it controls when the wrapper bypasses its own colorization. But `ratatui`'s `crossterm` event reader reads **input** (user terminal → PTY → child shell → `ratatui`). The wrapper's input path (user keystrokes → child via PTY master write) doesn't go through the colorization state machine at all; it's already passthrough. So input direction is fine, but the spec should say so explicitly (one sentence) rather than gesture at "v0.3.0 already solved it" — they're different code paths.

**(b) Resize forwarding.** When the user resizes the terminal while wrapped-TUI is running, SIGWINCH goes to tayf's wrapper signal thread (`src/signals.rs`). The wrapper currently `ioctl(TIOCSWINSZ)` on the PTY master, which propagates to the child shell — but does the child shell propagate to `ratatui` running under it? On most shells/setups yes (foreground process group receives SIGWINCH from the kernel via the TTY layer), but the spec doesn't pin this as a tested invariant. If `ratatui` doesn't see resize, its layout stays stuck on the old size and the experience degrades silently.

**Recommended fix.**
- One paragraph in §8.4 row C separating input-direction (already passthrough, unrelated to v0.3.0) from output-direction (v0.3.0 handles).
- Add SIGWINCH propagation as an explicit assertion in `tests/integration_tui_in_wrapper.rs` plan (§10.3): resize the outer PtyMaster, send a no-op key to TUI, capture, assert TUI's idea of `frame.size()` reflects new size.

---

### 🟡 I-6 — Scenario D silent-lost-edit window understated (§8.4)

§8.4 row D: "TUI #2 save → TUI #1'in `snapshot.source_hash` mismatch → TUI #1 sonraki `Ctrl+S` conflict mode'a düşer". This is correct **if TUI #1 reaches `Ctrl+S`**. The silent-loss window is:

1. TUI #1 opens. `snapshot.source_hash = X`.
2. TUI #1 makes edits in memory (no disk read yet).
3. TUI #2 opens. `snapshot.source_hash = X` (same).
4. TUI #2 saves. Disk hash becomes Y.
5. TUI #1's notify watcher (... but TUI doesn't have a notify watcher — only wrappers do).
6. TUI #1 user quits without saving (decides the edits weren't worth it).
7. TUI #2's saved state on disk is intact. No data loss.

Actually OK in this trace. But consider:

1-4 same as above.
5. TUI #1 hits `Ctrl+S`. SaveDiff modal opens — `Conflict` mode triggered (good).
6. User looks at the dual diff. Hits `y` to commit.
7. **§8.1 step 3 "Conflict if y": `let new_doc = toml_edit::parse(&disk_now)?; apply_edits_to_doc(&mut new_doc, &app.edits);`** — applies TUI #1's edits onto TUI #2's just-saved state.

**Problem:** `apply_edits_to_doc` operates on `RuleId` keys (§6.1: `Builtin(&'static str)`, `UserConfig(String)`, etc.). If TUI #2's save **added** a new user-config rule with the same name as one TUI #1 is trying to add, the conflict-resolution `apply_edits_to_doc` will silently overwrite TUI #2's addition (same `RuleId::UserConfig(name)` key in `PendingEdits.added`). The user clicked `y` thinking "merge my edits on top of theirs" but instead got "my edits clobber theirs by name collision".

This isn't a fatal flaw; it's a **subtle merge semantic** that the spec hand-waves. The spec should EITHER:
- (a) Specify the conflict-y semantic precisely: "by-key last-writer-wins; clicking `y` accepts that TUI #1's edits override any disk-side edits to the same `RuleId`", OR
- (b) Show TUI #1 a per-key conflict UI in conflict-y mode (more work).
- (c) Pin the silent-collision as a known limitation, à la v0.5.3 collision pins.

(a) + (c) is the lean answer. Add a `MergeConflictKeyCollision` test in §10.2 `save.rs` row.

---

### 🟡 I-7 — `reload.log` unbounded growth + concurrent write (§8.5)

§8.5: "log unbounded grow eder. v0.5.4 truncation YOK; v0.6+ rotation". 1 MB warn threshold mentioned.

Two issues:

**(a) Concurrent writes from multiple wrappers.** Two parallel `tayf bash` sessions both append to the same `reload.log` (each spawned `ReloadOrchestrator` creates its own `ReloadLogger` → both write to `<cfg-dir>/runtime/reload.log`). `OpenOptions::append` on POSIX uses `O_APPEND`, which is atomic for writes ≤ PIPE_BUF (typically 4096 bytes). Each line is small (~50-100 bytes) so this is fine in practice — but the spec should pin it ("each append is one `write(2)` ≤ PIPE_BUF; POSIX guarantees atomicity"), because the §10.2 `reload.rs` test row otherwise has no shape for `concurrent_appends_do_not_interleave_within_line`.

**(b) The 1 MB warn fires once per startup? Or every reload?** The spec says "ReloadOrchestrator startup'ta" — once. Means a session that crosses 1 MB during the session never warns. Practical effect: someone leaves a tayf wrapper open for a week, log hits 10 MB silently. v0.5.4 explicitly defers rotation — fine — but the warn should at least be **periodic** (check every Nth append, or on threshold-cross-via-append).

**Recommended fix.**
- Add the POSIX atomicity guarantee + the matching test.
- Either move the 1 MB warn to per-append (cheap — file metadata call) or document explicitly that it's startup-only.

---

### 🟡 I-8 — `dump.rs` and `status.rs` overload module name (§5.1, §5.3, §13.2)

`src/config_tui/status.rs` is both:
- The `tabs::status` tab (no, that's `src/config_tui/tabs/status.rs` — OK).
- The `tayf config status` sub-subcommand (`src/config_tui/status.rs`).

Two `status.rs` in the same module tree, different paths. Compiles fine, but confusing — `mod status;` in `mod.rs` brings in `status::*`; `mod status;` inside `tabs/mod.rs` brings in `tabs::status::*`. Module names in `use` paths will be ambiguous-looking in grep output (`grep -rn "status::" src/config_tui/`).

Same minor smell with `tabs/profiles.rs` vs `src/profiles.rs` (already in tree — was a v0.5.2 thing — but having `tabs/themes.rs` and `tabs/profiles.rs` shadow the top-level `themes.rs`/`profiles.rs` modules is a code-smell flag in a "file-per-concept" project).

**Recommended fix.** Rename the non-interactive subcommand modules:
- `src/config_tui/dump.rs` → `src/config_tui/dump_cmd.rs` (or `dump_action.rs`).
- `src/config_tui/status.rs` → `src/config_tui/status_cmd.rs`.

`tabs/{patterns,themes,profiles,status}.rs` keep their names (they're paired by tab — natural). Trivial rename, makes `grep`s unambiguous.

---

### 🟡 I-9 — `#[non_exhaustive]` on `RunArgs` is wrong (§4.1)

§4.1 puts `#[non_exhaustive]` on `RunArgs`. Question §15 #4 deliberates the same on `Cmd` (good — variants additive). But on `RunArgs` (a struct of CLI flags):

`#[non_exhaustive]` on a struct prevents downstream construction via struct literal (`RunArgs { shell: None, ... }`). The clap-derive `Args` impl uses the derive macro, which **builds the struct via the macro-generated builder**, not literal construction — so `#[non_exhaustive]` doesn't break clap parsing. But it **does break our own tests** if they construct `RunArgs` directly (current `cli.rs::tests` does `Args::try_parse_from(...)`, not literal construction, so today's tests pass). And it definitely breaks any downstream library consumer that wants `RunArgs::default()` semantics.

More importantly: the existing `Args` (v0.5.3) already has `#[non_exhaustive]`. The rename moves the field set into `RunArgs`. If we want `non_exhaustive` to apply to the public CLI surface, it must be on `Args` (the wrapper) — which the spec does — AND on `RunArgs` if we want to keep the same forward-additivity guarantee for the flag bag.

This is fine, just double-belt-and-braces. Worth checking that clap-derive's `ClapArgs` macro on a `#[non_exhaustive]` struct still compiles — pre-1.0 clap had a known issue here, possibly fixed by now. Spec should call it out as a Phase A1 verification step.

**Recommended fix.** Add a one-line note in §4.1: "Verify in Phase A1 that `#[derive(ClapArgs)]` + `#[non_exhaustive]` compiles cleanly on clap 4.6 — there was an early-clap-4 bug here; if it fails, drop `non_exhaustive` from `RunArgs` only (keep it on `Args`)."

---

### 🟡 I-10 — `config status` exit-code policy understated (§4.4, §15 #3)

§4.4 maps `tayf config status` failure to exit 0 ("config parse error → stderr warn line + exit 0; status remains partially useful"). §15 #3 flags this as open.

This conflicts with the broader exit-code policy: every other tayf command exits with EX_USAGE (64) on user config error. Treating `status` as "always 0 because it's read-only" makes it impossible to use in scripts (`tayf config status && echo ok`).

Options:
- (a) Spec's current proposal: 0 always (partial info). Scripts must parse stdout. Bad ergonomics.
- (b) Exit 0 only when status is fully renderable; 64 when any config-load error means status fields are `(unresolved)`.
- (c) Exit 64 on any parse error; print partial status anyway (UNIX-y — non-zero exit doesn't preclude stdout).

(c) matches existing tayf behavior most cleanly. (b) is acceptable. (a) is a footgun. Spec should pick **before** plan, not leave it as open.

**Recommended fix.** Make a decision in §4.4. Spec author preference: (c) — non-zero exit on user error, full partial info still printed.

---

## §3. Nits (🔵)

### 🔵 N-1 — `version_str()` regression in `cli.rs` refactor (§4.1)

The current `src/cli.rs:24-27` defines `fn version_str() -> &'static str` and uses it in `#[command(version = version_str())]`. The §4.1 spec snippet shows `Args` with the same `version = version_str()` but `RunArgs` without — fine, but worth pinning that `version_str()` stays at module scope (not moved into `Args`'s scope), so both `Args` and any future top-level command can reference it.

### 🔵 N-2 — `RuleEdit.styles` keyed by `StyleKey` enum but `StyleKey::Numbered(u32)` (§6.1)

In v0.5.0 we accept `styles."1"` keys; max practical capture index is ~99 (no real regex has 100+ groups). `u8` would be enough. But pinning `u32` here is fine and matches `regex::Captures::get(idx)` which takes `usize`. No action — pointing out for awareness.

### 🔵 N-3 — `rfc3339_ms_filename_safe` referenced but not defined (§8.1, §8.3)

§8.1 step 1 calls `rfc3339_ms_filename_safe(now())`; §8.3 describes the substitution. The function is named in two places but its definition is implicit. Plan should pin it to a tiny helper in `save.rs` (say, `fn ts_for_backup_filename(now: SystemTime) -> String`) so the test can independently fixture-pin the exact wording.

### 🔵 N-4 — Spec §11.3 "Manual termios + ANSI ~2700 LOC" probe path

Memory `feedback_dependency_minimalism` audit table says alternative LOC measured "during brainstorm at /tmp/ratatui-size-check probe". `/tmp/...` paths are ephemeral. The probe should either be (a) re-run + numbers recorded in spec for permanence, or (b) the spec should call the LOC figure an estimate with the methodology described in one sentence. Currently reads like a phantom citation.

### 🔵 N-5 — `nix` dep already exists; `OpenOptions::mode` + `custom_flags` need `OpenOptionsExt`

For I-2's mode-preservation fix, `std::os::unix::fs::OpenOptionsExt` is needed (in std). No new dep. Just worth confirming this in spec.

### 🔵 N-6 — `crossterm` version mismatch risk

ratatui 0.30 supports both `crossterm 0.28` and `crossterm 0.29` (features `crossterm_0_28` / `crossterm_0_29`). Spec §11.2 transitively lists `crossterm 0.29`. The `crossterm = ["std", "dep:ratatui-crossterm"]` feature alone defaults to one of these (currently 0.29 per ratatui upstream `Cargo.toml`). Worth confirming we're on 0.29 (newer) and that ratatui doesn't silently fall back. Add to A1 verification: `cargo tree -e features | grep crossterm`.

### 🔵 N-7 — `DEFAULT_PREVIEW_SAMPLE` literal contains a colon-separated `host:port` — `fqdn` collision

§9.3 `DEFAULT_PREVIEW_SAMPLE` includes `10.0.0.5:5432`. Per v0.5.3 known limitations (fqdn collision), this sample will render with the `fqdn` envelope swallowing the `:5432` port. Not a bug, but **users will see the collision in their preview every time they open the TUI** — exactly the pattern v0.5.5 is meant to fix. Two options:
- Pick a sample that avoids the collision shape (no `host:port`).
- Leave it as-is — it's an honest demonstration of current state.

Tilt toward avoiding it; the TUI is a learning surface for new users and presenting it with a known-buggy render is bad first-impression.

### 🔵 N-8 — Phase C2 is over-stuffed (§13.3)

§13.3 C2: "app.rs + events.rs (event loop skeleton + key dispatch) + render.rs (frame layout + narrow-term degradation gate) + tabs/* stubs". LOC: 250 + 200 + 180 + (4 tab stubs ≈ 40) ≈ 670 production LOC + TDD tests. That's not a "single Claude task" — that's a 2-3 task chain.

Split:
- C2a — `app.rs` + `events.rs` skeleton + global key dispatch + quit FSM.
- C2b — `render.rs` frame layout + narrow-term gate + status bar.
- C2c — `tabs/*` stubs + tab-strip routing.

Each ~200-250 LOC + tests, fits the lean-task shape per `feedback_lean_process_small_subversions` (even though §13 says v0.5.4 is NOT lean — task granularity should still be).

### 🔵 N-9 — `cargo public-api` not in CI

§5.3 references `cargo public-api diff`. tayf's CI does not run `cargo public-api`. Either:
- Drop the reference (just an aspirational note).
- Add `cargo public-api` to a A1 task as a one-shot baseline (NOT recurring CI; just a baseline snapshot to diff against during code review).

Lean choice: drop the reference. If we cared about public-api stability we'd have added the gate in v0.5.0 when the surface stabilized.

### 🔵 N-10 — §6.1 `ParsedConfigView.theme_ref: Option<String>` vs existing `theme: Option<String>`

Existing `config::GeneralSection` already has `pub theme: Option<String>`. Spec invents `theme_ref` — implies a different shape. Probably just a different snapshot, but the naming divergence will confuse readers. Use `theme` to match.

---

## §4. Strengths

1. **TUI on its own code path is the right architectural call.** §5.4 "DOKUNULMAZ" line for `runtime.rs` / `pipeline.rs` / `io_loop.rs` / `pty.rs` / `rules.rs` / `profiles.rs` / `themes.rs` (apply_rules path) is exactly the discipline that keeps v0.5.4 perf-bench-CI no-op. The architectural anchor in §1 is unambiguous.

2. **Atomic write + backup + conflict-aware diff is the correct semantic.** §8.1 D1 flow (read disk → hash compare → modal → commit) matches battle-tested patterns (git index, SQLite WAL commit, atomic config writers in systemd). The conflict-mode dual-diff render is a thoughtful UX answer — most TUIs ship with "overwrite or cancel", losing TUI-#1's work silently.

3. **Test strategy is honest about render-snapshot trade-off (§10.4).** The four reasons listed for skipping TestBackend snapshots are correct (SGR-sensitivity, Unicode-width drift, maintenance burden, behavioral coverage suffices). Re-adding snapshot tests when UI regression bites (v0.6+ demand-driven) is the right call — pay the cost when there's signal, not before.

4. **`#[non_exhaustive]` on `Cmd` from day one (§4.1).** Even with one variant, this is forward-additive discipline. Future subcommands (`tayf config new-profile`, `tayf doctor`, etc.) won't be source-breaking adds. Matches the existing `Error` / `Args` / `ThemeRuleErrorKind` pattern.

5. **Memory-mandate consumption pattern carried forward (§3.3).** Even though I-4 flags one false consumption, the *practice* of explicit fold-or-defer per memory is the right shape — it's what `feedback_consume_prior_review` exists to enforce. Beats silent omission.

6. **Brainstorm carve-out for collision fix → v0.5.5.** Refusing to bundle the architectural-collision fix into v0.5.4 (a TUI sub-version with a new ratatui dep) is the correct scope discipline. Keeps blast radius small. The two collision pins (`aws_arn_yields_to_interior_region_pattern_v0_5_3_limitation` + `docker_image_tag_registry_host_yields_to_fqdn_v0_5_3_limitation`) byte-pinned mean v0.5.5 will fail loud when it lands the fix.

---

## §5. Recommendation

**Revise spec before plan writing.** Specifically:

1. **C-1 first** — fix the `ratatui` feature line. This is a build break waiting to be discovered in A1; better to catch it now. Update §11.1 + §11.2 to agree.
2. **C-2 next** — bump `toml_edit` to `0.25.x`, re-run `cargo deny check` + audit table at that version. Update §11.1 + §11.2.
3. **C-3 declaration** — pick (a) honest break or (b) accessor shim; if (a), add `### Changed (breaking)` shape to the §13.9 release ceremony's CHANGELOG step.
4. **🟡 I-1 / I-2 / I-3** — atomic-write hardening: switch to `sync_all` on tmpfile, document EXDEV invariant, add mode preservation, decide tempfile vs hand-rolled guard, reorder rotation.
5. **🟡 I-4** — strip false `feedback_reload_precedence_snapshot` citation OR add the actual invariant (recommend latter).
6. **🟡 I-5 / I-6** — Scenario C: separate input/output direction prose + SIGWINCH test. Scenario D: pin the silent-collision merge semantic (recommend lean (a)+(c): documented as last-writer-wins by-key + test pin).
7. **🟡 I-10** — pick `config status` exit code (recommend (c) non-zero on user error + full partial info).
8. **🔵 N-8** — split C2 into three sub-tasks (C2a/C2b/C2c).

After revisions, re-run the spec-phase parallel review (per `feedback_spec_phase_parallel_review`). The UI/UX senior pass should specifically check N-7 (preview sample collision), the color-picker `Y` hybrid layout under narrow terminals (§7.4 width=60 gate), and the keyboard-binding overlap (`o` for "override" in both Patterns and Themes/Profiles tabs is consistent — good — but `o`, `d`, `e`, `c`, `r` are all single-letter direct in Patterns tab without modifiers; verify no clash with the `n`ew-pattern modal `n` global vs `n` in conflict modal vs `q`uit-confirm `n`o).

Once revised + re-reviewed → CLEAN_SHIP → proceed to plan.

---

**End of review.**
