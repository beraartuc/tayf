# Review: tayf v0.4.1 spec

**Reviewer:** Senior Rust + CI/DevOps architect (opus 4.7, 1M ctx — pre-implementation pass)
**Spec under review:** `/Users/bera/tayf/docs/superpowers/specs/2026-05-25-tayf-v0.4.1-bench-ci.md`
**Umbrella:** `/Users/bera/tayf/docs/superpowers/specs/2026-05-24-tayf-v0.3.6-v0.4-vision.md` §3.3
**Genesis:** v0.4.0 final cross-cutting review (`docs/superpowers/reviews/2026-05-25-rust-senior-v0.4.0-final-cross-cutting-review.md`) C-1 + 4 explicit Recommendations.
**Verification basis:** spec'in tüm citations'ı tree'ye karşı doğrulandı; criterion 0.x `estimates.json` shape `bheisler/criterion.rs` repo'sundaki `src/estimate.rs` üzerinden doğrulandı (`mean: Estimate { point_estimate: f64, ... }` field birebir); GitHub Actions context-missing semantics docs.github.com/contexts'ten doğrulandı ("dereference a nonexistent property → empty string"); `jq 1.8.1` macos-14 runner image manifest'inde preinstalled; `jq 1.7` ubuntu-24.04 runner image manifest'inde preinstalled.

---

## Verdict — APPROVE_WITH_REVISIONS

Spec mimari olarak doğru: criterion'ın `estimates.json` shape iddiası teknik olarak verified (`mean.point_estimate` field birebir mevcut, jq script doğru çağırıyor); `contains(github.event.pull_request.labels.*.name, 'X')` expression GHA'nın resmi "nonexistent → empty string" semantiği gereği push event'inde güvenli false döner (workflow main push'larında patlama yok, release ceremony'yi block etmiyor); per-OS baseline storage yaklaşımı M2-Pro-local vs CI-shared-host mismatch'ini doğru çözüyor; v0.4.0 final cross-cutting review'un C-1 + 5 Recommendations öğesinin **tamamı** §10 disposition table'da explicit honored (memory `feedback_consume_prior_review` mandatesi karşılanmış). Ancak rev2 öncesi dört orta-büyüklük (🟡) konunun çözülmesi gerekiyor — biri YAML bash-script semantik bir bug (set-eu/-pipefail eksikliği `jq` exit code'unu yutuyor + bootstrap edge case'inde yanlış yorumlanıyor), biri threshold computation'da bc-portability tuzağı (negatif delta'lar locale-dependent), biri umbrella divergence'ın §10'da hiç görünmemesi (umbrella §3.3 `change/estimates.json` der; spec `new/estimates.json` kullanır — farklı subdir, farklı semantik, açıklama lazım), biri bootstrap commit-back step'inin acceptance criterion'da pin'lenmemesi.

---

## Sanity-check ettiğim spec iddiaları

| Spec iddiası | Doğrulandı |
|---|---|
| Mevcut `.github/workflows/ci.yml` shape: `test` (ubuntu+macos matrix) + `audit` (ubuntu only); bench job yok | ✅ `.github/workflows/ci.yml:1-48` birebir |
| `actions/checkout@v5`, `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2` zaten ci.yml'de kullanılıyor (yeni dep yok) | ✅ ci.yml:18-22 |
| 4 bench function isimleri: `bench_apply_rules_ipv4_heavy/_mixed_syslog/_captures_heavy`, `bench_passthrough` | ✅ `benches/throughput.rs:52,78,108,132` |
| Bench grup yolları: `apply_rules/ipv4-heavy`, `apply_rules/mixed-syslog`, `apply_rules/captures-heavy`, `passthrough/write_all` | ✅ `throughput.rs:57+59,86+88,114+116,140+142` (group + bench_function) |
| `benches/BASELINE.md`'de v0.4.0 section + per-group floor referansı | ✅ BASELINE.md:434-505 (v0.4.0 section); per-group floor spec §6.3 BASELINE.md:484-487'de re-stated |
| `src/rules.rs:964` + `:993` halen bare `unreachable!()`; sibling `:938-942` zaten reason taşıyor | ✅ rules.rs:964, 993 bare; rules.rs:938-942 reason-carrying — birebir spec §2.1 anlatımı |
| v0.4.0 review C-1 + 4 Recommendations'ın §10'da disposition'ı | ✅ §10 5 bullet, hepsi explicitly enumerated (aşağıda meta-check) |
| Umbrella §3.3 (a/b/c) içinden (b) tercih edildi, normative | ✅ umbrella line 130 + spec §1 paragraph 4 + §4 |
| Umbrella §3.3 divergence: PR comment bot OUT OF SCOPE | ✅ spec §4 ilk bullet "no v0.5 carryover" explicit |
| macOS bench INCLUDED divergence | ❓ Spec §10 son bullet'ta "included (per-OS baseline files, like-for-like compare; ubuntu-only seçeneği reddedildi)" — explicit ama umbrella §3.3'te macOS yok/var açıkça konuşulmamış; divergence framing umbrella'nın "minor" + brainstorm yapılacak listesinde |
| criterion `target/criterion/<bench>/<name>/new/estimates.json` shape | ✅ bheisler/criterion.rs `src/estimate.rs` — `Estimates { mean: Estimate, median, std_dev, median_abs_dev, slope: Option<Estimate> }` + `Estimate { confidence_interval, point_estimate: f64, standard_error: f64 }`. Spec'in `jq '.mean.point_estimate'` çağrısı **birebir doğru** |
| `contains(github.event.pull_request.labels.*.name, 'X')` push event'inde safe-resolve | ✅ docs.github.com/actions/contexts: "dereference a nonexistent property → empty string"; `contains("", 'X')` → false; workflow patlamaz |
| `jq` preinstalled (macos-latest, ubuntu-latest) | ✅ macOS-14 readme `jq 1.8.1`; Ubuntu 24.04 readme `jq 1.7` — spec'in `if ! command -v jq` fallback'ı no-op (~30s tasarruf imkanı, ama harmless) |
| `Swatinem/rust-cache@v2` mevcut bench compile artifacts cache'ler | ✅ — RegexSet pre-filter + builtins compile already cached; bench rebuild minimal |

