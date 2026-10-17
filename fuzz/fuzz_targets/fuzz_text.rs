#![no_main]
#[path = "../common/mod.rs"]
mod common;
use libfuzzer_sys::fuzz_target;

// Uses the default profile's text formats.
fuzz_target!(|data: &[u8]| common::text_importer(&imt_core::importers::TextImporter, data));
