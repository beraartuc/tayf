# Review: tayf v0.3.6 + v0.4 umbrella vision

**Reviewer role:** senior Rust architect (independent)
**Doc under review:** `docs/superpowers/specs/2026-05-24-tayf-v0.3.6-v0.4-vision.md`
**Predecessor:** `docs/superpowers/specs/2026-05-23-tayf-v0.3-vision.md`
**Baseline reference:** `benches/BASELINE.md`, v0.3.5 section (HEAD `e969024`)
**Date:** 2026-05-24

## Verdict

**REVISE.** The umbrella is structurally sound and mirrors v0.3's shape well, but it ships **three load-bearing factual errors** that will mislead the v0.4.0 implementer, **one significant scope omission** (a public `#[doc(hidden)]` re-export of `apply_rules` that the §4.1 BC claim flatly contradicts), and **one infrastructure assumption** (`cargo bench` stdout has machine-readable change data) that is false on the toolchain pinned in `Cargo.toml`. None are fatal, but all must be fixed before the v0.3.6 / v0.4.0 sub-version spec is written, otherwise the spec will inherit the errors.

## Findings

### 🔴 Critical

#### C-1. §3.1 Fix A — the "valid: 1..=0" claim is wrong

**Location:** §3.1, Fix A, paragraph 1 ("Şu an `captures_len == 1` … durumunda mesaj `\"styles.\\\"1\\\": rule's regex has 0 capture groups (valid: 1..=0)\"` üretiyor").

**Problem:** Read `src/error.rs:97-107`. The actual `Display` implementation is:

```rust
let n = captures_len.saturating_sub(1);
write!(
    f,
    "styles.\"{}\": rule's regex has {} capture group{} (valid: 1..={})",
    group,
    n,
    if n == 1 { "" } else { "s" },
    n
)
```

For `captures_len == 1`, `n == 0`, so the rendered string is:

```
styles."1": rule's regex has 0 capture groups (valid: 1..=0)
```

That part of the doc's claim is correct.

**But the implication that this is an "edge case bug" is also right for a different reason than the doc states.** The doc treats `"valid: 1..=0"` as "matematiksel olarak boş aralık, kullanıcıya ne yapacağını söylemiyor." That reading conflates two issues:

1. **The grammar IS already correct** — `if n == 1 { "" } else { "s" }` correctly produces `"0 capture groups"` (plural) and `"1 capture group"` (singular).
2. **The "valid:" suffix is the actual UX bug** — printing `(valid: 1..=0)` is meaningless to users (no integers satisfy that range).

The proposed fix message — `"styles.\"{group}\": rule's regex has no capture groups; styles cannot be set"` — is a sound rewrite for `n == 0`, **but the umbrella must also say what to do when `n >= 1` and `group > n`** (the existing "out of range" case). The current Display handles both cases in one branch. If v0.3.6 only specializes `n == 0`, the `n >= 1 && group > n` branch keeps the existing format, which is fine. But the doc should call this out explicitly, otherwise the sub-version spec author may either (a) over-refactor and break the existing case, or (b) leave the existing message inconsistent in tone with the new one.

**Concrete fix:** In §3.1 Fix A, replace the "valid: 1..=0" framing with: "Şu anki Display tüm vakaları tek branch'le render ediyor; `n == 0` durumunda `(valid: 1..=0)` boş-range çıkıyor. Sub-version'da iki branch'e böl: `n == 0` → 'no capture groups; styles cannot be set'; `n >= 1` → mevcut '0 capture group(s) (valid: 1..=n)' (zaten doğru, dokunma). Pluralization helper bozulmasın." Also drop the phrasing implying the "valid:" part is meaningless in all cases — it isn't.

---

#### C-2. §4.1 BC claim is incomplete — `apply_rules` IS a public surface via `__bench__`

**Location:** §4.1 ("`apply_rules` `pub(crate)` olduğu için refactor kontrollü iç değişiklik — public API surface'i etkilenmez") and §3.2 Bölüm B ("`apply_rules`'un (`pub(crate) fn apply_rules<W: Write>`, `src/pipeline.rs:48`) … iki yaklaşım … (a) free function imzasına `&mut PipelineScratch` parametresi eklemek, veya (b) `apply_rules`'u Pipeline'ın `&mut self` method'una taşımak").