Public API impact sıfır (spec §2.1 sadece `unreachable!()` message string'i değiştirir, hot path byte-identical compile output; §2.3 CI workflow + §2.2 JSON dosyaları crate dışı). **Public API gate clean.**

---

## Findings

### 🔴 Critical

**Yok.** criterion API shape doğru verified, GHA context semantics safe, threshold matematiği temelde sound, public API dokunulmaz, security review'a gerek yok (CI workflow user-controlled input işlemiyor, sadece kendi repo'sunun bench output'unu okuyor).

### 🟡 Important

#### I-1. §2.3 bash script `set -euo pipefail` taşımıyor — `jq` parse failure veya boş baseline silently green döner

**Lokasyon:** spec §2.3 YAML, "Compare against baseline" step (`run: |` blok'u, satır 136-170 spec içinde).

**Sorun.** GitHub Actions `run: |` step'lerinin default shell'i `bash --noprofile --norc -eo pipefail {0}` (`-e` ve `-o pipefail` aktif), ama `-u` (unset variable) **yok**. Daha kritik: spec'in `Compare against baseline` step'i şu satırla başlıyor:

```bash
baseline="benches/baselines/latest/${{ matrix.os }}.json"
test -f "$baseline" || { echo "::warning::no baseline at $baseline; skipping comparison"; exit 0; }
```

`exit 0` baseline yokken step'i başarıyla bitiriyor — doğru. Ama loop içinde:

```bash
new_mean=$(jq '.mean.point_estimate' "target/criterion/${bench}/new/estimates.json")
base_mean=$(jq --arg b "$bench" '.benches[$b].mean_ns' "$baseline")
```

`jq` bir field bulamazsa `null` döner (exit 0). `new_mean=null` olur, sonraki `bc` çağrısı `(null - $base_mean) / $base_mean * 100` → bc syntax error → `delta_pct=""` (boş). Sonraki `awk -v d="" -v t="20" 'BEGIN { print (d > t) ? 1 : 0 }'` → awk d'yi 0 olarak değerlendirir → breach=0 → silently green. **§2.6 "Smoke test estimates.json shape" step bunu sadece `mean.point_estimate` için yakalar; baseline JSON'unun `.benches[$b].mean_ns` field'ı için smoke yok.** Eğer biri `benches/baselines/latest/<os>.json`'ı malformed bırakırsa (örn. `mean_ns` yerine `mean` yazarsa, veya bench key yanlış), CI yanlış pozitif "her şey yolunda" döner.

**Fix.** İki düzenleme spec §2.3 YAML'ına:

(a) Step'in başına `set -euo pipefail` ekle (idempotent; GHA default'unu güçlendirir, `-u` ekler):
```yaml
- name: Compare against baseline
  shell: bash
  run: |
    set -euo pipefail
    ...
```

(b) `jq -e` kullan (exit 1 if field is null or missing):
```bash
new_mean=$(jq -e '.mean.point_estimate' "target/criterion/${bench}/new/estimates.json") || {
  echo "::error::failed to read mean.point_estimate from ${bench}'s estimates.json"
  exit 1
}
base_mean=$(jq -e --arg b "$bench" '.benches[$b].mean_ns' "$baseline") || {
  echo "::error::baseline ${baseline} missing benches[\"$bench\"].mean_ns"
  exit 1
}
```

Bu, bootstrap edge case'inde (`base_mean == 0`) §5.2'deki guard ile karışmaz — `jq -e` null-on-missing'i yakalar, `0` literal değeri yakalamaz; guard `[[ "$base_mean" == "0" ]]` ayrı bir kontrol.

**Şiddet rasyonali:** Spec §6.3 ve §7'deki "estimates.json shape değişirse loud-fail" iddiası §2.6 smoke ile sadece `new/estimates.json` için karşılanıyor — baseline JSON için karşılanmıyor. Malformed baseline silently-green riski release ceremony'nin "baseline write commit" adımının her zaman doğru shape'te commit yapıldığını varsayar; bir gün eli kayan reviewer bunu kıracak. `jq -e` + `set -euo pipefail` defansif programlama, bir-satır maliyet.

#### I-2. `bc` ile floating-point comparison ve negatif delta'lar — locale ve scale tuzakları

**Lokasyon:** spec §2.3 YAML:

```bash
delta_pct=$(echo "scale=2; ($new_mean - $base_mean) / $base_mean * 100" | bc)
# Use awk for comparison (bc has portability issues with conditional)
breach=$(awk -v d="$delta_pct" -v t="$threshold_pct" 'BEGIN { print (d > t) ? 1 : 0 }')
```

**Sorun 1 (locale).** `bc` locale-aware: `LANG=tr_TR.UTF-8` shell'inde decimal separator `,` olur, `awk` parse'ı kırılır. GHA runner'larda `LANG=C.UTF-8` default ama explicit pin yok; bir gün matrix'e `LANG` env eklenirse silently kırılır.

**Sorun 2 (negatif delta).** Spec yorumu "apply_rules/* > +20%" diyor (yavaşlama threshold'u). `delta_pct < 0` ise (yani performance improvement), `awk 'd > t'` doğru `0` döner — false positive yok. Ama spec'in CHANGELOG/annotation messaging'i "regressed +${delta_pct}%" yazıyor; `delta_pct=-3.57` durumunda annotation "regressed +-3.57%" çıkar (cosmetic ama logu okuyana karışıktır).

**Sorun 3 (scale=2 precision loss).** `scale=2` `bc` çıktısı `0.79` veya `0.80` gibi — `awk` bunu sayı olarak parse'lar (doğru), ama threshold sınırına yakın delta'larda (örn. gerçek %20.001 → `bc scale=2` `20.00` → breach=0 false negative). %20'nin altı/üstü kritik decision boundary olduğu için scale=2 çok dar.

**Sorun 4 (`bc` ubuntu-24.04 default install'da yok — doğrulanmamış).** ubuntu-24.04 runner readme `bc` listelemiyor (yukarıda WebFetch ile doğrulandı). `bc` POSIX standard ve genelde preinstalled; ama GHA minimal images bazen `bc` paketini drop etmiş ("minimal" image preset'lerinde). **Defansif:** ya `bc` install fallback step ekle, ya da bash arithmetic + awk float ile değiştir.

**Fix önerisi (compound).** `bc` yerine sadece `awk` kullan — POSIX awk float arithmetic'i tam yetiyor ve ubuntu/macos'ta her zaman preinstalled:

```bash
# Compute delta and breach in one awk pass (no bc, no locale issues)
read -r delta_pct breach < <(awk -v new="$new_mean" -v base="$base_mean" -v t="$threshold_pct" '
  BEGIN {
    if (base == 0) { print "0 -1"; exit }  # caller already checks zero, defensive
    d = (new - base) / base * 100
    printf "%.2f %d\n", d, (d > t) ? 1 : 0
  }')
```

Locale: shell'in başında `LC_NUMERIC=C` pin'leyebilir; veya `awk -v LC_NUMERIC=C` (awk locale-aware değil POSIX, ama defansif).

Precision: scale=2 yerine `%.2f` round zaten yetiyor (display için); comparison `d > t` raw double üzerinden, precision loss yok.

**Şiddet rasyonali:** İki ayrı tool (`bc` + `awk`) yerine tek awk hem dep-minimal hem locale-safe. CI sub-µs jitter band'inin (±%15) içinde precision boundary'ye yaklaşan delta'lar (örn. captures-heavy +19.something%) ya yanlış-green ya yanlış-red olabilir; awk-only çözüm her iki tuzağı eler. ubuntu-24.04 `bc` belirsizliği de bonus side-benefit.

#### I-3. Umbrella §3.3 `change/estimates.json` der; spec `new/estimates.json` kullanır — §10 disposition yok

**Lokasyon:**
- Umbrella `docs/superpowers/specs/2026-05-24-tayf-v0.3.6-v0.4-vision.md:130`:
  > "(b) `target/criterion/<bench>/<name>/change/estimates.json` artifact parsing."
- Spec `docs/superpowers/specs/2026-05-25-tayf-v0.4.1-bench-ci.md:130`:
  > `file="target/criterion/${bench}/new/estimates.json"`

**Sorun.** Criterion `change/` subdirectory criterion-saved baseline'a karşı **delta** estimate'lerini yazıyor (`--save-baseline <name>` ile saved bir baseline varsa). `new/` mevcut run'ın absolute estimate'lerini yazıyor. Bu spec git'te canonical baseline tuttuğu için (criterion `--save-baseline` kullanmıyor), `change/` zaten bench-run'da yaratılmıyor — `new/` doğru tercih. **Ama umbrella'dan bilinçli bir divergence ve §10'da hiç görünmüyor.**

Memory `feedback_consume_prior_review` ve umbrella §7 "spec wins on divergence + must explicitly note" mandatesi gereği bu divergence §10'a düşmeli. Şu an §10'da:

- "Storage shape: hand-curated `benches/baselines/<version>/<os>.json` (umbrella §3.3 (b) `estimates.json` parsing pattern)."

bu satır (b) tercihi normatif yapıldığını söylüyor ama `change/` → `new/` subdir değişimini gizliyor. v0.4.0 spec §10 divergences'i (umbrella §3.2 Bölüm B (b) ve (ii) reddedildi) gibi explicit bir bullet eksik.

**Fix.** §10'a bir bullet ekle:

> - **Criterion artifact subdir:** umbrella §3.3 (b) `change/estimates.json` der; bu spec git-canonical baseline tuttuğu için `--save-baseline` kullanmıyor, dolayısıyla `change/` dir bench-run'da üretilmiyor — spec `new/estimates.json` okuyor (absolute estimate, delta'yı kendisi hesaplıyor). Umbrella §7 "spec wins on divergence" gereği explicit.

**Şiddet rasyonali:** Bug değil — `new/estimates.json` her bench-run'da deterministically üretilir, `change/` bizim kullanmadığımız bir feature'ın artifact'i. Ama memory `feedback_consume_prior_review` mandatesi sub-version spec'lerinin **her** umbrella divergence'ını §10'a koyması — v0.4.0 cycle bu disiplin başlattı, v0.4.1 onu sürdürmek zorunda. Bir sonraki cross-cutting review aksini "silently dropped umbrella deviation" diye yakalar.

#### I-4. Bootstrap commit-back step §8 acceptance criteria'da pin'lenmemiş — ship'ten sonra "real numbers eksik" durumu kabul edilir

**Lokasyon:**
- Spec §6.2 "Baseline bootstrap timeline" item 3: "CI artifact'tan real numbers'ı çıkar: workflow log'unda criterion `time: [low mean high]` lines var. Bu rakamları manual olarak `v0.4.0/<os>.json` dosyalarına yaz. Commit + push."
- Spec §8 acceptance criteria item 7: "`benches/baselines/v0.4.0/ubuntu-latest.json` ve `macos-latest.json` exist; **real (non-placeholder) values**; schema §2.2 ile uyumlu."

**Sorun.** Spec §6.2'nin bootstrap timeline'ı 5 step (placeholder commit → CI run → real number extract → manual commit → next-PR canonical compare). §8 acceptance criteria item 7 "real (non-placeholder) values" demin; **ama bootstrap step 3'ün hangi mechanism'le çalıştığı pin'lenmemiş.** Spec şöyle der: "workflow log'unda criterion `time: [low mean high]` lines var. Bu rakamları manual olarak ... yaz."

İki sorun:
1. **`time: [low mean high]` insan-okur ms/µs unit'lerinde** (`2.3335 ms`, `1.1563 µs`). Spec'in JSON schema'sı `mean_ns: <CI-MEASURED>` (nanoseconds, raw f64). Manuel conversion human-error riski yüksek (`2.3335 ms` → `2333500` ns; `1.1563 µs` → `1156.3` ns — birim karışıklığı kolay).
2. **Alternatif:** CI step'i `estimates.json`'ı doğrudan artifact upload edebilir (GHA `actions/upload-artifact@v4`), reviewer indirir ve içeriği baseline JSON'a wholesale kopyalar. Bu unit-error riskini eler.

**Spec §8 item 7'nin pin'lemediği:** "real non-placeholder values" hangi mekanizma ile alındı? Bootstrap'in **kim** ve **ne zaman** yapacağı belirsiz: v0.4.1 implementation plan içinde mi? v0.4.1 release ceremony §9.4 öncesi mi? §9'da Commit 4 (`bench: record v0.4.0 CI baseline numbers`) der — yani implementation phase'inin parçası, plan-time bootstrap commit'i. Ama acceptance criteria item 7'nin ne zaman geçtiği (placeholder + workflow-zero-guard ile ship → sonra real numbers commit?) muğlak.

**Fix önerisi.** Üç değişiklik spec'e:

(a) §8 acceptance criteria item 7'yi ikiye ayır:
- **7a.** `benches/baselines/v0.4.0/{ubuntu,macos}-latest.json` exist; ya placeholder (her field 0 + sample_count 0) ya real (CI-measured non-zero); schema §2.2 ile uyumlu.
- **7b.** Pre-tag release ceremony tamamlandıktan sonra (yani §9.4 öncesinde), `benches/baselines/v0.4.0/{ubuntu,macos}-latest.json` real (non-placeholder, mean_ns > 0) values içerir.

(b) §6.2 step 3'e: "CI bench step'i `target/criterion/` directory'sini `actions/upload-artifact@v4` ile artifact olarak upload eder; reviewer artifact'i indirir, `jq` ile `mean.point_estimate` field'larını okur, `v0.4.0/<os>.json` schema'sına commit eder. Manuel `time: [low mean high]` parsing ve ms/µs→ns unit conversion'ı YASAK — human-error riski."

(c) §2.3 YAML'a artifact upload step ekle:
```yaml
- name: Upload criterion artifacts (for baseline extraction)
  if: always()
  uses: actions/upload-artifact@v4
  with:
    name: criterion-${{ matrix.os }}
    path: target/criterion/
    retention-days: 14
```

**Şiddet rasyonali:** v0.4.1 implementation'ın critical path'i bu bootstrap. Manuel insan-okur output → JSON re-write step'i v0.4.1 ship gate'inin kalitesini bir reviewer'ın `bc` mental matematiğine bağlar. Artifact-upload + jq script'i 5 satır ekleyip risk'i sıfırlar. Ayrıca v0.4.2/v0.5 baseline re-record ceremony'sinde tekrar tekrar aynı insan adımı — automation worth-it.

### 🟢 Nice-to-have

#### N-1. `jq` preinstalled — `Install jq` step ~30s tasarrufla atılabilir

**Lokasyon:** spec §2.3 YAML, "Install jq" step (satır 120-124).

**Verification.** macos-14 runner image readme: `jq 1.8.1` preinstalled. Ubuntu 24.04 runner image readme: `jq 1.7` preinstalled. `if ! command -v jq &> /dev/null` fallback her zaman false döner, step no-op.

**Fix.** Step'i tamamen kaldır (jq her iki runner'da garanti preinstalled). Veya defansif tut ama not'la:
```yaml
# jq is preinstalled on ubuntu-latest (1.7) and macos-latest (1.8.1)
# per actions/runner-images. Skip the install step.
```

