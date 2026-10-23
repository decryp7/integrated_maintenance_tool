#![no_main]
#[path = "../common/mod.rs"]
mod common;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    if let Ok(profile) = debrief_core::Profile::from_toml(text) {
        let _ = debrief_core::profile::CompiledProfile::compile(profile);
    }
});