**Problem:** `src/lib.rs:319-348` re-exports `apply_rules` as `pub fn` inside `#[doc(hidden)] pub mod __bench__`:

```rust
pub fn apply_rules<W: Write>(
    line: &[u8],
    rules: &CompiledRules,
    out: &mut W,
) -> std::io::Result<()> {
    crate::pipeline::apply_rules(line, &rules.0, out)
}
```

This is technically public (callable from any downstream crate that knows the path), and `benches/throughput.rs:42` imports it as `use tayf::__bench__::{apply_rules, load_builtin_rules}`. The shim's signature is `(&[u8], &CompiledRules, &mut W)`. The two refactor options the doc proposes break this contract differently:

- **Option (a)** — adding `&mut PipelineScratch` to the free function: the shim must also acquire a `PipelineScratch`. Either (i) the shim hides it (allocates per-call, defeating the optimization for benches — invalidating the v0.4.0 BASELINE numbers themselves), or (ii) `CompiledRules` grows to hold one, changing its shape.
- **Option (b)** — making `apply_rules` a `&mut self` method on `Pipeline`: the free function disappears; the shim must construct a `Pipeline` (which currently requires `Arc<ArcSwap<Compiled>>`), or expose a new `Pipeline::scratch_apply_rules`-style entrypoint. The bench harness changes, the bench numbers shift, and the "zero-regression" floor in §4.2 becomes a comparison against an apple to a slightly different orange.

The doc is silent on this entire surface. It will mislead the v0.4.0 implementer into thinking the scratch refactor is a private rearrangement.

**Concrete fix:** Add a new §4.1 bullet:

> `apply_rules` is also re-exported via `#[doc(hidden)] pub mod __bench__` in `src/lib.rs:342` and is consumed by `benches/throughput.rs`. Either refactor option (a)/(b) must keep the bench-callable shape working *without per-call scratch allocation*, otherwise the v0.4.0 perf numbers measure the shim's allocator, not the scanner. Concrete preference: extend `CompiledRules` (the `__bench__` newtype) to carry the scratch, or expose a `Pipeline::__bench_apply` shim. Sub-version spec must pin the shim contract before any of the four pre-existing scratch Vecs migrate.

Also revise the §3.2 Bölüm B "iki yaklaşım" paragraph to call out the bench-shim consequence of each option.

---

#### C-3. §3.2 Bölüm A — `set.matches_into(line, &mut set_matches)` requires `SetMatches` not `Vec<usize>`

**Location:** §3.2, Bölüm A, step 1: "`let set_matches = pipeline.set_match_scratch.clear(); compiled.set.matches_into(line, &mut set_matches);`"

**Problem:** Two issues with this snippet that will trip the implementer:

1. **`regex::bytes::RegexSet` exposes `matches(haystack) -> SetMatches`, not `matches_into(&mut Vec<usize>)`.** There is no public `matches_into` method on `RegexSet` in regex 1.12 (the version pinned in `Cargo.toml`). The closest equivalent is iterating `SetMatches::iter()` and collecting indices into a caller-owned `Vec`. The doc's snippet won't compile.
2. **`Vec::clear()` returns `()`, not `&mut Vec<T>`.** Even setting aside (1), `let set_matches = pipeline.set_match_scratch.clear();` binds `set_matches` to `()`. The pattern is `pipeline.set_match_scratch.clear(); let set_matches = &mut pipeline.set_match_scratch;` or the call-site does the clear-then-extend in one block.

**Why critical:** The whole RegexSet refactor hinges on this API contact point. If the implementer copies the pseudocode literally, they hit a compile error and waste time deciding whether the umbrella was directional or normative. If they "fix it" by calling `set.matches(line).iter().collect::<Vec<_>>()`, they re-allocate per line and erase Bölüm B's gains.

**Concrete fix:** Rewrite the step 1 snippet to be either pseudocode or correct:

