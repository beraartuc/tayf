# tayf — Tasarım Dokümanı

Terminal-agnostik, PTY-tabanlı, regex ile gerçek-zamanlı çıktı renklendirme aracı. iTerm2'nin "Triggers" özelliğinin Linux/macOS/Windows üzerinde, herhangi bir terminal emulator'ında çalışan açık kaynaklı alternatifi.

> **Proje adı:** `tayf` — Türkçe "renk tayfı / ışık tayfı" (spektrum) anlamından. 4 harf, ASCII-temiz, telaffuzu evrensel. Tool'un yaptığı işin doğrudan metaforu: terminal çıktısının görünmez tayfını ortaya çıkarmak.

---

## 1. Vizyon

Kullanıcı terminalinde `tayf` (veya benzer isim) yazar. Bu komut, sistemin default shell'ini bir pseudo-terminal içinde başlatır, tüm output stream'ini ortadan geçirir, kullanıcının config dosyasında tanımladığı regex pattern'lerine göre renklendirme uygular ve gerçek terminale aktarır. Kullanıcı açısından farkı yoktur — her komut normal çalışır, ama IP adresleri, MAC adresleri, log seviyeleri, HTTP status kodları, timestamp'ler vs. otomatik renklenmiş görünür.

### Pazar Doğrulaması

> Oh My Zsh resmi GitHub Discussions'ta (Mayıs 2023, [#11687](https://github.com/ohmyzsh/ohmyzsh/discussions/11687)) tam olarak bu özellik soruluyor: "regex ile log satırlarını renklendirmek için zsh plugin'i". 186k yıldızlı, 2400+ contributor'lı bu topluluğun verdiği cevap: "Çözüm yok, harici olarak `grc` kur ve manuel config yaz."
>
> **Çıkarımlar:**
> - **Talep doğrulandı.** En büyük zsh topluluğunda bile ihtiyaç dile getiriliyor.
> - **Mevcut çözümler eksik.** Cevap pipe-tabanlı `grc`'ye yönlendiriyor — yani UX kötü (her komut için manuel alias).
> - **Niş açık.** Shell-agnostic, PTY-tabanlı, sıfır-config bir araç bu boşluğu doldurur.
> - **Konumlandırma.** "The iTerm2 Triggers experience for any terminal — works alongside your existing shell setup, no aliases or per-command wrapping."

### Hedefler
- **Terminal-agnostik:** Kitty, Alacritty, WezTerm, GNOME Terminal, Konsole, foot, Ghostty, Windows Terminal, hepsinde aynı şekilde çalışır.
- **Sıfır-konfigürasyon başlangıç:** Sensible default config ile kurar kurmaz iş görür.
- **Tek binary:** `cargo install` veya tek `curl | sh` ile kurulum.
- **Native performans:** Hızlı stream'lerde (örn. `cat /var/log/...`, `journalctl -f`) algılanır gecikme yaratmaz.
- **Genişletilebilir:** Kullanıcı kendi regex profillerini tanımlayabilir, hot-reload ile düzenleyebilir.

### Hedef Olmayan Şeyler (v1 için)
- Şebeke üzerinden çalışma (mosh gibi).
- Tam terminal emulator olma — sadece passthrough + renklendirme.
- Windows desteği (v1 sonrası).
- Komutlara müdahale (sadece görsel katman).

---

## 2. Mimari Karar: Neden PTY Wrapper?

İki temel yaklaşım vardır ve bu seçim her şeyi belirler.

### Seçenek A — Pipe Filtresi (red edildi)

Kullanım: `komut | tayf`. Tool stdin'den okur, regex uygular, stdout'a yazar.

| Artı | Eksi |
|---|---|
| Çok basit (200 satır kod) | Her komutu pipe etmek zorundasın |
| Hızlı geliştirilir | `vim`, `htop`, `less` gibi interaktif programlarda çalışmaz |
| Doğal Unix felsefesi | TTY davranışı bozulur (renk algılaması, buffer modu) |

`grc` ve `ccze` bu yaklaşımı kullanır. iTerm2 triggers deneyimini vermez.

### Seçenek B — PTY Wrapper (seçildi)

Tool bir pseudo-terminal açar, içinde kullanıcının shell'ini spawn eder. PTY master'dan gelen tüm output ortadan geçirilir, regex uygulanır, gerçek stdout'a yazılır. Stdin de aynı şekilde tersine.

| Artı | Eksi |
|---|---|
| Transparent — her komut otomatik renklenir | İmplementasyon karmaşık (~2000 satır) |
| İnteraktif programlarla uyumlu | Birçok edge case (alt-screen, signals, UTF-8) |
| Tek nokta kurulum (`.bashrc`'a `exec tayf`) | Performans kritik (her byte'ı taramak) |
| Terminal emulator'dan bağımsız | |

ChromaTerm bu yaklaşımı kullanır ve istediğimiz deneyimi verir.

**Karar: PTY wrapper.**

---

## 3. Sistem Mimarisi

```
┌─────────────────────────────────────────────────────────────────┐
│                    Gerçek Terminal Emulator                     │
│                   (Kitty / Alacritty / vs.)                     │
└─────────────────┬─────────────────────────────────▲─────────────┘
                  │ stdin                            │ stdout
                  ▼                                  │
┌─────────────────────────────────────────────────────────────────┐
│                        tayf (Rust binary)                      │
│                                                                 │
│   ┌──────────────┐    ┌──────────────────┐    ┌──────────────┐  │
│   │ Stdin Thread │───▶│  Signal Handler  │    │  Colorizer   │  │
│   │  (raw mode)  │    │ (SIGWINCH/INT)   │    │   Engine     │  │
│   └──────┬───────┘    └──────────────────┘    └──────▲───────┘  │
│          │                                           │          │
│          │                  ┌────────────────────────┘          │
│          ▼                  │                                   │
│   ┌──────────────────────────────────────────────────────────┐  │
│   │              VTE Parser (state machine)                  │  │
│   │  Tracks: SGR state, alt-screen, OSC, bracketed paste     │  │
│   └──────────────────────────────────────────────────────────┘  │
│          │                  ▲                                   │
└──────────┼──────────────────┼───────────────────────────────────┘
           │ PTY master       │ PTY master
           ▼ (write)          │ (read)
┌─────────────────────────────────────────────────────────────────┐
│                  Pseudo Terminal (kernel)                       │
└──────────┬──────────────────▲───────────────────────────────────┘
           ▼ slave            │ slave
┌─────────────────────────────────────────────────────────────────┐
│            Kullanıcının Shell'i ve Child Process'leri           │
│                 (bash / zsh / fish — ls, vim, ...)              │
└─────────────────────────────────────────────────────────────────┘
```

