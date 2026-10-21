#![no_main]
#[path = "../common/mod.rs"]
mod common;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    common::plugin_binary(&imt_plugin_windows::MinidumpImporter, data);
    let _ = imt_plugin_windows::is_minidump(data);
    if let Ok(imported) = imt_plugin_windows::read_minidump(data) {
        for r in &imported.records {
            serde_json::to_string(&r.fields).expect("fields serialise");
        }
    }
});
