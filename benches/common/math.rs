//! Pure aggregation math for the e2e overhead harness, shared between the
//! `e2e_overhead` bench (which uses it in `main`) and the `e2e_overhead_math`
//! integration test (which unit-tests it). Lives in a `benches/`
//! subdirectory so cargo does not auto-discover it as its own bench target;
//! both crates pull it in via `#[path]`.
//!
//! A `harness = false` bench cannot run `#[cfg(test)]` unit tests
//! (`cargo test --bench` runs its `main()`, and the test block is never
//! compiled in), so these functions are exercised by `tests/e2e_overhead_math.rs`,
//! a normal integration-test target that `cargo test` and CI run.

/// Median of a non-empty slice of sample times (milliseconds). Sorts a copy;
/// inputs are finite positive durations, so `partial_cmp` never sees NaN.
pub(crate) fn median(samples: &[f64]) -> f64 {
    assert!(!samples.is_empty(), "median requires a non-empty slice of samples");
    let mut v = samples.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).expect("sample times are finite"));
    let n = v.len();
    let mid = n / 2;
    if n % 2 == 1 {
        v[mid]
    } else {
        (v[mid - 1] + v[mid]) / 2.0
    }
}

/// Smallest sample (the least-perturbed run).
pub(crate) fn min_sample(samples: &[f64]) -> f64 {
    samples.iter().copied().fold(f64::INFINITY, f64::min)
}

/// Largest sample.
pub(crate) fn max_sample(samples: &[f64]) -> f64 {
    samples.iter().copied().fold(f64::NEG_INFINITY, f64::max)
}

/// Overhead of `tayf` over `cat` as a percentage: `(tayf - cat) / cat * 100`.
pub(crate) fn overhead_pct(tayf: f64, cat: f64) -> f64 {
    (tayf - cat) / cat * 100.0
}
