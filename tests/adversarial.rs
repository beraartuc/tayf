//! Adversarial-input regression tests. Each fuzz crash becomes a case here
//! (the fuzzer is discovery; these are the permanent, nightly-free guard).
use tayf::__bench__::BenchPipeline;

/// A CSI SGR sequence split at every byte boundary must reassemble to the
/// same output as the unsplit feed (chunk-boundary invariance).
#[test]
fn csi_split_across_chunks_is_chunk_invariant() {
    let input = b"\x1b[31mRED\x1b[0m plain\n";
    let mut whole = BenchPipeline::with_builtins();
    let mut whole_out = Vec::new();
    whole.feed(input, &mut whole_out).unwrap();

    for split in 1..input.len() {
        let mut p = BenchPipeline::with_builtins();
        let mut out = Vec::new();
        p.feed(&input[..split], &mut out).unwrap();
        p.feed(&input[split..], &mut out).unwrap();
        assert_eq!(out, whole_out, "split at {split} diverged from whole feed");
    }
}

/// Thousands of short OSC sequences in one line must not panic and must
/// drain (no unbounded growth). Asserts the feed completes and emits bytes.
#[test]
fn osc_flood_does_not_panic() {
    let mut line: Vec<u8> = Vec::new();
    for _ in 0..5000 {
        line.extend_from_slice(b"\x1b]0;t\x07");
    }
    line.push(b'\n');
    let mut p = BenchPipeline::with_builtins();
    let mut out = Vec::new();
    p.feed(&line, &mut out).unwrap();
    assert!(!out.is_empty(), "flood must produce output");
}

/// An OSC string exceeding the 4 KiB sequence cap forces a synthetic 7-bit
/// ST (`\x1b\\`). This is exactly why no-rules passthrough is NOT byte-
/// identical at the Pipeline::feed level (spec §4 A1.3). Pin the behavior.
#[test]
fn oversized_osc_injects_synthetic_st() {
    let mut input: Vec<u8> = b"\x1b]0;".to_vec();
    input.extend(std::iter::repeat_n(b'A', 5000)); // > SEQUENCE_BYTES_CAP (4096)
    input.push(b'\n');
    let mut p = BenchPipeline::with_builtins();
    let mut out = Vec::new();
    p.feed(&input, &mut out).unwrap();
    assert!(
        out.windows(2).any(|w| w == b"\x1b\\"),
        "over-cap OSC must inject a synthetic 7-bit ST"
    );
}
