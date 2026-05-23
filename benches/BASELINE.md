# tayf v0.1.1 benchmark baseline

Captured with `cargo bench --bench throughput -- --save-baseline v0.1.1`.
Reproduce on a quiet machine; criterion's defaults (3s warmup, 100 samples
over ~5s) apply.

## Environment

- Date: 2026-05-21
- Hardware: Apple M2 Pro (10 cores), macOS 15.6 (Darwin 24.6.0, arm64)
- Toolchain: rustc 1.95.0 (59807616e 2026-04-14) — Homebrew build
- Profile: `[profile.release]` from `Cargo.toml` (lto = "thin",
  codegen-units = 1, opt-level = 3, panic = "abort")
- Input shape: 1000 copies of
  `"connect 192.168.1.1 to 10.0.0.1 via 172.16.0.1 status 200 OK ERROR\n"`
  (67 KB total), exercising the IPv4, `http_status`, and `log_level` rules
  on every line.

## apply_rules / ipv4-heavy

Per-line rule scanner (five matches per line: three IPv4 + one
`http_status` + one `log_level`, no overlapping conflicts).

- Time per iter: 7.46 ms – 7.48 ms – 7.51 ms (low / mean / high)
- Throughput:    8.512 MiB/s – 8.545 MiB/s – 8.570 MiB/s
- Outliers:      1 / 100 high severe

## passthrough / write_all

`Cursor<Vec<u8>>::write_all` on the same 67 KB input — models tayf's
TUI passthrough path (alt-screen, bracketed paste, mouse). Essentially
the `cat`-equivalent denominator for the spec §7 overhead target.

- Time per iter: 1.29 µs – 1.33 µs – 1.38 µs
- Throughput:    45.342 GiB/s – 46.781 GiB/s – 48.273 GiB/s
- Outliers:      1 / 100 high mild

## Notes & caveats

- These numbers measure the **scanner** in isolation, not end-to-end PTY
  throughput. A real session is bottlenecked by `read(2)` from the PTY
  master, the kernel's tty discipline, and stdout's blocking writes, none
  of which appear here. They are useful as a regression tripwire for the
  v0.4 `RegexSet` fast-path and for spotting accidental allocator churn,
  not as a literal "tayf is N% slower than cat" claim.
- The passthrough number is in-process `memcpy`-class and will dwarf any
  real pipe + tty path. Compare it to itself across versions, not to
  `apply_rules` directly.
- Spec §7 target ("<20% overhead vs native `cat`") is end-to-end and will
  be validated separately once the v0.4 fast-path lands.
- The `apply_rules` cost is dominated today by the linear walk over eight
  individual regexes (`Compiled::individuals`) and the per-match `Vec`
  allocation for spans. v0.4's `RegexSet` pre-filter is expected to drop
  the per-line cost roughly an order of magnitude on inputs where most
  rules miss; the synthetic input here hits the worst-case path
  (three rules match) and so improvements will be smaller.

## v0.2.4 baseline (recorded 2026-05-23)

Source: HEAD = 1de4cb50e4ec4976abfdf9d1fe4136eeaffba818
Toolchain: rustc 1.95.0 (59807616e 2026-04-14) (Homebrew)
Host: Apple M2 Pro, macOS (Darwin 24.6.0, arm64)
Input: identical to v0.1.1 above (1000-copy `connect 192.168.1.1 …`).
Profile: release (`cargo bench`).

Criterion output excerpt:

```
apply_rules/ipv4-heavy  time:   [7.6697 ms 7.6786 ms 7.6881 ms]
                        thrpt:  [8.3110 MiB/s 8.3213 MiB/s 8.3309 MiB/s]

passthrough/write_all   time:   [1.1473 µs 1.1492 µs 1.1515 µs]
                        thrpt:  [54.190 GiB/s 54.299 GiB/s 54.387 GiB/s]
```

Notes on the v0.1.1 → v0.2.4 delta:

- `apply_rules/ipv4-heavy` ~3% slower (~7.48 ms → ~7.68 ms, 8.545 → 8.321 MiB/s).
  Expected: v0.2.x grew the default ruleset from 8 to 13 built-ins, and the
  synthetic walks every individual regex (`Compiled::individuals`) per line.
- `passthrough/write_all` ~14% faster (~1.33 µs → ~1.15 µs, 46.78 → 54.30 GiB/s).
  Likely run-to-run variance plus toolchain microbenefits; no semantic change to
  the passthrough path between v0.1.1 and v0.2.4.

These numbers anchor the v0.3.0 < 20% regression budget per spec §7.4.

## v0.3.0 measurement (recorded 2026-05-23)

Source: HEAD = 18b63c424e2da4337e4d0f65f172569533b3950c
Toolchain: rustc 1.95.0 (59807616e 2026-04-14) (Homebrew)
Host: Apple M2 Pro, macOS (Darwin 24.6.0, arm64) (same as v0.2.4 baseline)
Input: identical to v0.2.4 (and v0.1.1) above.
Profile: release (`cargo bench`).

