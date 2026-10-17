#![no_main]
#[path = "../common/mod.rs"]
mod common;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| common::text_importer(&imt_plugin_otel::OtlpImporter, data));
