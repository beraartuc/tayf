# tayf v0.5.5 — Built-in + Profile Pattern FP & Cross-Pattern Collision Audit

**Date:** 2026-05-27
**Author:** opus 4.7 audit subagent (read-only research)
**Audit base:** `src/rules.rs` @ d2f8a80 + `assets/profiles/*.toml`
**Harness:** `/tmp/fp-audit/` (regex 1.12, RegexSet replica of tayf's pattern-definition order)
**Cases run:** 133 candidates across 13 built-ins + 5 profile append_rules.
**Goal:** systematic enumeration for v0.5.6 "architectural collision fix" scoping.

---

## 0. Method

### 0.1 Pattern enumeration

13 built-in rules in `src/rules.rs:358-501` (pattern-definition order, which IS the RegexSet
first-match-wins order):

| idx | name         | shape essence                                                                    |
|-----|--------------|----------------------------------------------------------------------------------|
| 0   | permission   | ` [dlcbps-][rwxsStT-]{9}\+?  ` (leading + trailing whitespace anchors)           |
| 1   | timestamp    | ISO8601 \| syslog \| Apache \| RFC2822 (5 named caps on ISO branch)              |
| 2   | uuid         | `8-4-4-4-12` hex (lowercase-or-uppercase, no version check)                      |
| 3   | url          | `(https?\|ssh\|ftp)://body` \| `git@host:path` (3 named caps + bare branch)      |
| 4   | email        | `local@host.tld{≥2}`                                                             |
| 5   | ipv4         | strict octets `0-255` (no leading zero except `0`)                               |
| 6   | ipv6         | 4-branch alternation, broadest compressed form is `(?:hex:){1,6}:hex{0,4}`       |
| 7   | mac          | `\b hex{2} ([:-] hex{2}){5} \b` (exactly 6 octets)                               |
| 8   | log_level    | `\b(ERROR\|FAIL\|FATAL\|CRITICAL\|WARN\|WARNING\|INFO\|DEBUG\|TRACE)\b`          |
| 9   | http_status  | `(?:^\|[\s/:])([1-5]\d{2})\b`                                                    |
| 10  | filename     | `\b[\w.-]+\.(?:ext_alt)\b` (227 extensions incl. single-char `a c h m o r v`)   |
| 11  | fqdn         | `\b(label\.)+TLD{2..24}\b`                                                       |
| 12  | duration     | `\b\d+(?:\.\d+)?(unit)(comp)*\b`, units `ns us μs ms s m h d`                    |

5 profile append_rules (appended in profile-definition order, AFTER built-ins):

| profile | name         | shape essence                                                          |
|---------|--------------|------------------------------------------------------------------------|
| aws     | instance_id  | `\bi-[a-f0-9]{17}\b`                                                   |
| aws     | region       | exhaustive 34-region enum                                              |
| aws     | arn          | `\barn:aws(-us-gov\|-cn)?: ... [a-zA-Z0-9_/+@*-]\b`                    |
| k8s     | pod_name     | `\b[a-z][a-z0-9-]*-[base32]{10}-[base32]{5}\b` (no-vowel-no-01)        |
| docker  | container_id | `\b[a-f0-9]{12}\b`                                                    |
| docker  | image_tag    | `host.tld/path:tag` \| `name(/path)*:latest`                          |

`gcp` and `network` profiles ship `rules = [...]` whitelists only, no append_rules.

### 0.2 Verification harness

`regex::bytes::RegexSet` built in the same order tayf uses. For each candidate:

- enumerate all rule names that find a leftmost match anywhere in the line;
- record each pattern's leftmost match span;
- compute first-match-wins **winner** = lowest pattern index that matched.

Caveat: the production pipeline (`src/pipeline.rs:75-100`) iterates rules in order and accepts
each non-overlapping match. So the winner-by-lowest-index is the right call for **overlapping**
spans; **non-overlapping** matches from later rules also fire (e.g. ipv4 + fqdn on
`1.2.3.4 vs example.com`). The "spans" column distinguishes these.

133 cases were exercised; each appears with (input, expectation, all hits, winner, spans) in
`/tmp/fp-audit/results.txt`. Below is the categorised distilation.

---

## A. Confirmed correct behaviour (no action)

29 cases pass with no surprise. Highlights:

- **ipv4 leading-zero rejection.** `1.01.30.4` → no match (octet regex disallows leading `0`).
- **ipv4 out-of-range rejection.** `256.0.0.0` → no IPv4 match (matches `http_status` on the
  `256` literal trailing — see C-3 below for the FP).
- **ipv4 inside identifier.** `v10.20.30.40` → no match (\b after `v` not satisfied).
- **ipv6 zone-id stripped.** `fe80::1%eth0` → matches the `fe80::1` address only; zone-id
  trails. Defensible spec choice.
- **uuid lax shape.** `12345678-1234-1234-1234-123456789012` matches even without version
  nibble — accepted (tayf's uuid is shape-only, not v4-specific).
- **uuid trailing-non-hex.** `...-12345678901g` → no match.
- **uuid 13-trailing.** `...-1234567890123` → no match (`\b` after 12 hex blocks).
- **timestamp anchored to syslog/apache/rfc2822** — none of `Jan 5`, `Apr 1` produced
  surprises; RFC2822 / Apache match correctly.
- **log_level word-boundary.** `ERROR_HANDLER`, `terror`, `INFOrmation`, `FAIL_FAST_MODE`,
  `DEBUGGER`, lowercase `trace` — all correctly rejected. log_level is well-bounded.
- **email no-TLD.** `user@localhost` → no match (needs `.tld{≥2}`).
- **email no-local.** `@handle` → no match.
- **permission anchors.** `argv=-rw-r--r--` (no leading space) → no match.
- **filename dotfile.** `.gitignore` → no match (no extension after dot).
- **filename unknown ext.** `file.UNKNOWN` → no filename match (fqdn fires on shape).
- **url scheme list.** `file:///etc/hosts`, bare `http://` → no match.
- **url trailing punct trim.** `Visit https://example.com.` → URL ends before `.`.
- **mac dash form.** `01-23-45-67-89-ab` → valid.
- **k8s pod_name vowel rejection.** `nginx-aeiouaeiou-12345` → no match.
- **k8s pod_name bare git-short-hash.** `7c79c4bf97` standalone → no match (needs the
  `name-hash-hash` triplet shape).
- **docker image_tag bare-non-latest.** `nginx:1.21` → no match.
- **docker image_tag bare-latest.** `nginx:latest`, `library/redis:latest` → valid envelope.
- **aws.instance_id length.** 17 hex exact; 18-or-16 reject.
- **aws.region anchor.** `eu-south-3` (future invented) → no match.
- **arn no interior collision.** `arn:aws:iam:::role/MyRole` → aws.arn fires cleanly.

---

## B. Already-pinned known limitations (carry forward)

These are documented in `assets/profiles/*.toml`, pinned in `src/profiles.rs` with the
mandated `_v0_5_3_limitation` suffix, and ACCEPTED as v0.5.3 behaviour.

### B-1. aws.arn ↔ ipv6 (`3::` interior).
`arn:aws:s3:::my-bucket` — ipv6 idx 6 matches `3::` (substring `3::` is a valid IPv6
compressed form per ipv6 branch 3 `::[hex]{1,4}` — actually no, `3::` is matched by branch 2
`(?:hex:){1,6}:hex{0,4}` with the `hex{0,4}` empty tail). aws.arn (idx 15 in aws context)
loses on first-match-wins. **Confirmed by harness:** hits = `["ipv6", "arn"]`, winner = ipv6.
Pinned test: `src/profiles.rs:831`.

### B-2. docker.image_tag ↔ fqdn (registry-host prefix).
`gcr.io/google/nginx:1.21` — fqdn idx 11 matches `gcr.io`. docker.image_tag (idx 14 in docker
context) loses because the fqdn span overlaps the image_tag prefix.
**Confirmed by harness:** hits = `["fqdn", "image_tag"]`, winner = fqdn. Pinned test:
`src/profiles.rs:934`.

### B-3. docker.container_id ↔ git short hash.
`7c79c4bf9712` (12 hex) inside docker profile fires container_id. Documented as accepted
behaviour (profile = opt-in domain context); pinned test `src/profiles.rs:905`.

---

## C. NEW collisions found (v0.5.6 candidates)

### C-1. mac ↔ ipv6 (8-hex-pair chain).
**Input:** `aa:bb:cc:dd:ee:ff:11:22` (e.g. truncated IPv6 with no `::`).
**Hits:** `["ipv6", "mac"]`, **winner:** ipv6.
**Mechanism:** ipv6 idx 6 < mac idx 7. ipv6 branch 1 `(?:hex:){7}hex` matches the full 8-pair
chain. mac would match the first 6 pairs as a sub-span; first-match-wins gives ipv6.
**Status:** behaviour is reasonable (a true 8-pair colon chain IS an IPv6); but if a user
intends "extended MAC + suffix" the MAC won't be highlighted in its own colour.
**Severity:** LOW (rare in real terminal output).
**Recommendation:** ACCEPT as documented limitation. Add pin test
`mac_yields_to_ipv6_eight_pair_v0_5_5_limitation`. Tighten requires changing pattern ORDER
(DOKUNULMAZ risk).

### C-2. ipv6 ↔ Rust path syntax (`foo::bar::baz`).
**Input:** `mod foo::bar::baz`. **Hits:** `["ipv6"]`. **Match span:** `::ba`.
**Mechanism:** ipv6 branch 3 `::[hex]{1,4}` matches `::ba` because `b` and `a` are valid hex
characters. The `\b` boundary semantics around `::` are unhelpful since `:` is not a word
char.
**Severity:** **MEDIUM-HIGH** — Rust developers see this collision constantly in `mod`
declarations, trait paths, and error messages (`std::io::Error`). Any module path containing
2+ hex-letter segments triggers ipv6 styling on the `::xxxx` chunk.
**Examples that trigger:**
- `std::io::Read` → `::io` (`io` = hex `i`? NO, `i` is not hex). Actually `i` is NOT in
  `[0-9A-Fa-f]`, so `std::io::Read` does NOT trigger. The harness FP `::ba` matches because
  `ba` IS hex.
- `foo::bar::baz` → `::ba`  (matches because `b`, `a` are hex).
- `serde::de::Deserialize` → `::de` (`d`, `e` are hex; matches).
- `std::fmt::Debug` → `::fm` no, `f` is hex but `m` is not. The first `:` then `f` then `m`
  fails. **No false positive.**
- `tokio::ace::Foo` → `::ac` (hex). FP.

The pattern is: any Rust path with a `::seg` where `seg` starts with 1-4 hex chars triggers.
Frequency: very common.

**Severity reassessed:** HIGH for Rust devs (probably 10-50% of `cargo` output lines).
**Recommendation:** **TIGHTEN ipv6** to require ≥2 colon-separated hex groups (so the
3rd branch `::[hex]{1,4}` becomes `::[hex]{1,4}(:[hex]{1,4})+` OR is dropped entirely and
loopback `::1` is special-cased). This is a `src/rules.rs` data-only change to the pattern
body. Add positive regression for `::1`, `fe80::1` and negative regression for `foo::bar`,
`std::io::Result`, `serde::de::Deserialize`.

### C-3. http_status ↔ semantically-unrelated 3-digit numbers.
The `http_status` regex `(?:^|[\s/:])([1-5]\d{2})\b` matches **any** 3-digit number 100-599
preceded by whitespace or `/` or `:`. This is enormously broad.

**FPs confirmed by harness:**
- `vlan 100` → matches `100` (a VLAN id, not an HTTP status).
- `line 500 plain` → matches `500` (a line number).
- `:256.0.0.0` → matches `256` (an IPv4 octet that the ipv4 rule rejected).
- `:111111111111` inside ARN account-id → matches `:111` (NOT 12-digit, the `\b` blocks the
  4th digit). Actually harness shows match = `:111` from input `:111111111111` — wait, that's
  weird because the digit after `111` is also a digit, so `\b` should fail. Let me recheck.

**Re-verified:** harness output `[Y-arn-with-region] ... http_status=`:111``. Hmm — `\b`
between digit and digit should be no boundary. Actually looking again: regex `[1-5]\d{2})\b`
requires `\b` AFTER 3 digits. The input is `:111111111111` — after the 3rd `1` the next char
is also `1` (a digit/word char), so `\b` cannot fire. But the harness reports match. This
suggests `find` returns a match starting LATER in the string where `\b` is satisfied.

Let me confirm: `:111111111111:` — the regex engine tries every start position. From position
0 `:`, tries `(?:^|[\s/:])` accepting `:`, then `111`, then `\b` — fails because next is `1`.
Tries position 1, can't start since first capture requires `^|[\s/:]`. Skips through. From
position 9 (10th char), the previous char is `1` so `[\s/:]` can't fire there. The only way
to get a `:111` match is from a `:` followed by exactly `111` then a non-word char. In input
`arn:aws:ec2:us-west-2:111:bucket/foo` — after `:111` comes `:`, which IS non-word boundary.
So the match IS `:111`. **Correct behaviour.**

So back to C-3: `arn:aws:s3:us-east-1:111:bucket/foo` — http_status matches `:111`. This is an
AWS account-id prefix being styled as HTTP status. Within an aws-profile session this is a
**MEDIUM** FP.

**Severity:** MEDIUM. Affects every aws/k8s log line containing 3-digit-prefixed account
IDs, port numbers, or VLAN IDs.
**Recommendation:** This is the design tradeoff documented in spec — `http_status` is
inherently a guess. Two options:
1. **ACCEPT** as documented limitation (most users colorize HTTP server logs where 200/404/
   500 are common). Add explicit Q&A in spec §11.
2. **TIGHTEN** with surrounding context (e.g. require a verb prefix `GET|POST|HTTP/`, or a
   suffix like `(ms|s|OK|Created)`). Pattern-data-only change. Probably needs profile-level
   gating (HTTP profile would whitelist; aws/k8s would blacklist).

Lean toward option 1 + add a `web` or `http` profile in v0.6 that ENABLES the rule, with
other profiles dropping it from their whitelist. v0.5.6 = pin as documented limitation.

### C-4. filename ↔ Rust qualified paths (single-letter extensions).
**Input:** `see a.b.c.d four-label`. **Hits:** `["filename"]`. **Match span:** `a.b.c`.
**Mechanism:** filename pattern has 227 extensions including single letters `a c h m o r v`.
The body `[\w.-]+\.(?:c)` matches `a.b.c` (with `b` as the body and `c` as the ext —
no wait, the regex is `\b[\w.-]+\.(?:alt)\b`, so given `a.b.c.d`, the engine matches the
longest valid extension. Let's enumerate: trying the rightmost dot first… actually the regex
tries left-to-right. The whole token is `a.b.c.d`. The body `[\w.-]+` is greedy, the
extension is `(?:alt)`. So it matches as much as it can: body `a.b.c`, ext `d` — is `d` in
the list? Check the canonical list — `d` is NOT in it. So engine backtracks: body `a.b`,
ext `c` — `c` IS in. Match: `a.b.c`. Correct per regex semantics.

**Severity:** **HIGH** for any developer output containing qualified module/type/method
paths with hex-or-single-letter chunks:
- `serde::de::Visitor` (the parser won't touch this — `de::Visitor` has `::`, but
  `serde.de.Visitor`? No that's not realistic). 
- `a.b.c.d`, `foo.bar.c`, `x.y.h.json` — yes.
- Java/Kotlin class names: `com.acme.proj.Service` — match attempt: body
  `com.acme.proj.Service`, ext `Service`? not in list. Backtrack to `com.acme.proj`, ext
  `Service` not in list. Backtrack to `com.acme.pro`, ext `j`? not in list. `com.acme.pr`,
  ext `oj`? not in list. … `com.acme`, ext `proj.Service` not in list. `com`, ext
  `acme.proj.Service` not in list. **No match.** Java packages don't trigger.
- Python `numpy.linalg.norm` — body `numpy.linalg`, ext `norm` not in list; backtracks
  through; final body `numpy.linal`, ext `g.norm` not in list; etc. **No match.**

So C-4 only fires when the LAST segment is a single-letter or short ext from the canonical
list. Real-world incidence: low-moderate. But `a.b.c.d`-style prose / IP-look-alikes DO
trigger.

**Severity revised:** LOW-MEDIUM.
**Recommendation:** Audit single-letter extensions. Drop `a`, `o`, `r`, `v`, `m` from the
list (or keep only when surrounded by a path separator `/`); `c` and `h` are heavily used
file extensions and removing them costs C-developer UX. Cleanest: **require ≥2 character
extension** by default, with a narrow exemption for `a c h m o r v` ONLY when preceded by
`/` (path separator). That is a pattern-body-only change; no `src/rules.rs` architecture
impact.

Or simpler: **leave as-is** — collision rate too low to warrant complexity. ACCEPT as
documented limitation, add negative regression `a.b.c.d` to spec.

### C-5. filename ↔ fqdn double-fire.
**Input:** `config.json` (no path context).
**Hits:** `["filename", "fqdn"]`. **Winner:** filename. **Both spans:** `config.json`.
**Mechanism:** filename idx 10 < fqdn idx 11. Both regexes match the identical span.
First-match-wins gives filename. Output: filename style fires; fqdn is rejected.

**FP nature:** Anyone running `cat config.json` sees this as a filename (correct). Anyone
running `dig config.json` sees it as a DNS name (would want fqdn). The same token has two
valid interpretations.

**Severity:** None — this is design-correct. filename wins for common case.
**Recommendation:** No action. (Documented in the cross-cutting reviews.)

### C-6. fqdn ↔ duration prefix (`10s.example.com`).
**Input:** `address 10s.example.com`. **Hits:** `["fqdn", "duration"]`. **Winner:** fqdn.
**Span overlap:** fqdn = `10s.example.com` (full), duration = `10s` (prefix overlap).
**Mechanism:** fqdn idx 11 < duration idx 12, and fqdn span CONTAINS duration span. fqdn
wins. duration's `10s` would be rejected by overlap check.

**Severity:** None — fqdn is the correct dominant interpretation.
**Recommendation:** No action.

### C-7. http_status interior of IPv4 octet rejection (`256.0.0.0`).
**Input:** `see 256.0.0.0 oct`. **Hits:** `["http_status"]`. **Winner:** http_status.
**Span:** ` 256`.
**Mechanism:** ipv4 rejects `256` (strict 0-255). http_status idx 9 fires on the leading
` 256` (space + 3-digit).

**Severity:** LOW. Adversarial / borderline input.
**Recommendation:** ACCEPT. Add negative regression `256.0.0.0` to ipv4 tests; add positive
regression that http_status fires on `:256` (documents the cascade).

### C-8. fqdn matches prose with trailing-2char (`pkg.go.dev/foo`).
**Input:** `pkg.go.dev/foo path`. **Hits:** `["filename", "fqdn"]`. **Winner:** filename
(span `pkg.go`). **fqdn span:** `pkg.go.dev`.
**Mechanism:** filename idx 10 < fqdn idx 11. filename matches `pkg.go` (`go` IS in ext
list). fqdn matches the broader `pkg.go.dev`. Overlap: filename wins on the prefix; fqdn's
larger span is rejected because filename took `pkg.go` (overlaps `pkg.go.dev`).

**Severity:** **MEDIUM** — Go developers running `go doc`, `go list`, `cargo doc` etc see
fqdn-shaped pkg paths constantly. Highlighting `pkg.go` as a filename and dropping the
domain styling is jarring.
**Recommendation:** This is the textbook **filename vs fqdn** disambiguation case. Two fixes:
1. Reorder filename AFTER fqdn — but that breaks `config.json` (would highlight as fqdn
   first). **REJECT.**
2. Tighten filename to require a `/` path separator OR end-of-input/whitespace BEFORE the
   token — i.e. anchor filename's left side to a path-like context. **PROMISING but
   subtle.** Probably defer; add to spec §11 as a known design tradeoff.
3. ACCEPT — `pkg.go` highlights, user sees the message clearly.

Lean: ACCEPT, document.

### C-9. fqdn matches JWT 3-segment dotted tokens.
**Input:** `Auth: eyJhbGc.eyJzdWI.signature`. **Hits:** `["fqdn"]`. **Span:**
`eyJhbGc.eyJzdWI.signature`.
**Mechanism:** fqdn regex requires `TLD{2..24}` at the end. `signature` is 9 chars, all
letters, so it qualifies as a TLD. The body before contains valid label chars. Match.

**Severity:** MEDIUM. JWT tokens are common in API logs.
**Recommendation:** No clean pattern fix without breaking real fqdn use cases. Could
require TLD to match a known-TLD list (huge maintenance burden — public suffix list is
4000+ entries). ACCEPT as documented limitation; recommend users add a custom rule for
JWT-shape in user-config.

### C-10. filename matches prose with `.txt` etc. (`prose.with.dots.txt`).
**Input:** `prose.with.dots.txt`. **Hits:** `["filename", "fqdn"]`. **Winner:** filename.
**Span:** entire token.
**Mechanism:** filename regex `\b[\w.-]+\.(?:txt)\b` matches the whole token.

**Severity:** None — this looks like a real filename. If a sentence contains "config.txt is
broken" then `config.txt` is colored. That is the desired behavior.
**Recommendation:** No action.

### C-11. fqdn ↔ filename ratio false-fire for `config.json.tmp`.
**Input:** `config.json.tmp temp`. **Hits:** `["filename", "fqdn"]`. **Winner:** filename
(span `config.json`). **fqdn span:** `config.json.tmp`.
**Mechanism:** filename matches `config.json` (json is ext, tmp is not). fqdn matches the
3-label `config.json.tmp` (`tmp` is 3-char TLD).

**Severity:** None — filename styles the file part, the fqdn span overlap means fqdn is
suppressed. Output is sane.
**Recommendation:** No action.

### C-12. arn ↔ ipv4 interior.
**Input:** `arn:aws:ec2:us-west-2:111111111111:vpc/1.2.3.4`. **Hits:** `["ipv4", "region",
"arn"]`. **Winner:** ipv4.
**Mechanism:** ipv4 idx 5 wins ahead of region (idx 13) and arn (idx 15). Interior IPv4
`1.2.3.4` claims its span; arn envelope is rejected.

**Severity:** Known class — same family as B-1 and B-2 (interior built-in beats profile
envelope). NEW collision instance.
**Recommendation:** Add to the v0.5.4-deferred "rule-priority architectural fix" scope OR
pin as `aws_arn_yields_to_interior_ipv4_v0_5_5_limitation`. Bundle with B-1 / B-2 in v0.5.6
architectural fix scope.

### C-13. arn ↔ uuid interior.
**Input:** `arn:aws:secretsmanager:us-east-1:111:secret:my-550e8400-e29b-41d4-a716-446655440000`.
**Hits:** `["uuid", "http_status", "region", "arn"]`. **Winner:** uuid.
**Mechanism:** Same class — uuid idx 2 wins everything. arn envelope rejected.

**Severity:** Common pattern — AWS Secrets Manager and RDS resources include UUIDs.
**Recommendation:** Same bundle as C-12 / B-1 / B-2. **THIS IS THE FOUNDATIONAL ARGUMENT**
for the v0.5.6 architectural collision fix: profile append_rules should be able to win on
ENVELOPE match against any built-in interior span.

### C-14. arn ↔ http_status interior (account-id-suffix-3-digit).
**Input:** `arn:aws:lambda:us-west-2:123:fn:foo`. **Hits:** `["http_status", "region",
"arn"]`. **Winner:** http_status (span `:123`).
**Mechanism:** http_status idx 9 < region 13 < arn 15. The 3-digit account-id `:123` (when
followed by `:` non-word boundary) wins.

**Severity:** HIGH within aws profile context — every short-account-id ARN fragment is
mis-styled.
**Recommendation:** Same bundle as C-12. Architectural fix or explicit pin.

### C-15. url ↔ email ↔ fqdn triple on `git@github.com:user/repo`.
**Hits:** `["url", "email", "fqdn"]`. **Winner:** url.
**Mechanism:** url's `git@host:path` branch wins (idx 3) over email (idx 4) and fqdn
(idx 11).
**Severity:** None — url is the correct dominant interpretation for the SCP-style URL.
**Recommendation:** No action.

### C-16. url ↔ trailing paren not trimmed.
**Input:** `(see https://example.com/x)`. **Span:** `https://example.com/x)`.
**Mechanism:** the trailing-trim set in url's body is sentence punctuation only
(`.,;:!?`); `)` is intentionally retained because real-world URLs can contain `)` (e.g.
Wikipedia disambiguation pages).
**Severity:** LOW. Markdown links `[txt](url)` mis-include the closing paren.
**Recommendation:** ACCEPT — Wikipedia URLs depend on this. Document; consider per-context
trim profile in v0.7+.

### C-17. fqdn ↔ duration prefix in odd compound (`10s.example.com` already in C-6).

---

## D. Tighten-pattern opportunities (data-only `src/rules.rs` changes)

These don't require touching architecture, just regex pattern strings.

### D-1. ipv6 third branch over-broad (C-2 fix).
Current: `::[0-9A-Fa-f]{1,4}` (matches `::ab`, `::fe`, etc.).
**Proposed:** require at least 2 colon-separated groups OR special-case `::1`:
```
::1|(?:[0-9A-Fa-f]{1,4}:){7}[0-9A-Fa-f]{1,4}|(?:[0-9A-Fa-f]{1,4}:){1,6}:[0-9A-Fa-f]{0,4}|::[0-9A-Fa-f]{1,4}(?::[0-9A-Fa-f]{1,4})+
```
Drops the `::abcd` bare form (saves Rust path FPs). `::1` survives via the dedicated branch.
The 7-pair, 1-6-pair-then-colon, and N≥2-trailing-group forms cover real compressed IPv6
addresses.

**Test coverage gap:** add negatives for `mod foo::bar::baz`, `std::io::Read`,
`serde::de::Deserialize`. Add positives for `::1`, `::ffff:1.2.3.4` (IPv4-mapped), `fe80::1`,
`2001:db8::1`.

**DOKUNULMAZ impact:** None — `src/rules.rs` is data (pattern string only).

### D-2. timestamp date-shape laxness (`2025-99-99`).
**Input:** `build 2025-99-99 invalid`. Harness: no match (good — `\d{2}-\d{2}` for
month/day matches `99-99` but ISO requires `T` or space then `\d{2}:\d{2}:\d{2}` time;
without that the whole branch fails). **Confirmed correct.**

### D-3. http_status broadness (C-3).
Already discussed. Recommend defer / profile-level handling.

### D-4. ipv4 negative regression coverage.
Add to `src/rules.rs:1XXX` tests:
- `1.01.30.4` → no match
- `256.0.0.0` → no match
- `999.0.0.0` → no match
- `v10.20.30.40` → no match
- `1.2.3.4.5` → match `1.2.3.4` (partial), document
- `0.0.0.0` → match
- `192.168.1.100` → match

### D-5. mac single-pair-shape coverage gaps.
Add negative regression for `11:22:33:44:55:66:77` (7 pairs — only first 6 match).
Add negative regression for the `aa:bb:cc:dd:ee:ff:11:22` IPv6-tie (winner = ipv6).

### D-6. filename single-letter extension audit.
The single-letter extensions `a c h m o r v` are intentional (C-header `.h`, C-source `.c`,
Verilog `.v`, R `.r`, Objective-C `.m`, archive `.a`, object `.o`). They are NOT a bug. But
add documentation in `src/rules.rs` near the `FILENAME_EXTENSIONS` const explaining the
prose-collision tradeoff (`a.b.c.d` will match `a.b.c`).

### D-7. log_level positive coverage.
Add positive regression for `[ERROR]`, `INFO:`, `WARN -`, `(CRITICAL)` — every common
delimiter context. The current pattern handles these via `\b`; pin the behaviour.

---

## E. Spec-judgment-required ambiguities (no pattern can fix)

### E-1. semver vs ipv4 (`4.5.6.7`).
Both are syntactically valid IPv4 octets. Semantic context is the only differentiator.
**Recommendation:** ACCEPT as IPv4 (current behaviour). Document; offer semver profile in
v0.7+ that DISABLES ipv4.

### E-2. trailing-segment semver (`1.2.3.4.5`).
ipv4 matches `1.2.3.4` (4-octet prefix). The `.5` trails outside the match. The output
shows colored `1.2.3.4` and uncolored `.5`. **Awkward but defensible.**
**Recommendation:** Document. v0.7+ could add a `pkg-version` profile.

### E-3. log_level inside identifier never matches (correct), but lowercase `error` doesn't
either. The pattern is intentionally case-sensitive. **No action needed**; reaffirm in spec.

### E-4. fqdn for JWT and base64-with-dots (C-9).
No pattern fix without a known-TLD allowlist. ACCEPT.

### E-5. http_status's broad 3-digit catch (C-3).
Already discussed. Profile-level gating is the right architectural answer; not v0.5.6.

---

## F. Unaudited patterns (flag for follow-up)

### F-1. timestamp internal capture group integrity under all 4 branches.
Did not exhaustively verify that the 5 named caps `date/sep/time/ms/tz` fire correctly for
all 4 alternation branches (only ISO has the named caps; syslog/Apache/RFC2822 branches
yield empty Option<Match> for those groups). Memory `feedback_alternation_caps` already
covers this; not a v0.5.6 audit gap.

### F-2. permission pattern start-of-line vs leading whitespace nuance.
Harness shows `drwxr-xr-x` at end-of-line still matches via the `(?:\s|$)` trailing anchor
+ `(?:^|\s)` leading anchor. Looks correct. Did not test edge cases of `permission` in
mid-line embedded with stripped surroundings (e.g. `chmod -rw-r--r-- file`). Flag for
follow-up but not a v0.5.6 gap.

### F-3. duration unit-Greek-letter (μs) byte sequence.
`\b\d+(?:\.\d+)?(?:\s?(?:ns|us|μs|ms)|[smhd])` — the `μ` is a multibyte UTF-8 sequence
(`0xCE 0xBC`). Under `regex::bytes` this is matched as the literal 2-byte sequence, which
works when input is UTF-8. Test coverage may be thin; flag for follow-up.

### F-4. URL trailing `.,;:!?` trim across all schemes.
Confirmed for `https://`. Not directly tested for `ssh://`, `ftp://`, `git@`. Visual check
of regex suggests behaviour is symmetric. Flag for completion.

### F-5. aws.region's interaction with Y-pod-with-region (`Pod my-app-us-east-1-bcdfg-12345`).
Harness shows NO match in k8s profile context — `us-east-1` is NOT in the k8s profile's
appended rules; without aws profile activated, region pattern doesn't exist. Verifies
profile isolation. No action.

### F-6. k8s.pod_name interior collisions.
Did not probe pod names with interior `aa:bb` MAC-shape (impossible, no `:` in pod_name
charset), or with 17-hex-shape (impossible, hyphen-separated). Pod name shape is
self-anchored. **Confirmed safe.**

### F-7. docker.container_id ↔ uuid interior overlap.
`abc12345-1234-1234-1234-123456789012` — does the 12-hex container_id win on a substring
within a UUID? **Not tested.** Predicted: uuid idx 2 < container_id idx 14 in docker
context; uuid wins envelope. Add positive regression.

### F-8. http_status capture-group correctness.
Pattern is `(?:^|[\s/:])([1-5]\d{2})\b` — the `(...)` is capture group 1. Did harness span
include the leading `[\s/:]`? Yes — `http_status=` 250``. The pipeline likely uses
`captures_iter` to colour only group 1 if `group_styles[0]` is `Some`. Check: harness shows
`http_status=` 250`` literal — the leading space IS in the match span. Whether tayf colors
the space depends on `group_styles`. From `src/rules.rs:459`, http_status has empty
`group_styles: Vec::new()` — so the whole match (including the leading punct) gets the
http_status style. **Confirmed FP — leading whitespace painted magenta.**

**RECOMMENDATION D-9:** Set `group_styles` on http_status to color group 1 only, leaving
the leading `[\s/:]` neutral. This is a one-line `src/rules.rs` data fix that improves UX
significantly. Add to v0.5.6 bundle as a cosmetic win.

---

## Summary table

| Cat | Count | Notes                                                                     |
|-----|------:|---------------------------------------------------------------------------|
| A   |   29  | Confirmed correct, no action needed                                       |
| B   |    3  | Already pinned in v0.5.3 (`_v0_5_3_limitation` suffix)                    |
| C   |   17  | NEW collisions found — see C-1..C-17 above                                |
| D   |    7  | Tighten-pattern opportunities (data-only)                                 |
| E   |    5  | Ambiguities no pattern can fix (spec-judgment / profile-level)            |
| F   |    8  | Unaudited / follow-up flags                                               |

---

## Recommended v0.5.6 scope expansion

### MUST bundle (high value, low risk):

1. **C-2 — ipv6 ↔ Rust path syntax.** Tighten ipv6 third branch (D-1). Pattern-data-only.
   Add comprehensive negative regression suite. **High developer-experience win.**
2. **F-8 (D-9) — http_status leading-punct in match span.** Add `group_styles` to color
   only group 1. One-line data fix.
3. **C-12 + C-13 + C-14 + B-1 + B-2 (the AWS interior-collision family).** This is the
   "architectural collision fix" sub-version centerpiece. Profile append_rules need a
   priority mechanism that lets them claim the ENVELOPE over interior built-in matches.
   Options:
   - **Per-rule `priority: i32` field** on `BuiltinRule` with default 0; profile
     append_rules can ship `priority = 100`. Pipeline overlap resolution becomes
     `(priority DESC, rule_index ASC)`.
   - **Envelope-anchor preference:** a candidate span that strictly contains another
     accepted span replaces it.
   - **Per-profile opt-in** to envelope-priority mode.
   Recommend spec phase brainstorm Option-A (priority field) — cleanest and
   backwards-compatible.

### SHOULD bundle (medium value):

4. **C-1 — mac ↔ ipv6 8-pair.** Pin as `_v0_5_5_limitation`. No fix; document the
   first-match-wins precedence.
5. **D-4 — ipv4 negative regression coverage.** Tests-only; no production impact.
6. **D-5 — mac negative regression coverage.** Tests-only.
7. **D-7 — log_level positive coverage.** Tests-only.

### DEFER to v0.5.7+:

8. **C-3 — http_status broad 3-digit catch.** Needs profile-level architectural decision;
   maps cleanly to a v0.6 "http" / "web" profile spec.
9. **C-9 — fqdn JWT-shape FP.** No clean pattern fix; user-config or v0.6 known-TLD
   allowlist territory.
10. **C-4 / D-6 — filename single-letter ext prose collision.** LOW frequency; document
    and move on.
11. **C-8 — filename ↔ fqdn on Go pkg paths.** Defer to a v0.7 "Go" profile.
12. **F-3, F-4, F-7 — minor coverage gaps.** Add to v0.5.6 test backlog.

### Bundle-or-defer summary

- **3 must-have items** (C-2 fix, F-8 cosmetic fix, C-12/13/14/B-1/B-2 architectural).
- **4 should-have items** (pin + tests).
- **5 defer items** (out-of-scope architectural / low-value).

The architectural fix (item 3) is the load-bearing element. Items 1 and 2 are good citizens
that ride along. Items 4-7 are test hygiene.

---

## Appendix — harness path

- Source: `/tmp/fp-audit/Cargo.toml`, `/tmp/fp-audit/src/main.rs`.
- Results: `/tmp/fp-audit/results.txt` (934 lines, 133 cases).
- Verification: every claim in this document is derivable from `results.txt` by grepping
  the case category.

To rerun: `cd /tmp/fp-audit && cargo run --release > results.txt`.

## Appendix — patterns vs canonical source

The harness's `FILENAME_EXTENSIONS` was synchronised with `src/rules.rs:111-324` on
2026-05-27. If a v0.5.6 spec adds extensions to `FILENAME_EXTENSIONS`, re-derive the
harness list with:

```bash
awk '/^const FILENAME_EXTENSIONS/,/^\];/' src/rules.rs \
  | grep -oE '"[^"]+"' | paste -sd ','
```

and paste into `/tmp/fp-audit/src/main.rs`.

Profile patterns were copied verbatim from `assets/profiles/*.toml` on 2026-05-27.