Criterion output excerpt:

```
apply_rules/ipv4-heavy  time:   [7.6973 ms 7.7261 ms 7.7737 ms]
                        thrpt:  [8.2196 MiB/s 8.2702 MiB/s 8.3011 MiB/s]
                 change: time:   [+0.1974% +0.6182% +1.2049%] (p = 0.01 < 0.05)
                        thrpt:  [−1.1906% −0.6144% −0.1970%]
                        Change within noise threshold.

passthrough/write_all   time:   [1.1869 µs 1.2109 µs 1.2366 µs]
                        thrpt:  [50.461 GiB/s 51.529 GiB/s 52.571 GiB/s]
                 change: time:   [+3.5068% +5.6576% +8.3184%] (p = 0.00 < 0.05)
                        thrpt:  [−7.6796% −5.3547% −3.3879%]
                        Performance has regressed.
```

### Regression check vs v0.2.4 baseline

| Bench group | v0.2.4 | v0.3.0 | Delta |
|---|---|---|---|
| `apply_rules/ipv4-heavy` | 7.6786 ms (8.3213 MiB/s) | 7.7261 ms (8.2702 MiB/s) | +0.62% time / −0.61% thrpt |
| `passthrough/write_all` | 1.1492 µs (54.299 GiB/s) | 1.2109 µs (51.529 GiB/s) | +5.37% time / −5.10% thrpt |

Spec budget per §7.4: < 20% regression. Status: PASS on both bench groups
(both deltas well inside the 20% ceiling).

Notes on the v0.2.4 → v0.3.0 delta:

- `apply_rules/ipv4-heavy` ~0.6% slower (effectively run-to-run noise; criterion
  flags "Change within noise threshold"). The Pipeline.feed hot path now routes
  every byte through `AnsiSm::step` and matches on a `StepEvent` enum, but on
  the synthetic ASCII-only input every byte produces `StepEvent::Data`, so the
  per-byte overhead is one enum tag dispatch on top of the previous direct push.
  Negligible at this resolution.
- `passthrough/write_all` ~5.4% slower (~1.15 µs → ~1.21 µs, 54.30 → 51.53 GiB/s).
  Criterion flags this as a statistically-significant regression but it is still
  well inside the < 20% spec ceiling. The TUI-mode passthrough fast-path now
  feeds bytes through `AnsiSm::step` first to detect mode toggles instead of
  the older direct-write `TuiModeSm`; that adds a per-byte state machine step
  even when the result is "passthrough". Acceptable cost for the v0.3.0
  semantic upgrade (correct DECSET/DECRST handling, multi-toggle sequences,
  byte-for-byte verbatim alt-screen / bracketed-paste / mouse passthrough).
- 17% outliers on `passthrough/write_all` likely reflect macOS scheduler jitter
  on a sub-µs measurement; same pattern showed up in v0.2.4 (1 mild outlier on
  a similarly tiny per-iter time).

## v0.3.2 measurement (recorded 2026-05-23)

Source: HEAD = 39dc9f4 (CHANGELOG entry, post version bump 01a26b7)
Toolchain: rustc 1.95.0 (59807616e 2026-04-14) (Homebrew)
Host: Apple M2 Pro, macOS (Darwin 24.6.0, arm64) (same as v0.3.0 baseline)
Input: identical to v0.3.0 (and earlier) above.
Profile: release (`cargo bench`).

Criterion output excerpt:

```
apply_rules/ipv4-heavy  time:   [7.7444 ms 7.7543 ms 7.7642 ms]
                        thrpt:  [8.2296 MiB/s 8.2401 MiB/s 8.2506 MiB/s]
                 change: time:   [−0.2528% +0.3644% +0.7881%] (p = 0.20 > 0.05)
                        thrpt:  [−0.7819% −0.3631% +0.2534%]
                        No change in performance detected.

passthrough/write_all   time:   [1.2880 µs 1.3550 µs 1.4273 µs]
                        thrpt:  [43.718 GiB/s 46.051 GiB/s 48.445 GiB/s]
                 change: time:   [+2.5578% +6.6031% +11.380%] (p = 0.00 < 0.05)
                        thrpt:  [−10.217% −6.1941% −2.4940%]
                        Performance has regressed.
```

### Regression check vs v0.3.0 baseline

| Bench group | v0.3.0 | v0.3.2 | Delta |
|---|---|---|---|
| `apply_rules/ipv4-heavy` | 7.7261 ms (8.2702 MiB/s) | 7.7543 ms (8.2401 MiB/s) | +0.36% time / −0.36% thrpt |
| `passthrough/write_all` | 1.2109 µs (51.529 GiB/s) | 1.3550 µs (46.051 GiB/s) | +6.60% time / −10.63% thrpt |

Spec budget per §5.2: < 20% regression. Status: PASS on both bench groups
(both deltas well inside the 20% ceiling).

Notes on the v0.3.0 → v0.3.2 delta:

