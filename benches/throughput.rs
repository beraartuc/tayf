//! Throughput baseline benchmarks. Measures the hot paths that gate
//! tayf's spec §7 perf target (<20% overhead vs native `cat`):
//!
//! 1. `apply_rules / ipv4-heavy` — the per-line rule scanner against an
//!    IPv4-heavy synthetic input that exercises the IPv4 and `log_level`
//!    builtins simultaneously. None of these builtins
//!    carry capture-group styles, so the line is the "zero captures-styled
//!    rule" hot path: pure span emission, no `emit_capture_runs` work.
//! 2. `apply_rules / mixed-syslog` — realistic syslog-style fixture
//!    blending ISO timestamps (the one captures-styled built-in this
//!    fixture exercises) with syslog-format timestamps, IPs, log levels,
//!    and file paths. This is the bench that exposes the v0.3.5 Rev2 C-1
//!    overlap-vec regression class: if `accepted_spans` / `partition_point`
//!    isn't wired correctly, hot-path rules' linear scans inflate against
//!    captures-emitted runs.
//! 3. `apply_rules / captures-heavy` — every line carries an ISO timestamp,
//!    a POSIX-style permission string, and an HTTPS URL. All three v0.3.5
//!    built-ins with capture-group styles fire, exercising the boundary-
//!    event sweep + active-stack reuse under load.
//! 4. `passthrough` — the no-op write path tayf takes once a TUI mode is
//!    active (alt-screen / bracketed paste / mouse). Modelled as a raw
//!    `Write::write_all` to a `Cursor<Vec<u8>>`. Effectively the spec
//!    target's denominator (`cat`-equivalent).
//! 5. `apply_rules / profile-{aws,k8s,docker,gcp,network}` — v0.5.3 profile-
//!    active hot path. One bench per embedded profile, each operating on a
//!    synthetic domain-typical input (`benches/inputs/*_sample.txt`)
//!    repeated up to the same ~67 KB scale as the four benches above.
//!    Characterizes the per-line cost of the additional `append_rules`
//!    layer + optional built-in whitelist filter on top of the v0.1
//!    builtin scanner.
//!
//! Hot-path internals (`apply_rules`, `Compiled::load_builtins`,
//! `profiles::load_with` + `Compiled::load_with_theme`) live behind
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

use std::hint::black_box;
use std::io::{Cursor, Write};

use criterion::{criterion_group, criterion_main, Criterion, Throughput};

use tayf::__bench__::{apply_rules, load_builtin_rules, load_profile_rules, BenchScratch};

/// Synthetic IPv4-heavy input. Three IPv4 addresses and one log-level token
/// per line — four non-overlapping matches total, repeated to amortize
/// criterion's per-iter overhead. Verified to exercise the IPv4 and
/// `log_level` rules.
fn ipv4_heavy_input() -> Vec<u8> {
    "connect 192.168.1.1 to 10.0.0.1 via 172.16.0.1 status 200 OK ERROR\n".repeat(1000).into_bytes()
}

fn bench_apply_rules_ipv4_heavy(c: &mut Criterion) {
    let compiled = load_builtin_rules().expect("builtin rules compile");
    let input = ipv4_heavy_input();
    let mut scratch = BenchScratch::default();

    let mut group = c.benchmark_group("apply_rules");
    group.throughput(Throughput::Bytes(input.len() as u64));
    group.bench_function("ipv4-heavy", |b| {
        b.iter(|| {
            // Reuse a single buffer across iterations to focus the measurement
            // on the scanner, not the allocator. `Cursor` writes are infallible
            // here, but propagate the result so the optimizer can't elide it.
            let mut out = Cursor::new(Vec::with_capacity(input.len() * 2));
            apply_rules(black_box(input.as_slice()), &compiled, &mut scratch, &mut out)
                .expect("write to in-memory Cursor cannot fail");
            black_box(out);
        });
    });
    group.finish();
}

/// Mixed syslog-style fixture: half the lines have an ISO timestamp
/// (the one captures-styled built-in active here), half are syslog-format
/// with IPs, log levels, and file paths. Exercises the v0.3.5 selective
/// dispatch + match-level `partition_point` overlap check on a realistic
/// input distribution.
fn bench_apply_rules_mixed_syslog(c: &mut Criterion) {
    let compiled = load_builtin_rules().expect("builtin rules compile");
    // Repeat the fixture so per-iter work amortizes criterion's loop overhead
    // to the same ~67 KB scale as the IPv4-heavy bench above.
    let fixture: &[u8] = include_bytes!("../tests/fixtures/mixed_syslog.txt");
    let input = fixture.repeat(20);
    let mut scratch = BenchScratch::default();

    let mut group = c.benchmark_group("apply_rules");
    group.throughput(Throughput::Bytes(input.len() as u64));
    group.bench_function("mixed-syslog", |b| {
        b.iter(|| {
            let mut out = Cursor::new(Vec::with_capacity(input.len() * 2));
            for line in input.split(|&byte| byte == b'\n') {
                if line.is_empty() {
                    continue;
                }
                apply_rules(black_box(line), &compiled, &mut scratch, &mut out)
                    .expect("write to in-memory Cursor cannot fail");
            }
            black_box(out);
        });
    });
    group.finish();
}

