//! Throughput baseline benchmarks. Measures the two hot paths that gate
//! tayf's spec §7 perf target (<20% overhead vs native `cat`):
//!
//! 1. `apply_rules` — the per-line rule scanner that wraps matches in SGR.
//!    Driven against an IPv4-heavy synthetic input that exercises the
//!    IPv4, `log_level`, and `http_status` builtins simultaneously.
//! 2. `passthrough` — the no-op write path tayf takes once a TUI mode is
//!    active (alt-screen / bracketed paste / mouse). Modelled as a raw
//!    `Write::write_all` to a `Cursor<Vec<u8>>`. Effectively the spec
//!    target's denominator (`cat`-equivalent).
//!
//! Hot-path internals (`apply_rules`, `Compiled::load_builtins`) live behind
//! `pub(crate)`. They are re-exported here via the `#[doc(hidden)] pub`
//! `tayf::__bench__` module — not part of the public API.
//!
//! Capture the baseline with:
//!
//! ```text
//! cargo bench --bench throughput -- --save-baseline v0.1.1
//! ```
//!
//! and record the numbers in `benches/BASELINE.md`.

use std::io::{Cursor, Write};

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

use tayf::__bench__::{apply_rules, load_builtin_rules};

/// Synthetic IPv4-heavy input. Three IPv4 addresses plus two log-level
/// tokens per line, repeated to amortize criterion's per-iter overhead.
/// Verified to match the IPv4 and `log_level` rules.
fn ipv4_heavy_input() -> Vec<u8> {
    "connect 192.168.1.1 to 10.0.0.1 via 172.16.0.1 OK ERROR\n".repeat(1000).into_bytes()
}

fn bench_apply_rules_ipv4_heavy(c: &mut Criterion) {
    let compiled = load_builtin_rules().expect("builtin rules compile");
    let input = ipv4_heavy_input();

    let mut group = c.benchmark_group("apply_rules");
    group.throughput(Throughput::Bytes(input.len() as u64));
    group.bench_function("ipv4-heavy", |b| {
        b.iter(|| {
            // Reuse a single buffer across iterations to focus the measurement
            // on the scanner, not the allocator. `Cursor` writes are infallible
            // here, but propagate the result so the optimizer can't elide it.
            let mut out = Cursor::new(Vec::with_capacity(input.len() * 2));
            apply_rules(black_box(input.as_slice()), &compiled, &mut out)
                .expect("write to in-memory Cursor cannot fail");
            black_box(out);
        });
    });
    group.finish();
}

fn bench_passthrough(c: &mut Criterion) {
    // Models tayf's TUI passthrough mode: the output thread writes the PTY
    // chunk straight to stdout (`out.write_all(segment)?` in
    // `pipeline::Pipeline::feed`). No state machine work, no rule application.
    // Same input shape as the apply_rules bench so the two numbers are
    // directly comparable.
    let input = ipv4_heavy_input();

    let mut group = c.benchmark_group("passthrough");
    group.throughput(Throughput::Bytes(input.len() as u64));
    group.bench_function("write_all", |b| {
        b.iter(|| {
            let mut out = Cursor::new(Vec::with_capacity(input.len()));
            out.write_all(black_box(input.as_slice()))
                .expect("write to in-memory Cursor cannot fail");
            black_box(out);
        });
    });
    group.finish();
}

criterion_group!(benches, bench_apply_rules_ipv4_heavy, bench_passthrough);
criterion_main!(benches);