**Ana döngü:** İki async task / thread.

- **Input task:** Gerçek stdin → PTY master (değiştirilmeden)
- **Output task:** PTY master → VTE parser → Colorizer → gerçek stdout

VTE parser kritik çünkü stream'in yapısını anlamadan regex uygulanamaz.

---

## 4. Teknoloji Yığını

| Crate | Amaç | Notlar |
|---|---|---|
| `portable-pty` | PTY açma, child process spawn | WezTerm yazarı tarafından, en olgun seçenek |
| `regex` | Pattern matching | Rust'ın hızlı motoru, RegexSet desteği |
| `vte` | ANSI escape parser | Alacritty'nin de kullandığı state machine |
| `serde` + `toml` | Config parsing | TOML insan-dostu, YAML'dan daha az tuzak |
| `nix` | Düşük seviye Unix (ioctl, signals) | TIOCSWINSZ, raw mode |
| `crossterm` veya `termios` | Terminal mode kontrolü | İkisinden biri yeterli |
| `notify` | Config hot-reload | Opsiyonel — v1'de yok |
| `tracing` | Yapılandırılmış log | Debug için, üretimde sessiz |
| `clap` | CLI arg parsing | `tayf --config x.toml`, `tayf --profile network` |

**Tokio kullanılsın mı?** Sadece iki I/O stream var, async kompleksitesi gereksiz. Düz `std::thread` + blocking I/O yeterli ve daha az bağımlılık.

---

## 5. Konfigürasyon Formatı

`~/.config/tayf/config.toml`:

```toml
[general]
default_shell = "/usr/bin/zsh"        # boşsa $SHELL kullanılır
profile = "default"
respect_existing_colors = true        # mevcut ANSI'i ezme

[[rules]]
name = "IPv4"
pattern = '\b(?:\d{1,3}\.){3}\d{1,3}\b'
style = { fg = "yellow", bold = true }

[[rules]]
name = "IPv6"
pattern = '\b(?:[0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}\b'
style = { fg = "bright_yellow" }

[[rules]]
name = "MAC"
pattern = '\b(?:[0-9a-fA-F]{2}[:-]){5}[0-9a-fA-F]{2}\b'
style = { fg = "cyan" }

[[rules]]
name = "Log levels"
pattern = '\b(ERROR|FAIL|CRITICAL|FATAL)\b'
style = { fg = "white", bg = "red", bold = true }

[[rules]]
name = "HTTP status — error"
pattern = '\b[45]\d{2}\b'
style = { fg = "red" }

# Capture group başına farklı renk
[[rules]]
name = "Key-value"
pattern = '(\w+)=(\S+)'
groups = [
    { group = 1, style = { fg = "blue" } },
    { group = 2, style = { fg = "green" } },
]

# Profil bazlı override
[profiles.network]
inherit = "default"
extra_rules_file = "~/.config/tayf/network.toml"

[profiles.minimal]
rules = ["IPv4", "Log levels"]   # whitelist
```

### Renk dili
- 16 ANSI ismi: `black`, `red`, ..., `bright_white`
- 256-color: `color(178)`
- Truecolor: `"#ff8800"` veya `rgb(255, 136, 0)`
- Attribute: `bold`, `italic`, `underline`, `dim`, `blink`
- Tool runtime'da `$COLORTERM` ve `$TERM`'e bakıp en yüksek desteklenen depth'i otomatik seçer; truecolor değerleri 256'ya düşürür gerekirse.

---

## 6. Edge Case'ler

Bu bölüm bilinçli olarak detaylı. Her biri implementasyonda ayrı bir test case olmalı.

### 6.1 Chunk Sınırı Problemi

**Sorun:** PTY master'dan gelen `read()` çağrıları rastgele büyüklükte chunk döner. Bir IPv4 adresi `192.168.` + `1.1` şeklinde iki ayrı okumada bölünmüş gelebilir. Regex bunu bulamaz.