/// Captures-heavy fixture: every line fires all three v0.3.5 captures-styled
/// built-ins (timestamp + permission + url). Exercises `emit_capture_runs`
/// boundary-event sweep + active-group stack reuse under load; reusable
/// scratch vectors should keep per-line allocations to a single resize.
fn bench_apply_rules_captures_heavy(c: &mut Criterion) {
    let compiled = load_builtin_rules().expect("builtin rules compile");
    let fixture: &[u8] = include_bytes!("../tests/fixtures/captures_heavy.txt");
    let input = fixture.repeat(20);
    let mut scratch = BenchScratch::default();

    let mut group = c.benchmark_group("apply_rules");
    group.throughput(Throughput::Bytes(input.len() as u64));
    group.bench_function("captures-heavy", |b| {
        b.iter(|| {
            let mut out = Cursor::new(Vec::with_capacity(input.len() * 2));
            for line in input.split(|&byte| byte == b'\n') {
                if line.is_empty() {
                    continue;
                }
                apply_rules(black_box(line), &compiled, &mut scratch, &mut out)
                    .expect("write to in-memory Cursor cannot fail");
            }
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

// ---------------------------------------------------------------------------
// v0.5.3 — profile-active benches. Characterize the with-profile hot path
// against synthetic domain-typical input for each embedded profile (aws,
// k8s, docker, gcp, network). The four benches above (ipv4-heavy /
// mixed-syslog / captures-heavy / passthrough) continue to characterize
// the profile-INACTIVE default path; their baselines remain unchanged.
// ---------------------------------------------------------------------------

/// Shared helper. Mirrors `bench_apply_rules_mixed_syslog`'s shape (per-line
/// iteration with a hoisted `BenchScratch`) so the profile-active numbers
/// are directly comparable to the existing mixed-syslog baseline.
fn run_profile_bench(c: &mut Criterion, name: &str, profile: &str, fixture: &[u8]) {
    let compiled = load_profile_rules(profile).expect("embedded profile must load and compile");
    // Same ~67 KB scale as the existing apply_rules benches — a small fixture
    // (~10-15 lines) repeated until criterion's per-iter overhead amortizes.
    let input = fixture.repeat(80);
    let mut scratch = BenchScratch::default();

    let mut group = c.benchmark_group("apply_rules");
    group.throughput(Throughput::Bytes(input.len() as u64));
    group.bench_function(name, |b| {
        b.iter(|| {
            let mut out = Cursor::new(Vec::with_capacity(input.len() * 2));
            for line in input.split(|&byte| byte == b'\n') {
                if line.is_empty() {
                    continue;
                }
                apply_rules(black_box(line), &compiled, &mut scratch, &mut out)
                    .expect("write to in-memory Cursor cannot fail");
            }
            black_box(out);
        });
    });
    group.finish();
}

fn bench_profile_aws(c: &mut Criterion) {
    let fixture: &[u8] = include_bytes!("inputs/aws_sample.txt");
    run_profile_bench(c, "profile-aws", "aws", fixture);
}

fn bench_profile_k8s(c: &mut Criterion) {
    let fixture: &[u8] = include_bytes!("inputs/k8s_sample.txt");
    run_profile_bench(c, "profile-k8s", "k8s", fixture);
}

fn bench_profile_docker(c: &mut Criterion) {
    let fixture: &[u8] = include_bytes!("inputs/docker_sample.txt");
    run_profile_bench(c, "profile-docker", "docker", fixture);
}

fn bench_profile_gcp(c: &mut Criterion) {
    let fixture: &[u8] = include_bytes!("inputs/gcp_sample.txt");
    run_profile_bench(c, "profile-gcp", "gcp", fixture);
}

fn bench_profile_network(c: &mut Criterion) {
    let fixture: &[u8] = include_bytes!("inputs/network_sample.txt");
    run_profile_bench(c, "profile-network", "network", fixture);
}

criterion_group!(
    benches,
    bench_apply_rules_ipv4_heavy,
    bench_apply_rules_mixed_syslog,
    bench_apply_rules_captures_heavy,
    bench_passthrough,
    bench_profile_aws,
    bench_profile_k8s,
    bench_profile_docker,
    bench_profile_gcp,
    bench_profile_network,
);
criterion_main!(benches);