Severity: cosmetic. ~30s × her PR × her OS = ölçülebilir CI maliyet ama kritik değil.

#### N-2. §2.5 "label name kebab-case `bench-ci-strict`" — `.github/labels.yml` yok = labels yarı manuel

**Lokasyon:** spec §2.5:
> "**Label name:** `bench-ci-strict` (kebab-case GHA convention). Mevcut tayf labelları yok; bu ilk label. README'de dokümante edilir mi? **HAYIR** — internal CI mechanism, kullanıcı-yüzü dokümantasyon değil. `.github/labels.yml` veya benzeri label-config dosyası **YOK** (premature; ilk label'da config gereksiz)."

**Gözlem.** Label-config dosyası YAGNI'lemek doğru. Ama label'ın **kim** tarafından oluşturulduğu ve **ne zaman** belirtilmemiş. GitHub UI'da repo > Settings > Labels'a manuel "Create label: bench-ci-strict" gerekir. İlk PR'da label henüz yokken `contains(...)` zaten boş array'de false döner (safe), ama label'i kimin oluşturduğu ve hangi PR'da ilk kullanıldığı spec'te belgelenmemiş.

**Fix.** §2.5'e bir cümle ekle:
> Label v0.4.1 release ceremony §9.0 öncesi GitHub UI'dan manuel oluşturulur (repo > Settings > Labels > New label: `bench-ci-strict`, herhangi bir renk). İlk PR'da label yokken `contains(...)` boş array'e karşı `false` döner — workflow patlamaz, opt-in implicit-off.

