# Senior Rust Architecture Review — v0.1

**Tarih:** 2026-05-21
**Reviewer:** Bağımsız senior Rust engineer subagent (general-purpose, 10+ yıl systems programming briefing'iyle)
**Kapsam:** v0.1 modül yapısı, threading modeli, public API, error handling, güvenlik tehditleri, isimlendirme, eksikler
**Durum:** Tüm bulgular kullanıcı tarafından kabul edildi; spec ve plan'a uygulandı.

Bu doc bir audit kayıt artefaktıdır. İlerideki PR review'larında "bu karar nereden çıktı" sorusunun referansıdır.

---

## Briefing'in Özeti

Senior reviewer'a şunlar sağlandı:
1. `tayf-tasarim.md` (özellikle §2, §3, §4, §6, §7, §10)
2. `CLAUDE.md` (4 kural)
3. Kilitlenmiş kararlar: Rust + std::thread (tokio yok), single crate lib+bin, Linux+Unix+macOS day 1, shell discovery cascade, 7 hardcoded pattern, nix::sys::termios, TDD-for-pure-logic + smoke-for-PTY
4. Önerilen modül listesi (sonradan değiştirilen versiyon): `main`, `lib`, `cli`, `shell`, `pty`, `tty_guard`, `signals`, `io_loop`, `alt_screen`, `line_buffer`, `colorize`, `patterns`
5. Threading modeli proposal: 2 thread (input + output) + main; SIGWINCH için ne yapılacağı açık değildi

Reviewer'dan istenen: validation değil, sert eleştiri. Spesifik dosya/tip isimleri vererek bulgular.

---

## Bulgular ve Verilen Kararlar

### 1. Modül Dekompozisyonu — 3 Yanlış Kesim

| Reviewer'ın bulgusu | Kabul edilen düzeltme | Spec/Plan referansı |
|---|---|---|
| `alt_screen.rs` tek başına çok küçük; pipeline'a leak'liyor | `pipeline.rs` içine fold; gerçek state machine | spec §3.5, plan T6 |
| `colorize.rs` + `patterns.rs` ayrı tutmak premature | Tek `rules.rs` modülü | spec §3.1, plan T4 |
| `Session` overloaded olacak; `Arc<Mutex<Session>>` smell'i kaçınılmaz | `PtySession::into_parts() → (Reader, Writer, Resizer, ChildHandle)` | spec §3.3, plan T13 |
| Eksik modüller: `error.rs`, `style.rs`, `terminfo.rs`, `logging.rs`, `version.rs` | Hepsi eklendi | spec §3.1, plan T2/T3/T8/T9/T10 |
| `shell.rs` discovery cascade `--login` semantiğini de çözmeli | `ShellSpec` + `argv0(login: bool)` | spec §1, plan T7 |

### 2. Threading Modeli — Shutdown Protokolü Eksik

Reviewer'ın bulgusu: "Child exited, drain master, exit cleanly" diye bir cevap yoktu.

**Çözülen sorular:**
- Output thread shutdown: master `read()` Linux'ta `EIO`, macOS'ta `Ok(0)` → her ikisi "drain et + exit"
- Input thread shutdown: stdin asla kapanmaz; `master.write()` üzerinden `EPIPE` ile uyandırma (ama yine de `read()` bloke olabilir → input thread `join` edilmez)
- SIGWINCH race: ioctl master fd'yi etkiler, stdout race'i yok; child redraw'ı kabul edilir
- Signal handling: `signal_hook::iterator::Signals` ile **dedicated thread** + iterator API (closure-based registry değil)

**Spec referansı:** §3.4 (numaralı 10-adımlı shutdown protokolü).
**Plan referansı:** T15 (runtime), T16 (lib facade), T14 (signals).

### 3. Drop / Panic Safety

| Reviewer'ın notu | Karar |
|---|---|
| `Drop` + `set_hook` normal exit, `?`, `panic!` için yeterli | Kabul; T12'de impl |
| `std::process::exit` Drop'u atlar | `clippy.toml`'a `disallowed-methods` ile yasakla; tüm exit'ler `main.rs::main → ExitCode` üzerinden | T1, T17 |
| `SIGKILL`/`SIGSEGV`/`abort` in-process kurtarılamaz | README'de "tayf aniden öldürülürse `reset`" notu | T20 |
| FFI panic boundary'si tayf'da yok | OK |
| termios restore fd snapshot'ı saklanmalı | `TtyGuard` `fd` field'ı tutuyor | T12 |

### 4. Public API Shape

- **`Tayf::run(args) -> Result<ExitCode, Error>` facade** — `lib.rs`'de eklendi. `main.rs` orchestration'ı barındırmaz, lib API'sinden test edilebilir.
- **OutputProcessor: concrete struct**, trait object/generic değil — `Pipeline` concrete tutuldu.
- **signals::install → spawn_handler(resizer, child_pgid) → SignalGuard** — Session leak'i ortadan kalktı.

### 5. Error Handling — Tek Enum, `thiserror`

- Tek `tayf::Error` enum.
- Exit code mapping `main.rs`'de: child code | 64 (EX_USAGE) | 70 (EX_SOFTWARE) | 71 (EX_OSERR).
- Built-in regex compile error'ları unreachable; `expect("builtin pattern must compile")` izinli (CLAUDE.md §2).

### 6. Güvenlik — Mimari Düzeyde

| Sorun | Çözüm |
|---|---|
| `line_buffer` 64KB cap davranışı belirsiz | "Flush partial as-is, regex uygulanmaz" — kayıp yok | T5 |
| `AltScreenDetector` chunk-boundary split bug | 5-state machine (memmem değil) | T6 |
| `Pattern = (Regex, Style)` v0.4'te breaking change yaratacaktı | Day 1'den `Compiled { set, individuals, styles }` | T4 |
| ANSI emission kontrolsüz | `Style::to_sgr()` audit gate testi: yalnızca SGR | T3 |
| termios vs SIGCHLD race | Pratik race yok; SIGCHLD handler içinde `exit` çağırma | T14 |

### 7. Naming Düzeltmeleri

| Eski | Yeni | Neden |
|---|---|---|
| `io_loop.rs` | **`runtime.rs`** | "loop" event-loop çağrıştırıyor |
| `Session` | **`PtySession`** | ssh/shell/terminal session ile karışıyor |
| `Colorizer::process` | **`apply_rules(line, out)`** | vague verb |
| `signals::install` | **`signals::spawn_handler → SignalGuard`** | lifetime explicit |
| `alt_screen.rs` | (folded into pipeline) | — |
| `patterns.rs` | (folded into rules) | — |

### 8. Eksikler (Sonradan Eklendi)

- `error.rs` ✅ T2
- `style.rs` ✅ T3
- `terminfo.rs` ✅ T8 (color depth + isatty)
- `logging.rs` ✅ T9 (`TAYF_LOG` gated tracing)
- `version.rs` + `build.rs` ✅ T10 (`built` crate)
- `tests/common/mod.rs` PTY fixture ✅ T19
- `Cargo.toml` `[lib]` ve `[[bin]]` explicit ✅ T1
- `benches/throughput.rs` (criterion) — **v0.2'ye ertelendi** (kullanıcının "CI/CD post-v0.1" kararıyla uyumlu)

### 9. Reviewer'ın "Kod Yazılmadan Mutlaka Çözülmeli" Dediği 3 Şey

1. **Shutdown protokolünü numaralı sıra olarak yaz** ✅ spec §3.4 (10 adım)
2. **Alt-screen gerçek state machine ya da vte; naive memmem yasak** ✅ spec §3.5, plan T6
3. **`PtySession::into_parts` ve `Compiled { set, individuals, styles }` şekillerini şimdi kilitle** ✅ spec §3.3 ve §3.6, plan T13 ve T4

---

## Reviewer'ın Önerip Geri Çekilen Bir Karar

- "v0.1'de tam VTE entegrasyonu eklenmeli; naive state machine v0.1 zayıflığı" — kullanıcının "v0.1 iskelet" kararına saygıyla 5-state machine ile sınırlı kaldık. v0.3'te tam `vte` crate entegrasyonu gelir (master §11). Bu seçim spec §10'da açıkça "v0.1 limit" olarak dökümante edildi.

---

## Bu Doc'tan Çıkarılacak Genel Ders

- **Bağımsız subagent review erken aşamada paha biçilmez.** ~12 dakikalık review, ~80 saatlik implementasyonda en az 3-4 büyük refactor'ü önledi (modül bölünmeleri, into_parts, Compiled shape).
- **Naming kararları erken alınınca breaking change'ler ucuz.** `Session → PtySession` rename'i bir sed komutu; ama 200 LOC sonra public API kırılması olur.
- **Shutdown protokolü implementation'ın değil, mimarinin parçası.** Reviewer'ın #1 önerisi bunu spec'e yazmaktı, kod yazmadan.

---

_Bu doc kalıcı bir karar kayıt artefaktıdır. Tayf v0.1'in iz sürülebilir mimari geçmişidir._