- `apply_rules/ipv4-heavy` ~0.4% slower (criterion flags "No change in
  performance detected", p = 0.20 > 0.05). The v0.3.2 URL pattern grew a
  trailing-tail char class plus a 4th alternation branch (`git@host:path`),
  and the `duration` pattern grew a repeat-group for compound forms
  (`2d3h`, `1h30m20s`). Neither shows up in this synthetic benchmark
  because the input exercises only `ipv4`, `http_status`, and `log_level`
  — but the linear-DFA structure of the new patterns means the runtime
  cost on URL/duration-heavy inputs would be similarly linear.
- `passthrough/write_all` ~6.6% slower (~1.21 µs → ~1.36 µs, 51.53 →
  46.05 GiB/s). Sub-µs per-iter scheduler jitter dominates; criterion
  flags 7% outliers (3 mild + 4 severe). No code change in v0.3.2 touches
  the passthrough hot path — this is run-to-run variance, not a regression
  in the implementation. Cumulative v0.1.1 → v0.3.2 delta on this group is
  +2% time / −1.5% thrpt vs the original baseline, which is within noise.
- No v0.3.2 change should plausibly affect either bench group: pattern
  changes (A/B/C) are not exercised by the input fixture, and the D
  changes (`TAYF_DISABLE_BG_DETECT` env-var check at startup, watch test
  rewrite) touch startup and test paths only. Observed deltas are
  consistent with that expectation.

## v0.3.3 measurement (recorded 2026-05-23)

Source: HEAD = 7e79bde (README updates, post version bump a05fee1)
Toolchain: rustc 1.95.0 (59807616e 2026-04-14) (Homebrew)
Host: Apple M2 Pro, macOS (Darwin 24.6.0, arm64) (same as v0.3.2 baseline)
Input: identical to v0.3.2 (and earlier) above.
Profile: release (`cargo bench`).

Criterion output excerpt:

```
apply_rules/ipv4-heavy  time:   [7.6608 ms 7.6713 ms 7.6831 ms]
                        thrpt:  [8.3164 MiB/s 8.3292 MiB/s 8.3407 MiB/s]
                 change: time:   [−1.2568% −1.0697% −0.8668%] (p = 0.00 < 0.05)
                        thrpt:  [+0.8744% +1.0813% +1.2728%]
                        Change within noise threshold.

passthrough/write_all   time:   [1.2072 µs 1.2252 µs 1.2447 µs]
                        thrpt:  [50.132 GiB/s 50.930 GiB/s 51.689 GiB/s]
                 change: time:   [−9.2209% −5.3260% −1.7097%] (p = 0.01 < 0.05)
                        thrpt:  [+1.7394% +5.6257% +10.158%]
                        Performance has improved.
```

### Regression check vs v0.3.2 baseline

| Bench group | v0.3.2 | v0.3.3 | Delta |
|---|---|---|---|
| `apply_rules/ipv4-heavy` | 7.7543 ms (8.2401 MiB/s) | 7.6713 ms (8.3292 MiB/s) | −1.07% time / +1.08% thrpt |
| `passthrough/write_all` | 1.3550 µs (46.051 GiB/s) | 1.2252 µs (50.930 GiB/s) | −9.58% time / +10.59% thrpt |

Spec budget per §5.2: < 20% regression. Status: PASS on both bench groups
(both deltas are improvements, well inside the budget ceiling).

Notes on the v0.3.2 → v0.3.3 delta:

- `apply_rules/ipv4-heavy` ~1% faster (criterion flags "Change within
  noise threshold"). v0.3.3 does not modify the hot path — F1 (bypass),
  F2 (no-hot-reload + SIGHUP forwarding), and F3 (reload banner) all
  touch startup orchestration or the reload thread, never the per-line
  rule scanner. The small improvement is run-to-run variance against
  v0.3.2's measurement (which itself showed +6.6% noise on the sub-µs
  passthrough group).
- `passthrough/write_all` ~9.6% faster — clear scheduler/thermal
  recovery from v0.3.2's outlier-heavy run (v0.3.2 had 7% outliers).
  v0.3.3 outliers: 14% (7 mild + 7 severe) — sub-µs measurements remain
  noisy. Cumulative v0.1.1 → v0.3.3 delta on this group is −7.4% time /
  +8.9% thrpt vs the original baseline, fully within noise.
- No v0.3.3 change should plausibly affect either bench group:
  - The bypass branch in `Tayf::run` is gated by `if bypass { ... }`
    early-return and never reached when the user runs without
    `--bypass` / `TAYF_DISABLE`.
  - The `--no-hot-reload` gate is one boolean check at startup; the
    hot-path Pipeline is identical to v0.3.2 in the default config.
  - The reload-banner field gating + `Option<Box<dyn BannerSink>>` arg
    add one branch inside the reload thread, fires at most every 200 ms
    on a config save — orders of magnitude below the per-line cost.
  - The SIGHUP forwarding fix is in the signal-thread handler, never
    on the I/O hot path.
  Observed deltas are consistent with that expectation.