Severity: cosmetic. Implementer setup-step'te label oluşturursa unutmaz; bug değil.

#### N-3. `Smoke test estimates.json shape` step'i baseline JSON'unu da kontrol edebilir

**Lokasyon:** spec §2.3 "Smoke test estimates.json shape" step (satır 128-133).

**Gözlem.** Smoke `new/estimates.json` field'larını kontrol ediyor (her bench için `mean.point_estimate` mevcut mu). Eğer baseline JSON malformed ise (örn. `benches` field eksik, bench key yanlış yazılmış), smoke yakalamaz; "Compare against baseline" step'inde I-1'deki silently-green davranış tetiklenir.

**Fix.** Smoke step'ine baseline kontrolü de ekle:

```yaml
- name: Smoke test estimates.json + baseline JSON shape
  run: |
    set -euo pipefail
    for bench in apply_rules/ipv4-heavy apply_rules/mixed-syslog apply_rules/captures-heavy passthrough/write_all; do
      file="target/criterion/${bench}/new/estimates.json"
      test -f "$file" || { echo "::error::missing $file"; exit 1; }
      jq -e '.mean.point_estimate' "$file" > /dev/null || { echo "::error::unexpected schema in $file"; exit 1; }
    done
    baseline="benches/baselines/latest/${{ matrix.os }}.json"
    if [[ -f "$baseline" ]]; then
      for bench in apply_rules/ipv4-heavy apply_rules/mixed-syslog apply_rules/captures-heavy passthrough/write_all; do
        jq -e --arg b "$bench" '.benches[$b].mean_ns' "$baseline" > /dev/null || {
          echo "::error::baseline $baseline missing benches[\"$bench\"].mean_ns"
          exit 1
        }
      done
    fi
```

