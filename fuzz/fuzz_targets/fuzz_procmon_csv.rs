#![no_main]
#[path = "../common/mod.rs"]
mod common;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The shared context carries a file date, so time-of-day rows exercise the date rollover.
    common::plugin_text(&imt_plugin_windows::ProcmonCsvImporter, data);
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = imt_plugin_windows::procmon::result_level(s);
    }
});
