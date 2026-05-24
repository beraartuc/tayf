# Review: tayf v0.4.0 spec

**Reviewer:** Senior Rust architect (opus 4.7, 1M ctx — pre-implementation pass)
**Spec under review:** `/Users/bera/tayf/docs/superpowers/specs/2026-05-24-tayf-v0.4.0-regexset-fastpath.md`
**Umbrella:** `/Users/bera/tayf/docs/superpowers/specs/2026-05-24-tayf-v0.3.6-v0.4-vision.md` §3.2 + §4
**Genesis:** v0.3.7 final cross-cutting review I-1 (`docs/superpowers/reviews/2026-05-24-rust-senior-v0.3.7-final-cross-cutting-review.md`) — ZeroForbidden UserConfig arm carryover.
**Verification basis:** spec citasyonları byte-for-byte tree'ye karşı okundu; regex 1.12.3 RegexSet API'si docs.rs + GitHub kaynaktan doğrulandı; `regex::bytes::RegexSet::matches` her çağrıda `PatternSet::new()` allocate ediyor (regex_automata kaynak kodundan doğrulandı — semantik bir nokta, aşağıda I-3).

---

## Verdict — APPROVE_WITH_REVISIONS

Spec mimari olarak doğru, fix doğru shape'te, RegexSet API çağrıları regex 1.12 stable contract'ıyla uyumlu, capture-group zero-regression invariant'ı pin'lendi, divergence rationale'leri (umbrella §3.2 Bölüm B'den (b) ve (ii) tercihinden ayrılma) umbrella §7 mandatesi gereği §10'da açıkça not edildi ve teknik olarak da haklı. Ancak rev2 öncesi üç orta-büyüklük (🟡) konunun çözülmesi gerekiyor — ikisi commit-ordering tabanlı pre-commit gate breach'leri, biri test surface'in mekanik migration'ında gözden kaçmış bir `std::ptr::eq` kalıbı (Style by-value migration'ında derleme hatası verir).

---

## Sanity-check ettiğim spec iddiaları

| Spec iddiası | Doğrulandı |
|---|---|
| `Compiled.set: RegexSet` populated-but-unused, `#[allow(dead_code)]` notu `src/rules.rs:548-549`'da | ✅ |
| `apply_rules` mevcut imza ve dış loop shape (`src/pipeline.rs:48-116`) | ✅ |
| `emit_capture_runs` mevcut `'r` lifetime imzası ve hot path arm'ı (`src/pipeline.rs:161-219`) | ✅ |
| `Pipeline` struct field listesi (`src/pipeline.rs:227-242`) | ✅ |
| Bench shim `__bench__::apply_rules` (`src/lib.rs:319-349`) ve `benches/throughput.rs:42` callsite | ✅ |
| ZeroForbidden UserConfig arm `src/rules.rs:923-947` inline format | ✅ |
| `ThemeRuleErrorKind::CaptureGroupIndexZeroForbidden` Display `src/error.rs:94-96` | ✅ exact match |
| OutOfRange UserConfig delegation pattern `src/rules.rs:985-995` (v0.3.7 shipped) | ✅ — Fix Z dahi aynı şekli alır |
| Style struct shape: `Copy + Default`, 2× `Option<Color>` + 4 bool (`src/style.rs:391`) | ✅ |
| `Color: Copy` (`src/style.rs:9`) | ✅ |
| `Compiled::empty()` `RegexSet::empty()` döner (`src/rules.rs:585-594`) | ✅ |
| `regex::bytes::RegexSet::matches(haystack: &[u8]) -> SetMatches` (regex 1.12 stable) | ✅ docs.rs/regex/1.12.3 |
| `SetMatches::iter()` pattern-definition order'ında `usize` yield eder | ✅ "ascending with respect to the index of the regex that matched with respect to its position when initially building the set" |
| `matches_into(&mut Vec)` yok | ✅ — yalnızca `matches`, `matches_at`, `is_match`, `is_match_at` |
| `RegexSet` `sources` order'ında inşa edilir (`src/rules.rs:859-877`) → pattern-index `individuals[i]` ile birebir | ✅ |
| `Cargo.toml` version `0.3.7`, regex `1.12` | ✅ |
| Lib config: `#![warn(clippy::pedantic, clippy::cargo)]` (`src/lib.rs:41`) | ✅ |

Public surface check: `PipelineScratch` ve `BenchScratch` ikisi de internal (`pub(crate)` ve `#[doc(hidden)]`); `Pipeline::new` imzası değişmez; `Args`/`Error`/`ThemeRuleErrorKind` enum'ları dokunulmaz; `Compiled` struct shape stable (sadece `#[allow]` notu çıkar). **Public API impact sıfır** — spec §2.7 doğru.

---

## Findings

### 🔴 Critical

**Yok.** RegexSet API doğru, ordering invariant'ı korunuyor, capture-group emit logic dokunulmuyor, sanitize_for_display ve `Error::Config` `Display` path'leri etkilenmiyor, public API byte-stable.

### 🟡 Important

#### I-1. Mevcut `emit_capture_runs` unit testlerinde `std::ptr::eq` kullanımı Style by-value migration'ında derleme hatası verir — spec §2.3 bu kalıbı yakalamamış

**Lokasyon:** spec §2.3 "Test gövdeleri — `emit_capture_runs` unit test'leri" paragrafı:
> "Assertion'ların shape'i `(start, end, &Style)` → `(start, end, Style)` — `assert_eq!` ile pattern match aynen."

**Sorun.** Spec sadece `runs` Vec type'ının değişimini ve `default_style` borrow shape'ini değiştirmeyi anlatıyor. Ama mevcut testlerde 8 yerde **pointer-identity** assertion'ı var:

```
src/pipeline.rs:673: assert!(std::ptr::eq(runs[0].2, &default));
src/pipeline.rs:692: assert!(std::ptr::eq(runs[0].2, group_styles[0].as_ref().unwrap()));
src/pipeline.rs:732: assert!(std::ptr::eq(runs[0].2, group_styles[0].as_ref().unwrap()));
src/pipeline.rs:736: assert!(std::ptr::eq(runs[1].2, group_styles[1].as_ref().unwrap()));
src/pipeline.rs:740: assert!(std::ptr::eq(runs[2].2, group_styles[0].as_ref().unwrap()));
src/pipeline.rs:756: assert!(std::ptr::eq(runs[0].2, &default));
src/pipeline.rs:774: assert!(std::ptr::eq(runs[0].2, &default));
src/pipeline.rs:796: assert!(std::ptr::eq(runs[1].2, &default));
```

`std::ptr::eq` `(*const T, *const T)` ister; `runs[N].2` bir `Style` value oldu — `std::ptr::eq(<Style>, &Style)` tip uyuşmazlığı. **Mechanical edit ≠ compile-clean.**

**Fix.** Spec §2.3'e bir maddi paragraf ekle:

> Bu testlerdeki `std::ptr::eq(runs[N].2, &slot)` assertion'ları `assert_eq!(runs[N].2, slot)` (Style: `Copy + PartialEq`) ile değiştirilir. Eski test pointer-identity'sini (aynı `Compiled.styles[i]` slot'undan geldiğini) garanti ediyordu; yeni test value-equality garantiler. Pointer-identity argumantı v0.3.5'te `Compiled` snapshot'ına `&'r` borrow ile geldiği için anlamlıydı; Style by-value sonrası slot identity'si emit'e ulaşmıyor — value identity yeterli kontrat, hatta daha **anlamlı** (slot identity'sinin user-facing önemi yok; emit edilen `Style` value'sinin önemi var).

**Şiddet rasyonali:** Bu spec §9 commit ordering'inde commit 2 ("`refactor(pipeline): introduce PipelineScratch and thread through apply_rules` — §2.2; ayrıca §2.3 Style by-value değişikliği bundle") pre-commit `cargo test`'i mekanik olarak başarısız olur. Subagent task'ı "byte-identical mechanical edit" sanısıyla migration yaparsa 8 derleme hatası ile karşılaşır. Spec §2.3 paragrafının test gövdesinde "`assert_eq!` ile pattern match aynen" cümlesi yanıltıcı — `std::ptr::eq` testleri `assert_eq!` değil. CLAUDE.md §4 "Tests with the feature" mandatesi: migration test surface'i spec'te eksiksiz olmalı.

#### I-2. Spec §9 commit ordering'i bench surface'i (§2.6) ile sub-version test surface'ini (§2.2/§2.3) ayırmış — pre-commit gate'i ihlal eden ara state yaratır

**Lokasyon:** spec §9 release ceremony adımları 2, 3, 4 (commit-level ayrım).

**Sorun.** Commit ordering şu sırayla:

- **Commit 2.** `refactor(pipeline): introduce PipelineScratch and thread through apply_rules` (§2.2 + §2.3).
- **Commit 3.** `feat(pipeline): RegexSet pre-filter fast-path in apply_rules` (§2.4 + §2.5).
- **Commit 4.** `chore(bench): hoist BenchScratch outside b.iter` (§2.6 — bench shim + `benches/throughput.rs`).

Commit 2'de `pipeline::apply_rules` imzasına `scratch: &mut PipelineScratch` eklenir. **Bench shim `src/lib.rs:347` (`crate::pipeline::apply_rules(line, &rules.0, out)`)** ve **`benches/throughput.rs:64,93,120`** (`apply_rules(black_box(...), &compiled, &mut out)`) tüm bu üç bench callsite'ı derleme hatası verir.

Spec §5.5 (acceptance gate) bağlayıcı: "Pre-commit gate: `cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test`". `--all-targets` benches'i kapsıyor; `cargo test` benches'in derlemesini gerektiriyor. **Commit 2 ve Commit 3 ara state'inde gate kırılıyor.** Spec §9 son paragrafı "Yarım-broken state HOC'a girmemeli" diyor ama tam olarak bunu sub-version-içi commit boundary'sinde yaratıyor.

Üç olası çözüm:

- **(a)** Commit 4'ü Commit 2 ile birleştir: tek atomic commit "introduce PipelineScratch + Style by-value + thread through apply_rules and bench shim". Bench file güncellemesi (`benches/throughput.rs`) bu commit'in parçası. RegexSet pre-filter (Commit 3) hâlâ ayrı atomic — yeni semantik (set hit iteration) bağımsız bisect edilebilir.
- **(b)** Commit 4'ü Commit 2'den önce taşı: "bench shim accepts scratch parameter (no-op)" + Commit 2 "apply_rules signature uses it" + Commit 3 RegexSet. Daha karmaşık (no-op değişiklik yapmak için bench shim signature'ı önceden değişir); kabul edilebilir ama (a) daha temiz.
- **(c)** Commit 2'yi "WIP" toleransıyla geç + Commit 4 hemen ardından + final gate Commit 4 sonrası tek seferde. Kabul edilemez — sub-version her commit'inin atomic-ship-state olması umbrella §4.6 + memory `feedback_cross_cutting_review_value` precedent.

**Öneri: (a).** Spec §9'u şöyle güncelle:

> 2. `refactor(pipeline): introduce PipelineScratch, Style by-value, and bench shim` — §2.2 + §2.3 + §2.6 atomic bundle. `apply_rules` imza değişimi tek commit'te tüm caller'ları (production, unit test, bench shim, bench harness) günceller. Pre-commit gate clean ara state üretmek için tek atomic.
> 3. `feat(pipeline): RegexSet pre-filter fast-path in apply_rules` — §2.4 + §2.5. Bağımsız feature, ayrı bisect surface.

Commit 5 (yeni unit testler) hâlâ ayrı kalabilir — testler scratch infra'sını kullanır, infra Commit 2'de tamam.

**Şiddet rasyonali:** Spec §9 mevcut hâliyle subagent-driven plan execution'da iki ardışık task pre-commit gate'i kıracak. "Mechanical edit, byte-identical behavior" beklentisiyle Commit 2'yi merge'lemeye çalışan reviewer hem `cargo test --all-targets` hem `cargo clippy --all-targets`'tan red alır.

#### I-3. `RegexSet::matches()` her çağrıda heap-allocates — spec'in "per-line zero-allocation contract" iddiası teknik olarak yanlış; mitigation ve test name yeniden ifade edilmeli

**Lokasyon:** spec §1 ("Allocator pressure satır başına ms-scale değil µs-scale ama miss-heavy fixture'larda set-scan sonrası iş azaldığında allocator katkısı oransal büyür"), §2.6 ("scratch must be hoisted outside `b.iter` ... yoksa v0.4.0 BASELINE allocator'ı ölçer"), §5.2 item 1 (test adı `pipeline_scratch_is_reused_across_apply_rules_calls` + "per-line zero-allocation contract").

**Sorun.** `regex::bytes::RegexSet::matches(haystack)` her çağrıda `PatternSet::new(self.meta.pattern_len())` allocate eder (regex_automata 0.4 kaynağı, `regex/src/regexset/bytes.rs` `matches_at`):

```rust
pub fn matches_at(&self, haystack: &[u8], start: usize) -> SetMatches {
    let input = Input::new(haystack).span(start..haystack.len());
    let mut patset = PatternSet::new(self.meta.pattern_len());
    self.meta.which_overlapping_matches(&input, &mut patset);
    SetMatches(patset)
}
```

`PatternSet` `regex_automata::util::primitives::PatternSet` — internal `Vec<usize>` (alloc-feature-gated bitset). Pattern count = ~13 (built-ins), allocation small ama **sıfır değil**.

Spec'in iki iddiası bu gerçekle çelişiyor:

1. §5.2 item 1 test'i `cap_after_first == cap_after_second` invariant'ını "per-line zero-allocation contract" olarak çerçeveler. Test PipelineScratch capacity invariant'ı için doğru, ama "zero-allocation per-line" implication'ı yanlış — `RegexSet::matches` her satır bir küçük `Vec<usize>` allocate ediyor. Test ismi okuyucuyu yanıltır.
2. §2.6 bench shim rationale "scratch hoist'lanmazsa v0.4.0 BASELINE allocator'ı ölçer" — geçerli ama eksik; pre-filter'ın kendisi de fixed-cost allocator pressure ekliyor (~13 entry × 2 word = ~208 byte/line, jemalloc/system-allocator fast path).

**Fix.** İki düzenleme:

(a) **§5.2 item 1 test ismini ve doc-comment'ini yumuşat:**

```rust
/// Verifies that the caller-owned PipelineScratch Vecs retain their capacity
/// across apply_rules calls (the .clear() reuse contract). NOT a zero-
/// allocation invariant overall — regex::bytes::RegexSet::matches itself
/// allocates a small bitset (~pattern_count word) per call; that is a fixed
/// upstream cost outside PipelineScratch's surface.
#[test]
fn pipeline_scratch_capacity_preserved_across_apply_rules_calls() { ... }
```

(b) **§1 ve §2.6 rationale'lerini düzelt:** "per-call scratch allocation **PipelineScratch surface'i içinde** sıfır; `RegexSet::matches`'in upstream allocation'ı (small bitset, bounded pattern_len) caller'a opaktır ve v0.4 scope'unda optimization hedefi değil." Bu, BASELINE measurement'ının da `RegexSet::matches` allocator cost'unu içerdiğini açıkça yazar — measurement integrity'sini koruyan dürüst çerçeveleme.

**Şiddet rasyonali:** Bug değil — `apply_rules`'ın correctness'i etkilenmez, BASELINE measurement integrity'si etkilenmez (her iki sürüm de aynı `PatternSet::new` cost'unu yer), ve `PatternSet`'in heap cost'u 200B/line × 100k line/s = 20MB/s allocator pressure — jemalloc bunu 100ns'de halleder. Ama "zero-allocation" claim'i CLAUDE.md §4 ("Documentation with the feature") ve open-source-from-day-one credibility açısından düzeltilmeli. v0.4.1 bench-CI sub-version'ı bu cost'u histogram'a ekleyebilir — şimdi yanlış çerçeveleme yapmak v0.4.1 spec yazımını kirletir.

#### I-4. Cross-rule first-match-wins ordering invariant'ı için adanmış test yok — RegexSet refactor'unun en kritik kontrat noktası

**Lokasyon:** spec §5.2 item 3 conditional ("Mevcut v0.3.5 spec'inde overlap-detection test'leri var. Bunlara ek olarak ... Mevcut testlerden biri zaten bu kontratı pin'liyorsa redundant; yoksa eklenir.")

**Verification.** `grep -n 'first.match\|two_overlapping\|cross_rule' src/pipeline.rs tests/integration_capture_groups.rs` — **boş çıktı.** Mevcut `overlaps_accepted_*` ve `insert_accepted_*` testleri sorted-vec invariant'ını test eder, **cross-rule** first-match-wins'i değil. `apply_rules` unit testlerinden hiçbiri iki rule'un aynı substring'de hit ettiği bir line üzerinde "rule N+1'in match'i rule N'in match'ini bloke etmez ama tersi olur" pattern'ini test etmiyor.

**Şiddet.** RegexSet pre-filter'ının en kritik invariant'ı: `SetMatches::iter()` pattern-definition order'ında yield. Spec §2.4 "HashSet/BTreeSet/sort by anything-other-than-pattern-index YASAK" implementer notunu yazıyor — doğru ama mevcut test surface'i bu notu enforce etmiyor. Bir gelecek refactor (örn. "performans için unique'leştireyim, BTreeSet kullanayım") byte-identical capture-group output kontratı altında bile sessizce ordering'i bozabilir; mevcut `integration_capture_groups.rs` testleri tek-rule fixture'lar üzerine kurulu, cross-rule ordering'i test etmiyor.

**Fix.** Spec §5.2 item 3'ü conditional değil **zorunlu** yap. Test gövdesi:

```rust
#[test]
fn apply_rules_preserves_pattern_definition_order_for_overlapping_matches() {
    // Two synthetic rules where rule 0 and rule 1 both match overlapping
    // substrings on the same line. First-match-wins must give rule 0 the
    // span; rule 1's match must be dropped by accepted_spans overlap
    // detection. If RegexSet iteration ever switched away from pattern
    // order (e.g. via HashSet), rule 1 could pre-empt rule 0 silently.
    use crate::style::{Color, Style};
    use regex::bytes::{Regex, RegexSet};
    let red = Style { fg: Some(Color::Red), ..Style::DEFAULT };
    let blue = Style { fg: Some(Color::Blue), ..Style::DEFAULT };
    let compiled = Compiled {
        set: RegexSet::new([r"\d{3,5}", r"\d{2}"]).unwrap(),
        individuals: vec![
            Regex::new(r"\d{3,5}").unwrap(),
            Regex::new(r"\d{2}").unwrap(),
        ],
        styles: vec![red, blue],
        group_styles: vec![vec![], vec![]],
        uses_capture_styling: vec![false, false],
        respect_existing_colors: false,
    };
    let rules = ArcSwap::from_pointee(compiled);
    let mut scratch = PipelineScratch::default();
    let mut out: Vec<u8> = Vec::new();
    apply_rules(b"value 12345\n", &rules, &mut scratch, &mut out).unwrap();
    let s = String::from_utf8(out).unwrap();
    // Rule 0 (red, SGR 31) must wrap "12345"; rule 1 (blue, SGR 34) must
    // be suppressed by the overlap check — no SGR 34 in output.
    assert!(s.contains("\x1b[31m"), "rule 0 (red) must fire on '12345': {s:?}");
    assert!(!s.contains("\x1b[34m"), "rule 1 (blue) must be suppressed by overlap: {s:?}");
}
```

Bu test sadece yeni; mevcut hiçbir test pattern-order kontratını cross-rule axis'inde pin'lemiyor. Risk düşük (mevcut implementasyon doğru) ama **gelecek refactor'a karşı koruma** yok. CLAUDE.md §4 "Tests with the feature" + memory `feedback_test_assertion_specificity` (loose assertion'lar broken + fixed her ikisini satisfy eder; bu durumda hiç test yok).

**Şiddet rasyonali:** Risk olasılığı düşük (bu spec yanlış implementasyon yapma niyeti taşımıyor), ama impact yüksek (sessiz ordering regression, capture-group test'leri tek-rule olduğu için yakalamaz). Cost düşük (10 satır test). Memory `feedback_cross_cutting_review_value` aynı pattern'i v0.3.7'de teyit etti.

### 🟢 Nice-to-have

#### N-1. Spec'in §10 divergence açıklaması umbrella (ii)'yi haklı olarak reddediyor — ama daha net argüman mevcut, divergence rasyonali sağlamlaştırılabilir

**Lokasyon:** spec §10:
> "**`runs` lifetime:** Style by-value (umbrella §3.2 Bölüm B (ii) "rule_idx encoding" tercihinden ayrıldı — encoding capture-default vs group-overlay ayrımını rule_idx'le tek-byte ifade edemiyor, multi-variant enum overkill; Style Copy + ~16 byte, by-value trivial)."

**Gözlem.** Umbrella §3.2 Bölüm B (ii) "(start, end, rule_idx: u16) sakla, &Style'a emit'te resolve et — tercih edilen, en küçük footprint" diyor. Spec haklı olarak reddediyor — ama rasyonel **daha güçlü** ifade edilebilir. Sorunun özü: `apply_rules` runs vec'i iki ayrı style source'undan beslenir:

1. Hot path (`uses_capture_styling[i] == false`): `runs.push((start, end, compiled.styles[i]))` — rule-default style.
2. Captures path (`emit_capture_runs` içinden): `out.push((prev_pos, pos, group_styles[g-1].unwrap()))` veya `(prev_pos, pos, default_style)` — group overlay VEYA rule-default fallback.

`rule_idx: u16` tek başına emit'te `compiled.styles[i]` resolve eder ama **group overlay'i resolve edemez** — `(rule_idx, group_idx_or_zero)` lazım. Bu (`u16`, `u16`) tipinin total boyutu 4 byte; `(usize, usize, u16, u16)` runs entry'si 24 byte. Style by-value `(usize, usize, Style)` 32 byte (16+8+8 padding). Delta 8 byte, ama:

- (ii) shape'inde `Style` lookup'ı emit loop'unda iki kez branch alır (group_idx == 0 ise rule-default, değilse overlay).
- Style by-value shape'inde lookup `emit_capture_runs` içinde önceden yapılır; emit loop branch-free.

Branch eliminator emit hot path'inde (line başına 5-20 entry) Style by-value'yu net kazanan yapıyor. Spec bu nuance'ı eklerse v0.5+ "rule_idx tabanlı encoding'e geri dönelim" tartışmasına önceden cevap verir.

**Fix.** §10'da divergence paragrafına bir cümle ekle:

> Ek olarak, (ii) encoding emit loop'unda `if group_idx == 0` branch'i gerektirirdi; Style by-value bu branch'i `emit_capture_runs` içine push'lar (her match için bir kez, emit loop branch-free). Style by-value'nun 8-byte cache cost'u branch-elimination kazancını net pozitif yapar.

Severity: nit. Spec'in §10 mevcut açıklaması yeterince haklı; bu sadece argümanı **stronger than the spec admits** yapar.

#### N-2. Spec §5.3 test count "7 emit_capture_runs unit test" — gerçekte 8

**Lokasyon:** spec §5.3:
> "`src/pipeline.rs::tests` — 7 `emit_capture_runs_*` test, `runs` Vec type'ı `Vec<(usize, usize, Style)>`; `default_style` Style by-value."

**Verification.** `grep -nE '^\s*fn emit_capture_runs_' src/pipeline.rs` → 8 fonksiyon (`_no_styled_groups`, `_single_group`, `_two_adjacent`, `_nested_groups`, `_unfired_alternation`, `_gap_before`, `_gap_after`, `_scratch_reused`).

**Fix.** §5.3'te "7" → "8". Cosmetic ama subagent task task description'ında "7 test migrate" derse 8.'ini gözden kaçırabilir. (I-1 ile birlikte düşünüldüğünde önemli — `_scratch_reused` testi de `std::ptr::eq` yok ama yine de `runs` Vec type'ını taşıyor.)

#### N-3. Spec §5.2 item 1/2 test'leri `test_rules_arcswap()` helper'ı varsayıyor — mevcut değil

**Lokasyon:** spec §5.2 item 1 ve item 2 test gövdesi:
> `let rules = test_rules_arcswap();  // helper from existing tests`

**Verification.** `grep -n 'fn test_rules_arcswap' src/` → boş. Mevcut testlerin pattern'i inline:
```rust
let compiled = Compiled::load_builtins().unwrap();
let rules = ArcSwap::from_pointee(compiled);
```

**Fix.** Ya spec gövdesini inline pattern'e güncelle (3 satır, mevcut testlerle birebir), ya da §5.2 başında bir `fn test_rules_arcswap() -> ArcSwap<Compiled>` helper'ını mevcut testlerden refactor etme adımı tanımla. Tercih: inline — yeni helper introducing 2 yeni unit test için fazla ceremony.

Severity: nit. Subagent task implementer'ı zaten inline pattern'i görüp adapte eder; spec'in "helper from existing tests" yorumu yanıltıcı.

#### N-4. AC #3 test count claim "yaklaşık 408 → 411" — şu an 407

**Lokasyon:** spec §8 acceptance criteria item 3:
> "total count v0.3.7'den 3 yeni unit test (Fix Z2 + PipelineScratch reuse + RegexSet empty subset) artar (yaklaşık 408 → 411)."

**Verification.** `grep -rn '#\[test\]' src/ | wc -l` → **407**. Spec "yaklaşık 408" diyor — minor off-by-one. I-4'ün cross-rule ordering test'i de eklenirse delta 4 olur → 411. Numerical precision için spec'i "407 → 410 (+3 yeni unit test)" veya I-4 ile birlikte "407 → 411 (+4)" şeklinde düzeltebilir.

Severity: trivia. AC'nin operasyonel anlamı zaten "yeni N test eklenmesi"; pre-commit gate sayı sayma değil derleme+pass.

#### N-5. Spec §1 "5.'si gerek" — `set_match_scratch` aslında **6.** scratch, mevcut 4'e ek

**Lokasyon:** spec §1, paragraf 4:
> "`apply_rules` her satırda 4 Vec allocate ediyor (`accepted_spans`, `runs`, `event_scratch`, `active_scratch`); RegexSet hit indices'i için 5.'si gerek."

Bu accurate; ben yanıltıcı okudum. `(usize, usize, Style)` runs Vec'i `Style` by-value içermesi senkronize değil, bağımsız bir scratch. Toplam **5** scratch Vec, doğru.

(N-5 yanlış alarm — sil.)

#### N-6. Karar: v0.3.7'den miras flaky `integration_smoke::input_thread_joins_promptly_after_child_exit` pre-commitment'ı spec §5.4'te var — N-6 v0.3.7 review'undan miras alındı, doğru uygulandı

**Lokasyon:** spec §5.4 "Known acceptance-gate noise (N-6 pre-commitment, v0.3.7'den miras)" — v0.3.7 review N-6'sının bu spec'te §5.4 olarak inherit edildiğini açıkça not ediyor + diff surface enumeration'ı yapıyor + disposition pre-commit'leniyor (rerun --failed, ship'i blok etmez).

Bu **doğru uygulanmış** ve memory `feedback_flaky_watch_test` precedent'ine bağlanmış. **Action: hiçbir şey değiştirme** — sadece N-6 v0.3.7 → v0.4.0 inheritance'ının doğru taşındığını teyit ediyorum.

---

## Spec'in zaten doğru çözdüğü ve rev2'de tekrar açılmaması gereken kararlar (N-{previous})

- **N-1 (v0.3.6 cross-cutting → v0.3.7 I-1 → v0.4.0 §2.1)**. ZeroForbidden UserConfig arm cleanup'ı v0.4.0'a doğru ertelendi, atomic warm-up commit (§9 commit 2) olarak ilk sıraya konuldu, byte-identical user-facing output garantisi ile delegation pattern'i v0.3.7 OutOfRange + KeyMalformed precedent'iyle birebir. **Spec doğru çözdü.**
- **Umbrella §3.2 Bölüm B (b) "Pipeline method" tercihi reddi**. Spec §10 free-fn + `&mut PipelineScratch` parametresi tercihini disjoint field borrow + destructure boilerplate gerekmemesi rasyonali ile açıkladı; umbrella §7 mandatesi gereği divergence explicit. **Karar doğru, rasyonel sağlam.**
- **Umbrella §3.2 Bölüm B (ii) "rule_idx encoding" tercihi reddi**. Spec §10 Style by-value tercihini Style'ın Copy + ~16 byte olduğu için trivial olduğu rasyoneliyle açıkladı; N-1'de daha güçlü argüman önerildi ama mevcut açıklama da yeterli. **Karar doğru.**
- **Bench shim shape'i** — free-fn + `BenchScratch` newtype, scratch hoisted outside `b.iter`. Umbrella §3.2 Bölüm B'nin "per-call scratch allocation yasak" + §4.1 "BASELINE shim allocator'ını ölçer" kontratını koruyor. **Karar doğru** — sadece I-2 commit ordering'i ayrıca düzeltilmeli.
- **`#[allow(dead_code)]` notunun ve Compiled.set doc-comment'inin §2.5'te güncellenmesi**. Cleanup tek-satır + doc accuracy fix; v0.1'den beri rezerve edilen storage v0.4.0'da nakde çevriliyor. **Karar doğru.**
- **`integration_capture_groups.rs` zero-regression invariant gate (§5.1)**. Capture-group test surface byte-identical pass etmeli; mechanical proof gate spec'te pin'lendi. **Karar doğru** — refactor'un en kritik gate'i, eksiksiz.
- **§6.3 per-group floor (apply_rules >%5 review, >%20 release-block; passthrough >%25 review)** umbrella §4.2 ile birebir. **Karar doğru.**
- **§6.4 BASELINE.md update protocol** — measurement commit'i pre-tag, CHANGELOG `<FILL>` numbers BASELINE'dan dolu. **Karar doğru.**
- **§8 acceptance criteria #12 ve #13 grep guards** — Fix Z sonrası inline format kalmaz; `#[allow(dead_code)]` notu kalmaz. İki grep pattern'i de bağımsız olarak doğrulandı (post-fix bekleniyor empty). **Karar doğru** — ama §8.12'nin pattern'i (`grep -nP 'styles\.\\\\"0\\\\":'`) escape level olarak doğru ama okuması zor; pre-implementation review'da reviewer'a açıklayıcı bir comment eklenebilir (cosmetic, optional).

---

## Spec'in özetle iyi yaptıkları

- **API doğruluğu.** `regex::bytes::RegexSet::matches(&[u8]) -> SetMatches`, `SetMatches::iter()` pattern-definition order, `matches_into` yokluğu — üçü de regex 1.12 stable contract'ından doğrulandı. Umbrella'nın eski "matches_into" pseudocode hatası bu spec'te düzeltilmiş ve canonical pattern (§2.4'teki `scratch.set_match_scratch.extend(compiled.set.matches(line).iter())`) yazıldı.
- **Ordering invariant'ı.** `RegexSet::matches` index'lerinin pattern order'ında geldiği + `sources` (`src/rules.rs:859-877`) `individuals` ile birebir order'da inşa edildiği teyit. "HashSet/BTreeSet yasak" implementer notu güçlü — sadece test gate'i I-4'te eksik.
- **Borrow analizi.** Spec §2.2'nin "Borrow analizi" paragrafı over-explaining (mevcut callsite `apply_or_passthrough(&mut self, line: &[u8])` içinden çağrıyor; `line` argumandan, `self.buffer`'a borrow değil). Ama analiz **sonuç olarak doğru** — disjoint field borrow `&self.rules` + `&mut self.scratch` Rust 2021'de derlenir. Bench shim için `&mut rules.0` + `&mut scratch.0` aynı şekilde temiz.
- **Scope discipline.** `Compiled` struct field listesi dokunulmaz, `Pipeline::new` imzası değişmez, public API byte-stable, yeni Cargo dep yok, hot path dışında her şey untouched (runtime/signals/PTY/tty_guard/line_buffer'a sıfır dokunma — §5.4 flake disposition).
- **Capture-group zero-regression gate.** §5.1 mechanical proof gate explicit; `tests/integration_capture_groups.rs` byte-identical pass + test count v0.3.7 ile aynı + hiçbir test modify edilmedi gate'i pin'lendi. RegexSet refactor'unun en kritik kontratı.
- **Style by-value cache analizi.** §2.3 cache footprint delta (24→32 byte, ~33% artış) ve "tipik 5-20 entry/line tek L1 cache line içinde" gözlemi doğru — Style by-value'nun cache cost'u measurement-pin'lenecek bir nokta, kontrat değil; spec bunu doğru çerçevelemiş.
- **Empty rule set guard.** `Compiled::empty()` → `RegexSet::empty()` → `RegexSet::empty().matches(line)` empty `SetMatches` → `extend` no-op → bypass mode zero-iteration. Spec §2.4'te explicit.
- **CHANGELOG dürüstlüğü.** Üç ayrı "Changed" bullet (RegexSet, scratch reuse, ZeroForbidden delegation) — sub-version'un üç ortogonal işini ayrı yazıyor; kullanıcı changelog'tan ne değiştiğini net görür. CLAUDE.md §4 "Error messages are user-facing UX" mandatesi CHANGELOG'a da yansıyor.
- **§7 risk table** kapsamlı (8 satır, hepsi rasyonel azaltmalarla); umbrella'nın "umbrella vision güncellenebilir" mandatesi de §10'da pre-commit'lenmiş.
- **§9 release ceremony** memory `tayf release workflow` precedent'ine bağlı, pre-tag + tag + CHANGELOG date follow-up + umbrella shipped marker + final cross-cutting review zorunluluğu memory `feedback_cross_cutting_review_value` ile uyumlu. (Sadece I-2 commit ordering düzelmesi gerekiyor.)

---

## Summary

Spec mimari olarak doğru ve API'leri verified. Üç orta-önemli (🟡) konu rev2 öncesi çözülmeli:

1. **I-1** — §2.3 test migration paragrafına `std::ptr::eq` → `assert_eq!` mekanik adımı eksik; 8 mevcut emit_capture_runs assertion'ı Style by-value sonrası derlenmez.
2. **I-2** — §9 commit ordering'i pre-commit gate'i ihlal eden ara state yaratıyor; Commit 2 + Commit 4 birleştirilmeli (PipelineScratch + Style by-value + bench shim atomic), RegexSet (Commit 3) bağımsız kalır.
3. **I-3** — "Per-line zero-allocation contract" iddiası `RegexSet::matches`'in upstream `PatternSet::new` allocation'ı nedeniyle teknik olarak yanlış; §5.2 item 1 test ismi + §2.6 rationale yumuşatılmalı ("PipelineScratch surface'i içinde sıfır allocation; upstream RegexSet bitset cost'u opak").

🟡 I-4 cross-rule first-match-wins test gap'i — düşük olasılık, yüksek impact, ucuz cost. Eklenmesi şiddetle önerilir; eklenmezse en azından bir explicit acceptance criterion olarak pin'lenmeli ("mevcut overlap testleri pattern-order invariant'ını cross-rule axis'inde test etmiyor; future refactor için risk surface'i kabul edilmiştir").

🟢 N-1..N-4: divergence rationale stronger-than-spec-admits olarak yazılabilir, test count'lar (8 emit_capture_runs / 407 → 410-or-411), `test_rules_arcswap()` helper claim'i inline pattern'le değiştirilmeli — kozmetik, ship'i bloklamaz.

Rev2 sonrası APPROVE.

## Verdict — APPROVE_WITH_REVISIONS

**Apply for rev2:** I-1 (spec §2.3'e `std::ptr::eq` → `assert_eq!` mekanik adımı + 8 callsite enumeration ekle), I-2 (§9 commit 2 + commit 4 atomic birleştir veya commit ordering yeniden bant), I-3 (§5.2 item 1 test ismini + §2.6 bench rationale'ini "PipelineScratch surface zero-alloc; RegexSet upstream bitset opak" şeklinde yumuşat), ve I-4 (§5.2 item 3 cross-rule first-match-wins test'ini conditional değil zorunlu yap). N-1..N-4 polish; spec quality artar ama gate değiller.
