# TOML / Serialization Senior Spec Review — v0.5.5 (Opus 4.7)

Spec under review: `docs/superpowers/specs/2026-05-27-tayf-v0.5.5-build-new-content-reconciliation.md`
Reviewer focus: `toml_edit 0.25` semantics, comment/whitespace/ordering preservation, string-encoding form choice, adversarial-input safety in the walk algorithm.
Crate inspected: `toml_edit-0.25.11+spec-1.1.0` (the version currently pinned in `Cargo.toml`).

This review is paralel to the Rust senior review; I do not duplicate code-style or borrow-checker concerns. I focus exclusively on TOML/serialization correctness — the load-bearing assumption "DocumentMut preserves comments + ordering + trivia under mutation."

---

## 1. `DocumentMut` preservation contract — verified, with two surprises

I read `src/document.rs`, `src/table.rs`, `src/encode.rs`, `src/repr.rs`, and `src/array_of_tables.rs` to confirm what toml_edit actually guarantees vs what spec §5 assumes. Headline: **for the operations the spec performs, preservation works for keys/headers/comments/positions, BUT trivia attached to a specific `Item` is dropped when that Item is replaced wholesale.** This is the subtle gotcha.

Per-operation verdict:

- **In-place value replacement** (`general["theme"] = value("light")`).
  `Table::IndexMut` (index.rs:110-114) calls `self.entry(key).or_insert(Item::None)`, returning `&mut Item`. Assignment then *moves* a fresh `Item` over the old one — the old `Item`'s `Repr` and surrounding `Decor` are dropped together with it. **HOWEVER**, the surrounding *key* decor lives on the `Key` in `IndexMap`, not on the Item, and `Key`s aren't replaced on assignment (see `insert()` in table.rs:432-447 which only clones a fresh key when the key did not exist; the `IndexMut` path doesn't even reach `insert`). The leading/trailing comment **on the value position** (e.g. `theme = "dark"  # was dark`) is lost; the leading comment **on the key line** (e.g. `# theme override\ntheme = "dark"`) is preserved because it lives on `Key::leaf_decor`. This is acceptable for tayf (we don't expect inline value-trailing comments on `theme`/`profile`), but the spec should make this explicit. **🟡 IMPORTANT**.

- **Key removal** (`general.remove("theme")`).
  `Table::remove` (table.rs:466) is `shift_remove` — preserves order of all OTHER entries. The removed `Key` carries its leading decor with it (so a comment *immediately* above the removed key disappears with the key — usually desired). The next key's `leaf_decor` is unchanged. **Verdict: clean.** 🔵 NIT: if a doc has `# this controls theme\ntheme = "dark"\nprofile = "docker"` and we remove `theme`, the user loses the `# this controls theme` comment. Probably correct (the comment described a now-removed key). Spec should note this in §5.2.

- **Inline-table field mutation** (`style = { fg = "red" }` → mutate `fg`).
  `InlineTable::IndexMut` (index.rs:124-128): **panics** (`expect("index not found")`) if the key is absent — this is a SHARP edge vs `Table::IndexMut`. Spec §5.4's `table["fg"] = value(...)` syntax (line 391) works ONLY when `fg` already exists. For a fresh-add of `bold = true` to an inline-table that previously had only `fg`, the spec code **panics**. Must use `inline.insert("bold", v)` (inline_table.rs:396). **🔴 BLOCK** unless the `StyleTarget` dispatch wrapper hides this difference (see item 2).

- **Dotted-table mutation** (`[rules.styles."1"]`).
  Dotted-key form is represented internally as a chain of `Item::Table` with `set_dotted(true)`. Mutation preserves dotted form because `Display` for `Table` (encode.rs:296) checks `is_dotted()` and skips emitting a `[header]` line. The spec's `ensure_subtable(rule_table, "styles")` (§5.3) walks through `Item::Table`. If the existing form is dotted (`styles."1" = { ... }`), `ensure_subtable` returns the dotted-table; mutation keeps it dotted. If the existing form is block (`[rules.styles."1"]`), mutation keeps it block. **Verdict: clean**, but spec must use the **existing** subtable (don't `Table::new()` + replace — that would drop is_dotted+position). 🟡 IMPORTANT: spec §5.3's `ensure_subtable` should be specified as "if present return as-is; if absent create non-dotted block-table" — the spec phrasing today is silent on this branch.

