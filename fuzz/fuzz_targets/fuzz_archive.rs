#![no_main]
#[path = "../common/mod.rs"]
mod common;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = imt_core::archive::expand("fuzz.zip", data.to_vec());
    let _ = imt_core::archive::expand("fuzz.log.gz", data.to_vec());
    let _ = imt_core::archive::read_entry(data.to_vec(), "fuzz.zip", "a.log");
});
