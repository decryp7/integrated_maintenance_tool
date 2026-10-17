#![no_main]
#[path = "../common/mod.rs"]
mod common;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| common::capture_importer(&imt_plugin_profiling::perf::PerfScriptImporter, data));