```rust
// pseudocode, not literal code — sub-version spec pins exact shape
pipeline.set_match_scratch.clear();
pipeline.set_match_scratch.extend(compiled.set.matches(line).iter());
let hit_indices = &pipeline.set_match_scratch[..];
```

And add a footnote: "`regex::bytes::RegexSet::matches` returns a bitset-backed `SetMatches`; iterating once costs O(rules). Storing the hits in a `Vec<usize>` adds one bounds check per dispatch versus indexing the bitset directly. Sub-version spec benchmarks both shapes."

---

### 🟡 Important

#### I-1. §3.2 perf targets are over-confident given the input shape

**Location:** §3.2 perf delta table.

**Problem:** The v0.3.5 `apply_rules/ipv4-heavy` time is 2.42 ms on a 67 KB / 1000-line input with **13 patterns total**, of which 3 fire on every line (IPv4, http_status, log_level). The target is ≤ 1.0 ms (≥ 2.4× speedup).

What RegexSet actually saves:

- **What's eliminated:** 10 individual regex scans per line (the 10 patterns that don't match this fixture).
- **What's added:** one `RegexSet::matches(line)` call per line. Internally, `RegexSet` compiles the patterns into a single NFA/DFA-style multi-pattern matcher (regex-automata's meta engine), so the per-line cost is **not** "sum of 10 individual scans" — it's closer to "one scan that reports which subset hit." Still, the actual ratio depends entirely on the patterns' compiled shape (literal prefixes, anchored, length).
- **What dominates the remaining 2.42 ms:** the 3 rules that DO hit. RegexSet doesn't speed those up; they still run `find_iter`/`captures_iter` on the line.

If the 3 hitting rules account for, say, 60% of the 2.42 ms (consistent with the v0.3.5 BASELINE note that "the synthetic input here hits the worst-case path (three rules match)"), then RegexSet can at best take 2.42 ms → ~1.4 ms (eliminating the 10 misses, paying the set-scan cost). The ≤ 1.0 ms target requires the 10 misses to currently cost **>1.4 ms** out of 2.42 ms, plus the set-scan cost to be near-zero — both shaky assumptions on a 13-pattern set where most patterns are short literal-rich (`ERROR|FAIL|FATAL`).

**Why important, not critical:** The umbrella explicitly says targets are pinned in the sub-version spec ("Sub-version spec'i kesin sayıları pin'ler. **Floor:** hiçbir bench grup'unda v0.3.5'ten regression olmayacak."). So the headline numbers won't gate the release. But publishing ≤ 1.0 ms / ≤ 0.8 ms / ≤ 2.5 ms in an umbrella that defers to JIT pinning creates the wrong expectation: a v0.4.0 that lands at 1.5 ms / 1.1 ms / 3.2 ms ships against floor but reads as a miss against the umbrella.

**Concrete fix:** Reframe §3.2's table as **directional bands**, not point targets:

| Shape | v0.3.5 | v0.4.0 band | Notes |
|---|---|---|---|
| `ipv4-heavy` | 2.42 ms | 30-60% reduction (≈ 1.0-1.7 ms) | hit-heavy; saved work bounded by 10 misses, not 13 |
| `mixed-syslog` | 2.29 ms | 40-70% reduction | half-and-half hit ratio, RegexSet better leverage |
| `captures-heavy` | 4.52 ms | 10-20% reduction | every captures rule fires; RegexSet pre-filter mostly redundant |

Add one sentence: "Numbers below the upper band ≠ regression; sub-version spec calibrates to measurement, not umbrella prediction." This protects against the umbrella becoming an unfalsifiable promise.

---

#### I-2. §4.4 "Aho-Corasick fallback" claim needs a measurement gate, not an assertion

**Location:** §1.2 ("`regex` crate'in heuristic'i pure-literal alternation'da (`ERROR|FAIL|FATAL|CRITICAL`) zaten internal Aho-Corasick DFA'sına düşüyor; bizim ekleyeceğimiz routing duplicate iş olur") and §4.4 ("Aho-Corasick explicit dep'i değerlendirildi, **iptal** edildi — `regex` crate'in pure-literal alternation handling'i internal AC DFA'sına düşer").