**Çözüm:**
- Satır-temelli buffer: `\n` görene kadar bekle, sonra satırı işle.
- Prompt için timeout: 50ms boyunca `\n` gelmezse mevcut buffer'ı flush'la (interaktif prompt'larda satır sonu yoktur).
- Maksimum buffer boyutu (örn. 64KB) — DoS önleme.

```rust
fn process_chunk(&mut self, chunk: &[u8]) -> Vec<u8> {
    self.buffer.extend_from_slice(chunk);
    let mut output = Vec::new();
    while let Some(pos) = memchr::memchr(b'\n', &self.buffer) {
        let line = self.buffer.drain(..=pos).collect::<Vec<_>>();
        output.extend(self.colorize_line(&line));
    }
    // Timer ile flush'lanmamış kuyruk
    output
}
```

### 6.2 Mevcut ANSI Kodlarını Koruma

**Sorun:** `ls --color`, `git diff`, `grep --color`, `systemctl` zaten escape sequence yolluyor. Sen kendi rengini eklerken naive bir `\x1b[0m` (reset) kullanırsan, içine girdiğin renkli bağlamı kaybedersin.

Örnek bozulan output:
```
git diff şuna benzer:
\x1b[32m+ const ip = "192.168.1.1";\x1b[0m   # yeşil
```
Senin tool 192.168.1.1'i sarıya boyamak isterse şunu yazar:
```
\x1b[32m+ const ip = "\x1b[33m192.168.1.1\x1b[0m";\x1b[0m
                                          ^^^^^^^^^
                                          burası yeşili öldürdü
```

**Çözüm:** VTE parser ile aktif SGR state'i takip et. Senin eklediğin rengin sonunda `\x1b[0m` yerine "önceki SGR state"e geri dön. Veya: matching için ANSI-stripped versiyon kullan, ama enjeksiyonu orijinal byte stream üzerinde offset eşlemesiyle yap.

**Daha güvenli kural:** Eğer regex'in eşleştirdiği aralığın **içinde** zaten bir SGR sequence varsa, bu eşleşmeyi atla. Programın renklendirme niyetine saygı duy.

### 6.3 Alternatif Ekran Buffer'ı (Alt-Screen)

**Sorun:** `vim`, `htop`, `less`, `tmux`, `man` gibi programlar `\x1b[?1049h` yollayıp alternatif ekran buffer'ına geçer. Bu modda program absolute cursor positioning kullanır (`\x1b[5;10H` — 5. satır, 10. sütun). Sen ortaya regex eşleşmesi için SGR enjekte edersen byte sayısı değişir, ama cursor pozisyonları sabit olduğu için UI dağılmaz — ama renk leak'leri çok kötü olur.

**Çözüm:** Alt-screen mode'a girildiğini algıla (`\x1b[?1049h`, eski `\x1b[?47h`, `\x1b[?1047h`), bu modda **passthrough**'a geç. `\x1b[?1049l` görünce regex'leri yeniden devreye sok.

VTE parser bunu zaten bilir; sadece bir state flag'i yeterli.

### 6.4 SIGWINCH (Pencere Boyutu Değişimi)

**Sorun:** Kullanıcı terminal penceresini büyütüp küçültürse, child shell yanlış genişlikte yazar. Çünkü PTY'ın boyutunu gerçek terminalin boyutu değil, senin başlangıçta verdiğin boyut belirler.

**Çözüm:**
```rust
signal_hook::iterator::Signals::new(&[SIGWINCH])?;
// Sinyali yakaladığında:
let size = terminal_size()?;
pty.master.resize(PtySize { rows: size.rows, cols: size.cols, ... })?;
```

Buna ek olarak SIGWINCH'i child process group'a da yolla, bazı programlar (vim) bunu beklerken.

### 6.5 Stdin Raw Mode

**Sorun:** Default'ta stdin "cooked mode"dadır — kernel satır editor sağlar, Ctrl-C SIGINT olarak senin process'ine gelir. Sen interaktif bir shell çalıştırdığın için bu yanlış.

**Çözüm:** `termios.c_lflag &= !(ICANON | ECHO | ISIG)`. Tüm byte'lar ham haliyle PTY'a geçer, sinyal yorumlamasını child shell yapar.

**Önemli:** Process çıkarken termios'u eski haline geri yükle, yoksa kullanıcının shell'i kullanılamaz halde kalır. `Drop` trait ile RAII garantisi:

```rust
struct TerminalGuard { original: Termios }
impl Drop for TerminalGuard {
    fn drop(&mut self) { tcsetattr(0, TCSANOW, &self.original).ok(); }
}
```

Panic durumunda da çalışır (unwind ile). `std::panic::set_hook` ile ekstra güvenlik.

### 6.6 UTF-8 Multi-byte Karakterler

**Sorun:** Türkçe karakterler 2 byte, emoji 4 byte. Bir chunk'ın sonunda multi-byte sequence'in ortasında kesilirsen, hem regex matching bozulur hem de stdout'a invalid UTF-8 gönderirsin.

**Çözüm:**
- Buffer'a byte olarak ekle.
- İşlerken `std::str::from_utf8` deneyip, `Err`'in `valid_up_to()` değeri ile geçerli kısmı al, geri kalanı buffer'a bırak.
- `bstr` crate'i bu pattern'i kolaylaştırır.

### 6.7 OSC Sequences (Title, Hyperlinks, Semantic Prompts)

**Sorun:** OSC sequence'ları kendi sözdizimine sahip, BEL veya ST ile biter, içlerinde regex matching anlamlı değil hatta tehlikeli. Üç önemli kullanım var:

- `\x1b]0;başlık\x07` — pencere başlığı
- `\x1b]8;;https://...\x07link metni\x1b]8;;\x07` — hyperlink (OSC 8)
- `\x1b]133;A\x07 ... \x1b]133;B\x07 ... \x1b]133;C\x07` — **semantic prompt markers (OSC 133)**

OSC 133 özellikle önemli çünkü Powerlevel10k, modern Oh My Zsh temaları, fish prompt'u, starship hepsi bunu yolluyor. Terminal emulator (iTerm2, WezTerm, Kitty, Ghostty, VSCode terminal) bunlarla prompt'un nerede başlayıp bittiğini, çıktının nerede olduğunu anlıyor — "jump to previous prompt", komut süresi ölçümü gibi özellikler buna bağlı. Eğer regex bu sequence'in içine girip byte eklerse semantic parsing bozulur.

**Çözüm:** VTE parser zaten OSC'yi tanıyor. Parser durumu OSC'de iken matching tamamen kapalı tut, sequence aynen geçsin. Test matrisinde özellikle şu kombinasyonları kontrol et:

- zsh + Oh My Zsh + Powerlevel10k (OSC 133 + ekstra OSC'ler)
- WezTerm + zsh (OSC 7 — current directory)
- Kitty + fish + starship
- iTerm2 shell integration script aktif iken

### 6.8 Bracketed Paste Mode

**Sorun:** `\x1b[?2004h` ile etkinleştirilir. Aktif iken yapıştırılan metin `\x1b[200~...\x1b[201~` arasına sarılır. Eğer bu marker'ları regex'le karıştırırsan paste tespiti bozulur.

**Çözüm:** Bu mode'da paste içeriği passthrough — kullanıcının yapıştırdığı metni renklendirmek anlamsız ve potansiyel olarak yanlış.

### 6.9 Mouse Tracking

**Sorun:** `\x1b[?1000h` ve varyantları ile etkinleştirilir. Mouse event'leri input olarak terminal'den uygulamaya gelir, ham binary'dir. Bunlara dokunma.

**Çözüm:** Input passthrough zaten bunları transparent geçiyor olmalı. Ekstra iş yok ama test et.

### 6.10 Performans — Hot Path

**Sorun:** `cat /var/log/syslog`, `journalctl --no-pager`, `find /` gibi komutlar saniyede yüz binlerce byte üretir. Naive regex uygulaması 10x-100x yavaşlama yaratır.

**Çözüm:**
- **RegexSet:** Tüm pattern'leri tek geçişte tara, hangileri match etti onları öğren.
- **Aho-Corasick:** Literal substring set'leri için (örn. log level kelimeleri) regex'ten çok daha hızlı.
- **Allocation azaltma:** Hot path'te `Vec::new()` çağırma; reusable buffer kullan.
- **isatty kontrolü:** Stdout TTY değilse (`tayf | grep ...`) tüm regex'leri bypass et, sadece passthrough.
- **Streaming mode hint:** Bir process saniyede X byte'tan fazla üretiyorsa, line buffer yerine block-level matching'e geç.

**Hedef:** Native `cat` ile karşılaştırıldığında %20'den az yavaşlama.

### 6.11 Signal Forwarding

**Sorun:** Wrapper'a gelen SIGINT/SIGTERM child'a iletilmeli, ama doğru şekilde — child'ın process group'una.

**Çözüm:**
- Raw mode aktif olduğu için Ctrl-C zaten byte olarak PTY'a gidiyor; child shell SIGINT'i kendi alt process'lerine yolluyor.
- Ama wrapper'a doğrudan `kill -INT $PID` gelirse, bunu child process group'una forward et.
- Child exit ettiğinde wrapper'ın exit code'u child'ınkiyle eşleşmeli.

### 6.12 Tmux/Screen İçinde Çalışma

**Sorun:** `tmux` veya `screen` içinde çalışıyorsa, dış mux kendi escape sequence layer'ı ekleyebilir (DCS passthrough, `\x1bPtmux;...\x1b\\`).

**Çözüm v1:** Bilinen durum, dökümante et, ama özel handling yok. Çoğu kullanım çalışacaktır.

### 6.13 Capture Group Renklendirme

**Sorun:** `(\w+)=(\S+)` için anahtar mavi, değer yeşil yapmak istiyorsun. Tek bir SGR enjeksiyonu yetmiyor.

**Çözüm:** Config'te `groups` array'i tanımla, her group için ayrı style. Match'in capture grouplarını alıp sondan başa doğru SGR enjeksiyonu yap (sondan başa çünkü offset'ler kaymasın).

### 6.14 Çakışan Eşleşmeler

**Sorun:** İki regex aynı bölgeyi match ederse hangisi kazanır? `\d+` ve `\b\d{3}\b` çakışabilir.

**Çözüm:** Config'teki tanımlama sırası öncelik. Daha sonra `priority` field'ı eklenebilir. Algoritma: tüm match'leri topla, sort by (start, -priority), greedy non-overlapping seç.

### 6.15 Hot Reload

**İstenilen:** Config dosyası değişince yeniden yükle, programı restart etmeden.

**Çözüm:** `notify` crate ile dosya değişikliğini izle. Yeni config'i parse et, başarılıysa atomik swap (Arc<Config>). v1'de yok, v1.1'de eklenir.

> **Status:** v0.2.1'de uygulandı (2026-05-22). `notify 8.2` + 200 ms manuel debounce + `arc_swap::ArcSwap<Compiled>` ile atomik swap. SIGHUP manuel tetikleyici olarak destekleniyor; parse hatasında eski rule set korunur ve `warn_msg!` ile stderr'e yazılır. Detaylı tasarım: `docs/superpowers/specs/2026-05-22-tayf-v0.2.1-hot-reload.md`; uygulama planı: `docs/superpowers/plans/2026-05-22-tayf-v0.2.1.md`.

### 6.16 Output Pipe'a Yönlendirildiğinde

**Sorun:** `tayf | less` yapılırsa stdout TTY değildir; ANSI kod eklemek anlamsız ve `less`'in gözünden bozuk görünür.

**Çözüm:** `atty::is(Stream::Stdout)` ile kontrol et. TTY değilse renksiz passthrough.

---

## 7. Performans Hedefleri

| Senaryo | Hedef |
|---|---|
| İdle (kullanıcı yazıyor) | < 1ms gecikme |
| Komut çıktısı (örn. `ls`) | İnsan algılayamaz (<16ms) |
| `cat largefile.txt` | Native'in %120'sinden hızlı |
| `journalctl -f` (canlı log) | Düşmeyen frame rate |
| Memory footprint | < 20MB resident |

Benchmark suite kritik — her PR'da regression check.

---

## 8. Benzer Projeler — İnceleme Notları

İmplementasyon sırasında bu projeleri reference olarak incelemek ciddi zaman kazandırır.

### 8.1 ChromaTerm ⭐ (En Yakın Analog)
- **Link:** https://github.com/hSaria/ChromaTerm
- **Dil:** Python
- **Ne yapar:** Aynı bizimkinin yaptığı — PTY wrapping, regex tabanlı renklendirme, YAML config.
- **Ne öğreniriz:**
  - YAML config formatının olgunlaşmış hali — kendi TOML'umuzun tasarımı için.
  - Edge case handling yaklaşımları (özellikle alt-screen).
  - `--rgb` flag'iyle truecolor zorlaması.
  - Group highlighting nasıl yapılıyor.
- **Bizden farkı:** Python performansı zayıf, tek binary değil. Bizim Rust versiyonumuz hızlı + tek binary dağıtım sunar — gerçek bir niche var.

### 8.2 grc (Generic Colouriser)
- **Link:** https://github.com/garabik/grc
- **Dil:** Python
- **Ne yapar:** Pipe filtresi olarak çalışır. Her komut için ayrı bir config dosyası.
- **Ne öğreniriz:**
  - Pre-built konfigürasyon kütüphanesi — `ip`, `ping`, `traceroute`, `netstat`, `mount`, `df`, `ps` için hazır pattern'ler.
  - Bu pattern'leri bizim format'a port'lamak hızlı başlangıç sağlar.
- **Bizden farkı:** Pipe model; her komutu manuel pipe etmek gerek; interaktif çalışmaz.

### 8.3 ccze
- **Link:** https://github.com/madhouse/ccze
- **Dil:** C
- **Ne yapar:** Log dosyaları için renklendirici, hem pipe hem standalone çalışır.
- **Ne öğreniriz:**
  - C'de düşük seviye terminal handling.
  - Log-spesifik pattern'ler (syslog format, Apache log, vs.).
- **Bizden farkı:** Plugin sistemi karmaşık, sadece log odaklı.

### 8.4 colout
- **Link:** https://github.com/nojhan/colout
- **Dil:** Python
- **Ne yapar:** Tek pipe filtresi, ama çok güçlü — `colout 'regex' color1,color2,color3` gibi inline syntax.
- **Ne öğreniriz:**
  - CLI-only kullanım için ergonomik flag tasarımı.
  - Çoklu renk seçeneklerinin sunumu.

### 8.5 lnav (Log Navigator)
- **Link:** https://github.com/tstack/lnav
- **Dil:** C++
- **Ne yapar:** Logları tarihsel, format-farkındalıklı, SQL-sorgulanabilir şekilde gösterir.
- **Ne öğreniriz:**
  - Otomatik log format tespiti.
  - Smart timestamp parsing.
  - Highlight'ları farklı log tip'leri için scope'lama.
- **Bizden farkı:** Tam bir TUI, dedicated viewer; bizim aracımız ise universal.

### 8.6 Alacritty / VTE Crate
- **Link:** https://github.com/alacritty/vte
- **Dil:** Rust
- **Ne yapar:** ANSI escape sequence parser'ı, state machine olarak.
- **Ne öğreniriz / nasıl kullanırız:** Bunu doğrudan dependency olarak kullanıyoruz. Source code'una bakıp parser callback API'sını anlamak şart.
- **Trait `Perform`:** print, execute, hook, put, osc_dispatch, csi_dispatch, esc_dispatch metodlarını implement edersek tüm sequence'ları yakalarız.

### 8.7 WezTerm
- **Link:** https://github.com/wez/wezterm
- **Dil:** Rust
- **Ne yapar:** Tam terminal emulator, ama içinde `portable-pty` ve harika PTY abstraction'ları var.
- **Ne öğreniriz:** PTY açma, spawn, resize, signal handling — production kalitesinde örnekler.

### 8.8 Mosh
- **Link:** https://github.com/mobile-shell/mosh
- **Dil:** C++
- **Ne yapar:** PTY içeriklerini ağ üzerinden senkronize tutar.
- **Ne öğreniriz:** Stream'i predictive olarak parse etme; terminal state'i bir veri yapısı olarak modelleme.

### 8.9 expectrl
- **Link:** https://github.com/zhiburt/expectrl
- **Dil:** Rust
- **Ne yapar:** `expect`'in Rust portu — PTY içinde process spawn edip etkileşim test etme.
- **Ne öğreniriz:** Rust'ta PTY wrapping'in idiomatik şekli, async ile birlikte.

### 8.10 bat
- **Link:** https://github.com/sharkdp/bat
- **Dil:** Rust
- **Ne yapar:** `cat`'in syntax highlighting'li hali.
- **Ne öğreniriz:**
  - Renk teması formatı (Sublime Text `.tmTheme` reuse).
  - Cross-platform Rust dağıtım pipeline'ı (musl static linking, GitHub Actions, brew/AUR/scoop publishing).
- **Build/CI yapılarını inceleyip kopyala** — bizim için altın değerinde.

### 8.11 delta
- **Link:** https://github.com/dandavison/delta
- **Dil:** Rust
- **Ne yapar:** Git diff için pager + syntax highlighter.
- **Ne öğreniriz:**
  - ANSI-aware stream processing.
  - Mevcut renkleri koruyup üzerine ek renk uygulama (bizim 6.2 edge case'imiz!).
- **Source code 6.2 için en iyi referans.** `src/paint.rs` özellikle.

### 8.12 zellij
- **Link:** https://github.com/zellij-org/zellij
- **Dil:** Rust
- **Ne yapar:** Modern tmux alternatifi.
- **Ne öğreniriz:** PTY multiplexing, signal forwarding, resize handling — büyük ölçekte üretim kalitesinde Rust örneği.

---

## 9. Dağıtım ve Kurulum

### Tek binary
Hedef: tek dosya, sıfır runtime dependency.

```bash
# musl ile static link
cargo build --release --target x86_64-unknown-linux-musl
```

### Dağıtım kanalları
- **GitHub Releases:** prebuilt binaries (Linux x86_64/aarch64 musl, macOS x86_64/aarch64).
- **`cargo install tayf`** — Rust kullanıcıları için.
- **Install script:** `curl -fsSL https://tayf.sh/install.sh | sh` (bat'ın yaklaşımı).
- **Paket yöneticileri:**
  - Homebrew (macOS, Linux)
  - AUR (Arch)
  - COPR (Fedora) — Silverblue için kritik, çünkü layered package olarak rpm-ostree ile kurulur.
  - Flathub? Terminal aracı için Flatpak ergonomik değil, atla.
  - `apt` / `dnf` — community packaging'a bırakılır.

### Silverblue özelinde
İki yol:
1. `~/.local/bin/tayf` koy, `~/.bashrc` veya `~/.zshrc` sonuna `[[ $- == *i* && -z "$TAYF_ACTIVE" ]] && exec tayf` ekle. `TAYF_ACTIVE` env var'ı recursive loop'u önler.
2. Toolbox/distrobox içinde çalıştır.

### Shell entegrasyonu
İlk sefer kurulumda `tayf init bash | tayf init zsh | tayf init fish` komutları ile shell config'ine eklenecek satırları ürettir — starship/zoxide'in yaptığı pattern.

---

## 10. Shell Framework'leri ile Uyumluluk

Tayf PTY katmanında çalıştığı için kullanıcının shell yapılandırmasıyla **çakışmaz, tamamlar**. Bu bölüm yaygın shell araç-zincirleriyle birlikte kullanım davranışını dökümante eder.

### 10.1 Katman Modeli

Anahtar fikir: shell framework'leri shell process'inin **içinde** çalışır (prompt, completion, line editor); tayf ise shell process'inin **dışında**, PTY I/O katmanında çalışır. İki katman birbirini görmez:

```
┌──────────────────────────────────────┐
│   Terminal Emulator (Kitty, vb.)     │
└──────────────────┬───────────────────┘
                   │ PTY I/O
┌──────────────────▼───────────────────┐
│   tayf (PTY wrapper — output regex) │  ◀── BİZİM KATMANIMIZ
└──────────────────┬───────────────────┘
                   │ stdin/stdout
┌──────────────────▼───────────────────┐
│   zsh / bash / fish (shell process)  │
│   ┌────────────────────────────────┐ │
│   │ Oh My Zsh / Prezto / Starship  │ │  ◀── SHELL FRAMEWORK KATMANI
│   │ zsh-syntax-highlighting        │ │
│   │ zsh-autosuggestions / vi-mode  │ │
│   └────────────────────────────────┘ │
└──────────────────────────────────────┘
```

Tayf shell'in ne çalıştırdığını umursamıyor — sadece byte stream'i ortadan geçiriyor. Shell framework'leri terminal'den gelen byte'ların PTY üzerinden geldiğini görmüyor — onlar için her şey normal terminal gibi.

### 10.2 Uyumluluk Matrisi

| Araç | Tip | Uyum | Not |
|---|---|---|---|
| **Oh My Zsh** (core + temalar) | Shell config framework | ✅ Tam | Prompt OSC sequence'ları aynen geçer. Plugin'leri etkilemez. |
| **Powerlevel10k** | Zsh teması | ✅ Tam | OSC 133 semantic prompt'ları passthrough; instant prompt çalışır. |
| **Prezto** | Zsh framework | ✅ Tam | OMZ ile aynı mantık. |
| **zsh-syntax-highlighting** | Input renklendirme | ✅ Tam | ZLE içinde çalışıyor; tayf input'u aynen geçiriyor. |
| **zsh-autosuggestions** | Input önerisi | ✅ Tam | ZLE içinde; passthrough. |
| **fast-syntax-highlighting** | Alternatif input highlight | ✅ Tam | — |
| **vi-mode / zsh-vi-mode** | Modal input | ✅ Tam | Sadece input handling; etkilenmez. |
| **fish** | Shell | ✅ Tam | Default shell olarak spawn edilebilir. |
| **starship** | Cross-shell prompt | ✅ Tam | Standart ANSI + OSC; passthrough. |
| **atuin** | History yönetimi | ✅ Tam | TUI alt-screen kullanır; alt-screen passthrough kapsar. |
| **zoxide** | Smart cd | ✅ Tam | Output renklendirilebilir (zoxide TUI'si alt-screen'de). |
| **fzf** | Fuzzy finder | ✅ Tam | Alt-screen → passthrough. |
| **tmux** | Multiplexer | ⚠️ Kısmi | Çalışır ama DCS passthrough nüansları var; v1'de bilinen sınır. |
| **GNU screen** | Multiplexer | ⚠️ Kısmi | tmux ile aynı durum. |
| **Oh My Zsh `colored-man-pages`** | Plugin | ✅ Tam | `man` çıktısı ANSI ile gelir; tayf mevcut renkleri koruyarak ek pattern uygular. Kombinasyon güçlü. |
| **Oh My Zsh `colorize` (ccat/cless)** | Plugin | ✅ Tam | Statik dosya renklendirme; tayf üzerine ek katman. |
| **Oh My Zsh `grc` plugin** | Plugin | ⚠️ Gereksiz | İki sistem birden çalışır, çakışmaz ama redundant. Pattern'leri tayf config'ine taşımak önerilir. |
| **bat** (cat alternatifi) | CLI tool | ✅ Tam | bat zaten ANSI yolluyor; tayf üzerine ek. |
| **delta** (git diff) | CLI tool | ✅ Tam | Aynı şekilde — ANSI üst üste binse de ANSI-aware behavior bozmaz. |
| **lazygit / k9s / btop** | TUI tool | ✅ Tam | Tamamen alt-screen → passthrough. |

### 10.3 Oh My Zsh Kullanıcısı İçin Kurulum

Kullanıcının hiçbir OMZ plugin'i değiştirmesine gerek yok. Sadece bir satır eklenir:

```zsh
# ~/.zshrc — Oh My Zsh source'undan SONRA, dosyanın sonunda
[[ -z "$TAYF_ACTIVE" && $- == *i* ]] && exec tayf
```

- `$- == *i*` interaktif shell olduğunu doğrular (script çağrılarında devreye girmesin)
- `TAYF_ACTIVE` env var'ı sonsuz döngüyü önler (tayf kendi spawn ettiği zsh'a bu var'ı set eder)
- `exec` kullanılması mevcut zsh process'ini tayf ile yer değiştirir, ekstra fork yaratmaz

Alternatif ve daha temiz yöntem: terminal emulator'ın **launch command**'ını `tayf` yapmak. Bu durumda `.zshrc`'ye dokunmaya gerek yok, OMZ tayf'ın varlığından hiç haberdar olmaz.

Örnek — Kitty `kitty.conf`:
```
shell tayf
```

Örnek — GNOME Terminal: Preferences → Profile → Command → "Custom command" = `tayf`.

### 10.4 Konumlandırma Mesajı

README'de ve dökümanda tekrarlanması gereken net cümle:

> Tayf **shell'in altında**, **terminal'in üstünde** çalışır. Oh My Zsh, fish, starship, atuin gibi mevcut araçlarına dokunmaz — onlar zaten kurulu olduğu gibi çalışmaya devam eder. Tayf sadece komut çıktılarına bir görsel katman ekler.

Bu mesaj iki şeyi başarır:
1. "Yine bir framework mi takacağım" endişesini kaldırır.
2. Mevcut shell ekosistemiyle rekabet etmediğini, **tamamladığını** vurgular.

### 10.5 Bilinen Etkileşim Sınırları

- **Tmux içinde tayf, ya da tayf içinde tmux?** İdeali: terminal → tayf → tmux → shell. Tayf'ı en dışta tutarsanız tüm tmux panel'lerini renklendirir. Tersi (tayf'ı tmux içinde başlatmak) panel başına kopya process anlamına gelir, çalışır ama kaynak israfı.
- **SSH ile uzak makineye girince ne olur?** Tayf lokal makinede çalışıyor, SSH ile gelen byte'lar normal output gibi — yani uzak makinenin çıktıları da renklendirilir. Bu genelde istenen şey ama bazı kullanıcılar uzak prompt'larıyla çakışma yaşayabilir. Çözüm: `tayf toggle` kısayolu (örn. Ctrl+F12) ile geçici olarak passthrough'a geçmek.
- **`exec bash` veya `exec fish` sonrası?** Tayf'ın spawn ettiği shell'in içinde başka bir shell exec'lenirse, hâlâ aynı PTY'da çalıştığı için tayf çalışmaya devam eder.

---

## 11. Yol Haritası

### v0.1 — Çalışan İskelet ✅ shipped
- [x] PTY açma, shell spawn, iki yönlü kopyalama
- [x] Stdin raw mode + Drop guard
- [x] SIGWINCH handling (+ integration test v0.2.1'de eklendi)
- [x] Çıkış kodu propagation
- [x] Hardcoded 8 regex ile renklendirme (ipv4, ipv6, mac, log_level, http_status, filename, fqdn, duration)
- Patch'ler: v0.1.1 (CI matrix), v0.1.2 (dep cleanup), v0.1.3 (criterion 0.5 → 0.8)

### v0.2 — Config + Hot Reload + Expanded Builtins ✅ shipped
- [x] **v0.2.0** — TOML config parser; CLI flag'leri (`--config`, `--no-color`; `--profile` v0.5'e ertelendi); override / append / disable by name; friendly regex compile errors; color depth downgrade (truecolor → 256 → basic16)
- [x] **v0.2.1** — Config hot reload via `notify` 8 + SIGHUP; 200ms manuel debounce; `arc_swap::ArcSwap<Compiled>` ile atomik rule swap; pipeline per-line snapshot
- [x] **v0.2.2** — Built-in pattern set 8 → 13: `permission` (POSIX ls -l), `timestamp` (ISO-8601/syslog/Apache/RFC 2822 + obsolete US zones), `uuid`, `url` (https?/ssh/ftp), `email`

### v0.2.3 — Preset Themes (sıradaki, patch release)
**Sorun:** Mevcut built-in renkleri dark-bg terminal varsayımıyla seçildi. Light-bg kullanıcılarda `BrightBlack` (timestamp), `White + dim` (permission), `BrightYellow` (ipv6) ya görünmüyor ya okunmuyor. Terminal portability bug'ı.
- [ ] `assets/themes/dark.toml` — mevcut default'lar, explicit
- [ ] `assets/themes/light.toml` — light-bg için ayarlanmış renkler
- [ ] `assets/themes/solarized-dark.toml`, `solarized-light.toml` (opsiyonel)
- [ ] `--theme <name>` CLI flag VEYA `[general] theme = "light"` config option (brainstorming aşamasında karar)
- [ ] README'de theme listesi + ekran görüntüleri
- Hedef olmayan: otomatik bg detection (v0.3 kapsamında)

### v0.3 — ANSI Doğruluğu
- [ ] VTE parser entegrasyonu (vte crate)
- [ ] Alt-screen passthrough (vim/htop çalışırken renklendirme suspend)
- [ ] Mevcut SGR state koruma (`respect_existing_colors` gerçekten devreye girer)
- [ ] OSC / bracketed paste handling
- [ ] Otomatik terminal bg detection (COLORFGBG / OSC 11)
- [ ] Trailing-punctuation trim in URL match (`https://example.com.` → `.` dışarda)
- [ ] `git@host:path` SSH URL alt-form (`url` pattern'ın 4. branch'i)
- [ ] `--bypass` / `TAYF_DISABLE=1` env-var escape hatch
- [ ] `--no-hot-reload` opt-out flag
- [ ] Reload inline overlay banner ("tayf: config reloaded", OSC 133 ile)
- [ ] Duration pattern: bare `s`/`m`/`h` units geri eklenebilir (ANSI awareness sağlandığı için SGR çakışması kaybolur)

### v0.4 — Performans
- [ ] RegexSet fast-path (compiled `set` field zaten populated, switch sadece burada)
- [ ] Aho-Corasick literal optimizasyonu
- [ ] Benchmark suite genişletme + CI'da regression detection
- [ ] Streaming heuristics

### v0.5 — Genişletilebilirlik + Config TUI
- [ ] Capture group renklendirme (`$1 kırmızı $2 yeşil`)
- [ ] Profile system (`[profiles.network]`, `tayf --profile network`)
- [ ] Built-in profile kütüphanesi: network, k8s, docker, web logs, AWS, GCP
- [ ] `tayf config` interaktif TUI (ratatui ile, fullscreen, resize-safe; canlı önizleme, color picker, theme switch)
- [ ] `tayf config dump` — built-in pattern'leri TOML olarak dök
- [ ] `tayf config status` — watcher / reload debug görüntüsü

### v1.0 — Olgunluk
- [x] Hot reload (master roadmap'te v1.0 idi; v0.2.1'de öne alındı)
- [ ] Comprehensive test suite genişletme (property-based ekle)
- [ ] Belgelenmiş public API stabilizasyonu
- [ ] Web sitesi + kurulum dökümanı
- [ ] Pre-built binary dağıtımı (cargo-dist; Linux/macOS x64+arm64, GitHub Releases, Homebrew tap)
- [ ] crates.io publish (`cargo install tayf`)

### Sonraki
- Windows desteği (ConPTY)
- Plugin sistemi (WASM ile?)
- Statistical/contextual rules (örn. "her benzersiz IP'ye farklı renk")

---

## 12. Açık Sorular

1. **İsim doğrulaması.** Proje adı `tayf` olarak belirlendi. Geriye `cargo search tayf` ile crates.io rezervasyon kontrolü, GitHub'da `tayf-cli` / `tayf-rs` benzeri çakışma araması, ve domain müsaitliği (`tayf.dev`, `tayf.sh`, `tayf.rs`) kaldı.
2. **Default config nereden geliyor?** Binary içine embed mi, ilk çalıştırmada `~/.config`'e mi yazılıyor? Hibrit: binary'de embedded var, kullanıcı override edebilir.
3. **Config validation hatası nasıl iletilir?** Regex compile error'ı kullanıcıya friendly şekilde gösterilmeli (line number, position, "your regex `\b\d+` failed because...").
4. **Login shell mi spawn edelim?** `bash -l` vs `bash` — kullanıcının `.profile`'ının çalışması önemli olabilir.
5. **`SHELL` env var'ını mı kullanmalı, `/etc/passwd` mi?** Genelde `SHELL` ama edge case'ler var.
6. **Hangi default kurallar built-in olmalı?** Az ama keskin: IPv4, IPv6, MAC, ISO 8601 timestamp, HTTP status, log level. Diğerleri opt-in profile.
7. **Performance regresyon eşiği nedir?** Native `cat`'ten %20 fazla yavaşlama kabul edilir mi, yoksa %10 mu hedef?
8. **`exec tayf` yaklaşımı nasıl yumuşatılır?** Kullanıcı tayf'ı devre dışı bırakmak isterse ne yapar? `tayf --bypass` env var ile sessizce passthrough, ya da `.zshrc`'deki satıra `[[ -n "$TAYF_DISABLE" ]] && unset TAYF_DISABLE || exec tayf` gibi escape hatch.
9. **Powerlevel10k instant prompt ile yarış durumu?** P10k çok agresif start-up optimizasyonu yapıyor; tayf'ın başlatma overhead'i bunu hissedilir hale getirebilir. Ölçülmeli.
10. **OSC 133 markers tayf tarafından da yollanmalı mı?** Tayf kendi UI mesajlarını (örn. `tayf: config reloaded`) yazarken bu marker'ları kullanmalı mı, yoksa shell'in işine karışmamalı mı?

---

## 13. Referanslar

### Teknik
- [POSIX termios specification](https://pubs.opengroup.org/onlinepubs/9699919799/basedefs/termios.h.html)
- [XTerm Control Sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html) — ANSI escape sequence bible
- [ANSI escape code (Wikipedia)](https://en.wikipedia.org/wiki/ANSI_escape_code)
- [VT100.net](https://vt100.net/) — eski terminal protokolleri
- [Linux `pty(7)` man page](https://man7.org/linux/man-pages/man7/pty.7.html)
- [OSC 133 / FinalTerm semantic prompt spec](https://gitlab.freedesktop.org/Per_Bothner/specifications/blob/master/proposals/semantic-prompts.md)
- [iTerm2 Shell Integration](https://iterm2.com/documentation-shell-integration.html) — semantic prompt'un en bilinen örneği

### Pazar Araştırması
- [Oh My Zsh Discussion #11687 — "Colorize ZSH Terminal Logs based on custom regex"](https://github.com/ohmyzsh/ohmyzsh/discussions/11687) — bu projenin pazar doğrulaması; 186k yıldızlı topluluk grc'ye yönlendiriliyor
- [ChromaTerm issues / discussions](https://github.com/hSaria/ChromaTerm/discussions) — kullanıcıların gerçek dünya pain point'leri

### Topluluk
- [r/commandline](https://reddit.com/r/commandline)
- [Hacker News terminal etiketi](https://news.ycombinator.com/) — benzer projeler genelde burada yayınlanır

### Rust öğrenme materyali (PTY ve düşük seviye)
- [The Rust nix crate examples](https://github.com/nix-rust/nix/tree/master/test)
- [WezTerm'in pty modülü kaynak kodu](https://github.com/wez/wezterm/tree/main/pty/src)

---

_Bu dokümanın amacı tasarım kararlarını sabitlemek, edge case'leri unutmamak ve benzer projeler üzerinden hızlı öğrenmektir. Implementasyon sırasında güncel tutulmalı._
