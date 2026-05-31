#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|pattern: &str| {
    tayf::__fuzz__::regex_compile(pattern);
});