**Problem:** This claim is **mostly true in current regex versions** (regex-automata's meta engine does fall back to multi-pattern literal optimizations including a Teddy-style SIMD prefilter, and Aho-Corasick is reachable for some literal-heavy shapes). But the umbrella states it as a rationale-clinching certainty, when in practice the heuristic has known **non-triggering edge cases**:

- Case-insensitive flag (`(?i)ERROR|FAIL`) often does NOT take the pure-literal Teddy path (it's reachable but conditional on the unicode case-folding state).
- Word-boundary anchors (`\bERROR\b|\bFAIL\b`) sometimes prevent the literal-only path because `\b` is not a literal.
- The `log_level` builtin in tayf currently uses **what shape**? The umbrella doesn't verify. If it's `\b(ERROR|FAIL|FATAL|CRITICAL|WARN|INFO|DEBUG)\b`, the `\b` likely keeps it on the NFA path, and the "AC fallback is automatic" claim doesn't apply to this exact pattern.

So: rejecting an explicit `aho-corasick` dep is the right call (it's a transitive dep of `regex` already, and a manual literal router would duplicate work the engine already does on most patterns). But the rationale phrasing in §1.2 / §4.4 commits to an automatic-fallback story that may not hold for tayf's actual patterns.

**Concrete fix:** In §4.4, soften the rationale:

> Aho-Corasick explicit dep değerlendirildi, **iptal** edildi. `regex` crate'in meta-engine'i pure-literal alternation'da Teddy/AC tabanlı multi-pattern prefilter'a düşebilir; manuel routing eklemek hem duplicate iş hem de bizim patternlerin (`\b`, `(?i)`) hangi prefilter yoluna düştüğünü ölçmeden kararı verir. v0.4.0 RegexSet ship sonrası BASELINE.md eğer `log_level`-dominated fixture'da hâlâ hot-loop görüyorsa, v0.5+'da yeniden açılır — explicit dep yerine pattern shape değişikliği (örn. `?-u` flag, `\b` kaldırma) öncelikle değerlendirilir.

Same softening in §1.2.

---

#### I-3. §3.3 v0.4.1 — `cargo bench` stdout is NOT a stable machine-readable surface

**Location:** §3.3 ("`cargo bench --bench throughput -- --baseline <last_release>`; criterion'un `change:` line'larını parse → %20 spec threshold breach varsa PR comment") and §4.4 ("Yeni dep yok — criterion zaten dep, GitHub Actions runner'lar zaten konfigüre").

**Problem:** Vanilla `cargo bench` invokes the criterion 0.x runtime, which emits human-formatted change lines like:

```
                 change: time:   [+0.62% −0.36% +1.20%] (p = 0.01 < 0.05)
                        Change within noise threshold.
```

The format is not stability-guaranteed. The actual stable machine-readable JSON output (the `benchmark-complete` message format with the `change` field including a `change` enum of `NoChange`/`Improved`/`Regressed`) is emitted only by **`cargo-criterion`**, a separate binary (`cargo install cargo-criterion`) which is **not pinned in `Cargo.toml`**.

So the "no new dep" claim is borderline:

- If the workflow installs `cargo-criterion` in CI, that's not a Cargo dependency (it's a CI tool), so the "no new dep" claim technically holds. But the umbrella doesn't mention it.
- If the workflow parses `cargo bench` stdout, that's a fragile contract — criterion has changed its stdout format between 0.4 / 0.5 / 0.8 minor versions in the past, and the parser will silently break on the next bump.
- The third option — parsing `target/criterion/<bench>/<name>/change/estimates.json` (criterion's per-bench on-disk artifact) — is reasonably stable but not officially documented as a public surface.

**Concrete fix:** In §3.3, replace the stdout-parsing hand-wave with a sub-version decision point:

> Sub-version spec'i karşılaştırma surface'ini pin'ler. Üç seçenek:
> (a) `cargo-criterion` install (workflow-level tool; "no new Cargo dep" hâlâ doğru ama README dev-setup section'ında belirtilir);
> (b) `target/criterion/**/estimates.json` artifact parsing (officially undocumented ama stabil; sub-version'da regression test ekle);
> (c) `cargo bench` stdout parsing (en kırılgan; reddedildi).
> Tercih (b), `cargo-criterion`'ın transitive surface'inden kaçınmak için. v0.4.1 spec'i bu seçimi normative yapar.

And in §4.4, replace "Yeni dep yok" with "Yeni Cargo dep yok; v0.4.1 sub-version'da seçilen workflow tool (`cargo-criterion` veya yerleşik script) Cargo manifest'ine girmez."

---

#### I-4. §4.2 "no regression on any bench group" floor needs a noise definition

**Location:** §4.2 ("v0.4 boyunca **hiçbir bench grup'unda v0.3.5'ten regression olmayacak** (floor)") and §3.3 ("%20 spec threshold breach varsa PR comment + workflow annotation. **Auto-fail değil** (sub-µs jitter koruması)").

**Problem:** The two statements coexist uncomfortably. §4.2 says "no regression"; §3.3 says "20% threshold, annotation-only." The reader has to infer that "regression" in §4.2 means ">20% deviation from v0.3.5 baseline," but `passthrough/write_all` is documented in BASELINE.md as swinging ±14% between releases purely from sub-µs scheduler jitter. A literal "no regression on any bench group" floor is **already violated** by v0.3.4 → v0.3.5 (`passthrough/write_all` +14.81% time / -12.91% throughput).

So the floor as written is unenforceable. Either:

- Interpret strictly: v0.4.0 must beat v0.3.5 on `passthrough/write_all`, which is below noise control of the implementer.
- Interpret loosely: "no regression beyond noise band," but the noise band differs per bench group (sub-µs benchmarks much wider than ms benchmarks).

**Concrete fix:** In §4.2, replace the one-line floor with a per-group budget:

> Per-bench-group floor (v0.4 boyunca):
> - `apply_rules/*` (ms-scale): v0.3.5'ten >5% slower → review gate, >20% slower → block.
> - `passthrough/write_all` (sub-µs): v0.3.5'ten >25% slower → review gate (historical noise band is ±15%).

And in §3.3, clarify that the "20% threshold annotation" is the workflow's coarse single-number rule, NOT the §4.2 contract — the two policies operate at different layers (CI annotation vs release-blocking review).

This also addresses the "test coverage gaps / canonical pinning set" concern from the brief: explicitly say which bench rows are hard-gate vs soft-gate.

---

#### I-5. §3.2 ArcSwap hot-reload semantics under Pipeline-owned scratch

**Location:** §3.2 Bölüm B and §4.1.

**Problem:** The current `apply_rules` does `let snapshot: Arc<Compiled> = compiled_handle.load_full();` at the top of every call (`src/pipeline.rs:53`), then borrows `compiled.individuals[i]`, `compiled.styles[i]`, `compiled.group_styles[i]` for the duration of the call. The reloader thread can swap a new `Compiled` mid-stream; the in-flight call holds its `Arc` and finishes against the old snapshot. This is the core hot-reload safety invariant.

If `apply_rules` becomes a `Pipeline` method (option b in §3.2 Bölüm B), the scratch Vecs live in `Pipeline` (Pipeline is `&mut self` per-call inside `apply_or_passthrough`, fine). But if the scratch Vecs hold borrowed references into the snapshot (e.g., `runs: Vec<(usize, usize, &Style)>` is the current shape — note `&Style` borrows into `compiled.styles[i]`), the borrow lifetime must end before the next `load_full()` snapshot is taken. With per-call scratch this is automatic (the Vec drops on return); with Pipeline-owned scratch, the implementer must `clear()` the Vec **before** the next `load_full()` to avoid a lifetime mismatch between scratch (`'compiled_old`) and the new snapshot.

This is a real compile-time concern, not a runtime one — Rust's lifetimes will catch the mistake, but it constrains the refactor: `runs: Vec<(usize, usize, &'a Style)>` cannot be `Pipeline`-owned if `'a` borrows from a per-call snapshot. The fix is to store `Style` by value (cheap, ~16 bytes) or to store the rule index `(usize, usize, u16)` and look up the style on emit.

The doc waves at this with "Pipeline-owned scratch Vec'leri" but doesn't acknowledge the lifetime constraint.

**Concrete fix:** Add a sub-bullet to §3.2 Bölüm B:

> Lifetime constraint: current `runs: Vec<(usize, usize, &Style)>` borrows into the per-call `Arc<Compiled>` snapshot. Pipeline-owned scratch must either (a) store `Style` by value (small enough), (b) store `(start, end, rule_idx)` and resolve to `&Style` at emit time, or (c) keep `runs` per-call and only Pipeline-own the index/event scratches. Sub-version spec picks one; (b) is preferred (1 byte vs ~16, no Style clone semantics question).

Note that `event_scratch: Vec<(usize, OpenClose, u32)>` and `active_scratch: Vec<u32>` and the new `set_match_scratch: Vec<usize>` are all already lifetime-free — they migrate cleanly. Only `runs` and `accepted_spans` (which is `Vec<(usize, usize)>` — also lifetime-free, fine) need attention; `runs` is the only one with a real constraint.

---

### 🔵 Nits

#### N-1. §1.2 / §4.4 — Aho-Corasick already IS a transitive dep

**Location:** §1.2 ("Aho-Corasick explicit dep + manuel literal routing") and §4.4.

**Problem:** Phrasing implies adding Aho-Corasick would be a *new* crate. It's already in `Cargo.lock` as a transitive dep of `regex`. The decision is whether to make it a *direct* dep (and use it via its own API for hand-rolled literal routing), not whether to add it.

**Concrete fix:** Replace "Aho-Corasick explicit dep" with "Aho-Corasick **direct** dep" or "Aho-Corasick *direkt* crate kullanımı (zaten `regex`'in transitive dep'i)."

---

#### N-2. §2 dependency-graph note — "paralel ship edilebilir teorik olarak" overstates

**Location:** §2, table footer ("v0.4.0 → v0.3.5 (Pipeline shape, captures dispatch invariant'ı korunmalı). v0.3.6'ya bağımlı değil — paralel ship edilebilir teorik olarak").

**Problem:** v0.3.6 (a patch on the v0.3 line) and v0.4.0 (a minor) can't be "shipped in parallel" in a meaningful sense — they target different release lines, and shipping v0.4.0 before v0.3.6 means v0.3.6's bugfix is also in v0.4.0 (it would be a regression for v0.4.0 to NOT carry the v0.3.6 fix). The note is technically about commit independence, but reads like release-line independence.

**Concrete fix:** Rephrase: "v0.4.0 → v0.3.5 (Pipeline shape, captures dispatch invariant'ı korunmalı). v0.3.6 patch'i v0.4.0 koduna da cherry-pick'lenir (forward-port mandatory); commit'ler bağımsız ama ship sırası v0.3.6 → v0.4.0 zorunlu."

---

#### N-3. §3.1 Fix B — test fix preference (a) vs (b) is decided, but framed as open

**Location:** §3.1 Fix B paragraph 2 and §6 question 1.

**Problem:** §3.1 already states "Tercih (a) çünkü syslog branch'in 1 vs N SGR sayısı `log_level` rule'ının fixture'da hit etmesine bağlı — substring survival contract'ı daha net." §6 then re-opens it as an "açık soru." Pick one — if §3.1 commits, §6 should drop it; if §6 is the real decision point, §3.1 shouldn't preempt it.

**Concrete fix:** Drop §6 item 1; the §3.1 reasoning is already strong enough to settle it at umbrella level. Or, conversely, downgrade §3.1's "Tercih (a)" to "Sub-version brainstorming'inde finalize" and keep §6 as the decision gate.

---

#### N-4. §3.2 file path off-by-one

**Location:** §3.2 Bölüm B ("`apply_rules`'un (`pub(crate) fn apply_rules<W: Write>`, `src/pipeline.rs:48`)").

**Problem:** Verified — `src/pipeline.rs:48` is correct (`pub(crate) fn apply_rules<W: Write>(` is on line 48). The doc's other line-number citations (§3.1 Fix B `src/pipeline.rs:539`, `src/pipeline.rs:546`; `tests/integration_capture_groups.rs:165`; §3.2 Bölüm A `src/rules.rs:549-550`) also verified accurate. Nothing to fix here — but the brief asked me to verify, so confirming: **all cited line numbers are correct as of HEAD `e969024` / v0.3.5.**

---

#### N-5. §3.2 Bölüm B — `accepted_spans` is missing from the migrated-list narrative

**Location:** §3.2 Bölüm B paragraph 1 ("her çağrısında allocate edilen 4 Vec — `accepted_spans`, `runs`, `event_scratch`, `active_scratch` — Pipeline struct'a field olarak taşınır + yeni `set_match_scratch` eklenir").

**Problem:** Reads cleanly, but the order conflicts with `apply_rules`'s actual variable order (`accepted_spans`, `runs`, `event_scratch`, `active_scratch` — yes, matches). False alarm on the list itself, but the **5th scratch — the `sources: Vec<String>` allocated in `compile_merged_rules` (`src/rules.rs:841`) on every reload — is per-reload not per-line**, so it correctly does NOT belong in this list. The umbrella might want to add a one-line "non-goal: per-reload allocations untouched in v0.4.0" to forestall scope creep in sub-version brainstorming.

**Concrete fix:** Append to §3.2 Bölüm B: "**Scope sınırı:** v0.4.0 sadece per-line scratch'leri Pipeline-owned yapar. Per-reload (`compile_merged_rules`'un `sources`, `individuals`, `styles`, `group_styles` allocate'i) ve startup allocate'leri v0.4.0 scope dışı."

---

#### N-6. §5 sıralama önerisi diagram — v0.4.1'den sonra "v0.4 done → v0.5 brainstorming" lonely block

**Location:** §5 ASCII diagram.

**Problem:** v0.3 umbrella's diagram ends with "v0.3 done → v0.4 (RegexSet fast-path) brainstorming." This umbrella ends with "v0.4 done → v0.5 brainstorming." Same shape, but v0.5 has no agreed-upon scope referenced — it could be capture-group naming (§1.2), profile system, or `tayf config` TUI. Add one bracketed hint for symmetry.

**Concrete fix:** "v0.4 done → v0.5 (capture-group naming / profile system / config TUI — umbrella sub-version brainstorming) brainstorming."

---

#### N-7. §4.3 security claim about RegexSet attack surface

**Location:** §4.3 ("v0.4.0 RegexSet — `regex` crate zaten dep, RegexSet API'si aynı crate'in stable parçası, yeni surface area yok").

**Problem:** Mostly true, but worth one sentence on a real concern: **ReDoS exposure is per-pattern, and `RegexSet` is constructed once at startup from the same pattern strings**. So RegexSet adds no new attacker-controlled input path. **But** the umbrella should note: `RegexSet::new` itself can fail on aggregate size limits (the codebase already handles this at `src/rules.rs:877` via `.map_err(Error::from)`). The hot path is fine; the only "new surface" is at compile time, and it's already error-handled.

**Concrete fix:** Append to §4.3 RegexSet bullet: "RegexSet compile'da aggregate size cap'i (regex `RegexSet::new`'in default'u) tetiklenebilir; `src/rules.rs:877` zaten `.map_err(Error::from)` ile handle ediyor. Hot path ReDoS exposure değişmez — set bir bütün olarak `regex` crate'in lineer-time engine'ini kullanır."

---

#### N-8. §3.2 Bölüm A — RegexSet cross-rule ordering invariant unstated

**Location:** §3.2 Bölüm A paragraph 2.

**Problem:** The doc says "Sonra `compiled.individuals` üzerinde tam tarama yerine sadece `set_matches` içindeki index'ler için dispatch. Selective dispatch'ın v0.3.5'te kurulan dual-path mantığı … **aynen korunur**, sadece dış loop kısa." This is correct in spirit. But it's worth one sentence on the ordering invariant: `apply_rules`'s outer loop is **`for (i, re) in compiled.individuals.iter().enumerate()`** — rule definition order, which determines "first match wins" overlap resolution (see `apply_rules` doc-comment at line 39). `RegexSet::matches` returns hits in pattern-index order (which is the same as rule-definition order, since `sources` is built in rule-definition order at `src/rules.rs:841,859`). So iterating `set_matches.iter()` preserves the invariant.

Worth saying because a naïve implementer might iterate a `HashSet<usize>` (unordered) and silently break first-match-wins.

**Concrete fix:** Append to §3.2 Bölüm A paragraph 2: "Cross-rule ordering invariant: `RegexSet::matches` index'leri pattern-definition order'ında verir (`sources` builds in rule order at `src/rules.rs:841`); `apply_rules`'un first-match-wins overlap kontratı korunur. **Implementer notu:** iteration order'ı bozan bir container (HashSet, BTreeSet) ARAYA SOKULMAZ — `SetMatches::iter()` veya sorted `Vec<usize>` zorunlu."

---

## What the doc gets right

Calibration list — these are the parts the rev2 author should **not** touch:

1. **Scope bundling rationale (§ preamble).** Tying v0.3.6 + v0.4.0 + v0.4.1 under one umbrella because the v0.3.5-deferred scratch-Vec item bridges v0.3.6's bugfix discipline and v0.4.0's RegexSet refactor is a sound call. Two separate umbrellas would have made the scratch-Vec ownership a homeless decision.
2. **§1.2 deferred-list discipline.** Pulling Aho-Corasick / streaming heuristics out and naming them as deferred (with rationale) is much better than the v0.3 umbrella's looser "v0.5+" hand-wave. The "measurement gates open it back up" framing in I-2 is the only refinement needed.
3. **§3.1 v0.3.6 scope minimalism.** "Hot path'e dokunulmaz, BASELINE.md değişmez. `Compiled.set` hâlâ unused." — exactly the right level of restraint for a pure-bugfix patch. The implementer can't accidentally inflate v0.3.6 into a perf release with this framing.
4. **§4.5 zero-regression invariant for capture-group tests.** Singling out `tests/integration_capture_groups.rs` as the byte-identical pin set is correct. The v0.3.5 BASELINE specifically called out C-1 (the `accepted_spans` / `partition_point` regression class); the umbrella inherits that vigilance.
5. **§5 sequencing rationale.** v0.3.6 first (small, fast, calms the v0.3 line) → v0.4.0 (headline) → v0.4.1 (needs v0.4.0 baseline) is the right order, and the "pause noktaları" framing keeps the option to stop after v0.4.0 open.
6. **§7 "this doc edited when…" governance.** Identical to v0.3 umbrella, identical to good practice — sub-version conflicts must update the umbrella in a separate commit, not silently drift.
7. **§3.2 Bölüm A reading of the v0.3.5 invariants.** The substantive claim that capture-group styling's `accepted_spans` + `partition_point` + `emit_capture_runs` algorithm is RegexSet-compatible **is correct** (verified by tracing `src/pipeline.rs:64-94` — RegexSet just trims the outer-loop indices; the inner per-rule dispatch is byte-identical). The brief asked me to verify this; confirming, no cross-rule ordering invariant is violated as long as the iteration order of `set.matches(line)` is preserved (see N-8).

---

## Summary

- **🔴 Critical:** 3 (C-1 wrong "valid: 1..=0" framing; C-2 missing `__bench__` public-surface acknowledgement; C-3 broken `matches_into` API call in pseudocode)
- **🟡 Important:** 5 (I-1 over-confident perf targets; I-2 AC fallback claim too absolute; I-3 fragile `cargo bench` stdout parsing; I-4 unenforceable per-group floor; I-5 ArcSwap lifetime constraint on Pipeline-owned `runs`)
- **🔵 Nits:** 8 (N-1 transitive vs direct dep phrasing; N-2 release-line vs commit independence; N-3 Fix B duplicate decision point; N-4 confirmation that all cited line numbers verified accurate; N-5 scope-creep guard for per-reload allocations; N-6 v0.5 hint for symmetry; N-7 RegexSet compile-time error handling; N-8 RegexSet iteration-order invariant)

Total: **16 findings**. The umbrella's bones are good; rev2 is a tightening pass, not a rewrite. After C-1/C-2/C-3 are fixed and I-1/I-3/I-5 acknowledged, the v0.3.6 + v0.4.0 sub-version brainstorms can start with confidence.