Severity: nice-to-have; I-1 fix (`jq -e` in compare step) zaten yakalar — bu sadece earlier-fail.

#### N-4. `needs: test` ile matrix-job dependency: hangi `test` matrix entry'sinin pass etmesi gerekiyor?

**Lokasyon:** spec §2.3 YAML:
```yaml
bench-regression:
  ...
  needs: test  # only run if test passes (don't bench broken code)
```

**Gözlem.** GHA semantics: `needs: test` matrix job ise, **tüm** matrix entry'lerinin yeşil olması gerekir. ubuntu-latest test fail olursa macos-latest bench job de çalışmaz (defansif, doğru). Ama spec metni "test geçmeden bench çalışma" der; literal yorum "tek bir test entry yeterli" da olabilir. Mevcut davranış doğru — "tüm test entries pass" → bench. Sadece netleştirilebilir.

**Fix.** Spec §2.3 commentary'sine ekle:
> `needs: test` GHA tüm matrix entries'in pass etmesini gerektirir; ubuntu test fail olursa macos bench de skip'lenir (ve tersi). Bu kasıtlı — bench artık per-OS isolated ama "test broken" macOS-only bir bug değilse cross-OS regression genelde yapısal, broken code üzerinde bench measurement yanıltıcı.

Severity: documentation polish.

#### N-5. CHANGELOG entry'sindeki "PR open via the `[bench-ci-strict]` PR label" cümle yapısı bozuk

**Lokasyon:** spec §3 CHANGELOG entry:
> "PR open via the `[bench-ci-strict]` PR label to upgrade annotations to errors that fail the workflow."

