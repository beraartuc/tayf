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
Host: Apple M2 Pro, macOS (Darwin 24.6.0, arm64), Bera-MacBook-Pro.local

Criterion output excerpt:

```
apply_rules/ipv4-heavy  time:   [7.6697 ms 7.6786 ms 7.6881 ms]
                        thrpt:  [8.3110 MiB/s 8.3213 MiB/s 8.3309 MiB/s]

passthrough/write_all   time:   [1.1473 µs 1.1492 µs 1.1515 µs]
                        thrpt:  [54.190 GiB/s 54.299 GiB/s 54.387 GiB/s]
```

These numbers anchor the v0.3.0 < 20% regression budget per spec §7.4.
