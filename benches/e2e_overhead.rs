//! End-to-end PTY-vs-`cat` overhead measurement harness (spec §7).
//!
//! Drives the release `tayf` binary inside a real PTY and times the
//! streaming phase of `cat <corpus>` against a bare `/bin/sh` running the
//! same command. Reports `overhead% = (tayf - cat) / cat * 100` per corpus
//! shape against the spec §7 `<20%` target, plus throughput in MiB/s.
//!
//! NOT a criterion bench (`harness = false`): criterion's iteration model
//! fits in-process microbenchmarks, not subprocess + PTY spawning. This is
//! a custom `fn main()` measurement tool, run locally with
//! `cargo bench --bench e2e_overhead`. Numbers are recorded by hand in
//! `benches/BASELINE.md`. Timing covers the streaming phase only (stdin
//! write -> EOF); process spawn and the ~200 ms startup grace are excluded
//! symmetrically from both sides (see spec §5.5).
//!
//! Tuning env vars: `TAYF_E2E_SAMPLES` (default 10), `TAYF_E2E_WARMUP`
//! (default 3), `TAYF_E2E_BYTES` (default 16 MiB).

// reason: functions are wired up in main() in the next commit (Task A3)
#![allow(dead_code)]

/// Median of a non-empty slice of sample times (milliseconds). Sorts a copy;
/// inputs are finite positive durations, so `partial_cmp` never sees NaN.
fn median(samples: &[f64]) -> f64 {
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
fn min_sample(samples: &[f64]) -> f64 {
    samples.iter().copied().fold(f64::INFINITY, f64::min)
}

/// Largest sample.
fn max_sample(samples: &[f64]) -> f64 {
    samples.iter().copied().fold(f64::NEG_INFINITY, f64::max)
}

/// Overhead of `tayf` over `cat` as a percentage: `(tayf - cat) / cat * 100`.
fn overhead_pct(tayf: f64, cat: f64) -> f64 {
    (tayf - cat) / cat * 100.0
}

fn main() {
    // Filled in Task A3.
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)] // reason: harness=false bench; test fns use these via super::*
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn median_odd_count() {
        assert!(close(median(&[3.0, 1.0, 2.0]), 2.0));
    }

    #[test]
    fn median_even_count() {
        assert!(close(median(&[1.0, 2.0, 3.0, 4.0]), 2.5));
    }

    #[test]
    fn median_single() {
        assert!(close(median(&[5.0]), 5.0));
    }

    #[test]
    fn median_unsorted_input() {
        assert!(close(median(&[10.0, 2.0, 8.0, 4.0, 6.0]), 6.0));
    }

    #[test]
    fn min_and_max() {
        let s = [3.0, 1.0, 2.0];
        assert!(close(min_sample(&s), 1.0));
        assert!(close(max_sample(&s), 3.0));
    }

    #[test]
    fn overhead_twenty_percent() {
        assert!(close(overhead_pct(120.0, 100.0), 20.0));
    }

    #[test]
    fn overhead_zero_when_equal() {
        assert!(close(overhead_pct(100.0, 100.0), 0.0));
    }

    #[test]
    fn overhead_negative_when_faster() {
        assert!(close(overhead_pct(90.0, 100.0), -10.0));
    }
}