**Sorun.** "PR open via" grammar bozuk — kullanıcı PR'ı label'le açmıyor, label'i mevcut PR'a yapıştırıyor. "Opt in via the `[bench-ci-strict]` PR label to upgrade..." daha doğru.

**Fix.** CHANGELOG cümlesini düzelt:
> "Opt in by labeling the PR with `bench-ci-strict` to upgrade annotations to errors that fail the workflow."

Severity: cosmetic, ama CLAUDE.md §4 "Error messages are user-facing UX" CHANGELOG'a da yansır — kullanıcı bu cümleyi release notes'ta okur.

#### N-6. Memory entry'nin description field'ı `feedback-review-calibration-en-tr` (kebab); dosya adı `feedback_review_calibration_en_tr.md` (snake)

**Lokasyon:** spec §2.8 memory entry frontmatter:
```yaml
name: feedback-review-calibration-en-tr
```

**Gözlem.** Mevcut memory dosyalarının pattern'i (örn. `feedback_dependency_minimalism.md`, `feedback_cross_cutting_review_value.md`) frontmatter name + dosya adı arasında consistency yok — bazı dosyalar snake, bazı kebab. Bu spec ikisi de kullanıyor (frontmatter kebab, dosya snake). Tutarlılık iyi olur ama existing memory dir convention'ı belirsiz; deferring.

Severity: trivia. Memory loader pattern'i tolere ediyor.

#### N-7. Spec §1 son paragrafta `change/estimates.json` umbrella ifadesi quote'lanmamış

**Lokasyon:** spec §1, paragraf 4 (Baseline storage gerekçe):
> "(b) `estimates.json` parsing (umbrella §3.3 (b)) `jq` ile direkt okunur, GHA runner'larda `jq` zaten var."

**Gözlem.** Umbrella §3.3 (b) tam metni `change/estimates.json` artifact parsing. Spec bunu paraphrase ederek `estimates.json` der; doğru ama I-3'teki divergence'ın okuyucu için ilk işaretini gizler. §1'in açıklayıcılığı kayboluyor.

**Fix.** §1 paragraf 4'ü hafifçe genişlet:
> "(b) `estimates.json` parsing (umbrella §3.3 (b)) `jq` ile direkt okunur. Umbrella `change/` subdir'inden bahsediyor; spec `new/` subdir kullanır (§10 disposition'ında açıklanan divergence — git-canonical baseline tuttuğumuz için `change/` üretilmiyor). GHA runner'larda `jq` zaten var."

Severity: documentation polish; I-3 ile birlikte düşünüldüğünde küçük bir ek.

---

## Meta-check: §10 disposition of v0.4.0 review items (consume-prior-review mandatesi)

v0.4.0 cross-cutting review (`docs/superpowers/reviews/2026-05-25-rust-senior-v0.4.0-final-cross-cutting-review.md`) açtığı 5 öğe:

| # | Item | Spec §10 ele alımı | Disposition |
|---|---|---|---|
| 1 | **C-1**: bare `unreachable!()` arms at rules.rs:964 + 993 | §10 ilk bullet "(C-1)" + §2.1 detay | ✅ HONORED — fold to v0.4.1 §2.1, atomic warm-up commit (§9 Commit 2) |
| 2 | **Recommendation 1**: Fold C-1 into v0.4.1 cleanup | §2.1 + §9 Commit 2 | ✅ HONORED — identical to item #1 (review used C-1 + Rec1 as paired statement) |
| 3 | **Recommendation 2**: Open v0.4.1 spec by enumerating prior review's items | §10 bullet "Fold-or-defer disposition" — meta-acknowledgment | ✅ HONORED — spec §10 ilk bullet explicitly "HONORED tüm 5 madde" der; bu review document'in kendisi bu prosedürün outputu |
| 4 | **Recommendation 3**: Reviewer calibration EN/TR mismatch = 🟡 minimum | §2.8 memory entry write + §10 dördüncü bullet "strict mode label" + §9.7 release ceremony adımı | ✅ HONORED — yeni memory entry içeriği aynen v0.4.0 review §3 önerisini takip ediyor |
| 5 | **Recommendation 4**: captures-heavy +7.93% as v0.4.0 baseline, not regression | §6.1 threshold matrix v0.4.0 baseline'a karşı; §1 motivasyon paragrafı "v0.4.0 captures-heavy +7.93%'i tesadüfen yakaladık" + baseline canonical v0.4.0 | ✅ HONORED — implicit ama doğru; tüm threshold computation v0.4.0 baseline'a karşı, v0.3.5 aspirasyonel floor değil |
| 6 | **Recommendation 5**: No v0.4.0.1 hotfix | §1 motivasyon "Üç sub-version'lık carryover zinciri ... v0.4.1 cleanly closes" + spec scope v0.4.0.1 değil v0.4.1 | ✅ HONORED — implicit, scope decision'la karşılandı |

**Sonuç: §10 disposition consistent ve complete.** v0.4.0 review'un 5 numbered Recommendations + C-1'in **tamamı** ya explicit §10 bullet ile ya spec'in scope/yapısı ile honored. Bir tane bile silently dropped öğe yok. Memory `feedback_consume_prior_review` mandatesi karşılandı.

