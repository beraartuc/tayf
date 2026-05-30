//! In-process `Pipeline::feed` micro-bench (spec v0.8.1 §2.2).
//!
//! Drives the pipeline hot path over the three e2e corpus shapes, writing to
//! a reused in-memory `Vec` sink so real PTY/stdout cost is excluded — that
//! cost is attributed separately by the `e2e_overhead` bypass differential.
//! Comparing shapes splits the pipeline's internal cost: `prose` is the
//! AnsiSm + line-buffer + RegexSet-miss floor; `log` adds full apply_rules +
//! SGR emission; `ansi` exercises the pre-colored SGR-detection path.
//!
//! Reuses the corpus templates from `benches/inputs/` (DRY with e2e). A
//! single `BenchPipeline` + `Vec` sink are hoisted out of `b.iter` so the
//! measurement reflects steady-state `feed`, not per-iter allocation. Each
//! template is newline-terminated, so the line buffer fully drains every
//! iteration and pipeline state is deterministic across iterations.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};

use tayf::__bench__::BenchPipeline;

/// Repeat a template to ~the same ~1 MiB scale for all three shapes so the
/// per-iter work is comparable across bench rows.
fn scaled(template: &[u8]) -> Vec<u8> {
    let target = 1024 * 1024;
    let reps = (target / template.len().max(1)).max(1);
    template.repeat(reps)
}

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
// reason: byte count for criterion Throughput fits u64.
fn bench_shape(c: &mut Criterion, name: &str, template: &[u8]) {
    let input = scaled(template);
    let mut group = c.benchmark_group("pipeline_feed");
    group.throughput(Throughput::Bytes(input.len() as u64));
    group.bench_function(name, |b| {
        let mut pipeline = BenchPipeline::with_builtins();
        let mut sink: Vec<u8> = Vec::with_capacity(input.len() * 2);
        b.iter(|| {
            sink.clear();
            pipeline.feed(black_box(input.as_slice()), &mut sink).expect("feed to Vec cannot fail");
            black_box(&sink);
        });
    });
    group.finish();
}

fn bench_prose(c: &mut Criterion) {
    bench_shape(c, "prose", include_bytes!("inputs/e2e_prose.txt"));
}

fn bench_log(c: &mut Criterion) {
    bench_shape(c, "log", include_bytes!("inputs/e2e_log.txt"));
}

fn bench_ansi(c: &mut Criterion) {
    bench_shape(c, "ansi", include_bytes!("inputs/e2e_ansi.txt"));
}

criterion_group!(benches, bench_prose, bench_log, bench_ansi);
criterion_main!(benches);
