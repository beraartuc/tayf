#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Two checks: (1) empty-rules apply_rules byte-identity oracle, per line;
    // (2) full builtins feed as a crash-finder over arbitrary chunking.
    for line in data.split_inclusive(|&b| b == b'\n') {
        tayf::__fuzz__::pipeline_apply_rules_identity(line);
    }
    tayf::__fuzz__::pipeline_feed_builtins(data);
});