Sadece tek meta-eksiklik: **umbrella §3.3'ten `change/` → `new/` subdir divergence** §10'da görünmüyor (I-3 finding'i). Bu v0.4.0 review öğelerinden değil umbrella divergence'ı, ama §10'un genel kontratı "umbrella ve prior review'dan tüm açık öğeler bu bullet listesinde ya fold edilir ya defer edilir" olmalı. Memory `feedback_consume_prior_review` mantığını umbrella divergence'larına da extend etmek doğal olur (v0.4.0 spec §10 bu pattern'i zaten kullandı — umbrella §3.2 Bölüm B (b) ve (ii) tercihinden ayrılmaları açıkça §10'a koydu).

---

## Spec'in zaten doğru çözdüğü ve rev2'de tekrar açılmaması gereken kararlar (N-{previous})

- **N-prev-1.** Fix C atomic warm-up commit (§9 Commit 2 first) v0.4.0 Fix Z (ZeroForbidden) pattern'iyle birebir — risk-free, hot path byte-identical. Spec doğru positioned.
- **N-prev-2.** `[bench-ci-strict]` PR label opt-in escalation — sub-µs jitter false-positive auto-fail'i önlüyor + reviewer'a manuel kontrol fırsatı veriyor. Default warning, opt-in error doğru disiplin.
- **N-prev-3.** Per-OS baseline files (`benches/baselines/<version>/{ubuntu,macos}-latest.json`) — M2-Pro-local vs CI-shared-host mismatch'i ortadan kaldırıyor; like-for-like compare yapısal olarak garanti.
- **N-prev-4.** Umbrella §3.3'ten (a) cargo-criterion + (c) stdout parsing reddi — §4 OUT OF SCOPE'larda açıkça rationale'iyle. (b) doğru seçim, tool-install zero-dep.
- **N-prev-5.** `needs: test` defansif gate — broken code'da bench ölçüm yanıltıcı; doğru.
- **N-prev-6.** macOS bench INCLUDED — umbrella'da macOS-only/ubuntu-only ayrımı muğlaktı; spec §10 son bullet "ubuntu-only seçeneği reddedildi; CI vs local M2 Pro mismatch'i ortadan kaldırır" rasyoneli ile pin'ledi. Doğru karar (host'lar arası non-portable performance assumption'ları gün ışığına çıkarır).
- **N-prev-7.** `actions/checkout@v5` + `dtolnay/rust-toolchain@stable` + `Swatinem/rust-cache@v2` mevcut workflow ile aynı — yeni third-party action surface yok, cargo-cache mevcut warm cache'i kullanır.
- **N-prev-8.** Bootstrap baseline-zero guard (§5.2 step 3'te `[[ "$base_mean" == "0" ]]` skip) — placeholder commit'in CI'da sıfır-bölme'ye yol açmaması doğru ele alınmış. (I-1 ile birleştirildiğinde guard hâlâ doğru çalışır; `jq -e` null'i ayrı yakalar.)
- **N-prev-9.** Threshold matrix §6.1 release-block'a hizalı (jitter band'inin üstünde) — false-positive-tolerant CI annotation policy + finer-grained spec §6.3 floor human-judgment review-gate ayrı katman olarak korunmuş. İki-katman politika doğru.
- **N-prev-10.** §9 release ceremony pre-tag baseline write commit step → tag → post-tag CHANGELOG date follow-up + umbrella shipped marker + final cross-cutting review — memory `tayf release workflow` + v0.4.0 precedent'ine uyumlu. Yeni "pre-tag baseline write" step temiz integrate edilmiş.
- **N-prev-11.** Lib test count v0.4.0 baseline (410) korunur — Fix C behavior değiştirmez, yeni test yok, zero-regression invariant pin'lenmiş. Doğru.

---

## Spec'in özetle iyi yaptıkları

- **Disposition table'ı (§10) v0.4.0 review'un 5 öğesinin tamamını HONORED.** Memory `feedback_consume_prior_review` mandatesi tartışmasız karşılanmış; v0.4.1 cycle'ın `unreachable!("reason")` carryover'ı 3 sub-version sonra cleanly close ediyor.
- **criterion API shape verification.** `estimates.json`'ın `mean.point_estimate` field'i criterion 0.x source'undan birebir doğrulandı (`bheisler/criterion.rs:src/estimate.rs` — `Estimate { confidence_interval, point_estimate: f64, standard_error: f64 }`). Spec'in `jq '.mean.point_estimate'` çağrısı **byte-doğru**. §2.6 smoke step bu shape'in regression'ını yakalar (criterion 0.9+ bump'ta değişirse loud-fail).
- **GHA expression safety.** `contains(github.event.pull_request.labels.*.name, 'X')` push event'inde docs.github.com/contexts "nonexistent property → empty string" semantiği gereği güvenle `false` döner. Release ceremony main push'larında workflow patlamaz, bu critical.
- **Per-OS baseline storage.** M2-Pro local vs CI shared host'un absolute timing mismatch'i `<os>.json` ayrımıyla yapısal olarak çözülmüş — like-for-like compare garanti. v0.4.0 BASELINE.md'nin macOS-only M2 Pro numbers'ı tek-runner artefakttı; v0.4.1 onu çoğullaştırarak tutarlı CI compare yapısı kuruyor.
- **Threshold matrix iki-katman ayrım.** CI annotation (operasyonel, false-positive tolerant) vs spec §6.3 floor (ship-time human judgment, finer-grained) ayrımı umbrella §4.2'den naklen ve normative. Sub-µs jitter band'inin (±%15) farkında, captures-heavy +7.93%'in baseline olarak fold edildiği — v0.4.0 review Recommendation #4 honored.
- **Bootstrap timeline'ın spec-vs-plan separation.** §6.2'de 5-step bootstrap (placeholder → CI → real numbers → commit → next-PR canonical) spec-time pin'lenmiş; plan-time detail (kim, hangi PR, hangi commit) plan'a delegate. Spec/plan boundary doğru çizilmiş — spec final-shape kontratı, plan execution sırası.
- **Zero new Cargo dep, zero new third-party action.** `jq` GHA-preinstalled (her iki OS), `bc`/`awk` POSIX (mostly), `actions/upload-artifact` (eklenirse) zaten mevcut official action. Memory `feedback_dependency_minimalism` + CLAUDE.md §3 dependency review'a takılmaz.
- **§4 OUT OF SCOPE listesi cömert.** 8 yedi açık reddedilmiş öğe (PR comment bot, auto-baseline-regen, commit-level granularity, cargo-criterion install, stdout parsing, macOS-specific fixture, JSON schema dosyası, history dashboard) — v0.5+ carryover surface'ini şimdiden temizliyor. "v0.4 minor temiz kapanır, v0.5 carryover-free başlar" iddiası §10 son bullet ile pin'lenmiş.
- **§7 risk table 8 satır.** Her risk için olasılık + etki + azaltma; özellikle "macOS GHA shared host'ta absolute timing ubuntu'dan tamamen farklı" risk'i Yüksek-Düşük assessment'i + per-OS baseline mitigation'ı — tasarımın motivation'ıyla risk table'ın iç tutarlılığı sağlam.
- **§5.2 CI workflow self-testing first PR plan.** Bootstrap edge case (placeholder zero numbers) için baseline-zero guard explicit; first PR'da workflow ilk fire log inceleme adımı pin'lenmiş. Spec sadece "bootstrap edge case'i handle edilir" kontratını yazıyor, exact step plan'a — doğru spec/plan ayrımı.
- **CHANGELOG dürüstlüğü.** Iki bullet (CI bench-regression + `unreachable!` reason strings) ayrı; user-facing impact net (CI mechanism + internal code-quality fix). v0.4.1'in iki ortogonal işi karıştırılmıyor. CLAUDE.md §4 "Error messages are user-facing UX" CHANGELOG'a da yansıyor.

---

## Summary

Spec mimari olarak doğru ve dış API'leri (criterion `estimates.json` shape + GHA expression context semantics + jq preinstalled status) verified. v0.4.0 cross-cutting review'un 5 numbered Recommendations + C-1 öğesinin tamamı §10 disposition'da HONORED — memory `feedback_consume_prior_review` mandatesi başarıyla karşılanmış. Bootstrap timeline ve per-OS baseline yaklaşımı M2-Pro-local vs CI-shared-host problemini yapısal çözüyor.

Rev2 öncesi çözülmesi gereken dört orta-büyüklük (🟡) konu var:

1. **I-1** — §2.3 "Compare against baseline" step `jq -e` kullanmıyor + `set -euo pipefail` yok; malformed baseline JSON veya `jq` field-miss silently green döner. Iki satır defansif programlama gerekiyor.
2. **I-2** — `bc` ile floating-point delta + `awk` ile comparison portability tuzakları (locale + scale=2 precision boundary + ubuntu-24.04 `bc` belirsizliği). Tek-pass awk-only computation hem dep-minimal hem locale-safe hem precision-doğru.
3. **I-3** — Umbrella §3.3 `change/estimates.json` der; spec `new/estimates.json` kullanır (doğru tercih çünkü `--save-baseline` kullanmıyoruz, `change/` üretilmiyor). §10 disposition'a explicit bullet eklenmeli — memory `feedback_consume_prior_review`'un umbrella divergence'larına extension'ı + v0.4.0 spec §10 precedent'i.
4. **I-4** — Bootstrap commit-back step §8 acceptance criteria item 7'de ne zaman geçtiği (placeholder OK vs real-numbers required) muğlak + manuel ms/µs→ns conversion human-error riski. `actions/upload-artifact@v4` ile criterion artifact'i upload + jq script'i ile şablon JSON yazma → reviewer tek-komut.

🟢 N-1..N-7 polish: `Install jq` step kaldırılabilir (preinstalled), label oluşturma step'i §2.5'e netleştirilebilir, smoke step'i baseline JSON'unu da kapsayabilir, `needs: test` matrix semantics belgelenebilir, CHANGELOG cümle yapısı düzeltilebilir ("PR open via" → "opt in by labeling"), memory frontmatter naming convention checked, §1 umbrella divergence'ın ilk işareti gizlendi. Kozmetik, ship'i bloklamaz.

Rev2 sonrası APPROVE.

## Verdict — APPROVE_WITH_REVISIONS

**Apply for rev2:** I-1 (`jq -e` + `set -euo pipefail` `Compare against baseline` step'ine), I-2 (`bc` → tek-pass awk; locale + precision + dep-minimal), I-3 (§10'a `change/` → `new/` subdir divergence bullet'i ekle), I-4 (§8 acceptance item 7'yi 7a/7b'ye böl + §2.3 YAML'a artifact upload step + §6.2 step 3'e jq script-based extract). N-1..N-7 polish; spec quality artar ama gate değiller.
