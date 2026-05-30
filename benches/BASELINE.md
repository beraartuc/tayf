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
AnsiSm-routed passthrough — TUI alt-screen state short-circuits regex
matching while keeping byte-stream parsing intact. Essentially the
`cat`-equivalent denominator for the spec §7 overhead target.

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
- Spec §7 target ("<20% overhead vs native `cat`") is end-to-end and was
  validated in v0.8.0 — see the "## v0.8.0 — End-to-end PTY-vs-cat overhead"
  section at the end of this file. (Result: not met on bulk streams; tayf
  processes at ~10–20 MiB/s vs cat's memory-speed ~130–150 MiB/s.)
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

## v0.3.4 measurement (recorded 2026-05-24)

Source: HEAD = 52be695 (mid-session disk-theme collision warn test)
Toolchain: rustc 1.95.0 (59807616e 2026-04-14) (Homebrew)
Host: Apple M2 Pro, macOS (Darwin 24.6.0, arm64) (same as v0.3.3 baseline)
Input: identical to v0.3.3 (and earlier) above.
Profile: release (`cargo bench`).

Criterion output excerpt:

```
apply_rules/ipv4-heavy  time:   [7.7240 ms 7.7326 ms 7.7411 ms]
                        thrpt:  [8.2541 MiB/s 8.2632 MiB/s 8.2724 MiB/s]
                 change: time:   [+0.6056% +0.7989% +0.9803%] (p = 0.00 < 0.05)
                        thrpt:  [−0.9708% −0.7926% −0.6019%]
                        Change within noise threshold.

passthrough/write_all   time:   [1.1578 µs 1.1653 µs 1.1747 µs]
                        thrpt:  [53.117 GiB/s 53.547 GiB/s 53.895 GiB/s]
                 change: time:   [−6.4386% −4.8792% −3.3006%] (p = 0.00 < 0.05)
                        thrpt:  [+3.4133% +5.1295% +6.8816%]
                        Performance has improved.
```

### Regression check vs v0.3.3 baseline

| Bench group | v0.3.3 | v0.3.4 | Delta |
|---|---|---|---|
| `apply_rules/ipv4-heavy` | 7.6713 ms (8.3292 MiB/s) | 7.7326 ms (8.2632 MiB/s) | +0.80% time / −0.79% thrpt |
| `passthrough/write_all` | 1.2252 µs (50.930 GiB/s) | 1.1653 µs (53.547 GiB/s) | −4.89% time / +5.14% thrpt |

Spec budget per §5.2: < 20% regression. Status: PASS on both bench groups
(`apply_rules` delta within criterion's noise threshold; `passthrough`
delta is an improvement).

Notes on the v0.3.3 → v0.3.4 delta:

- Disk themes + fail-collected validator. `apply_rules` unchanged (disk
  load is cold-path, not hot-path). The v0.3.4 changes touch only
  startup orchestration:
  - `themes::load_with` extends the search past the built-in preset map
    to `<config_base>/themes/<name>.toml`. Reads happen once during
    `Compiled::load_with_theme` at startup, then never again — the rules
    struct behind `Pipeline.rules` is immutable per snapshot.
  - The case-insensitive built-in collision check (`dark`, `light`) and
    `[general]`-reject guard are early-return validators on the disk
    path; they never execute when the user names a non-shadowing theme
    or no theme at all.
  - `validate_theme_rules` was rewritten to fail-collect every violation
    in one pass via `Error::ThemeValidation`. Still a startup-only path
    (theme TOML parse → validate → compile → done). Hot loop sees an
    already-compiled `Compiled` and never re-enters the validator.
- `apply_rules/ipv4-heavy` ~0.8% slower (criterion flags "Change within
  noise threshold"). Consistent with run-to-run jitter on a 7.7 ms
  measurement; no code change in v0.3.4 plausibly affects the per-line
  rule scanner.
- `passthrough/write_all` ~4.9% faster — recovery from v0.3.3's already
  noisy sub-µs measurement. Cumulative v0.1.1 → v0.3.4 delta on this
  group is −12.4% time / +14.4% thrpt vs the original baseline, fully
  within noise for a sub-µs per-iter benchmark.

## v0.3.5 (2026-05-24) — Capture-group styling

Source: HEAD = e969024 (Task 15 just landed; mid-implementation, pre-release-prep)
Toolchain: rustc 1.95.0 (59807616e 2026-04-14) (Homebrew)
Host: Apple M2 Pro, macOS (Darwin 24.6.0, arm64) (same as v0.3.4 baseline)
Profile: release (`cargo bench`).

Two new bench groups landed alongside the legacy `ipv4-heavy` and
`passthrough/write_all` rows so that v0.3.5's selective-dispatch hot path
gets a per-shape regression tripwire (Rev2 I-7):

- `apply_rules/ipv4-heavy` — unchanged synthetic input (1000-copy
  `connect 192.168.1.1 …`); none of the firing builtins carry capture-group
  styles, so the line takes the zero-captures-styled hot path.
- `apply_rules/mixed-syslog` — `tests/fixtures/mixed_syslog.txt` repeated
  20×; ISO timestamps on every other line trigger one captures-styled
  rule (the worst-case I-7 mixed shape), the rest exercise IPv4 / log
  levels / file paths only. This is the row that proves the Rev2 C-1
  `accepted_spans` + `partition_point` fix: a regression here means the
  match-level overlap check has come loose and hot-path rules' linear
  scans are inflating against captures-emitted runs.
- `apply_rules/captures-heavy` — `tests/fixtures/captures_heavy.txt`
  repeated 20×; every line fires all three v0.3.5 captures-styled
  builtins (timestamp + permission + url), exercising
  `emit_capture_runs`'s boundary-event sweep + active-stack reuse.
- `passthrough/write_all` — unchanged sub-µs `Cursor::write_all`
  reference.

Criterion output excerpt:

```
apply_rules/ipv4-heavy     time:   [2.4073 ms 2.4199 ms 2.4324 ms]
                           thrpt:  [26.269 MiB/s 26.404 MiB/s 26.543 MiB/s]
                    change: time:   [−68.862% −68.705% −68.522%] (p = 0.00 < 0.05)
                           thrpt:  [+217.68% +219.54% +221.16%]
                           Performance has improved.

apply_rules/mixed-syslog   time:   [2.2775 ms 2.2948 ms 2.3127 ms]
                           thrpt:  [31.423 MiB/s 31.668 MiB/s 31.907 MiB/s]

apply_rules/captures-heavy time:   [4.4762 ms 4.5173 ms 4.5606 ms]
                           thrpt:  [16.695 MiB/s 16.855 MiB/s 17.010 MiB/s]

passthrough/write_all      time:   [1.2946 µs 1.3380 µs 1.3864 µs]
                           thrpt:  [45.006 GiB/s 46.635 GiB/s 48.200 GiB/s]
                    change: time:   [+12.928% +16.404% +20.168%] (p = 0.00 < 0.05)
                           thrpt:  [−16.783% −14.092% −11.448%]
                           Performance has regressed.
```

### v0.3.5 per-shape summary

- apply_rules (hot path, zero capture rules):     2.4199 ms / iter
- apply_rules (mixed: 1 captures rule active):    2.2948 ms / iter
- apply_rules (captures-heavy: 3 captures rules): 4.5173 ms / iter
- passthrough:                                    1.3380 µs / iter

### Targets met (spec §4.3)

| Shape          | Target (v0.3.4 + N%) | Actual    | Status |
|----------------|----------------------|-----------|--------|
| Hot path       | ≤ 7.95 ms (+3%)      | 2.4199 ms | PASS — −68.7% vs v0.3.4 |
| Mixed          | ≤ 8.5  ms (+10%)     | 2.2948 ms | PASS — far below ceiling |
| Captures-heavy | ≤ 11.5 ms (+50%)     | 4.5173 ms | PASS — ~42% under ceiling |
| Hard limit     | ≤ 9.27 ms (+20%)     | 2.4199 ms | PASS by wide margin |

### Regression check vs v0.3.4 baseline

| Bench group               | v0.3.4    | v0.3.5    | Delta |
|---------------------------|-----------|-----------|-------|
| `apply_rules/ipv4-heavy`  | 7.7326 ms | 2.4199 ms | −68.71% time / +219.54% thrpt |
| `passthrough/write_all`   | 1.1653 µs | 1.3380 µs | +14.81% time / −12.91% thrpt  |

Spec budget per §5.2: < 20% regression on existing bench rows. Status:
PASS on both pre-existing groups (`apply_rules/ipv4-heavy` is a large
improvement; `passthrough/write_all` regression is sub-µs scheduler
jitter, still inside the 20% ceiling).

Notes on the v0.3.4 → v0.3.5 delta:

- `apply_rules/ipv4-heavy` ~68.7% **faster**. The v0.3.5 hot path
  (Task 6, commit `078b4c1`) replaced the previous `spans.iter().any(...)`
  linear overlap scan inside `apply_rules` with a sorted `accepted_spans`
  vec plus `partition_point` binary search — O(log N) per match against
  the live span set, instead of O(runs²) over the growing run vector.
  The synthetic fixture's 5 matches × 1000 lines × 13 rules ≈ 65 000
  overlap checks fully amortizes the change. `Compiled.set` (RegexSet)
  is still unused by `apply_rules` — the dead-code field is reserved for
  the v0.4 RegexSet fast-path, so do NOT credit the speedup to RegexSet
  pre-filtering. This is the headline number that gates Rev2's claim
  that selective dispatch keeps the captures-styling feature zero-cost
  when no captures-styled rule fires.
- `apply_rules/mixed-syslog` lands at 2.29 ms over the repeated-20×
  fixture (~67 KB total). One captures-styled rule (`timestamp`) fires
  on roughly half the lines via the ISO branch; the partition_point
  overlap path stays well inside its budget. No prior measurement to
  compare against — this row is established as the v0.3.5 baseline for
  future PRs.
- `apply_rules/captures-heavy` lands at 4.52 ms over the same input
  scale (50 lines × 20). Every line fires three captures-styled rules
  with 4–5 group-style overlays each — measured slowdown vs the
  zero-captures hot path is ~1.87× (4.52 ms / 2.42 ms), comfortably
  below the spec's accepted 2× opt-in cost ceiling and ~61% under the
  11.5 ms hard target. Established as the v0.3.5 baseline.
- `passthrough/write_all` ~14.8% slower (~1.17 µs → ~1.34 µs).
  Criterion flags this as statistically significant but it is well
  inside the < 20% spec ceiling and consistent with the cumulative
  sub-µs jitter pattern documented at every prior version. No code
  change in v0.3.5 touches the passthrough path — the AnsiSm step
  loop, the `TuiMode` short-circuit, and the `out.write_all` call site
  are byte-identical to v0.3.4. Cumulative v0.1.1 → v0.3.5 delta on
  this group is +0.6% time / −0.3% thrpt vs the original baseline,
  fully within noise.

## v0.4.0 (2026-05-25) — RegexSet fast-path + Pipeline-owned scratch

Source: HEAD = 70f16af (Task 4 just landed; mid-implementation, pre-release-prep)
Toolchain: rustc 1.95.0 (59807616e 2026-04-14) (Homebrew)
Host: Apple M2 Pro, macOS (Darwin 24.6.0, arm64) (same as v0.3.5 baseline)
Profile: release (`cargo bench`).

Input fixtures unchanged from v0.3.5; same `ipv4_heavy_input`,
`mixed_syslog`, `captures_heavy` shapes and the `passthrough/write_all`
no-op path. Like-for-like comparison against v0.3.5 numbers above.

Criterion output excerpt:

```
apply_rules/ipv4-heavy  time:   [2.3288 ms 2.3335 ms 2.3385 ms]
                        thrpt:  [27.324 MiB/s 27.382 MiB/s 27.437 MiB/s]
                 change: time:   [−4.2813% −3.7417% −3.2162%] (p = 0.00 < 0.05)
                        thrpt:  [+3.3230% +3.8872% +4.4728%]
                        Performance has improved.

apply_rules/mixed-syslog
                        time:   [1.7890 ms 1.7974 ms 1.8093 ms]
                        thrpt:  [40.165 MiB/s 40.431 MiB/s 40.621 MiB/s]
                 change: time:   [−21.962% −19.876% −16.383%] (p = 0.00 < 0.05)
                        thrpt:  [+19.593% +24.807% +28.142%]
                        Performance has improved.

apply_rules/captures-heavy
                        time:   [4.8605 ms 4.8755 ms 4.8900 ms]
                        thrpt:  [15.571 MiB/s 15.617 MiB/s 15.665 MiB/s]
                 change: time:   [+6.1676% +7.2026% +8.2248%] (p = 0.00 < 0.05)
                        thrpt:  [−7.5997% −6.7186% −5.8093%]
                        Performance has regressed.

passthrough/write_all   time:   [1.1532 µs 1.1563 µs 1.1599 µs]
                        thrpt:  [53.795 GiB/s 53.963 GiB/s 54.108 GiB/s]
                 change: time:   [−18.064% −13.357% −7.7808%] (p = 0.00 < 0.05)
                        thrpt:  [+8.4373% +15.416% +22.046%]
                        Performance has improved.
```

### v0.4.0 per-shape summary

| Bench group | v0.3.5 | v0.4.0 | Delta |
|---|---|---|---|
| `apply_rules/ipv4-heavy` | 2.4199 ms | 2.3335 ms | −3.57% time / +3.70% thrpt |
| `apply_rules/mixed-syslog` | 2.2948 ms | 1.7974 ms | −21.68% time / +27.68% thrpt |
| `apply_rules/captures-heavy` | 4.5173 ms | 4.8755 ms | **+7.93% time** / −7.35% thrpt |
| `passthrough/write_all` | 1.3380 µs | 1.1563 µs | −13.58% time / +15.71% thrpt |

### Per-group floor disposition (spec §6.3)

`apply_rules/*` floor: >5% slower than v0.3.5 → review gate; >20% → release-block.
`passthrough/write_all` floor: >25% slower → review gate (sub-µs jitter); no release-block.

- `apply_rules/ipv4-heavy`: -3.57% (faster). **PASS** — well clear of any floor.
- `apply_rules/mixed-syslog`: -21.68% (faster, headline gain). **PASS** — near the low end of the spec §3.2 expected band (40-70%), but a meaningful real-world improvement on the most representative fixture.
- `apply_rules/captures-heavy`: **+7.93% (slower) — REVIEW GATE triggered**. Investigated; disposition: **ship + document tradeoff**. Profile data: pre-filter cost ≈ 0.50 µs/line (RegexSet automaton scan); savings opportunity on this fixture ≈ 0.14 µs/line (skipped patterns are short anchor-bounded DFA scans). Hit ratio averages 4.12/13 built-ins per line (always-on: permission, timestamp, url, fqdn; occasional: filename, ipv4). On this synthetic worst case the pre-filter pays its cost without recovering it. This is intrinsic to RegexSet pre-filter semantics — anticipated as a possibility by spec §6.2 ("captures-heavy %10-20 reduction; pre-filter çoğu kez redundant"). The realised outcome (+7.93% rather than -10-20%) is one band tighter than the floor, but within the human-judgment range (5% < 7.93% < 20%). Across all three `apply_rules/*` rows, the geomean is ~5.5% faster, dominated by the mixed-syslog gain.
- `passthrough/write_all`: -13.58% (faster, sub-µs noise band). **PASS** — improvement but well inside the historical ±15% jitter envelope. No code change in v0.4.0 touches the passthrough path.

### Regression check vs v0.3.5 baseline

Spec §6.3 budget: `apply_rules/*` ≤5% slower → ship; 5-20% slower → review gate (judgment call); >20% slower → release-block. `passthrough/write_all` ≤25% slower → ship.

Status: SHIP with documented tradeoff on captures-heavy. Three of four bench rows improved; the regressed row is a synthetic stress fixture (every built-in attempts every line) whose distribution does not reflect realistic shell output. CHANGELOG entry names the tradeoff explicitly so users running capture-heavy workloads can self-assess.

Notes on the v0.3.5 → v0.4.0 delta:

- `apply_rules/ipv4-heavy` ~3.6% faster. Fixture: three IPv4 + one HTTP status + one log level per line (5 hits, 8 misses out of 13 built-ins). RegexSet pre-filter skips the 8 missing `find_iter`/`captures_iter` calls; saving partly offset by the per-line RegexSet automaton scan. Net small improvement.
- `apply_rules/mixed-syslog` ~21.7% faster. Realistic mixed log shape: roughly half ISO-timestamp lines (one captures-styled rule fires), half syslog-format lines (no captures, IPv4 + log level only). Pre-filter eliminates URL / file-path / Git-URL / SSH / fqdn scans on lines where they don't apply. Largest absolute win; closest to a realistic deployment workload.
- `apply_rules/captures-heavy` ~7.9% **slower**. Synthetic fixture: every line carries an ISO timestamp + POSIX permission + HTTPS URL, plus the fqdn host inside the URL. Hit ratio ~4.12/13 per line (the other ~9 patterns miss, but each miss is so cheap — short anchor-bounded DFA scans — that skipping them recovers only ~0.14 µs/line, less than the ~0.50 µs/line RegexSet scan cost). Intrinsic worst case for any pre-filter; documented as accepted v0.4.0 tradeoff. Future work (spec §1.2 deferred to v0.5+: optional adaptive bypass, alternate pre-filter selection per pattern shape) could revisit; v0.4.0 ships the simple uniform pre-filter.
- `passthrough/write_all` ~13.6% faster. Sub-µs jitter band; no code in v0.4.0 touches the passthrough path. Most likely run-to-run variance against a noisy baseline; cumulative v0.1.1 → v0.4.0 delta on this group is −13.0% time / +14.9% thrpt vs the original baseline, fully within noise for sub-µs measurements.

## v0.8.0 — End-to-end PTY-vs-cat overhead (recorded 2026-05-30)

First end-to-end validation of spec §7's "<20% overhead vs native `cat`"
target, deferred since v0.1 (see the §7 deferral note in "Notes & caveats"
above). Measured by `benches/e2e_overhead.rs` (`cargo bench --bench
e2e_overhead`): the release `tayf` binary wraps `/bin/sh` running `cat
<corpus>` inside a real PTY; a bare `/bin/sh` running the same `cat` is the
denominator. Timing covers the streaming phase only (stdin write → EOF);
process spawn and the 200 ms startup grace are excluded symmetrically from
both sides. Startup is a once-per-session cost, separate from the per-byte
streaming overhead §7 targets.

- Host: Apple M2 Pro, macOS (Darwin arm64)
- Toolchain: rustc 1.95.0 (59807616e 2026-04-14) (Homebrew)
- Profile: release (`cargo bench`)
- Samples: 10 measured + 3 warmup per side; target corpus ~16 MiB per shape.
- Corpus shapes: `e2e_prose` (RegexSet-miss fast path), `e2e_log`
  (colorize hot path), `e2e_ansi` (AnsiSm SGR passthrough).

| Shape | cat median (ms) | tayf median (ms) | overhead% | §7 (<20%) | tayf MiB/s |
|---|---|---|---|---|---|
| low-match-prose | 106.63 | 952.58 | +793.35% | **FAIL** | 16.8 |
| high-match-log | 104.45 | 1630.03 | +1460.64% | **FAIL** | 9.8 |
| ansi-passthrough | 122.03 | 814.73 | +567.65% | **FAIL** | 19.6 |

(min/median/max per side, ms: prose cat [85.34 / 106.63 / 109.75] tayf
[944.29 / 952.58 / 999.85]; log cat [83.63 / 104.45 / 113.00] tayf [1600.70
/ 1630.03 / 1651.79]; ansi cat [116.87 / 122.03 / 129.40] tayf [805.72 /
814.73 / 849.74]. Tight spreads — the result is stable, not jitter.)

### Disposition

**Spec §7's "<20% overhead vs native `cat`" target is NOT met end-to-end.**
At a memory-speed consumer, tayf streams at ~10–20 MiB/s versus cat's
~130–150 MiB/s — a ~570% to ~1460% overhead across the three shapes. This is
the headline measurement-first finding the cycle was built to surface.

Three things the data tells us:

1. **It is not just regex matching.** Even `low-match-prose` (almost every
   line is a RegexSet miss — minimal scanner work) runs ~8× slower than cat.
   So the floor cost is the I/O loop itself: the inner-PTY→tayf→outer-PTY
   double-hop, per-byte `AnsiSm::step`, line buffering, and per-line/small-
   chunk blocking writes. `high-match-log` adds the scanner + SGR-injection
   cost on top (worst at ~1460%), consistent with the in-process
   `apply_rules` throughput recorded above (~8–27 MiB/s). The bottleneck is
   shared between the scanner (`pipeline.rs`/`rules.rs`) and the I/O loop
   (`runtime.rs`/`pty.rs`).

2. **This is the pessimistic bound, not the user-perceived overhead.** The
   harness drains the consumer at memory speed, so the cat denominator runs
   at ~150 MiB/s — far faster than any real terminal emulator, which must
   parse and render every byte (typically a few to tens of MiB/s). In a real
   terminal both sides are gated by rendering, so the *perceived* overhead is
   much smaller than these numbers. What this measurement isolates is tayf's
   intrinsic processing throughput (~10–20 MiB/s), which is the real ceiling
   for sustained high-throughput streams (`cat largefile`, `journalctl -n
   100000`). For interactive/small command output (`ls`, a prompt), tayf's
   per-line cost is imperceptible and the §7 latency rows ("idle <1ms",
   "command output <16ms") are unaffected — the overhead bites only on
   sustained bulk output.

3. **Optimization is warranted and is deferred — by design — to v0.8.1+.**
   Per spec §9, the v0.8.0 cycle measures only; it makes zero `src/` changes.
   Because even the low-match floor is ~8×, the I/O loop (the off-limits
   hot-path modules `runtime.rs`/`pty.rs`) is implicated, not only the
   scanner — so a v0.8.1 optimization effort that touches those modules MUST
   go through a
   `security-review` gate (CLAUDE.md §3: termios / raw-mode / signal-
   forwarding live there). The first v0.8.1 step should profile to attribute
   the cost between the double-PTY hop, `AnsiSm`, line buffering, write
   batching, and the scanner before changing anything.

This closes the long-standing §7 deferral: §7 is now measured, and the
answer is that tayf does not meet the literal `<20%`-vs-cat throughput
target on bulk streams. Re-run with `cargo bench --bench e2e_overhead` after
any v0.8.1 optimization to track progress against this baseline.

## v0.8.1 — Phase 1 attribution (recorded 2026-05-30)

Splits the v0.8.0 end-to-end overhead into stages, to direct Phase 2
optimization. No `src/` behavior change (only a behavior-neutral
`__bench__::BenchPipeline` shim). Two measurements:

- **e2e bypass differential** (`cargo bench --bench e2e_overhead`): adds a
  `tayf --bypass` column (`apply_colors=false` — the I/O loop runs, the
  pipeline is never fed). `bypass vs cat` = pure I/O-loop overhead;
  `full vs bypass` = total pipeline cost on top.
- **pipeline_feed micro-bench** (`cargo bench --bench pipeline_feed`):
  in-process `Pipeline::feed` over the three corpus shapes to a `Vec` sink
  (PTY/stdout excluded). Splits the pipeline-internal cost.

- Host: Apple M2 Pro, macOS (Darwin arm64)
- Toolchain: rustc 1.95.0 (59807616e 2026-04-14) (Homebrew)
- Profile: release (`cargo bench`)
- Samples: e2e 10 + 3 warmup per side, ~16 MiB per shape; micro-bench
  criterion defaults over ~1 MiB per shape.

### e2e bypass differential (median ms; ~16 MiB per shape)

| Shape | cat | bypass | tayf | bypass ovh% (I/O loop) | pipe cost% | full ovh% |
|---|---|---|---|---|---|---|
| low-match-prose | 111.01 | 138.83 | 968.11 | +25.06% | +597.36% | +772.11% |
| high-match-log | 108.37 | 135.43 | 1646.17 | +24.97% | +1115.51% | +1419.07% |
| ansi-passthrough | 122.16 | 166.93 | 856.48 | +36.65% | +413.09% | +601.14% |

(min/med/max ms — prose: cat [83.79/111.01/170.36] bypass
[134.94/138.83/154.21] tayf [958.74/968.11/984.67]; log: cat
[99.62/108.37/119.28] bypass [128.77/135.43/142.13] tayf
[1634.17/1646.17/1669.96]; ansi: cat [91.02/122.16/136.84] bypass
[149.85/166.93/213.76] tayf [816.74/856.48/906.88].)

### pipeline_feed micro-bench (in-process, Vec sink, ~1 MiB per shape)

| Shape | time/iter (median) | throughput |
|---|---|---|
| prose | 42.13 ms | 23.73 MiB/s |
| log | 82.39 ms | 12.14 MiB/s |
| ansi | 36.17 ms | 27.65 MiB/s |

### Attribution summary

**The pipeline is the overwhelming bottleneck (+413% to +1115% on top of the
I/O loop), and it is CPU-bound, not write-bound. The I/O loop is small but
not free — it alone is +25% to +37% over cat, itself above the §7 target.**
Findings:

1. **I/O loop overhead (bypass vs cat) is +25–37%.** The double-PTY hop +
   `read(2)` + per-chunk blocking write cost a quarter-to-a-third over cat —
   modest next to the pipeline, but already past the §7 `<20%` line on its
   own. The other ~6–15× of full overhead is **all pipeline** (`pipe cost%` =
   +413% to +1115% on top of bypass).

2. **The pipeline is slow even writing to a `Vec` (no syscall):** the
   in-process micro-bench runs at 12–28 MiB/s vs cat's ~150 MiB/s. So
   **write batching (hypothesis H3: `BufWriter` in `runtime.rs`) is NOT the
   dominant cost** — a `Vec` sink removes all write-syscall cost and the
   pipeline is still ~5–12× slower than cat. H3 could trim the +25–37% I/O
   layer but cannot touch the dominant pipeline cost; de-prioritized vs
   H1/H4 by the data.

Cross-check (consistency): e2e prose full = 968 ms over 16 MiB ≈ 16.5 MiB/s;
micro-bench prose = 23.7 MiB/s (no PTY/write). Same order of magnitude — the
gap is the I/O layer the micro-bench excludes. The two measurements agree.

Within the pipeline (micro-bench shape deltas):
- **prose (23.7 MiB/s)** = per-byte machinery (`AnsiSm::step` per byte +
  `LineBuffer::feed_byte_with_overflow` per byte) + a per-line
  `RegexSet::matches` scan that mostly misses. apply_rules does almost no
  matching work here, so this is largely the **per-byte loop + line-buffer
  floor** (hypotheses H1 `line_buffer.rs` + H4 `pipeline.rs`).
- **log (12.1 MiB/s, slowest)** = prose floor + full `apply_rules` on hits
  (`find_iter` + SGR emit). It is ~2× slower than prose (82 ms vs 42 ms) —
  that delta is the matching + SGR-emit cost on a high-match line.
- **ansi (27.6 MiB/s, fastest)** = per-byte machinery + SGR-sequence routing
  but **`apply_rules` is skipped** (`respect_existing_colors` default → SGR
  lines pass verbatim). ansi is only ~17% faster than prose (36 ms vs 42 ms)
  despite skipping the matcher entirely — so the per-line `RegexSet::matches`
  scan that prose pays is a *modest* cost; the dominant floor in BOTH shapes
  is the **per-byte loop**, which even the fastest shape cannot beat (27.6
  MiB/s ≪ cat's ~150).

**Fundable bottlenecks, ranked by data (for the Phase 2 checkpoint):**

| Rank | Bottleneck | Module | DOKUNULMAZ? | Why (data) |
|---|---|---|---|---|
| 1 | Per-byte `Instant::now()` + O(L²) `memchr` rescan in line buffering (H1) | `line_buffer.rs` | **No** (low risk) | Paid by every shape; ~16M clock reads + quadratic rescans per 16 MiB. Highest ROI / lowest risk. |
| 2 | Per-byte `AnsiSm::step` + single-byte buffer feed → chunk-level (H4) | `pipeline.rs` | **Yes** (security-gate) | The per-byte loop itself; biggest structural win, higher risk. |
| 3 | Per-line `RegexSet::matches` scan + double `Arc::load_full` (H2) | `pipeline.rs` | **Yes** (security-gate) | prose-vs-ansi gap shows the per-line scan costs even on miss. |
| — | Write batching / `BufWriter` (H3) | `runtime.rs` | Yes | **De-prioritized** — the I/O loop is +25–37% (could trim) but the Vec-sink bench shows the pipeline is still ~5–12× slow with zero write cost, so H3 cannot touch the dominant cost. |

This feeds the Phase 2 scope checkpoint — no optimization is chosen here;
this section only attributes the cost.

## v0.8.1 — Phase 2 line_buffer O(1) fast path (recorded 2026-05-30)

First Phase-2 optimization (hypothesis H1, narrowed): `LineBuffer::feed_byte_with_overflow`
delegated to the general `feed_with_overflow`, which `memchr`-rescanned the
whole accumulated buffer on every byte (O(L²) across a line). Replaced with an
O(1) push + single `byte == b'\n'` check, exploiting the invariant that the
buffer never holds an interior newline (a `debug_assert` now guards it). The
per-byte `Instant::now()` clock — a Phase-1 hypothesis — was first measured by a
throwaway spike and found negligible (~1%), so it was left in place. Not a
DOKUNULMAZ change (only `src/line_buffer.rs`); no security gate needed.
Behavior byte-identical (788 lib tests pass, +2 regression tests).

### pipeline_feed micro-bench (in-process, Vec sink) — Phase 1 → Phase 2

| Shape | P1 time | P2 time | P1 thrpt | P2 thrpt | delta (P1→P2) |
|---|---|---|---|---|---|
| prose | 42.13 ms | 21.89 ms | 23.73 MiB/s | 45.67 MiB/s | −48.0% time / +92.5% thrpt |
| log | 82.39 ms | 52.63 ms | 12.14 MiB/s | 19.00 MiB/s | −36.1% time / +56.5% thrpt |
| ansi | 36.17 ms | 17.84 ms | 27.65 MiB/s | 56.05 MiB/s | −50.7% time / +102.7% thrpt |

(All three roughly halve. The per-byte line-buffer overhead this fix removed is
paid by every shape regardless of matching, so even ansi — whose bytes mostly
route through the SGR-sequence path — gains, because each Data byte still went
through the O(L²) buffer. Deltas computed from the recorded P1 medians vs the P2
run; criterion's own `change:` for prose is smaller because the throwaway clock
spike had overwritten prose's stored criterion baseline.)

### e2e end-to-end overhead — Phase 1 → Phase 2 (median ms, ~16 MiB/shape)

Columns: cat / bypass are this (P2) run's medians; tayf and full-ovh% shown for
both phases.

| Shape | cat (P2) | bypass (P2) | tayf P1 | tayf P2 | full ovh% P1 | full ovh% P2 |
|---|---|---|---|---|---|---|
| low-match-prose | 103.40 | 140.16 | 968.11 | 700.64 | +772.11% | +577.58% |
| high-match-log | 103.41 | 136.54 | 1646.17 | 1354.09 | +1419.07% | +1209.39% |
| ansi-passthrough | 116.40 | 156.29 | 856.48 | 598.17 | +601.14% | +413.88% |

(min/med/max ms P2 — prose: cat [96.15/103.40/112.98] bypass
[121.24/140.16/233.03] tayf [687.28/700.64/705.45]; log: cat
[92.14/103.41/114.79] bypass [127.03/136.54/156.71] tayf
[1334.16/1354.09/1404.66]; ansi: cat [107.54/116.40/130.56] bypass
[153.49/156.29/167.26] tayf [590.07/598.17/605.77]. cat/bypass shift
slightly run-to-run; the tayf-side drop is the real signal.)

### Disposition

One non-DOKUNULMAZ change cut end-to-end tayf streaming time by **−28% (prose,
968→701 ms), −18% (log, 1646→1354 ms), −30% (ansi, 856→598 ms)**, lifting tayf
bulk throughput from ~16.5 to ~23 MiB/s on prose. The in-process micro-bench
roughly halved on every shape (prose −48%, log −36%, ansi −51%). Real,
low-risk progress. **§7's `<20%`-vs-cat is still far off** — the remaining cost
is the per-byte `AnsiSm::step` loop + single-byte buffer feed (H4) and, on
matching lines, `apply_rules` (the log shape stays the worst). H4 is the next
lever but lives in DOKUNULMAZ `pipeline.rs` → a future security-gated step. See
the Phase-1 fundable-bottlenecks table above.