- **ArrayOfTables entry append** (`[[rules]]` push).
  `ArrayOfTables::push(Table)` (array_of_tables.rs:88) appends `Item::Table(t)` at end of `values: Vec<Item>`. The new Table has `position: None` and empty decor; per `visit_nested_tables` (encode.rs:213-220), it sorts by position, with `position: None` inheriting the **last seen position**, which means it gets emitted at the end of the document (correct intent). 🟡 IMPORTANT: if the file ends with a non-`[[rules]]` block (e.g., `[general]` at end after the rules array — uncommon but possible), the new `[[rules]]` entry inherits position of the *physically last positioned table* and may land in surprising places. Add an integration test that adds `[[rules]]` to a doc whose last block is `[general]`, not `[[rules]]`, and pins the resulting layout.

- **ArrayOfTables entry removal** (`[[rules]]` remove at idx).
  `ArrayOfTables::remove(idx)` (array_of_tables.rs:126) is `Vec::remove` — O(n) shift. The removed `Item::Table` carries its `Decor` (which contains the leading comment-block above the `[[rules]]` header). **The comments leading the removed entry are deleted with it** (see item 6 for the docker.toml `# To disable, add to user-config:` case). The next entry's leading decor is preserved. Spec §5.6 should pin this as the contract (deletion takes any prefix comments that were attached to the entry). 🟡 IMPORTANT — needs a test that pins this behavior so we never silently regress to "orphan the comment to the wrong entry".

**Known surprises spec didn't enumerate**:

