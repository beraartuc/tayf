# v0.5.4 spec — UI/UX senior review

**Reviewer:** opus 4.7 senior, TUI specialist (gitui/lazygit/helix/bottom/zellij idiom basis)
**Spec:** `docs/superpowers/specs/2026-05-26-tayf-v0.5.4-config-tui.md`
**Date:** 2026-05-26
**Companion mockups consulted:** `tui-layout.html` (A chosen), `tabs-preview-strategy.html` (A2 chosen), `color-picker.html` (Y chosen), `save-diff.html` (D1 chosen)
**Reference rules:** `CLAUDE.md` §3 (security: terminal restore on any exit path) + v0.5 vision §3.5 (frontend-design loop mandate).

---

## Verdict

**NEEDS_REVISION**

Three findings are UX-critical and will lock in poor experience if shipped as specified: (a) the F-key dual-binding collision under narrow-terminal mode; (b) `Enter`-semantic asymmetry between Patterns and Themes/Profiles tabs; (c) the in-modal `y` quit-confirm default that defaults to data loss. None require architectural rework, but all must be pinned in the spec text — not left to implementer judgement — because they are decisions the spec is purporting to encode.

A further set of 🟡 findings reflect ergonomic gaps the spec does not pin (search scope, `m` double-confirm, edit-then-quit wording, modal stacking, hex go-to-index, terminfo color-depth fallback wording, mini-preview auto-hide visual signal). Each one is small surface area; collectively they will produce a noticeable "raw v0.5.4" feel that v0.6 will spend cycles polishing.

---

## §1. Critical findings (🔴)

### 🔴 #1 — F-key dual-binding collision under narrow-terminal short-tab letters (§7.4 + §12.1)

**Collision.** §7.4 spec'in narrow-terminal table'ı 60-79 column'da tab letter'larını `P/T/F/S` olarak veriyor. §12.1 global key tablosunda **`F` = "Full-preview overlay"**. Aynı zamanda §12.1'de **direct tab key'leri** `1/2/3/4` ile pin'lenmiş — yani `F` letter strip'te yalnızca **görsel** (tab markeri), keyboard binding değil. Ancak:

