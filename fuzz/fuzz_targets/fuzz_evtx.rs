#![no_main]
#[path = "../common/mod.rs"]
mod common;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    common::plugin_binary(&imt_plugin_windows::EvtxImporter, data);
    let _ = imt_plugin_windows::is_evtx(data);
    let _ = imt_plugin_windows::read_evtx(data);
});