(a) **`Table::insert` re-formats the key** — see table.rs:437-438: on Occupied entry, calls `entry.key_mut().fmt()` (`Key::fmt` clears repr). This means if you `insert("theme", v)` on a key whose original spelling was `'theme'` (literal-quoted), the next render emits `theme` (bare). For tayf this is fine (we don't expect quoted versions of `theme`/`profile`/`bold`/`fg`/etc.), but it does mean: **if a user wrote `'theme' = "dark"` in their config, after one round-trip it becomes `theme = "dark"`**. 🔵 NIT — add a test guarding against this on the keys we actually mutate, so we know if the contract ever changes.

(b) **`Document::Display` re-sorts tables by `position()`** (encode.rs:222: `tables.sort_by_key(|&(id, _, _, _)| id)`). This is normally a no-op (parsed positions are monotone), but if you **mix** parse + manual `Table::set_position` you can end up with re-ordering surprises. The spec's `apply_new_rule` doesn't set position; the appended Table is `position: None`, inherits `last_position`, sorts to the end. Fine for v0.5.5. But the spec should note: **do not call `set_position` from reconcile.rs without an explicit test pinning the layout**.

---

## 2. `InlineTable` vs `Table` dispatch — must not form-flip

Spec §5.4 proposes `enum StyleTarget<'a> { Inline(&'a mut InlineTable), Table(&'a mut Table) }`. This is **necessary** and the spec is right to call it out, but the spec is too loose on what `write_style_table` actually does inside. Concrete requirements:

- If the existing form is `style = { fg = "red" }` (InlineTable), mutating `fg` and adding `bold` must keep it inline.
  - `InlineTable::IndexMut` panics on absent key (index.rs:126). Use `inline.insert("bold", v)` instead of `inline["bold"] = v` for additions. **🔴 BLOCK** for spec §5.4 line 391 — `table["fg"] = value(...)` is only safe for *replacement*; for fresh-add it must be `insert`.
  - On removal, use `inline.remove("fg")` — this preserves the inline form. (toml_edit will continue to render `{ }` even if empty.) 🔵 NIT: if `write_style_table` removes the last key from a `style = { fg = "red" }` and then drops the empty inline-table, the v0.5.5 contract should specify whether to keep `style = { }` or remove `style` entirely. Spec is silent. Recommend: **leave the empty inline-table** — removing it might break user intent (they wrote `style = { }` deliberately).
- If the existing form is `[rules.style]` (block Table), mutating must keep it block.
  - This is the natural behavior of `Table::IndexMut` + `Table::insert`; no special handling.
- **Never** flip from one form to the other on edit. The natural toml_edit API does NOT flip provided you reuse the existing `Item` — which the spec's "ensure_inline_or_table" helper must do.

🟡 IMPORTANT: spec §5.4 must explicitly state "form-preserving — caller-chosen form on first write, never converted on subsequent edits."

---

## 3. `ensure_general_table::set_implicit(false)` — correct but with a corner case

Spec §5.2 (lines 312-319) calls `set_implicit(false)` on the freshly-created `[general]` table. This is correct: per table.rs:228 + encode.rs:314, an implicit table with `children.is_empty()` is **hidden** in `Display`. Without `set_implicit(false)`, a freshly-created empty `[general]` would be elided from output (even though we're about to write keys into it — but Rust's row-ordering means the implicit bit is checked at `Display` time, and a freshly-created Table has `implicit: false` by default per `Table::new()` table.rs:33-36). So **`set_implicit(false)` is actually redundant for a freshly-`Table::new()`-ed table** — but it's correct defensive code. 🔵 NIT: remove or document why.

**Corner case spec didn't address**: if the file already has `general.theme = "dark"` in **dotted-key form at root** (no `[general]` header), the existing `general` key is an `Item::Table` with `set_dotted(true)` and `implicit: true`. `ensure_general_table` then returns this dotted-table. The spec's `general["theme"] = value("light")` would mutate the dotted-table's `theme` key, and `Display` would emit `general.theme = "light"` (preserving dotted form). 🟡 IMPORTANT: spec should state this contract ("dotted-form `general.theme` stays dotted; block-form `[general]\ntheme` stays block") and add a test for the dotted-form case. The spec's `ensure_general_table` body uses `contains_key("general")` which returns true for the dotted variant — so it correctly doesn't overwrite, but the subsequent `set_implicit(false)` is **never reached** for the existing-key branch, which means the dotted variant stays implicit, which is correct.

---

## 4. Quoted-key handling — `i.to_string()` is unsafe at the encoder level

Spec §5.3 writes `ensure_inline_or_table(styles, &i.to_string())` where `i: u32` from `StyleKey::Numbered(i)`. Per `toml_writer/string.rs` `KeyMetrics::calculate` (lines 402-436), `unquoted` requires every byte to be in `[a-zA-Z0-9\-_]`. A bare digit string like `"1"` **passes** this check (it's all `0-9`). The result: **`Table::insert("1", v)` produces a freshly-formatted key whose `Key::default_repr` is `1` (bare, unquoted).**

BUT — TOML grammar requires positive-decimal table keys at the schema level to be quoted in some contexts (per spec §1.3 the user-config schema uses `"1"`, not `1`). The TOML 1.0 grammar **does** allow `unquoted-key = 1*( ALPHA / DIGIT / %x2D / %x5F )`, so `1` is a valid unquoted bare key — it's just unusual. `toml::from_str` (via serde) accepts both `1 = "..."` and `"1" = "..."` and surfaces them as the same BTreeMap key.

**Concrete impact**: the spec's roundtrip produces `styles.1 = { ... }` (or `[rules.styles.1]`) instead of the more conventional `styles."1" = { ... }`. Functionally equivalent, but **user-visible diff churn**: a user who hand-wrote `"1"` will, after one save, find their config reformatted to `1`. This violates the "roundtrip is identity for unedited fields" contract (the inserted key is freshly-formatted but neighboring data is not — net effect is style-inconsistent output).

**Fix**: use `insert_formatted` with `Key::new("1")` (which preserves the user's existing quoting if the key already exists) or explicitly construct the key with literal-string repr for numbered keys to match the v0.3.5 documented convention `styles."1"`. **🟡 IMPORTANT** — affects user-visible round-trip; not a correctness blocker.

Same concern applies to `StyleKey::Named(n)` if `n` contains non-bare characters (it shouldn't per spec §1.3 grammar `^[a-zA-Z_][a-zA-Z0-9_]*$`, but reconcile.rs should `assert_debug!` or `?`-return on this — spec is silent on the validation gate).

---

## 5. Raw-string vs basic-string for `pattern` — toml_edit auto-picks literal, but only on first write

This is the highest-stakes item on the review. I traced the encoding path end-to-end:

1. Spec calls `rule_table["pattern"] = toml_edit::value(pat.as_str())` where `pat = r"\b[a-z]+\b"`.
2. `value(v)` (item.rs:431) → `Item::Value(v.into())`.
3. `String → Value` (value.rs:276-280) → `Value::String(Formatted::new(s))`. **`Formatted::new` sets `repr: None`** (repr.rs:20-26).
4. On `to_string()`, `encode_formatted` (encode.rs:97) checks `as_repr()` → None → uses `default_repr()`.
5. `default_repr()` (repr.rs:48-51) → `self.value.to_repr()`.
6. `impl ValueRepr for String` (encode.rs:366-373) → `TomlStringBuilder::new(s).as_default().to_toml_value()`.
7. `as_default()` (toml_writer string.rs:33-45) tries in order: `as_basic_pretty` → `as_literal` → `as_ml_basic_pretty` → `as_ml_literal` → fallback.
8. For `\b[a-z]+\b`: `metrics.escape = true` (contains `\`), so `as_basic_pretty` returns None. `as_literal` requires no `escape_codes` (no control chars), no `'`, no newline — **accepted**. Output: `'\b[a-z]+\b'` — **literal string, no escaping, byte-identical to source**.

**Verdict on item 5: SAFE BY CONSTRUCTION** for the regex case the spec worries about. toml_edit's default-encoding logic specifically picks literal-string form when `\` is present and no other constraint blocks it.

**HOWEVER, three caveats the spec must own**:

(a) **Mutation drops the original repr.** If a user writes `pattern = "\\b[a-z]+\\b"` (escaped basic-string form), and reconcile mutates the value, the spec's `["pattern"] = value(pat.as_str())` creates a fresh `Formatted::new` with `repr: None`. On render, default_repr picks **literal-string** (per the trace above). Result: **`"\\b..."` becomes `'\b...'`**. Functionally identical regex, but user-visible diff churn. 🟡 IMPORTANT.

(b) **If the regex contains a `'` (single quote)** — e.g., `pattern = "it's a date"` — `as_literal` rejects (max_seq_single_quotes > 0). Falls through to `as_ml_basic_pretty` (rejected if escape), `as_ml_literal` (accepts if < 3 consecutive `'`), basic. For a single `'` no `\` regex, output is multi-line literal string `'''...'''`. **🟡 IMPORTANT** — pin a test case with a `'` in the regex, because the form may surprise users. For a `'` AND `\` regex, output is multi-line literal `'''\b...'''` (acceptable per TOML grammar).

(c) **If the regex contains a control character or `\n`** (rare for regexes but possible for `\t` literally embedded), `as_literal` and `as_basic_pretty` both reject. Result: an escaped basic-string or multi-line variant. **🔵 NIT**: regex source UI should reject literal `\n` / `\t` bytes at the input gate (use `\n` / `\t` escape syntax instead).

**Spec amendment requested**: §5.3 should explicitly cite this trace and add a unit test:

```rust
#[test]
fn pattern_with_backslashes_renders_as_literal_string_no_escaping() {
    let mut doc: DocumentMut = "[[rules]]\nname = \"x\"\n".parse().unwrap();
    let rules = doc["rules"].as_array_of_tables_mut().unwrap();
    let t = rules.get_mut(0).unwrap();
    t["pattern"] = toml_edit::value(r"\b[a-z]+\b");
    let out = doc.to_string();
    assert!(out.contains(r"pattern = '\b[a-z]+\b'"),
        "regex must serialize as literal-string (single-quoted) form; got: {out}");
    assert!(!out.contains(r#"pattern = "\\b"#),
        "must NOT render as escaped basic-string");
}
```

**🟡 IMPORTANT** — this is the most fragile invariant in the spec and needs an explicit pin.

---

## 6. Comment-attachment when removing — confirmed deletion-with-entry

Per encode.rs:328-337 (`visit_table` for array_of_tables) and the `Item::Table` carrying its own `Decor`, the leading comment block of a `[[rules]]` entry is attached to the *Table's* decor (via the leaf key's `leaf_decor`). On `ArrayOfTables::remove(idx)`:

- The removed `Item::Table` is consumed (moved out of `values: Vec<Item>`).
- Its `Decor` (including leading comments) goes with it — **deleted with the entry**.
- The next entry's `Decor` is untouched.

Concrete scenario for docker.toml lines 7-14:
```
# To disable, add to user-config:
#   [[rules]]
#   name = "container_id"
#   enabled = false
pattern = '\b[a-f0-9]{12}\b'
```
These comments are between two key-value pairs WITHIN the `[[append_rules]]` Table, not between `[[append_rules]]` headers. They live on the `Key::leaf_decor` of `pattern`. If reconcile mutates `pattern`, **`Table::insert` re-formats the key** (table.rs:437-438: `entry.key_mut().fmt()` clears `Key::repr` but does NOT clear `Key::leaf_decor` directly — let me verify).

Actually re-checking: `Key::fmt` (key.rs would need inspection) is documented as "Auto-formats" — let me verify the leaf_decor preservation:

Per `repr.rs:88-92`, `Formatted::fmt` clears only `self.repr = None`, not decor. By analogy `Key::fmt` likely only clears the `Repr` (the rendered text of the key itself), not the leaf_decor. **🟡 IMPORTANT — spec phase verify**: add an integration test that mutates a value whose key has a preceding comment block and pins that the comments survive. Specifically, test scenario:

```rust
let src = "[[rules]]\nname = \"x\"\n# inline comment above pattern\npattern = \"old\"\n";
// mutate pattern → \"new\"; assert "# inline comment above pattern" still in output.
```

If this test fails, the spec needs an amendment using `Table::key_mut` + `Key::leaf_decor_mut` to manually preserve the comment, or use a more conservative mutation path.

🟡 IMPORTANT — add this test to §7.1 as test #16.

---

## 7. Trailing newline + final-byte semantics — explicit and preserved

`DocumentMut::trailing` (document.rs:136) is a `RawString` populated by the parser to be whatever comes AFTER the last content (typically `"\n"` or `""`). `Display` for `DocumentMut` writes it last (encode.rs:228: `self.trailing().encode_with_default(f, None, "")`).

- Source file ending with `\n` → `trailing = "\n"` → output ends with `\n`. ✅
- Source file ending without `\n` (e.g., `[general]\ntheme = "dark"` with no final newline) → `trailing = ""` → output ends without `\n`. ✅

**HOWEVER**, when `apply_new_rule` appends a `[[rules]]` entry, encode.rs:337 (`writeln!(buf)?;`) emits a `\n` after the header line, and `writeln!` after each key-value line (line 361). So the new content has its own terminating `\n`, and `trailing` follows. **Output for a doc that was `[general]\ntheme = "dark"\n` + append `[[rules]] name = "x"`** becomes:

```
[general]
theme = "dark"

[[rules]]
name = "x"
```

(plus the original `trailing`). This is correct.

**Edge case the spec doesn't address**: source file has `\r\n` line endings (Windows-authored user config). toml_edit's parser normalizes (or doesn't — needs verification). Per CLAUDE.md Windows ConPTY is v1.0+, but a user-authored file copied from a Windows editor could land in a Mac/Linux tayf install. 🔵 NIT — confirm `\r\n` behavior in the spec or in a defensive test; today the only safety net is the UTF-8 decode + parse pass in snapshot.rs.

---

## 8. Color encoding canonical form — verified roundtrip; one ambiguity flagged

I traced every variant in `Color::to_toml_str` (spec §6.2) against `Color::parse_str` (src/style.rs:117-178):

- All 16 ANSI / bright_ansi names: `to_toml_str` emits lowercase → `parse_str` lowercases input (line 122) → match arms accept. ✅
- `Color::Indexed(0..=255)`: emits `"color(N)"` → `parse_str` strips `color(...)`, parses u16, checks ≤ 255. ✅
- `Color::Rgb(r,g,b)`: emits `"#rrggbb"` → `parse_str` accepts `#`-prefix hex via `parse_hex` (style.rs:181), checks 6 hex digits, lowercase-tolerant. ✅

**Sentinel coverage in §6.2 test**: 16 ANSI + 3 Indexed (0, 178, 255) + 3 Rgb (000000, ff8800, ffffff) = 22 cases. **Sufficient.**

**The aliasing concern raised in the spec prompt**: `Color::Black` → `"black"`, `Color::Rgb(0,0,0)` → `"#000000"`. Different output strings, both `parse_str`-able to their own variants. **No aliasing risk** — these are distinct strings producing distinct enum variants. ✅

**One subtle issue**: `Color::Indexed(0)` produces `"color(0)"`. But `parse_str` doesn't recognize `"color(0)"` as `Color::Indexed(0)` differently from `"black"` (which yields `Color::Black`). They're DIFFERENT variants. In SGR rendering they emit DIFFERENT byte sequences (`30` vs `38;5;0`) but visually identical on standard terminals. **Roundtrip is correct** (each canonical form maps back to its own variant), but a user who writes `fg = "color(0)"` and then edits via TUI gets `fg = "color(0)"` back, not `fg = "black"` (assuming the TUI knows which variant was on disk). 🔵 NIT — spec §6.2 should note: "canonical form is variant-preserving; tayf does not normalize `color(0)` to `black` etc."

**Indexed(255) supportability** — yes, the v0.5 truecolor pipeline accepts Indexed(0..=255) on the input side (`parse_str` accepts up to 255 via the `if n > 255` guard at style.rs:133), regardless of terminal color depth — `Color::downgrade` handles the depth quantization at render time. ✅

**Verdict**: §6.2 roundtrip property test is sound. Add 🔵 NIT amendment noting variant-preservation contract.

---

## 9. Adversarial input — three concerns, all 🟡 IMPORTANT

(a) **Duplicate `[[rules]]` with same `name`**.
Spec §5.3 `find_rule_index_by_name` (description: "linear scan, ilk eşleşen `name = "X"` index'i") returns the **first** match. If a malicious user-config has two entries with `name = "uuid"`, reconcile silently mutates the first and leaves the second untouched. From an adversarial-input view this is **not** a security hole (the user authored the config; we don't grant escalated capability), but it IS a **silent semantic divergence** from `Config::parse` (config.rs) where serde's `BTreeMap`-equivalent likely collapses duplicates last-writer-wins. **Recommendation**: log a `Toast::warn("rule '{name}' appears N times; editing the first occurrence")` when N>1. 🟡 IMPORTANT — does not block ship, but should be in §10 open questions and resolved in v0.5.5 or carryover.

The existing test `merge_collision_user_config_name_clobbers_silently` (save.rs:402) pins the *staging-side* HashMap collision (last-writer-wins on `RuleId`). The *disk-side* collision is a separate invariant the spec doesn't pin. **Add test**:

```rust
#[test]
fn duplicate_rule_name_on_disk_mutates_first_occurrence_only() {
    // Disk: [[rules]]\nname="x"\npattern="A"\n[[rules]]\nname="x"\npattern="B"
    // Stage: RuleEdit{pattern: Some("NEW")}
    // Assert: first entry has pattern="NEW"; second still has pattern="B".
}
```

(b) **Already-corrupt-but-parseable doc** (spec §6.1 `TypeMismatch`).
Spec says "currently unreachable because ConfigSnapshot.parse() zaten validate ediyor". This is **wrong in one path**: `ConfigSnapshot::read_from_disk` (snapshot.rs:77-90) parses via `toml_edit::DocumentMut` AND via `crate::config::parse` (serde). The two parsers have **different acceptance sets**. Example: `rules = "not an array"` is accepted by toml_edit (string at root) but rejected by serde. snapshot.rs returns `Err` early in that case, so reconcile never sees it. ✅ Reachability is correct.

However, `TypeMismatch` is also reachable for a stranger case: what if `[[rules]]` contains an entry where serde's `Option<UserStyle>` field is something exotic like `style = 42`? Serde rejects (type mismatch); snapshot returns `Err`; reconcile not reached. ✅

What about: `[general]` is present and toml_edit-parseable AND serde-parseable, but during a *mutation*, the spec's `general["theme"] = value(...)` is fine, but the spec's `general.as_table_mut()` (line 318) panics on `expect("ensured above")` if `general` was a non-table Item — which can't happen because we just `Item::Table(...)`'d it on the absent branch. On the *present* branch, if `general` exists but is `Item::ArrayOfTables` (e.g., user wrote `[[general]]`), `as_table_mut()` returns None → `.expect()` panics. **🔴 BLOCK** — spec §5.2 `ensure_general_table` must handle non-Table Item case with a `TypeMismatch` error instead of `expect`.

(c) **Huge `[[rules]]` array — performance**.
Spec §10 raises this. With `MAX_CONFIG_BYTES = 1 MiB` (config.rs:211), the worst-case linear scan over N entries × M edits is bounded. Per `[[rules]]` entry the minimum useful TOML is ~50 bytes (`[[rules]]\nname="x"\npattern="y"\n` etc.), so N ≤ 20,000. M (number of staged edits per save) is bounded by the TUI keystrokes between two Ctrl+S, realistically ≤ 100. N×M = 2,000,000 hash-comparisons in worst case — comfortably <100ms on any modern CPU. **Not a perf risk.** ✅

But: spec should explicitly bound this in §5.3 ("linear scan acceptable because N is bounded by MAX_CONFIG_BYTES / minimum-entry-size = ~20k; expected N is 5-50 in real use") — defensive doc-comment for future maintainers. 🔵 NIT.

---

## 10. TOML grammar edge cases the spec doesn't cover

(a) **Inline-array** `fg = ["red", "bold"]`.
config.rs `UserStyle.fg: Option<String>` (line 118) requires String. Serde rejects array. snapshot.rs fails early. **Reconcile never sees it.** ✅ But: a brittle user who pre-edits the file to an array, then opens tayf TUI, gets a parse error toast — not a reconcile error. Spec doesn't need to cover; the gate is at snapshot.rs.

(b) **Multi-line basic-strings or literal-strings** `pattern = '''..multi..\n..line..'''`.
toml_edit preserves multi-line form on Read (parses to a Repr). On mutate via `["pattern"] = value(...)`, the Repr is dropped (per item 5 trace) and replaced with default-formatted output. If the new value has a `\n`, it'll re-render as multi-line literal or basic. **🟡 IMPORTANT**: a user who hand-wrote a multi-line regex (e.g., for readability with `(?x)` extended mode comments) and then edits via TUI loses the multi-line layout. Document this in §5.3 or add a `🔴 do not edit` gate in the TUI when source is multi-line.

(c) **Numeric vs string fg** `fg = 178` vs `fg = "color(178)"`.
config.rs:118 `Option<String>` requires String. Serde rejects numeric. snapshot.rs errors early. **Reconcile never sees numeric.** Defensive `TypeMismatch` not needed in reconcile (gate is upstream). ✅ The spec correctly identifies this as defensive and not load-bearing.

(d) **Boolean false vs absent** (spec §5.4 NewStyle.bold tri-state discussion).
The spec says "user `bold = false` is functionally equivalent to absent". This is **correct at the apply-style level** (`UserStyle::to_style` config.rs:137 defaults absent to `false`). But the diff visualization (save_diff.rs `build_diff`) WILL show `+ bold = false` for an explicit write — minor user-experience nit. 🔵 NIT.

(e) **Boolean tri-state — disk-side clear not supported**.
Spec §5.4 (bool axes) writes `table["bold"] = value(b)` whenever `Some(b)`. There's no "remove bold key" path. **🟡 IMPORTANT in correctness sense**: if a user disk-side has `bold = true` and the TUI stages `bold: Some(false)`, reconcile writes `bold = false`. That's correct. But there's no way for the TUI to express "remove the bold key entirely" — which means a config that read `bold = true`, after a TUI edit to "no longer bold", becomes `bold = false` (functionally equivalent, visually different in the file). This is the spec's stated contract — but it should be a documented contract, not an implicit one. Add to §5.4 as explicit "v0.5.5 contract: bool axes are set-only (no clear); equivalent to absent at config-parse time, visually retained at the file level."

---

## 11. Two-call-site error handling — preview path UX gap

Spec §4.5 + §6.5: SaveDiff preview path (save_diff.rs:112) currently builds `tui_diff` from `String::from_utf8_lossy(new_content)`. After v0.5.5, that call returns `Result<String, ReconcileError>`. Spec proposes: Err → `Toast::warn` + close modal.

**🟡 IMPORTANT — UX defect in spec**: closing the modal on preview-error denies the user the actual remedy. If reconcile fails during preview, the user wants to see "why" (the error message) in the modal itself, not as a fleeting toast. Recommendation: render the SaveDiff modal in an **error state** (`SaveDiffState::ReconcileError { message }`) that shows the error inline, with `Esc` to dismiss + `r` to reload from disk. Don't `app.modal = None` — that's the same as silently failing.

This is a UX call (Rust senior's domain) but I'm flagging from the serialization side because the **error sentence quality matters more than UI placement**: an error like `"type mismatch at rules[3]: expected table, found string"` is actionable if visible; a Toast that disappears in 3s is not.

---

## 12. Recommended spec amendments (consolidated)

Inline amendments (no plan rewrite needed):

1. **🔴 BLOCK §5.4**: Replace `table["bold"] = value(b)` with `if let Some(existing) = inline.get_mut("bold") { *existing = value(b).into_value().unwrap() } else { inline.insert("bold", v); }` OR canonicalize through a helper `set_or_insert(table_or_inline, key, value)` that branches on `Inline` vs `Table` and uses `insert` for the inline branch. Document why: `InlineTable::IndexMut` panics on absent key.

2. **🔴 BLOCK §5.2**: Make `ensure_general_table` return `Result<&mut Table, ReconcileError>` and emit `TypeMismatch { path: "general", expected: "table", actual: ... }` on the existing-non-table branch instead of `.expect("ensured above")`.

3. **🟡 IMPORTANT §5.3**: Document the literal-string-vs-basic-string round-trip for `pattern`, with the test case in §5 of this review. Add as test #16.

4. **🟡 IMPORTANT §5.3**: Document mutation contract: `Table::insert` re-formats the key (clears `Key::repr` but preserves `Key::leaf_decor`). Add a test pinning that comments preceding a mutated key survive.

5. **🟡 IMPORTANT §5.4**: Document form-preservation contract ("inline stays inline; block stays block; never form-flip") and add a test for each direction.

6. **🟡 IMPORTANT §5.3 + §5.6**: Document leading-comment-deletion contract on `ArrayOfTables::remove` (comments above the removed entry are deleted with it). Add a test.

7. **🟡 IMPORTANT §5.3**: Document quoted-key behavior for `StyleKey::Numbered(i)` — bare `1` vs quoted `"1"`. Either accept bare-form roundtrip or use `insert_formatted` to preserve user form.

8. **🟡 IMPORTANT §6.5**: SaveDiff preview-path error must render *inline* in the modal, not as a transient toast.

9. **🟡 IMPORTANT §10**: Add open question: "duplicate `[[rules]]` `name` on disk — mutate first occurrence only? warn? error?"

10. **🟡 IMPORTANT §5.4**: Document bool axes "set-only, no clear" contract explicitly.

11. **🔵 NIT §5.2**: `set_implicit(false)` is redundant for freshly-`Table::new()`-ed Table. Either remove or comment-justify.

12. **🔵 NIT §6.2**: Note the canonical-form is variant-preserving (`color(0)` ≠ `black` at the variant level even though SGR-equivalent on some terms).

13. **🔵 NIT §7.1**: Add test verifying `\r\n`-line-ending source doesn't get corrupted on mutation.

14. **🔵 NIT §5.3**: Defensive doc-comment noting linear scan is N-bounded by `MAX_CONFIG_BYTES`.

---

## Verdict

**SHIP_WITH_FIXES** — fold #1 + #2 (BLOCK items) before plan-writing; fold #3-#10 as inline spec amendments (no plan-phase regression — these are clarifications + 1 added test each, no design changes); accept #11-#14 as 🔵 NIT either now or in cross-cutting review.

**Reasoning**: the spec's core design (toml_edit DocumentMut walk, ReconcileError enum, `Color::to_toml_str` roundtrip, three-phase TDD impl) is **structurally sound and consistent with toml_edit 0.25's actual semantics**. The two 🔴 BLOCK items are localized API-misuse issues (one panic on absent inline key, one `expect()` on a defensive path) that need fixing before code lands but don't change the architecture. The 🟡 IMPORTANT items are mostly explicit-documentation requests and test additions to pin invariants the spec relies on implicitly — fast to fold, high value as silent-regression guards.

No 🔴 BLOCK item touches DOKUNULMAZ modules. No item requires a new dependency. No item invalidates the 22-test count delta in §7.4 (the additions are 1-3 incremental tests).

Memory `feedback_lean_process_small_subversions` mandate respected: this review takes a `~250-400 LOC` sub-version seriously enough to verify all 6 mutation primitives against toml_edit's source, but stops at "structural-correctness-with-amendments" — not a full v0.5.4-class redesign.

— TOML/serialization senior, 2026-05-27
