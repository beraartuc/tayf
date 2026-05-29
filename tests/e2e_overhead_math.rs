//! Unit tests for the e2e overhead harness's pure aggregation math.
//!
//! The math lives in `benches/common/math.rs`, shared with the `e2e_overhead`
//! bench via `#[path]`. A `harness = false` bench cannot run `#[cfg(test)]`
//! unit tests — `cargo test --bench` runs its `main()` and the test block is
//! never compiled in — so the TDD coverage lives here, in a normal
//! integration-test target that `cargo test` (and CI) runs.

#[path = "../benches/common/math.rs"]
mod math;

use math::{max_sample, median, min_sample, overhead_pct};

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