1. Spec bunu **explicit söylemiyor**. Implementer veya kullanıcı tab letter'ı keyboard shortcut sanıp `F` basacak → full-preview overlay açılacak → "Profiles tab açmaya çalışıyordum" friction.
2. Short-tab letter'ı `F` (Profiles için) zaten **yanlış mnemonic** — Profiles için `P` daha doğal, fakat `P` global'de "preview toggle". Spec bu collision'ı zaten itiraf eden bir baskıyla `F`'i seçmiş ama dokümante etmemiş.
3. Ek olarak `F1` (help) ile `F` (full-preview) birbirine yakın key — F-row genelde terminal emulator'larda **rebind'li** (tmux prefix, terminal app menu), `F1` reliably çalışmıyor (memory: Apple Terminal F1 = Help app, kullanıcının `F1` basışı tayf'a hiç ulaşmıyor). Spec §12.6 `?` / `F1` alternatif veriyor — bu doğru, ancak `F` tek başına global aynı problemden mustarip.

**Failure flow:** Kullanıcı 70-column iPad terminal'inde `tayf config` açıyor → short-tab `P T F S` görüyor → "F = Profiles" sanıyor → `F` basıyor → full-screen preview overlay → kullanıcı şaşırıyor → `Esc` → 5 saniye kayıp + güven kaybı.

**Fix:**
- Short-tab letter'ları tab seçimine bind ETME — pin'le: "tab letter'ları yalnızca visual label; tab dispatch sadece `1`/`2`/`3`/`4` + `Tab`/`Shift+Tab` ile".
- Short-tab letter mapping'ini ya değiştir (örn. **`Pa Th Pr St`** iki-harf veya **`P T R S`** — R for pRofiles), veya §12.6 help overlay'da bu non-binding'i açıkça belirt: "tab strip letters are labels, not shortcuts".
- `F` global'i daha güvenli bir tuşa taşı: `Shift+P` veya `Ctrl+P` (`P` toggle ile çiftlenir, "modifier amplifies" idiom). Alternatif: `F` yerine **`V`** ("View full preview") — Helix idiom paralel.
- §12.1'e açıkça yaz: F-row key'leri (F1-F12) terminal-dependent; **her F-key için non-F fallback** mandatory.

---

### 🔴 #2 — `Enter` semantik asimetrisi: Patterns vs Themes/Profiles (§12.2 + §12.3)

**Collision.** §12.2 Patterns tab'ında `Enter` = "Detail pane'e focus geç". §12.3 Themes / Profiles tab'ında `Enter` = "Active set (general.theme / general.profile yaz)". **Aynı tuş, üç tab arasında farklı action** — ve hiç birinde "default action" idiom'ı yok.

**Failure flow A** (theme yanlışlıkla aktive):
1. Kullanıcı Themes tab'da scroll ediyor (`j/k`) — sadece browse etmek istiyor.
2. `Enter` basıyor — sanki Patterns tab'daki gibi detail pane'e geçeceğini sanıyor (muscle memory cross-tab).
3. Active theme **silently değişiyor** — `[general] theme = X` write edildi (PendingEdits'e), `*` modified marker yandı.
4. Kullanıcı fark ettiğinde, "saved hi yet" → recovery = `r` ya da `Esc` (spec hangisi bind'li net değil; `r` Patterns'da "reset").

**Failure flow B** (Themes/Profiles'da detail'a girilmek istense):
1. Kullanıcı Themes detail görüntülemek istiyor — `Enter` aktive ediyor (istemediği yan-etki).
2. Detail view ekranı **yok** spec'te — Themes/Profiles tab şemasında detail pane yok mu yoksa active set hep aynı pane'de mi? Spec §5.2 "Themes tab — List + active marker + switch" — detail görüntüleme **eksik feature**.

**Fix:**
- `Enter` semantiğini **tüm tab'larda uniform** yap. İki seçenek:
  - **(a) "Enter = detail / preview" uniformly**, active set için ayrı tuş (örn. `a` for "activate" veya `Space`). Helix/lazygit idiom'a yakın.
  - **(b) "Enter = activate (commit current row)" uniformly**, Patterns'da edit'e geçmek için ayrı tuş zaten var (`e`). Spec §12.2'deki "Enter = focus detail" gereksiz, kaldır.
- Karar her ne olursa olsun, spec'e **gerekçe yaz** — gelecek bakan implementer (veya yeni TUI extension) için load-bearing.
- Yan-etkili tuşları **default'tan uzaklaştır** — `Enter` her TUI'da en kolay basılan tuş; active-theme write gibi mutation'ı `Enter`'a bind etmek kullanıcı hatasını dış-stake eder.

---

### 🔴 #3 — Edit-then-quit confirm modal'ı pin'lenmemiş; default risk veri kaybı (§12.1 + §15)

**Risk.** §12.1: "`Ctrl+C` / `q` — Çıkış (pending edits varsa Confirm modal)". Confirm modal'ın **wording'i, key binding'i, default'u** spec'te tanımlı değil. §15 açık-sorular listesinde de yok. Bu spec authority gap — implementer "Discard? [y/n]" yazıp default `y`-on-Enter koysa kullanıcı yanlışlıkla `Enter` basışıyla data lose edebilir.

**Failure flow:**
1. Kullanıcı 30 dakika TUI'da edit yaptı (10 rule override, 3 yeni rule).
2. Bir şey kontrol etmek için terminal'e geçmek istiyor → muscle memory `Ctrl+C`.
3. Confirm modal açıldı: "Discard changes? [y/n]" — kullanıcı önceki habit'le `Enter` basıyor (default `y`).
4. **30 dakika veri çöpe**. Backup file de yok — save edilmemişti.

**Fix — spec'e ekle (§12.1 altına yeni alt-section):**

```markdown
### §12.1.1 — Quit-with-unsaved-edits confirm modal

Trigger: `q` / `Ctrl+C` when `app.edits.is_dirty()`.

Wording (byte-pinned, EN-only per CLAUDE.md §1):
  "You have unsaved changes. Discard and quit?
   [n] Cancel (return to editor)  [s] Save and quit  [d] Discard and quit"

Key bindings:
| Key | Action |
| `n` / `Esc` / `Enter` | Cancel (DEFAULT — safe path) |
| `s` | Save then quit (Ctrl+S flow → SaveDiff modal stays inline; on save success, auto-quit) |
| `d` | Discard and quit (destructive) |

Default = Cancel. Enter selects default. `d` requires a deliberate keystroke; no
`Enter` short-cut to discard. No `y`/`n` binary that biases yes-handedness.
```

**Why three-choice not binary:** "Save and quit" is the **most likely** user intent after 30 minutes of work. Binary "discard? y/n" forces user to either lose work or cancel + then manually `Ctrl+S` + then `q` again. Three-choice modal eliminates the friction and the foot-gun simultaneously. lazygit / gitui both use this pattern.

---

## §2. Important findings (🟡)

### 🟡 #1 — Mini-preview auto-hide visual signal missing (§7.4 + §9.5)

§7.4 narrow-terminal degradation 80×18-23 aralığında mini-preview'ı **sessizce gizler**. §9.5 header `─── live preview ─── [s] sample [P] hide [F] full ───` — preview visible iken `P` hint var. Auto-hide olduğunda:
- Kullanıcı kaybolan preview'ı görmüyor, eski `P` hint kayboldu.
- Edit yaparken canlı feedback yok — pattern compile error olsa bile yansıtılmıyor (sadece compile_error message status bar'da görünüyor mu? Spec belirtmemiş).
- Resize-on yapsa preview geri gelsin — keyfi ergonomik.

**Fix:**
- Auto-hide olduğunda **status bar'a one-char marker** ekle: `[P-hidden: resize ≥24h or press P]` veya kısa ikon. Discoverable + actionable.
- `P` toggle'ı auto-hide'a **override etmeli** (kullanıcı 80×20'de bile `P` basarsa preview göstermek istiyor → 5 satır ödün ver). Spec §7.4 "auto-hide" + §12.1 "P toggle" arasındaki precedence pin değil.
- Pattern compile error mini-preview visible olmadığında bile **status bar'da görünür yapılmalı** — şu an spec §9.1 "preview son-good rules ile devam eder" diyor ama hata göstergesi sadece mini-preview header'da implied. Edit yapıp save'e geçen kullanıcı broken pattern'i fark etmeli.

---

### 🟡 #2 — Search yalnız Patterns tab'ında (§12.1 + §2.2 implied)

`/` global key spec'te "Patterns tab'ında: rule name filter" — yani Themes/Profiles/Status'ta no-op. v0.5.3 ile 5 built-in profile + disk profiller; v0.6+ ecosystem expansion'la 20+ profile pekala olabilir. Disk theme dizini de keyfi büyüyebilir.

**Failure flow:** Kullanıcı 30 disk profile'ı arasında `aws-prod-eu` arıyor → Profiles tab'ında `/` basıyor → no-op → `j/k` ile 30 satır scroll → her seferinde mental cost.

**Fix:**
- **`/` global'i tab-aware yap** — Themes tab'da theme-name filter, Profiles tab'da profile-name filter, Status tab'da log line filter (reload.log içinde search).
- Spec §12.1 satırını şöyle yeniden yaz: "`/` — Search (current tab list filter)".
- Implementation cost minimal — `SearchState` zaten §6.1 Modal enum'da var; tab dispatch fn'i `app.search.filter` peek edip render fn'i filtered list verir. ~30 LOC ek.

Spec scope discipline mandate'ini açıkça ihlal etmiyor — "search" zaten kapsamda, sadece tab routing eksik.

---

### 🟡 #3 — Color picker 256-palette navigation yavaş; go-to-index shortcut yok (§12.4)

§12.4 color picker 256-palette section'ında `←→` value, `↑↓` row jump (grid view varsa). idx 137'ye gitmek için `←→` 137 kere veya row-jump ile birkaç on tıklama. Spec hex input section'da `0-9 a-f` accept ediyor — ama 256 idx için `0-9` accept yok.

**Failure flow:** Kullanıcı `helix` veya bilinen 256-color theme'inden `color 137` öğrenmiş → tayf'a girmek için 100+ keystroke.

**Fix:**
- 256-palette section focuslu iken **digit input = direct idx jump**. Type `137` `Enter` → cursor 137'ye atlar.
- Veya: hex input'a `idx:137` syntax (komut benzeri). Daha karmaşık, gerekmeyebilir.
- Spec §12.4 tablosuna ek satır: "`0-9` (256-palette section) — type idx (3-digit max) + `Enter` accept".

---

### 🟡 #4 — `m` (merge mode discard) destructive ama tek tuş onaylı (§8.1 + §12.5)

§8.1 conflict mode `m` = "discard TUI edits, reload disk". 30 dakika TUI work + manuel disk edit conflict → kullanıcı `m` basışıyla **TUI work atılıyor**. Tek tuş, double-confirm yok. Yan-tuş misskey'i (`n` yakın layout'ta) → veri kaybı.

**Failure flow:** Kullanıcı diff modal okudu → `n` basmak istiyor (cancel) → klavye latency / muscle slip → `m` basılı → TUI state silindi → backup yok (TUI edits diske hiç yazılmamıştı).

**Fix:**
- `m` öncesi second-confirm: `"Discard TUI changes and reload disk? [y/n]"` veya wording-pin. lazygit `gx` pattern paralel (destructive iki tuş).
- Alternatif: TUI edits'i `app.discarded_snapshot` field'inde saklayıp `u` (undo) bind'i ile geri al edilebilir yap (5-dakika TTL). Daha karmaşık, scope dışı olabilir — minimum çözüm ekstra confirm.
- Spec §8.1 modal key dispatch tablosuna "`m` Conflict — opens secondary 'Sure?' modal; require explicit `y`" satırı ekle.

---

### 🟡 #5 — `y` clean-mode silent merge — preview yok (§8.1)

§8.1 clean-mode `y` = `commit_save(...)` direkt. Conflict-mode `y` = "TUI changes wins + preserves manual line" (mockup'tan) — toml_edit reconcile sonucu **diff modal'da gösterilmiyor**. Kullanıcı reconcile output'unu sadece save sonrası disk'ten görüyor.

**Failure flow:** Conflict mode `y` → reconcile fail (toml_edit edge case) → `Modal::Error` (§8.2 mapping). Ama reconcile success'te kullanıcı **gerçekte ne yazılıyor görmüyor** — manuel line korundu mu, ordering OK mı, comment'ler nerede? Belirsiz.

**Fix:**
- Conflict mode `y` öncesi **third diff section** ekle modal'a: "Merged result preview" (~10 line, scroll). Sadece conflict mode'da; clean mode'da gerek yok (snapshot.raw_bytes + edits = output direkt).
- Spec §8.1 modal mode tablosuna kolon ekle veya yeni section: "Conflict mode `y` → diff modal updates to show 3rd panel 'Will write:' → second `y` commits".
- Veya minimum: spec'e açıkça yaz "Conflict mode `y` is best-effort merge; toml_edit failure → Modal::Error (recoverable). User encouraged to review backup file post-save."

---

### 🟡 #6 — Modal stacking pin değil (§7.2)

§7.2 "Modal absorbs" → tab keys görmez. Ama: ColorPicker open iken kullanıcı `Ctrl+S` basıyor → ne olur?
1. **A**: Ctrl+S ignore (modal absorbs all global keys) — kullanıcı save için modal'ı kapatmalı.
2. **B**: Ctrl+S save modal open, color picker üstüne (stacked modals).
3. **C**: Ctrl+S commit current modal (color accept) + sonra save modal aç.

Spec hangisi olduğunu söylemiyor. Implementer tahmin edecek; muhtemelen A. Ama:
- Bazı global key'ler (`Ctrl+C` quit, `Esc` close) modal'da çalışıyor → tutarsızlık.
- A seçilse bile `Ctrl+C` exception olarak hep çalışmalı (terminal-killer safety).

**Fix:**
- §7.2'ye explicit kolon: "Modal absorbs ALL keys except: `Esc` (close modal), `Ctrl+C` (force quit with edits-dirty confirm)". `Ctrl+S` modal içinde ignored.
- Modal stacking explicitly forbidden. Implementer `Option<Modal>` field bir modal olduğu için stacking compile-time imkansız (✓), ama spec dilinde "modals do not stack" cümlesi yararlı.

---

### 🟡 #7 — Terminfo color-depth fallback UX text pin değil (§11.2 + §12.4)

Mockup `color-picker.html` "⚠ terminfo: 8-color" warning gösteriyor. Spec §12.4 color picker section'da bu fallback wording'i pin etmemiş. Kullanıcı `TERM=xterm` (8-color) altında tayf config açtığında:
1. 256-palette section grayed out mu, hidden mu, "unsupported" mu? Belirsiz.
2. Truecolor hex input'u accept etse bile run-time render edemeyecek. Warning render-time mi save-time mi?
3. Kullanıcı `--no-color`-set ortamda TUI hiç açılmamalı mı yoksa fallback mı?

**Fix:**
- §12.4'e color-depth fallback policy alt-section: "Detected color depth N (8/16/256/truecolor). Sections beyond detected depth are visually grayed + show warning line at modal top. User selection in unsupported depth saved verbatim (forward-compatible); render-time fallback handled by existing `style` module."
- §11.2 terminal_caps detection helper specification — `terminal_caps` field §6.1'de var ama nasıl populate edildiği yok. `terminfo` crate? `$COLORTERM`/`$TERM` heuristic? Mevcut tayf bg-detect path ile entegre mi?

---

### 🟡 #8 — Help overlay context-aware değil (§12.6)

§12.6 "4-column grid" tüm binding'leri listele. Top-of-head count: global (10) + Patterns (9) + Themes/Profiles (3) + ColorPicker (8) + SaveDiff (4) ≈ 34 bind. 80×24'te 4-column × ~20 row = 80 cell yer var → fits — ama bunların **çoğu mevcut context'te uygulanabilir değil**.

**Failure flow:** Patterns tab'da ColorPicker modal açık → `?` basıyor → 34 binding görüyor → ColorPicker'ı geri çağıran tuşları (Tab/Shift+Tab section nav vs.) bulması saniyeler sürüyor.

**Fix:**
- Help overlay **context-aware** yap: önce current modal/tab key'leri (örn. "ColorPicker: Tab/Shift+Tab/←→/Enter/Esc/N/0-9a-f"), sonra global key'ler ayrı section.
- 4-column grid sabit kalsın; section header'ları "Active modal", "Current tab", "Global" şeklinde.
- Spec §12.6 satırını "context-aware: prioritize active modal/tab bindings; global keys grouped at bottom" şeklinde genişlet.

---

### 🟡 #9 — First-run UX pin değil (§6.1 + §3 implied)

§6.1 `source_path: None` = "no config file existed at startup" — accepted ama spec **first-run flow'unu hiç anlatmıyor**. Kullanıcı `tayf config` ilk kez açtığında:
1. Boş Patterns list (sadece built-in'ler görünür mü? source_path None ise user-config section "(none yet)" mı?).
2. Bir built-in seçip `o` (override) → user-config'e kopyalanıyor → kaydedince `~/.config/tayf/config.toml` create ediliyor (atomic). OK ama:
3. Kullanıcı 10 built-in override etmek istese 10 ayrı `o` keystroke gerek. `tayf config dump > config.toml` shortcut yok.

**Failure flow:** Yeni kullanıcı TUI'yi açtı, "her şeyi customize edeyim" niyetinde → 10 keystroke + 10 ayrı modal flow → cazibe kaybı, terminal'e geri dönüp `tayf config dump > ~/.config/tayf/config.toml` yapması daha hızlı.

**Fix:**
- First-run banner: ilk kez TUI açıldığında (source_path None) **inline tooltip** veya tek-tuş ipucu: "No user config found. Press `D` to initialize from built-in dump, `o` to override individual rules."
- `D` binding'i ekle (`dump-to-user-config`) — sadece source_path None iken aktif. Confirm modal: "Initialize ~/.config/tayf/config.toml from built-in catalog? [y/n]".
- Veya: `tayf config init` ayrı non-interactive subcommand. Spec scope expansion oluyor — minimum çözüm in-TUI shortcut.

---

### 🟡 #10 — `n` (new pattern) vs search input `n` çakışması belirsiz (§12.1 + §12.2)

§12.1 `/` search (Patterns tab'ında active) + §12.2 `n` "New user pattern modal". Kullanıcı `/foo` ile search filter aktif, sonra search input'tan **çıkmadan** `n` basışı:
- Search filter input içinde `n` karakter mi olur (filter "fon" gibi)?
- Yoksa global `n` binding mi tetiklenir (yeni pattern modal)?

Spec söylemiyor. Genellikle text input bindings'i absorbe eder ama spec §7.2 "modal absorbs" sadece modal'a değiniyor; search şu an Modal enum'unda (`Search(SearchState)`) — yani de facto modal? Ama search filter aktifken list scroll edilebiliyor mu? Bu da unclear.

**Fix:**
- §12.1 `/` semantiği netleştir: "search activates a sticky filter; `Esc` clears, `Enter` confirms and returns to list navigation (filter remains visible in status bar)".
- Filter active iken (post-Enter): `n`/`d`/`e`/global keys çalışır.
- Filter editing iken (within `/foo` input): yalnız text input keys + `Enter`/`Esc`.
- Visual indicator: status bar'da `filter: "foo"` görünür → kullanıcı state'i bilir.

---

## §3. Nits (🔵)

### 🔵 #1 — `Ctrl+S` terminal XON/XOFF risk (§12.1)

Çoğu modern terminal emulator (alacritty, kitty, iTerm2, gnome-terminal, wezterm) flow-control'u disable ediyor default'ta veya crossterm raw mode bunu yutabiliyor. Ama klasik xterm + tmux nesting altında `Ctrl+S` zaman zaman screen freeze yapıyor. **Spec'in problemi değil**, ama:
- Fix: `Ctrl+S` yanında **`Ctrl+W`** veya **`Alt+S`** alt-binding. Vim users `:w` muscle var.
- Spec §12.1'e tek satır: "Save fallback `Ctrl+W` (alt-binding for terminals with XON/XOFF inferno)".

### 🔵 #2 — `j/k` Vim partial — `h/l` yok (§12.2)

Spec `j/k` vertical scroll için Vim-friendly, ama `h/l` horizontal yok. Patterns tab list+detail layout'unda detail pane'e `Enter`'la geçilirken `l` (right) Vim-natural olurdu. Spec eksiklik değil ama beklenti çatışması — Vim user'ı `l` basacak, sessiz no-op alacak.

**Fix:** §12.2'ye one-line: "`h` / `l` — focus jump left/right (list ↔ detail pane in Patterns tab; section ↔ section in ColorPicker)".

### 🔵 #3 — Toast vs Modal::Error consistency (§6 + §8.2)

§6 enum Toast + Modal::Error iki ayrı tip. §8.2 error path tablosu bazen Toast bazen Modal::Error veriyor. Distinction implicit: blocking vs non-blocking. Spec'e bir satır guidance:
- "Toast: non-blocking, auto-dismiss 3s, used for save-success + non-fatal save-warning (e.g. fsync_dir failure)."
- "Modal::Error: blocking, requires Esc, used for save-failure that requires user attention (perm denied, conflict reconcile failure)."

Pin'leyince implementer her hata için tutarlı seçer.

### 🔵 #4 — `?` `F1` help — `h` da idiomatic (§12.1)

Çoğu TUI'da (helix, lazygit) `?` + `F1` + **`h`** üçü de help. Spec `h` reserve etmiyor — `j/k` zaten Vim, `h/l` partial. `h` Vim-mode'da left-arrow olduğundan kullanmak overload — ama help için `h` Emacs-idiom değil. **Net çıkar yok**, deðerlendirme: `h` reserve etmeden bırak (left-jump için kullanırsan açık kalır), `F1` + `?` zaten yeterli.

### 🔵 #5 — `Esc` overload (§12.1)

§12.1 `Esc` = "Modal close / edit field cancel / search clear". Üç anlam. Stacking durumlarda (edit field within modal) hangi action öncelik? Spec söylemiyor; doğru sıra "deepest context first" (edit field cancel > modal close > search clear). Bu standard idiom; pin'le bir cümle ile yeter.

### 🔵 #6 — Backup naming long + ugly (§8.3)

`config.toml.tayf-backup-2026-05-26T17-18-42-123Z` — 47 karakter ek. Aynı dizinde 5 backup → ls output unreadable. Save-diff modal'da backup path gösteriliyor (mockup'tan) — modal width'ini zorluyor. Kısaltma önerisi: `.toml.tayf-bak.<timestamp>` → 36 char. Veya hidden directory: `~/.config/tayf/.backups/<timestamp>.toml` (kullanıcının config.toml dizini temiz kalır). v0.6 nit; spec değişikliği değil.

### 🔵 #7 — Confirm modal default key idiom (§12.1 + 🔴#3 ile bağlantılı)

🔴#3 fix'i quit confirm için three-choice öneriyor. Aynı disiplin **delete confirm** (`d` Patterns tab'da) için de geçerli olmalı. Spec §12.2 `d` "Delete (user-config rules only)" — confirm yok mu? Pin'le: silent destructive action olmasın.

### 🔵 #8 — `g`/`G` go-to-top/bottom — search/filter ile etkileşim (§12.2)

`g G` Vim idiom — filtered list aktif iken filtered top/bottom mı yoksa absolute top/bottom mı? Filtered olmalı (user's mental model). Spec pin'lemiyor; one-line.

### 🔵 #9 — Sample input default Unicode-rich değil (§9.3)

`DEFAULT_PREVIEW_SAMPLE` regex-rich (IP, timestamp, log_level, UUID, container_name) ama **Unicode test edemiyor**. tayf v0.3+ Unicode-aware (VTE state machine + Unicode-width). Sample'a bir satır Unicode: `[2026-05-26T17:18:45Z] INFO ñame=façade pid=4096 status=完了`. Color picker test + Unicode width edge case'i kullanıcıya görünür.

### 🔵 #10 — Status bar `theme:` `profile:` truncation 60-col'da (§7.4 implied)

§7.4 60-79 column'da "Main pane daralır" — ama status bar'ın `hot-reload  profile: none  theme: dark  [?] [q]` ~55 chars yer kapsar. 60-col'da fitting borderline; 50-col altı (Spec block render < 60) için zaten kapanıyor. 60-65 col'da text overflow olmamalı; status bar layout (Constraint::Length) Min(...) ile shrinkable text widget kullanılmalı. Pin değil ama implementer'a one-line.

---

## §4. Strengths

1. **A2 layout choice (persistent mini-preview)** doğru karar — pattern editing'in temel UX'i live feedback; alt-tab cycle Friction'ı (A1) yıkıcı olurdu. Mockup gerekçesi sağlam.
2. **D1 conflict-aware save** kullanıcı muteber. Manuel edit'i ezmeyen bir TUI editor TUI editor'lar arasında nadir (zellij config edit'i overwrites!). v0.5.4 burada gitui/lazygit kalibresinde.
3. **CLAUDE.md §3 terminal restore RAII + panic hook** §7.5'te explicit pin'li — terminal corruption riskini bilen senior tasarım. v0.5'in en güvenli sub-version'ı bu açıdan.
4. **Hot path immutability mandate** (§2.1 "DOKUNULMAZ", §5.4 "byte-identical") — TUI feature'ı PTY wrapper'a sızdırmama disiplini exemplary. Bench-CI yeniden çalıştırılma yok.
5. **Render-snapshot test'leri açıkça reject** (§10.4) — ratatui ekosisteminden bilinen acı (her UI tweak snapshot regen). State-machine + non-interactive integration coverage doğru tradeoff.
6. **Frontend-design loop mandate consumed** (§13.8) — kullanıcı 2026-05-25 instruction'ı dispatch'lerde önceden uygulanmış; mockup'lar brainstorm phase'inde üretilmiş, spec sadece encode ediyor.
7. **toml_edit choice over plain toml** (§8.1, §11.2) — round-trip preserve = user manual edit'leri korur, hot-reload coexistence'ın yarısı bu karara dayanıyor. Doğru tradeoff (+1 dep, kullanıcı güveni).

---

## §5. Recommendation

**NEEDS_REVISION.** Spec architectural olarak güçlü ve frontend-design loop disiplinini gerçekten kullanmış. Critical findings'in üçü de spec text gaps — implementation'a yapılan karar değil, **spec'in karar vermediği yerler**. Düzeltmesi 1-2 saatlik spec text iterasyonu:

1. **🔴 #1** — F-key collision + short-tab letter binding pin (§7.4 + §12.1, 2 satır eklenti + binding mapping revize).
2. **🔴 #2** — Enter semantik uniformity (§12.2 + §12.3, semantik rationalization + 1 sentence rationale).
3. **🔴 #3** — Quit confirm modal three-choice spec (§12.1.1 yeni alt-section ~10 satır).

🟡 findings'ten **en az 4 tanesi pin edilmeli** (özellikle #1 mini-preview signal, #4 `m` double-confirm, #6 modal stacking explicit, #9 first-run UX) — diğerleri implementer'a not olarak bırakılabilir ama v0.6 polish iş listesine girer.

Bu revize sonrası CLEAN_SHIP. Spec'in iskeleti sağlam — sadece "TUI editing decisions burada kararlaşır" mandate'ini kendi içinde 100% uygulamamış. Critical 3'ünü pin'leyip sub-version dispatch'i gönderebilirsin; implementer karşılaştığı her ambiguity için spec'e geri dönmek zorunda kalmaz.

**Cross-cutting note (memory `feedback_consume_prior_review` paralel):** Bu review'ın critical 3'ü v0.5.4 spec phase'inde fold edilirse, v0.5.4 final cross-cutting review'da implementation TUI key behavior'ları için review surface dramatically küçülür — yani şimdi pin'lemenin compound faydası var.
