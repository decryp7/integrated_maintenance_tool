#![no_main]
#[path = "../common/mod.rs"]
mod common;
use libfuzzer_sys::fuzz_target;

// The first byte picks the importer (W3C/IIS, access log, HAR); the rest is the file.
fuzz_target!(|data: &[u8]| {
    let Some((&sel, rest)) = data.split_first() else { return };
    match sel % 3 {
        0 => common::plugin_text(&imt_plugin_web::W3cImporter, rest),
        1 => common::plugin_text(&imt_plugin_web::AccessLogImporter, rest),
        _ => common::plugin_text(&imt_plugin_web::HarImporter, rest),
    }
});
