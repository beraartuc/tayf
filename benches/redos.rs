//! Linear-scaling proof: the `regex` crate is linear-time by construction
//! (no backtracking), so this bench shows input 2x => time ~2x for built-in
//! patterns on adversarial-but-valid input — it demonstrates the linear
//! guarantee, it does not hunt for super-linear blowup (impossible).
//!
//! Size/dfa-limit rejection of state-exploding patterns is validated
//! separately by the `load_rejects_pattern_exceeding_size_limit` unit test
//! (src/rules.rs) and the `regex_compile` fuzz target — not here. See spec §4 A3.
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use std::time::Duration;
use tayf::__bench__::{apply_rules, load_builtin_rules, BenchScratch};

fn bench_linear_scaling(c: &mut Criterion) {
    let rules = load_builtin_rules().expect("builtins compile");
    let mut group = c.benchmark_group("redos/linear-scaling");
    for &n in &[1_024usize, 2_048, 4_096, 8_192] {
        // Adversarial-but-valid: a long run that the ipv4/url/timestamp
        // builtins partially engage but never match — worst-case scan.
        let mut line = vec![b'9'; n];
        line.push(b'\n');
        group.throughput(Throughput::Bytes(line.len() as u64));
        group.bench_function(format!("len-{n}"), |b| {
            let mut scratch = BenchScratch::default();
            let mut out: Vec<u8> = Vec::with_capacity(line.len() + 32);
            b.iter(|| {
                out.clear();
                apply_rules(&line, &rules, &mut scratch, &mut out).unwrap();
            });
        });
    }
    group.finish();
}

criterion_group! {
    name = redos;
    config = Criterion::default().measurement_time(Duration::from_secs(5));
    targets = bench_linear_scaling
}
criterion_main!(redos);
