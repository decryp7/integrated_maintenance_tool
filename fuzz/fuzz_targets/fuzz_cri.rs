#![no_main]
#[path = "../common/mod.rs"]
mod common;
use libfuzzer_sys::fuzz_target;

// Kubernetes CRI container logs: sniff and import never panic.
fuzz_target!(|data: &[u8]| common::text_importer(&imt_plugin_formats::CriImporter, data));
