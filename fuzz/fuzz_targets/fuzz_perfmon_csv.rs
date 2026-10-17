#![no_main]
#[path = "../common/mod.rs"]
mod common;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    common::text_importer(&imt_plugin_windows::PerfmonCsvImporter, data);
    common::text_importer(&imt_plugin_windows::NfoImporter, data);
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = imt_plugin_windows::perfmon::parse_counter_path(s);
    }
});
