#![no_main]
#[path = "../common/mod.rs"]
mod common;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| common::plugin_text(&debrief_plugin_otel::OtlpImporter, data));
