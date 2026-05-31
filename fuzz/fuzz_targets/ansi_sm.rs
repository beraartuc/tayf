#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    tayf::__fuzz__::ansi_sm(data);
});
