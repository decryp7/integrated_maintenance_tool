#![no_main]
#[path = "../common/mod.rs"]
mod common;
use libfuzzer_sys::fuzz_target;

// SQL Server ERRORLOG (UTF-8 or UTF-16, decoded like the workspace does): sniff and import never panic.
fuzz_target!(|data: &[u8]| common::text_importer(&imt_plugin_formats::SqlErrorLogImporter, data));
